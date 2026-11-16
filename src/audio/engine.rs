//! The audio-thread side of the graph.
//!
//! The UI owns the "truth" (nodes, cables, positions) and sends the engine
//! commands: new processors, removals and a pre-sorted topology. The engine
//! never sorts or allocates while rendering. Each device buffer is one
//! *pass* (up to [`PASS`] frames): the engine walks `Topology::order` and
//! runs each node over the whole pass before the next — the graph has no
//! cycles, so this is exactly the same as going block by block, but a node's
//! state stays in cache, and plugins get the pass in one call (some cost a
//! lot per call, whatever its length). Built-in nodes still run in blocks of
//! [`BLOCK`] frames inside the pass.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Instant;

use cpal::{FromSample, Sample};

use super::params::{EngineShared, TEMPO_RANGE};
use super::sync::{self, Follow, Timeline};

pub type NodeId = u32;

/// Blocks in the longest pass (device buffers longer than a pass are split).
pub const PASS_BLOCKS: usize = 32;
/// Longest pass, in frames: also the largest block plugins are given.
pub const PASS: usize = BLOCK * PASS_BLOCKS;
pub const MAX_INPUTS: usize = 24;

// Blocks, MIDI and the transport come from the node SDK: built-in nodes and
// plugin nodes see the same types.
pub use audioboard_node::buffer::{BLOCK, Buf, MIDI_CAP, MidiBuf, MidiMsg, ProcessCtx};

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

/// What a node that takes a whole pass at once receives (see
/// [`Processor::process_pass`]).
pub struct PassInputs<'a> {
    /// Input 0, block by block (None: unplugged).
    pub audio: Option<&'a [Buf]>,
    /// MIDI on the MIDI input, block by block (frames within each block).
    pub midi: &'a [MidiBuf],
    pub midi_connected: bool,
}

pub trait Processor: Send {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize);

    /// Takes a whole pass in one go ([`Self::process_pass`]) rather than
    /// block by block.
    fn wants_pass(&self) -> bool {
        false
    }

    /// The whole pass (`n` frames, from `ctx.beat`) at once; `outputs` is
    /// block by block, `[block][port]`. For nodes that say [`Self::wants_pass`].
    fn process_pass(&mut self, _ctx: &ProcessCtx, _inputs: &PassInputs, _outputs: &mut [Box<[Buf]>], _n: usize) {}

    /// MIDI this node produced in its last `process` (nodes with a MIDI output).
    fn midi_out(&self) -> Option<&MidiBuf> {
        None
    }
}

/// A processor plus its output buffers for a whole pass, allocated on the
/// UI thread.
pub struct Slot {
    proc: Box<dyn Processor>,
    /// Audio out, `[block][port]`.
    outputs: Box<[Box<[Buf]>]>,
    /// MIDI out of each block (nodes with a MIDI output only).
    midi: Box<[MidiBuf]>,
    /// Bypassed: input 0 goes straight to output 0. The node keeps running
    /// underneath, so switching back is seamless.
    bypass: Arc<AtomicBool>,
    /// How far output 0 has crossfaded to the input (0: the node's own).
    dry: f32,
    /// Frames since the node stopped being heard (see [`TopoEntry::heard`]).
    unheard: usize,
    /// The outputs' gain while fading back in after a rest unheard (1: none).
    fade: f32,
}

/// Bypass crossfades take this long.
const BYPASS_SECONDS: f32 = 0.015;

/// A node nobody hears keeps running this long (a cable dragged to another
/// input comes back seamlessly), then rests until it is heard again.
const UNHEARD_SECONDS: f32 = 2.0;

/// Coming back from a rest unheard, a node's outputs fade in over this long:
/// its state is as it was when it stopped, so it may not join on smoothly.
const RESUME_SECONDS: f32 = 0.02;

impl Slot {
    pub fn new(proc: Box<dyn Processor>, outputs: usize) -> Box<Self> {
        let blocks = (0..PASS_BLOCKS).map(|_| vec![[[0.0; BLOCK]; 2]; outputs.max(1)].into_boxed_slice());
        let midi = if proc.midi_out().is_some() { PASS_BLOCKS } else { 0 };
        Box::new(Self {
            proc,
            outputs: blocks.collect(),
            midi: vec![MidiBuf::default(); midi].into_boxed_slice(),
            bypass: Arc::new(AtomicBool::new(false)),
            dry: 0.0,
            unheard: 0,
            fade: 1.0,
        })
    }

    /// Fades block `k`'s outputs in, after a rest unheard.
    fn fade_in(&mut self, k: usize, n: usize, step: f32) {
        if self.fade >= 1.0 {
            return;
        }
        let start = self.fade;
        for port in self.outputs[k].iter_mut() {
            let mut g = start;
            for i in 0..n {
                g = (g + step).min(1.0);
                port[0][i] *= g;
                port[1][i] *= g;
            }
        }
        self.fade = (start + step * n as f32).min(1.0);
    }

    /// The switch the UI flips to bypass this node.
    pub fn bypass(&self) -> Arc<AtomicBool> {
        self.bypass.clone()
    }

    /// Bypass: crossfades block `k`'s output 0 to `input` (silence when
    /// unpatched) and back.
    fn blend_bypass(&mut self, k: usize, input: Option<&Buf>, n: usize, step: f32) {
        let target = if self.bypass.load(Ordering::Relaxed) { 1.0 } else { 0.0 };
        if self.dry == 0.0 && target == 0.0 {
            return;
        }
        let Some(out) = self.outputs[k].first_mut() else { return };
        for i in 0..n {
            self.dry = if target > self.dry { (self.dry + step).min(1.0) } else { (self.dry - step).max(0.0) };
            for (c, ch) in out.iter_mut().enumerate() {
                let x = input.map_or(0.0, |b| b[c][i]);
                ch[i] += (x - ch[i]) * self.dry;
            }
        }
    }
}

pub struct TopoEntry {
    pub id: NodeId,
    /// For each input port: (source node, source output port).
    pub inputs: [Option<(NodeId, usize)>; MAX_INPUTS],
    /// Node feeding the MIDI input.
    pub midi_input: Option<NodeId>,
    /// Feeds the audio device: its first output is summed into the mix (a
    /// mixer whose main output isn't patched elsewhere).
    pub sink: bool,
    /// What it makes reaches the device (or it must run anyway, see
    /// `Graph::sync`). A node that isn't heard rests after
    /// [`UNHEARD_SECONDS`]: nothing that is heard depends on it.
    pub heard: bool,
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
    /// One block's inputs, for built-in nodes.
    inputs: Box<Inputs>,
    /// A whole pass's input 0 and MIDI, for nodes that take passes.
    pass_audio: Box<[Buf]>,
    pass_midi: Box<[MidiBuf]>,
    /// The sinks' sum over the pass.
    mix: Box<[Buf]>,
    /// The output level now, gliding to the shared one.
    output: f32,
    limiter: Limiter,
    /// Steers the beat to an external MIDI clock, while following one.
    follow: Follow,
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
        let follow = Follow::new(&shared.sync);
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
            pass_audio: vec![[[0.0; BLOCK]; 2]; PASS_BLOCKS].into_boxed_slice(),
            pass_midi: vec![MidiBuf::default(); PASS_BLOCKS].into_boxed_slice(),
            mix: vec![[[0.0; BLOCK]; 2]; PASS_BLOCKS].into_boxed_slice(),
            output: 1.0,
            limiter: Limiter::new(sample_rate),
            follow,
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

    /// Renders one pass of `frames` (up to [`PASS`]) into `mix`.
    fn process_pass(&mut self, frames: usize) {
        let Engine {
            ctx,
            nodes,
            topo,
            inputs,
            pass_audio,
            pass_midi,
            mix,
            ..
        } = self;
        let blocks = frames.div_ceil(BLOCK);
        let len = |k: usize| (frames - k * BLOCK).min(BLOCK);
        for buf in mix[..blocks].iter_mut() {
            *buf = [[0.0; BLOCK]; 2];
        }
        let bypass_step = 1.0 / (BYPASS_SECONDS * ctx.sample_rate);
        let resume_step = 1.0 / (RESUME_SECONDS * ctx.sample_rate);
        let unheard_limit = (UNHEARD_SECONDS * ctx.sample_rate) as usize;
        let pass = *ctx;
        // The context of block `k`: its beat, and the rewind only at the start.
        let block_ctx = |k: usize| ProcessCtx {
            beat: if pass.playing { pass.beat + pass.beats_per_sample * (k * BLOCK) as f64 } else { pass.beat },
            restart: pass.restart && k == 0,
            ..pass
        };

        for entry in &topo.order {
            let Some(slot) = nodes.get_mut(&entry.id) else {
                continue;
            };
            if entry.heard {
                if slot.unheard >= unheard_limit {
                    slot.fade = 0.0;
                }
                slot.unheard = 0;
            } else if slot.unheard >= unheard_limit && rests_unheard() {
                // Resting: what reads its outputs isn't heard either, so
                // it rests too (it stopped being heard no later than this).
                continue;
            } else {
                slot.unheard += frames;
            }
            let wants_pass = slot.proc.wants_pass();
            #[cfg(test)]
            let _timing = profile::Timing::start(entry.id);
            if wants_pass {
                // Input 0 and the MIDI input for the whole pass, then one call.
                let audio = entry.inputs[0].and_then(|(sid, port)| nodes.get(&sid).map(|s| (s, port)));
                if let Some((src, port)) = audio {
                    for (k, buf) in pass_audio[..blocks].iter_mut().enumerate() {
                        *buf = src.outputs[k][port];
                    }
                }
                let midi = entry.midi_input.and_then(|sid| nodes.get(&sid)).filter(|s| !s.midi.is_empty());
                for (k, buf) in pass_midi[..blocks].iter_mut().enumerate() {
                    match midi {
                        Some(src) => buf.copy_from(&src.midi[k]),
                        None => buf.clear(),
                    }
                }
                let connected = audio.is_some();
                let Some(slot) = nodes.get_mut(&entry.id) else { continue };
                let pass_inputs = PassInputs {
                    audio: connected.then(|| &pass_audio[..blocks]),
                    midi: &pass_midi[..blocks],
                    midi_connected: entry.midi_input.is_some(),
                };
                slot.proc.process_pass(&pass, &pass_inputs, &mut slot.outputs[..blocks], frames);
                for k in 0..blocks {
                    slot.blend_bypass(k, connected.then(|| &pass_audio[k]), len(k), bypass_step);
                    slot.fade_in(k, len(k), resume_step);
                }
            } else {
                for k in 0..blocks {
                    let n = len(k);
                    // Copy connected inputs; unconnected ones are only
                    // flagged, since processors read inputs through `connected`.
                    for (i, src) in entry.inputs.iter().enumerate() {
                        let upstream = src.and_then(|(sid, port)| nodes.get(&sid).and_then(|s| s.outputs[k].get(port)));
                        inputs.connected[i] = upstream.is_some();
                        if let Some(buf) = upstream {
                            for (d, s) in inputs.bufs[i].iter_mut().zip(buf) {
                                d[..n].copy_from_slice(&s[..n]);
                            }
                        }
                    }
                    match entry.midi_input.and_then(|sid| nodes.get(&sid)).filter(|s| !s.midi.is_empty()) {
                        Some(src) => inputs.midi.copy_from(&src.midi[k]),
                        None => inputs.midi.clear(),
                    }
                    inputs.midi_connected = entry.midi_input.is_some();

                    let Some(slot) = nodes.get_mut(&entry.id) else { break };
                    slot.proc.process(&block_ctx(k), inputs, &mut slot.outputs[k], n);
                    slot.blend_bypass(k, inputs.get(0), n, bypass_step);
                    slot.fade_in(k, n, resume_step);
                    if !slot.midi.is_empty() {
                        match slot.proc.midi_out() {
                            Some(events) => slot.midi[k].copy_from(events),
                            None => slot.midi[k].clear(),
                        }
                    }
                }
            }

            if entry.sink
                && let Some(slot) = nodes.get(&entry.id)
            {
                for (k, m) in mix[..blocks].iter_mut().enumerate() {
                    let n = len(k);
                    for (ch, out) in m.iter_mut().zip(&slot.outputs[k][0]) {
                        for (a, b) in ch[..n].iter_mut().zip(&out[..n]) {
                            *a += *b;
                        }
                    }
                }
            }
        }
        if ctx.playing {
            ctx.beat += ctx.beats_per_sample * frames as f64;
        }
        ctx.restart = false;
    }

    /// The transport for a callback of `frames`: the board's own (its
    /// tempo, Start and Stop) or, while following one, an external MIDI
    /// clock's (see `sync`). Publishes the result for the clock sent out.
    fn transport(&mut self, frames: usize) {
        let sync = &self.shared.sync;
        let now = sync::now_ns();
        let sr = self.ctx.sample_rate;
        let (slowest, fastest) = TEMPO_RANGE;
        let board = self.shared.tempo.get().clamp(slowest, fastest) as f64 / (60.0 * sr as f64);
        if sync.follow.load(Ordering::Relaxed) {
            let clock = sync.clock();
            let (beat, per_sample, playing, rewound) = self.follow.step(&clock, now, self.ctx.beat, frames, sr);
            self.ctx.beat = beat;
            self.ctx.restart |= rewound;
            self.ctx.playing = playing;
            // No clock coming: the board's tempo, for plugins and delays.
            self.ctx.beats_per_sample = if clock.alive(now) { per_sample } else { board };
            // The UI shows the transport the clock sets.
            self.shared.running.store(playing, Ordering::Relaxed);
        } else {
            self.follow = Follow::new(sync);
            self.ctx.beats_per_sample = board;
            self.ctx.playing = self.shared.running.load(Ordering::Relaxed);
        }
        sync.publish(Timeline {
            beat: self.ctx.beat,
            bpm: self.ctx.beats_per_sample * 60.0 * sr as f64,
            playing: self.ctx.playing,
            at_ns: now,
            latency_ns: (frames as f64 / sr as f64 * 1e9) as u64,
        });
    }

    /// Fills an interleaved device buffer, the board on its first two channels.
    #[cfg(test)]
    pub fn render<T: Sample + FromSample<f32>>(&mut self, data: &mut [T], channels: usize) {
        self.render_at(data, channels, 0);
    }

    /// Fills an interleaved device buffer, the board on channels `first`
    /// and `first + 1` (0-based), silence on the others.
    pub fn render_at<T: Sample + FromSample<f32>>(&mut self, data: &mut [T], channels: usize, first: usize) {
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
        let first = first.min(channels.saturating_sub(2));
        self.transport(frames);
        // What the input captured meanwhile, for the Audio In nodes.
        self.shared.input.begin_render(frames);
        let mut mono = [0.0f32; PASS];
        let mut done = 0;
        let recording = self.shared.tap.recording();
        let output = self.shared.output.get();

        // Always processed, even when stopped: tails ring out.
        while done < frames {
            let n = (frames - done).min(PASS);
            self.process_pass(n);

            let mut lowest = 1.0f32;
            for i in 0..n {
                let block = &self.mix[i / BLOCK];
                self.output += (output - self.output) * self.ctx.smooth;
                let (l, r, gain) = self.limiter.frame(block[0][i % BLOCK] * self.output, block[1][i % BLOCK] * self.output);
                lowest = lowest.min(gain);
                let (l, r) = (l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0));
                if recording {
                    self.shared.tap.push(l, r);
                }
                mono[i] = 0.5 * (l + r);
                let frame = &mut data[(done + i) * channels..(done + i + 1) * channels];
                if channels == 1 {
                    frame[0] = T::from_sample(mono[i]);
                } else {
                    frame.fill(T::EQUILIBRIUM);
                    frame[first] = T::from_sample(l);
                    frame[first + 1] = T::from_sample(r);
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

/// Nodes nobody hears rest (always, but in benchmarks comparing with
/// [`profile::KEEP_ALL`]).
#[inline]
fn rests_unheard() -> bool {
    #[cfg(test)]
    return !profile::KEEP_ALL.load(Ordering::Relaxed);
    #[cfg(not(test))]
    true
}

/// Time spent in each node, for the benchmarks: off until `enable`, per
/// thread.
#[cfg(test)]
pub mod profile {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use super::NodeId;

    /// Every node runs, heard or not (as before nodes rested unheard).
    pub static KEEP_ALL: AtomicBool = AtomicBool::new(false);

    thread_local! {
        static TIMES: RefCell<Option<HashMap<NodeId, f64>>> = const { RefCell::new(None) };
    }

    pub fn enable() {
        TIMES.with(|t| *t.borrow_mut() = Some(HashMap::new()));
    }

    /// The seconds spent in each node since `enable`, and stops counting.
    pub fn take() -> HashMap<NodeId, f64> {
        TIMES.with(|t| t.borrow_mut().take()).unwrap_or_default()
    }

    /// Adds the time until it is dropped to its node.
    pub struct Timing(Option<(NodeId, Instant)>);

    impl Timing {
        pub fn start(id: NodeId) -> Self {
            let on = TIMES.with(|t| t.borrow().is_some());
            Self(on.then(|| (id, Instant::now())))
        }
    }

    impl Drop for Timing {
        fn drop(&mut self) {
            if let Some((id, start)) = self.0 {
                let s = start.elapsed().as_secs_f64();
                TIMES.with(|t| {
                    if let Some(m) = t.borrow_mut().as_mut() {
                        *m.entry(id).or_default() += s;
                    }
                });
            }
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
