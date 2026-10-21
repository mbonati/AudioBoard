//! The audio-thread side of the graph.
//!
//! The UI owns the "truth" (nodes, cables, positions) and sends the engine
//! commands: new processors, removals and a pre-sorted topology. The engine
//! never sorts or allocates per block; it walks `Topology::order`, copies each
//! node's inputs from upstream output buffers and runs the processor.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Instant;

use cpal::{FromSample, Sample};

use super::params::{EngineShared, TEMPO_RANGE};

pub type NodeId = u32;

/// Frames processed per internal block. Device buffers are split into these.
pub const BLOCK: usize = 64;
pub const MAX_INPUTS: usize = 8;

pub struct ProcessCtx {
    pub sample_rate: f32,
    /// One-pole coefficient for ~10 ms parameter smoothing.
    pub smooth: f32,
    /// Transport position at the start of the block, in beats. Advances only
    /// while audio runs, so every rhythmic node shares one clock.
    pub beat: f64,
    /// Beats per sample at the current tempo.
    pub beats_per_sample: f64,
    /// The transport is playing. Stopped, sources fall silent and rhythm
    /// generators end their notes, while effects keep processing their
    /// tails; the beat stands still.
    pub playing: bool,
    /// The transport was just rewound to beat 0 (Stop, then Start): rhythm
    /// generators start over and end the notes they were holding.
    pub restart: bool,
}

/// Events a MIDI cable can hold per block (more are dropped).
pub const MIDI_CAP: usize = 128;

/// A raw MIDI message at a frame within the block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MidiMsg {
    pub frame: u16,
    pub data: [u8; 3],
}

/// One block of MIDI, fixed size so the audio thread never allocates.
#[derive(Clone)]
pub struct MidiBuf {
    events: [MidiMsg; MIDI_CAP],
    len: usize,
}

impl Default for MidiBuf {
    fn default() -> Self {
        Self { events: [MidiMsg::default(); MIDI_CAP], len: 0 }
    }
}

impl MidiBuf {
    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn push(&mut self, frame: usize, data: [u8; 3]) {
        if self.len < MIDI_CAP {
            self.events[self.len] = MidiMsg { frame: frame as u16, data };
            self.len += 1;
        }
    }

    pub fn events(&self) -> &[MidiMsg] {
        &self.events[..self.len]
    }

    fn copy_from(&mut self, other: &MidiBuf) {
        self.len = other.len;
        self.events[..other.len].copy_from_slice(other.events());
    }
}

/// One block of stereo audio, `[left, right]`. Every cable carries stereo;
/// mono sources write the same signal to both channels.
pub type Buf = [[f32; BLOCK]; 2];

pub struct Inputs {
    pub bufs: [Buf; MAX_INPUTS],
    pub connected: [bool; MAX_INPUTS],
    /// MIDI arriving on the node's MIDI input this block.
    pub midi: MidiBuf,
    /// A cable is plugged into the MIDI input.
    pub midi_connected: bool,
}

impl Inputs {
    pub fn get(&self, i: usize) -> Option<&Buf> {
        self.connected[i].then(|| &self.bufs[i])
    }

    /// Left channel only: the mono view used by control (CV) inputs.
    pub fn cv(&self, i: usize) -> Option<&[f32; BLOCK]> {
        self.get(i).map(|b| &b[0])
    }
}

pub trait Processor: Send {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize);

    /// Sinks feed the audio device: their first output is summed into the mix.
    fn is_sink(&self) -> bool {
        false
    }

    /// MIDI this node produced in its last `process` (nodes with a MIDI output).
    fn midi_out(&self) -> Option<&MidiBuf> {
        None
    }
}

/// A processor plus its output buffers, allocated on the UI thread.
pub struct Slot {
    proc: Box<dyn Processor>,
    outputs: Vec<Buf>,
}

impl Slot {
    pub fn new(proc: Box<dyn Processor>, outputs: usize) -> Box<Self> {
        Box::new(Self {
            proc,
            outputs: vec![[[0.0; BLOCK]; 2]; outputs.max(1)],
        })
    }
}

pub struct TopoEntry {
    pub id: NodeId,
    /// For each input port: (source node, source output port).
    pub inputs: [Option<(NodeId, usize)>; MAX_INPUTS],
    /// Node feeding the MIDI input.
    pub midi_input: Option<NodeId>,
}

/// Nodes in processing order (every node comes after its sources).
#[derive(Default)]
pub struct Topology {
    pub order: Vec<TopoEntry>,
}

pub enum Command {
    AddNode(NodeId, Box<Slot>),
    RemoveNode(NodeId),
    SetTopology(Box<Topology>),
}

/// What the engine lets go of (removed nodes, old topologies), sent back to
/// be freed on the UI thread: freeing memory (a delay line is megabytes)
/// can stall the audio thread long enough to click.
pub struct Garbage {
    _slot: Option<Box<Slot>>,
    _topology: Option<Box<Topology>>,
}

/// Room in the garbage channel; when full, the engine frees in place.
const GARBAGE_CAP: usize = 1024;

/// Hasher for node ids: they are small distinct integers, so a multiply
/// spreads them well enough, far cheaper than SipHash on every lookup.
#[derive(Default)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 << 8 | b as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u32(&mut self, n: u32) {
        self.0 = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

type IdMap<V> = HashMap<NodeId, V, BuildHasherDefault<IdHasher>>;

pub struct Engine {
    rx: Receiver<Command>,
    trash: SyncSender<Garbage>,
    trash_rx: Option<Receiver<Garbage>>,
    shared: Arc<EngineShared>,
    /// `EngineShared::rewinds` as last seen.
    rewinds: u32,
    ctx: ProcessCtx,
    nodes: IdMap<Box<Slot>>,
    topo: Box<Topology>,
    inputs: Box<Inputs>,
    mix: [[f32; BLOCK]; 2],
    limiter: Limiter,
}

/// Keeps the output under full scale. A sum of loud plugins would otherwise
/// be cut off hard at ±1, a harsh crackle. The output is delayed by
/// [`LOOKAHEAD`] frames so the level can be turned down smoothly *before* a
/// peak arrives: the gain each frame needs is tracked over the lookahead
/// window, averaged over it (a ramp down that is still low enough at the
/// peak), and recovers over about 150 ms. Below the ceiling it does nothing.
struct Limiter {
    /// The delayed frames, and the gain each one needs.
    delay: [[f32; 2]; LOOKAHEAD],
    need: [f32; LOOKAHEAD],
    /// Recent window minima, averaged into the ramp.
    mins: [f32; LOOKAHEAD],
    sum: f32,
    pos: usize,
    gain: f32,
    release: f32,
    /// Frames in the window that need turning down, and window minima
    /// below 1 still in the average: while both are zero (almost always)
    /// the limiter only delays.
    pending: usize,
    ramping: usize,
}

/// Limiter lookahead: 1.3 ms at 48 kHz.
const LOOKAHEAD: usize = 64;

impl Limiter {
    /// −0.3 dBFS.
    const CEILING: f32 = 0.966;

    fn new(sample_rate: f32) -> Self {
        Self {
            delay: [[0.0; 2]; LOOKAHEAD],
            need: [1.0; LOOKAHEAD],
            mins: [1.0; LOOKAHEAD],
            sum: LOOKAHEAD as f32,
            pos: 0,
            gain: 1.0,
            release: 1.0 - (-1.0 / (0.15 * sample_rate)).exp(),
            pending: 0,
            ramping: 0,
        }
    }

    /// Takes a frame in, gives the frame from [`LOOKAHEAD`] frames ago out,
    /// limited, with the gain applied.
    #[inline]
    fn frame(&mut self, l: f32, r: f32) -> (f32, f32, f32) {
        let peak = l.abs().max(r.abs());
        let need = if peak > Self::CEILING { Self::CEILING / peak } else { 1.0 };
        let [out_l, out_r] = self.delay[self.pos];
        self.delay[self.pos] = [l, r];
        // Nothing to limit, nothing ramping: just the delay.
        if need >= 1.0 && self.pending == 0 && self.ramping == 0 && self.gain >= 1.0 {
            self.pos = (self.pos + 1) % LOOKAHEAD;
            return (out_l, out_r, 1.0);
        }
        // The lowest gain any frame between the one leaving and this one needs.
        let window = self.need.iter().fold(need, |m, &g| m.min(g));
        self.pending = self.pending + usize::from(need < 1.0) - usize::from(self.need[self.pos] < 1.0);
        self.ramping = self.ramping + usize::from(window < 1.0) - usize::from(self.mins[self.pos] < 1.0);
        self.need[self.pos] = need;
        // Averaged over the window: every value averaged is at most the
        // outgoing frame's need, so the ramp is low enough when it leaves.
        self.sum += window - self.mins[self.pos];
        self.mins[self.pos] = window;
        self.pos = (self.pos + 1) % LOOKAHEAD;
        if self.pos == 0 {
            // Re-sum now and then: no drift from rounding.
            self.sum = self.mins.iter().sum();
        }
        let ramp = (self.sum / LOOKAHEAD as f32).min(1.0);
        self.gain = if ramp < self.gain { ramp } else { self.gain + (ramp - self.gain) * self.release };
        (out_l * self.gain, out_r * self.gain, self.gain)
    }
}

impl Engine {
    pub fn new(rx: Receiver<Command>, shared: Arc<EngineShared>, sample_rate: f32) -> Self {
        let (trash, trash_rx) = sync_channel(GARBAGE_CAP);
        Self {
            rx,
            trash,
            trash_rx: Some(trash_rx),
            rewinds: shared.rewinds.load(Ordering::Relaxed),
            shared,
            ctx: ProcessCtx {
                sample_rate,
                smooth: 1.0 - (-1.0 / (sample_rate * 0.010)).exp(),
                beat: 0.0,
                beats_per_sample: 0.0,
                playing: true,
                restart: false,
            },
            nodes: IdMap::with_capacity_and_hasher(256, Default::default()),
            topo: Box::default(),
            inputs: Box::new(Inputs {
                bufs: [[[0.0; BLOCK]; 2]; MAX_INPUTS],
                connected: [false; MAX_INPUTS],
                midi: MidiBuf::default(),
                midi_connected: false,
            }),
            mix: [[0.0; BLOCK]; 2],
            limiter: Limiter::new(sample_rate),
        }
    }

    /// Where what the engine lets go of is freed: the UI thread drains it.
    /// Taken once.
    pub fn take_trash(&mut self) -> Option<Receiver<Garbage>> {
        self.trash_rx.take()
    }

    fn discard(&self, slot: Option<Box<Slot>>, topology: Option<Box<Topology>>) {
        // A bounded channel's sending side never allocates; if it is full
        // (nobody draining), the value is simply freed here.
        let _ = self.trash.try_send(Garbage { _slot: slot, _topology: topology });
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                Command::AddNode(id, slot) => {
                    let old = self.nodes.insert(id, slot);
                    if old.is_some() {
                        self.discard(old, None);
                    }
                }
                Command::RemoveNode(id) => {
                    let old = self.nodes.remove(&id);
                    self.discard(old, None);
                }
                Command::SetTopology(t) => {
                    let old = std::mem::replace(&mut self.topo, t);
                    self.discard(None, Some(old));
                }
            }
        }
    }

    fn process_block(&mut self, n: usize) {
        let Engine {
            ctx,
            nodes,
            topo,
            inputs,
            mix,
            ..
        } = self;

        for ch in mix.iter_mut() {
            ch[..n].fill(0.0);
        }
        let (slowest, fastest) = TEMPO_RANGE;
        ctx.beats_per_sample = self.shared.tempo.get().clamp(slowest, fastest) as f64 / (60.0 * ctx.sample_rate as f64);
        ctx.playing = self.shared.running.load(Ordering::Relaxed);

        for entry in &topo.order {
            // Copy connected inputs; unconnected ones are only flagged, since
            // processors read inputs through `connected`.
            for (k, src) in entry.inputs.iter().enumerate() {
                let upstream = src.and_then(|(sid, port)| nodes.get(&sid).and_then(|s| s.outputs.get(port)));
                inputs.connected[k] = upstream.is_some();
                if let Some(buf) = upstream {
                    for (d, s) in inputs.bufs[k].iter_mut().zip(buf) {
                        d[..n].copy_from_slice(&s[..n]);
                    }
                }
            }

            let midi_source = entry.midi_input.and_then(|sid| nodes.get(&sid)).and_then(|s| s.proc.midi_out());
            match midi_source {
                Some(events) => inputs.midi.copy_from(events),
                None => inputs.midi.clear(),
            }
            inputs.midi_connected = entry.midi_input.is_some();

            let Some(slot) = nodes.get_mut(&entry.id) else {
                continue;
            };
            slot.proc.process(ctx, inputs, &mut slot.outputs, n);

            if slot.proc.is_sink() {
                for (ch, out) in mix.iter_mut().zip(&slot.outputs[0]) {
                    for (m, s) in ch[..n].iter_mut().zip(&out[..n]) {
                        *m += *s;
                    }
                }
            }
        }
        if ctx.playing {
            ctx.beat += ctx.beats_per_sample * n as f64;
        }
        ctx.restart = false;
    }

    /// Fills an interleaved device buffer.
    pub fn render<T: Sample + FromSample<f32>>(&mut self, data: &mut [T], channels: usize) {
        let _ftz = FlushDenormals::new();
        clock::begin();
        let started = Instant::now();
        self.drain_commands();
        let rewinds = self.shared.rewinds.load(Ordering::Relaxed);
        if rewinds != self.rewinds {
            self.rewinds = rewinds;
            self.ctx.beat = 0.0;
            self.ctx.restart = true;
        }

        let channels = channels.max(1);
        let frames = data.len() / channels;
        let mut mono = [0.0f32; BLOCK];
        let mut done = 0;

        // Always processed, even when stopped: tails ring out.
        while done < frames {
            let n = (frames - done).min(BLOCK);
            self.process_block(n);

            let mut lowest = 1.0f32;
            for i in 0..n {
                let (l, r, gain) = self.limiter.frame(self.mix[0][i], self.mix[1][i]);
                lowest = lowest.min(gain);
                let (l, r) = (l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0));
                mono[i] = 0.5 * (l + r);
                let frame = &mut data[(done + i) * channels..(done + i + 1) * channels];
                if channels == 1 {
                    frame[0] = T::from_sample(mono[i]);
                } else {
                    frame[0] = T::from_sample(l);
                    frame[1] = T::from_sample(r);
                    for s in &mut frame[2..] {
                        *s = T::EQUILIBRIUM;
                    }
                }
            }
            self.shared.scope.write(&mono[..n]);
            if lowest < 0.9999 {
                self.shared.limiting.fetch_max(-20.0 * lowest.log10());
            }
            done += n;
        }
        self.shared.beat.set((self.ctx.beat % 1024.0) as f32);

        if frames > 0 {
            let available = frames as f32 / self.ctx.sample_rate;
            let load = started.elapsed().as_secs_f32() / available;
            let prev = self.shared.load.get();
            self.shared.load.set(prev + (load - prev) * 0.05);
            self.shared.peak_load.fetch_max(load);
            if load > 1.0 {
                self.shared.overloads.fetch_add(1, Ordering::Relaxed);
            }
            self.shared.frames.store(frames as u32, Ordering::Relaxed);
            clock::end(std::time::Duration::from_secs_f32(available));
        }
    }
}

/// When the audio thread works, so the UI can call into plugins, which
/// share a lock with it, while it is idle: right after a callback, with
/// most of a buffer's time before the next one. One audio output per app.
mod clock {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
    use std::time::{Duration, Instant};

    static EPOCH: OnceLock<Instant> = OnceLock::new();
    static BUSY: AtomicBool = AtomicBool::new(false);
    /// When the last callback ended, and the callbacks' period (ns).
    static END: AtomicU64 = AtomicU64::new(0);
    static PERIOD: AtomicU64 = AtomicU64::new(0);

    fn now() -> u64 {
        EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
    }

    pub(super) fn begin() {
        BUSY.store(true, Relaxed);
    }

    pub(super) fn end(period: Duration) {
        END.store(now(), Relaxed);
        PERIOD.store(period.as_nanos() as u64, Relaxed);
        BUSY.store(false, Relaxed);
    }

    /// The audio thread is between callbacks, early enough that a short call
    /// into a plugin ends before it needs the plugin again (or there is no
    /// audio running at all).
    pub fn audio_idle() -> bool {
        let period = PERIOD.load(Relaxed);
        if period == 0 {
            return true;
        }
        let since = now().saturating_sub(END.load(Relaxed));
        if since > 4 * period {
            // No callbacks lately: the device is gone or stopped.
            return true;
        }
        !BUSY.load(Relaxed) && since < period / 2
    }

    /// Waits (briefly) for [`audio_idle`]; false if it did not come.
    pub fn wait_audio_idle(limit: Duration) -> bool {
        let start = Instant::now();
        while !audio_idle() {
            if start.elapsed() > limit {
                return false;
            }
            std::thread::sleep(Duration::from_micros(250));
        }
        true
    }
}

pub use clock::{audio_idle, wait_audio_idle};

/// Flushes denormal floats to zero while alive. Decaying tails (reverb,
/// resonators, filters) otherwise end in denormals, which are many times
/// slower to compute on x86 and cause CPU spikes as sounds fade out.
struct FlushDenormals {
    #[cfg(target_arch = "x86_64")]
    saved: u32,
    #[cfg(target_arch = "aarch64")]
    saved: u64,
}

impl FlushDenormals {
    #[cfg(target_arch = "x86_64")]
    #[allow(deprecated)]
    fn new() -> Self {
        use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        // SAFETY: only the flush-to-zero (bit 15) and denormals-are-zero
        // (bit 6) modes change, and the previous state is restored on drop.
        let saved = unsafe { _mm_getcsr() };
        unsafe { _mm_setcsr(saved | 0x8040) };
        Self { saved }
    }

    #[cfg(target_arch = "aarch64")]
    fn new() -> Self {
        let saved: u64;
        // SAFETY: sets only the FZ bit (24) of FPCR, restored on drop.
        unsafe {
            std::arch::asm!("mrs {}, fpcr", out(reg) saved);
            std::arch::asm!("msr fpcr, {}", in(reg) saved | 1 << 24);
        }
        Self { saved }
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn new() -> Self {
        Self {}
    }
}

impl Drop for FlushDenormals {
    #[cfg(target_arch = "x86_64")]
    #[allow(deprecated)]
    fn drop(&mut self) {
        // SAFETY: restores the state read in `new`.
        unsafe { std::arch::x86_64::_mm_setcsr(self.saved) };
    }

    #[cfg(target_arch = "aarch64")]
    fn drop(&mut self) {
        // SAFETY: restores the state read in `new`.
        unsafe { std::arch::asm!("msr fpcr, {}", in(reg) self.saved) };
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn drop(&mut self) {}
}
