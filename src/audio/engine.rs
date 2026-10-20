//! The audio-thread side of the graph.
//!
//! The UI owns the "truth" (nodes, cables, positions) and sends the engine
//! commands: new processors, removals and a pre-sorted topology. The engine
//! never sorts or allocates per block; it walks `Topology::order`, copies each
//! node's inputs from upstream output buffers and runs the processor.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
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

pub struct Engine {
    rx: Receiver<Command>,
    shared: Arc<EngineShared>,
    ctx: ProcessCtx,
    nodes: HashMap<NodeId, Box<Slot>>,
    topo: Box<Topology>,
    inputs: Box<Inputs>,
    mix: [[f32; BLOCK]; 2],
}

impl Engine {
    pub fn new(rx: Receiver<Command>, shared: Arc<EngineShared>, sample_rate: f32) -> Self {
        Self {
            rx,
            shared,
            ctx: ProcessCtx {
                sample_rate,
                smooth: 1.0 - (-1.0 / (sample_rate * 0.010)).exp(),
                beat: 0.0,
                beats_per_sample: 0.0,
            },
            nodes: HashMap::with_capacity(256),
            topo: Box::default(),
            inputs: Box::new(Inputs {
                bufs: [[[0.0; BLOCK]; 2]; MAX_INPUTS],
                connected: [false; MAX_INPUTS],
                midi: MidiBuf::default(),
                midi_connected: false,
            }),
            mix: [[0.0; BLOCK]; 2],
        }
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                Command::AddNode(id, slot) => {
                    self.nodes.insert(id, slot);
                }
                Command::RemoveNode(id) => {
                    self.nodes.remove(&id);
                }
                Command::SetTopology(t) => self.topo = t,
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

        for entry in &topo.order {
            for (k, src) in entry.inputs.iter().enumerate() {
                let upstream = src.and_then(|(sid, port)| {
                    nodes.get(&sid).and_then(|s| s.outputs.get(port))
                });
                let dst = &mut inputs.bufs[k];
                match upstream {
                    Some(buf) => {
                        for (d, s) in dst.iter_mut().zip(buf) {
                            d[..n].copy_from_slice(&s[..n]);
                        }
                        inputs.connected[k] = true;
                    }
                    None => {
                        for d in dst.iter_mut() {
                            d[..n].fill(0.0);
                        }
                        inputs.connected[k] = false;
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
        ctx.beat += ctx.beats_per_sample * n as f64;
    }

    /// Fills an interleaved device buffer.
    pub fn render<T: Sample + FromSample<f32>>(&mut self, data: &mut [T], channels: usize) {
        let started = Instant::now();
        self.drain_commands();

        let channels = channels.max(1);
        let frames = data.len() / channels;
        let running = self.shared.running.load(Ordering::Relaxed);
        let mut mono = [0.0f32; BLOCK];
        let mut done = 0;

        while done < frames {
            let n = (frames - done).min(BLOCK);
            if running {
                self.process_block(n);
            } else {
                for ch in self.mix.iter_mut() {
                    ch[..n].fill(0.0);
                }
            }

            for i in 0..n {
                let l = self.mix[0][i].clamp(-1.0, 1.0);
                let r = self.mix[1][i].clamp(-1.0, 1.0);
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
            done += n;
        }
        self.shared.beat.set((self.ctx.beat % 1024.0) as f32);

        if frames > 0 {
            let available = frames as f32 / self.ctx.sample_rate;
            let load = started.elapsed().as_secs_f32() / available;
            let prev = self.shared.load.get();
            self.shared.load.set(prev + (load - prev) * 0.05);
        }
    }
}
