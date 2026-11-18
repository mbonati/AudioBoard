//! The interface a node is written against: [`Node`] makes the sound,
//! block by block, from what [`Io`] hands it, reading its settings through
//! [`Shared`].

use std::sync::atomic::Ordering;

use crate::abi::{RawParams, RawProcess};
use crate::buffer::{BLOCK, Buf, MIDI_CAP, MidiMsg, ProcessCtx};
use crate::def::Descriptor;

/// A node: built on the UI thread, then run on the audio thread.
///
/// Rules for [`Node::process`], which runs on the audio thread: no
/// allocation, no locks, no I/O, no waiting. Allocate everything (delay
/// lines, tables) in [`Node::new`]. A panic is caught: the node stops and
/// outputs silence from then on.
pub trait Node: Send + 'static {
    /// What the node is, for the host's menus, panel and projects.
    const DESCRIPTOR: Descriptor;

    /// A new node. Runs on the UI thread: allocate here.
    fn new(init: &Init) -> Self
    where
        Self: Sized;

    /// One block: `io.frames()` frames (at most [`BLOCK`]) from `ctx.beat`.
    /// Every output must be written for those frames.
    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io<'_>);
}

/// What a node gets when it is built.
pub struct Init {
    pub sample_rate: f32,
    /// Its settings, shared with the UI.
    pub params: Shared,
}

/// A node's view of its settings ([`crate::Params`], owned by the host):
/// read knobs, the mode, the switch and extra data; publish display values.
/// Copyable and lock-free; indexes out of range read 0 and write nothing.
#[derive(Clone, Copy)]
pub struct Shared {
    raw: RawParams,
}

// SAFETY: every pointer is to an atomic the host keeps alive for as long as
// the node exists (see `RawParams`).
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    /// # Safety
    /// The pointers in `raw` must stay valid for as long as this value (and
    /// its copies) are used: the host guarantees it for the node's life.
    pub unsafe fn from_raw(raw: RawParams) -> Self {
        Self { raw }
    }

    /// Knob `i`'s value.
    #[inline]
    pub fn get(&self, i: usize) -> f32 {
        if i < self.raw.n_values as usize {
            // SAFETY: in bounds; see `from_raw`.
            unsafe { (*self.raw.values.add(i)).get() }
        } else {
            0.0
        }
    }

    /// The selected mode.
    #[inline]
    pub fn mode(&self) -> usize {
        // SAFETY: see `from_raw`.
        unsafe { (*self.raw.mode).load(Ordering::Relaxed) as usize }
    }

    /// The switch.
    #[inline]
    pub fn toggled(&self) -> bool {
        // SAFETY: see `from_raw`.
        unsafe { (*self.raw.toggle).load(Ordering::Relaxed) }
    }

    /// Extra saved value `i` ([`crate::NodeDef::data`]).
    #[inline]
    pub fn data(&self, i: usize) -> f32 {
        if i < self.raw.n_data as usize {
            // SAFETY: in bounds; see `from_raw`.
            unsafe { (*self.raw.data.add(i)).get() }
        } else {
            0.0
        }
    }

    /// Changes extra saved value `i` (it is saved with the project).
    #[inline]
    pub fn set_data(&self, i: usize, v: f32) {
        if i < self.raw.n_data as usize {
            // SAFETY: in bounds; see `from_raw`.
            unsafe { (*self.raw.data.add(i)).set(v) }
        }
    }

    /// Publishes display value `i` for the panel.
    #[inline]
    pub fn set_viz(&self, i: usize, v: f32) {
        if i < self.raw.n_viz as usize {
            // SAFETY: in bounds; see `from_raw`.
            unsafe { (*self.raw.viz.add(i)).set(v) }
        }
    }

    /// Display value `i` as last published.
    #[inline]
    pub fn viz(&self, i: usize) -> f32 {
        if i < self.raw.n_viz as usize {
            // SAFETY: in bounds; see `from_raw`.
            unsafe { (*self.raw.viz.add(i)).get() }
        } else {
            0.0
        }
    }
}

/// One block's audio and MIDI, in and out.
pub struct Io<'a> {
    frames: usize,
    inputs: &'a [Buf],
    connected: &'a [bool],
    outputs: &'a mut [Buf],
    midi_in: &'a [MidiMsg],
    midi_connected: bool,
    midi_out: &'a mut [MidiMsg],
    midi_out_len: usize,
    /// Written instead of an output the node doesn't have.
    scratch: Buf,
}

impl<'a> Io<'a> {
    /// An `Io` over buffers the caller owns (tests, or a host running SDK
    /// nodes in process). `connected[i]` says whether input `i` is patched.
    pub fn new(
        frames: usize,
        inputs: &'a [Buf],
        connected: &'a [bool],
        outputs: &'a mut [Buf],
        midi_in: &'a [MidiMsg],
        midi_out: &'a mut [MidiMsg],
    ) -> Self {
        Self {
            frames: frames.min(BLOCK),
            inputs,
            connected,
            outputs,
            midi_in,
            midi_connected: !midi_in.is_empty(),
            midi_out,
            midi_out_len: 0,
            scratch: [[0.0; BLOCK]; 2],
        }
    }

    /// # Safety
    /// `raw` must describe valid buffers for the duration of `'a`, as the
    /// host's `RawProcess` does during a `process` call.
    pub unsafe fn from_raw(raw: &'a RawProcess) -> Self {
        // SAFETY: the host passes pointers valid for the call, with these
        // lengths (null only with a length of 0).
        unsafe {
            let slice = |p: *const Buf, n: u32| if p.is_null() { &[][..] } else { std::slice::from_raw_parts(p, n as usize) };
            let n_out = if raw.outputs.is_null() { 0 } else { raw.n_outputs as usize };
            let n_midi_out = if raw.midi_out.is_null() { 0 } else { (raw.midi_out_cap as usize).min(MIDI_CAP) };
            Self {
                frames: (raw.frames as usize).min(BLOCK),
                inputs: slice(raw.inputs, raw.n_inputs),
                connected: if raw.connected.is_null() { &[] } else { std::slice::from_raw_parts(raw.connected, raw.n_inputs as usize) },
                outputs: if n_out == 0 { &mut [] } else { std::slice::from_raw_parts_mut(raw.outputs, n_out) },
                midi_in: if raw.midi_in.is_null() { &[] } else { std::slice::from_raw_parts(raw.midi_in, raw.n_midi_in as usize) },
                midi_connected: raw.midi_connected,
                midi_out: if n_midi_out == 0 { &mut [] } else { std::slice::from_raw_parts_mut(raw.midi_out, n_midi_out) },
                midi_out_len: 0,
                scratch: [[0.0; BLOCK]; 2],
            }
        }
    }

    /// Frames in this block (at most [`BLOCK`]).
    #[inline]
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Input `i`, if patched.
    #[inline]
    pub fn input(&self, i: usize) -> Option<&Buf> {
        (self.connected.get(i) == Some(&true)).then(|| self.inputs.get(i)).flatten()
    }

    /// Input `i`'s left channel, if patched: the mono view control (CV)
    /// inputs use.
    #[inline]
    pub fn cv(&self, i: usize) -> Option<&[f32; BLOCK]> {
        self.input(i).map(|b| &b[0])
    }

    /// Output `i` (a scratch buffer when the node has no such output).
    #[inline]
    pub fn output(&mut self, i: usize) -> &mut Buf {
        match self.outputs.get_mut(i) {
            Some(out) => out,
            None => &mut self.scratch,
        }
    }

    /// Every output.
    #[inline]
    pub fn outputs(&mut self) -> &mut [Buf] {
        self.outputs
    }

    /// Input `i` (or silence) and output `j` at once, for effects.
    pub fn through(&mut self, i: usize, j: usize) -> (Option<&Buf>, &mut Buf) {
        let input = (self.connected.get(i) == Some(&true) && i < self.inputs.len()).then(|| &self.inputs[i]);
        let out = match self.outputs.get_mut(j) {
            Some(out) => out,
            None => &mut self.scratch,
        };
        (input, out)
    }

    /// MIDI that arrived on the MIDI input this block, in frame order.
    #[inline]
    pub fn midi(&self) -> &[MidiMsg] {
        self.midi_in
    }

    /// A cable is plugged into the MIDI input.
    #[inline]
    pub fn midi_connected(&self) -> bool {
        self.midi_connected
    }

    /// Sends a MIDI message from the MIDI output at `frame` (dropped when
    /// the block's room, [`MIDI_CAP`] events, is full).
    #[inline]
    pub fn send_midi(&mut self, frame: usize, data: [u8; 3]) {
        if self.midi_out_len < self.midi_out.len() {
            self.midi_out[self.midi_out_len] = MidiMsg { frame: frame.min(BLOCK - 1) as u16, data };
            self.midi_out_len += 1;
        }
    }

    /// MIDI sent so far this block.
    pub fn midi_sent(&self) -> &[MidiMsg] {
        &self.midi_out[..self.midi_out_len]
    }

    /// Silence on every output.
    pub fn silence(&mut self) {
        let n = self.frames;
        crate::dsp::silence(self.outputs, n);
    }
}
