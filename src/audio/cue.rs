//! Song changes made on the audio thread (see `crate::song`): a [`Cue`] is a
//! scene's settings, a key and a tempo that the engine applies exactly on the
//! beat it is due, between two passes, so a section lands on the bar to the
//! sample whatever the UI is doing. Continuous knobs may glide to their new
//! values over a few bars (MORPH); everything else switches at once.
//!
//! [`KeyShift`] transposes the notes a node sends by the song's key, and
//! remembers the shift each note started with, so its note-off matches even
//! when the key changes while it sounds.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use super::engine::{MidiBuf, NodeId};
use super::modules::ModuleParams;
use super::params::{MixerParams, OscParams, Waveform};

/// A cue due now, whatever the beat (also while stopped).
pub const NOW: f64 = f64::NEG_INFINITY;

/// What a node's settings live in.
pub enum Target {
    Module(Arc<ModuleParams>),
    Osc(Arc<OscParams>),
    Mixer(Arc<MixerParams>),
}

/// One setting of a node.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Field {
    Knob(u16),
    Data(u32),
    Mode,
    Toggle,
    Bypass,
    Wave,
    Fader(u16),
    Pan(u16),
    Aux(u16),
    Mute(u16),
    Solo(u16),
    Master,
    AuxMaster,
    MasterMute,
}

/// How a setting goes to its new value during a MORPH.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glide {
    /// At once, on the cue's beat.
    Step,
    /// In a straight line.
    Linear,
    /// In equal ratios (frequencies, times).
    Ratio,
}

#[derive(Clone, Copy, Debug)]
pub struct Set {
    pub field: Field,
    pub to: f32,
    pub glide: Glide,
    /// The value when the cue was applied (where a glide starts).
    pub from: f32,
}

impl Set {
    pub fn new(field: Field, to: f32, glide: Glide) -> Self {
        Self {
            field,
            to,
            glide,
            from: to,
        }
    }
}

/// A node's part of a cue.
pub struct NodeCue {
    pub target: Target,
    pub bypass: Arc<AtomicBool>,
    pub sets: Box<[Set]>,
}

impl NodeCue {
    fn read(&self, f: Field) -> f32 {
        let r = Ordering::Relaxed;
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        match (&self.target, f) {
            (_, Field::Bypass) => flag(self.bypass.load(r)),
            (Target::Module(p), Field::Knob(i)) => p.get(i as usize),
            (Target::Module(p), Field::Data(i)) => p.data.get(i as usize).map_or(0.0, |d| d.get()),
            (Target::Module(p), Field::Mode) => p.mode() as f32,
            (Target::Module(p), Field::Toggle) => flag(p.toggled()),
            (Target::Osc(p), Field::Knob(i)) => p.get(i as usize),
            (Target::Osc(p), Field::Wave) => p.wave.load(r) as f32,
            (Target::Mixer(p), Field::Fader(c)) => p.fader[c as usize].get(),
            (Target::Mixer(p), Field::Pan(c)) => p.pan[c as usize].get(),
            (Target::Mixer(p), Field::Aux(c)) => p.aux[c as usize].get(),
            (Target::Mixer(p), Field::Mute(c)) => flag(p.mute[c as usize].load(r)),
            (Target::Mixer(p), Field::Solo(c)) => flag(p.solo[c as usize].load(r)),
            (Target::Mixer(p), Field::Master) => p.master.get(),
            (Target::Mixer(p), Field::AuxMaster) => p.aux_master.get(),
            (Target::Mixer(p), Field::MasterMute) => flag(p.master_mute.load(r)),
            _ => 0.0,
        }
    }

    fn write(&self, f: Field, v: f32) {
        let r = Ordering::Relaxed;
        let on = v >= 0.5;
        match (&self.target, f) {
            (_, Field::Bypass) => self.bypass.store(on, r),
            (Target::Module(p), Field::Knob(i)) if (i as usize) < p.len() => p.set(i as usize, v),
            (Target::Module(p), Field::Data(i)) => {
                if let Some(d) = p.data.get(i as usize) {
                    d.set(v);
                }
            }
            (Target::Module(p), Field::Mode) => p.set_mode(v.max(0.0).round() as usize),
            (Target::Module(p), Field::Toggle) => p.set_toggle(on),
            (Target::Osc(p), Field::Knob(i)) if (i as usize) < p.values.len() => p.set(i as usize, v),
            (Target::Osc(p), Field::Wave) => {
                p.set_waveform(Waveform::from_u32(v.max(0.0).round() as u32))
            }
            (Target::Mixer(p), field) => {
                let ch = |c: u16| (c as usize) < p.channels();
                match field {
                    Field::Fader(c) if ch(c) => p.fader[c as usize].set(v),
                    Field::Pan(c) if ch(c) => p.pan[c as usize].set(v),
                    Field::Aux(c) if ch(c) => p.aux[c as usize].set(v),
                    Field::Mute(c) if ch(c) => p.mute[c as usize].store(on, r),
                    Field::Solo(c) if ch(c) => p.solo[c as usize].store(on, r),
                    Field::Master => p.master.set(v),
                    Field::AuxMaster => p.aux_master.set(v),
                    Field::MasterMute => p.master_mute.store(on, r),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// A song change: scene settings, the key each node plays in and the tempo,
/// due on beat `at`. Made on the UI thread; the engine only writes atomics.
pub struct Cue {
    /// Reported back in [`SongShared::applied`] once applied.
    pub id: u32,
    /// The beat it is due on ([`NOW`]: at once). Cues due on a later beat
    /// wait while the transport is stopped.
    pub at: f64,
    pub nodes: Vec<NodeCue>,
    /// Beats the continuous settings glide over (0: they switch too).
    pub morph: f64,
    /// The transposition each node's notes get, in semitones (nodes not
    /// listed keep theirs).
    pub shifts: Vec<(NodeId, i8)>,
    /// The tempo from here on, in BPM.
    pub tempo: Option<f32>,
    /// Stops the transport (and rewinds it), as Stop does.
    pub stop: bool,
}

impl Cue {
    pub fn new(id: u32, at: f64) -> Self {
        Self {
            id,
            at,
            nodes: Vec::new(),
            morph: 0.0,
            shifts: Vec::new(),
            tempo: None,
            stop: false,
        }
    }

    /// Writes the settings: those that glide only remember where they
    /// start from when `glides`.
    pub fn apply(&mut self, glides: bool) {
        for node in &mut self.nodes {
            for i in 0..node.sets.len() {
                let s = node.sets[i];
                if glides && s.glide != Glide::Step {
                    node.sets[i].from = node.read(s.field);
                } else {
                    node.write(s.field, s.to);
                }
            }
        }
    }

    /// The gliding settings at `t` (0 at the cue, 1 at the end of MORPH).
    pub fn glide(&self, t: f64) {
        let t = t.clamp(0.0, 1.0) as f32;
        for node in &self.nodes {
            for s in node.sets.iter() {
                let v = match s.glide {
                    Glide::Step => continue,
                    Glide::Linear => s.from + (s.to - s.from) * t,
                    Glide::Ratio if s.from > 0.0 && s.to > 0.0 => s.from * (s.to / s.from).powf(t),
                    Glide::Ratio => s.from + (s.to - s.from) * t,
                };
                node.write(s.field, if t >= 1.0 { s.to } else { v });
            }
        }
    }
}

/// What the engine tells the UI about the song.
pub struct SongShared {
    /// The id of the last cue applied.
    pub applied: AtomicU32,
    /// The transport's position in beats (not wrapped), as `f64` bits.
    beat: AtomicU64,
}

impl Default for SongShared {
    fn default() -> Self {
        Self {
            applied: AtomicU32::new(0),
            beat: AtomicU64::new(0f64.to_bits()),
        }
    }
}

impl SongShared {
    pub fn beat(&self) -> f64 {
        f64::from_bits(self.beat.load(Ordering::Relaxed))
    }

    pub fn set_beat(&self, beat: f64) {
        self.beat.store(beat.to_bits(), Ordering::Relaxed);
    }
}

/// No note sounding on this key.
const SILENT: i8 = i8::MIN;
/// A note that fell outside 0–127 when shifted: its note-off is dropped too.
const DROPPED: i8 = i8::MAX;
/// The drums' channel (10) is never transposed.
const DRUMS: u8 = 9;

/// Transposes a node's MIDI output by the song's key.
pub struct KeyShift {
    pub shift: i8,
    /// The shift each sounding note started with, by channel and note.
    sounding: [[i8; 128]; 16],
    held: u32,
}

impl Default for KeyShift {
    fn default() -> Self {
        Self {
            shift: 0,
            sounding: [[SILENT; 128]; 16],
            held: 0,
        }
    }
}

impl KeyShift {
    /// Transposes `buf` in place. Every note is tracked, also unshifted,
    /// so a note started before the key changes ends where it started.
    pub fn apply(&mut self, buf: &mut MidiBuf) {
        if buf.events().is_empty() {
            return;
        }
        let len = buf.events().len();
        let events = buf.room();
        let mut kept = 0;
        for r in 0..len {
            let mut e = events[r];
            let ch = e.channel();
            let kind = e.kind();
            if ch != DRUMS && matches!(kind, 0x80 | 0x90 | 0xA0) {
                let note = e.data[1] as usize & 0x7F;
                let slot = &mut self.sounding[ch as usize][note];
                let on = kind == 0x90 && e.data[2] > 0;
                let shift = if on {
                    if *slot == SILENT {
                        self.held += 1;
                    }
                    let to = note as i32 + self.shift as i32;
                    *slot = if (0..128).contains(&to) {
                        self.shift
                    } else {
                        DROPPED
                    };
                    *slot
                } else if *slot == SILENT {
                    self.shift
                } else if kind == 0xA0 {
                    *slot
                } else {
                    let s = *slot;
                    *slot = SILENT;
                    self.held = self.held.saturating_sub(1);
                    s
                };
                if shift == DROPPED {
                    continue;
                }
                let to = note as i32 + shift as i32;
                if !(0..128).contains(&to) {
                    continue;
                }
                e.data[1] = to as u8;
            }
            events[kept] = e;
            kept += 1;
        }
        buf.set_len(kept);
    }
}
