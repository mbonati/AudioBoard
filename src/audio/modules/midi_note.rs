//! MIDI Note: a keyboard as an envelope. Holding a key swells ENV up over
//! ATTACK, releasing it fades over RELEASE; VEL is the same shape scaled by
//! velocity. Both swing −1…+1, so into an oscillator's AM (at 100%) they go
//! from silence to full level. The played note can drive any pitch knob
//! directly (see the MAP button in the UI). With a MIDI cable plugged in
//! (from a sequencer or an arpeggiator) it plays that instead of the
//! keyboard.

use std::sync::Arc;

use super::steps::{Held, note_event};
use super::{ModuleDef, ModuleParams, time_coeff};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit};

pub const CHANNEL: usize = 0;
pub const ATTACK: usize = 1;
pub const RELEASE: usize = 2;
pub const TRANSPOSE: usize = 3;

const PARAMS: [ParamSpec; 4] = [
    ParamSpec::lin("CHANNEL", 0.0, 16.0, 0.0, Unit::Channel),
    ParamSpec::log("ATTACK", 0.002, 10.0, 0.05, Unit::Seconds),
    ParamSpec::log("RELEASE", 0.01, 20.0, 1.5, Unit::Seconds),
    ParamSpec::lin("TRANSPOSE", -24.0, 24.0, 0.0, Unit::Semitones).bipolar(),
];

/// `viz[ENV]`: the envelope, 0..1, for the trace.
pub const ENV: usize = 0;
/// `viz[NOTE]`: the note being played (after TRANSPOSE), or −1 before any.
pub const NOTE: usize = 1;
/// `viz[CABLE]`: 1 while a MIDI cable plays the node.
pub const CABLE: usize = 2;
/// `viz[SEQ]`: notes played from the cable, so the UI sees each new one.
pub const SEQ: usize = 3;

pub static DEF: ModuleDef = ModuleDef {
    name: "MIDI Note",
    prefix: "NOTE",
    hint: "keyboard → envelope, pitch",
    inputs: &[],
    outputs: &["ENV", "VEL"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: None,
    toggle_default: false,
    map: true,
    midi_in: true,
    midi_out: false,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: 4,
};

pub fn channel(p: &ModuleParams) -> usize {
    p.get(CHANNEL).round() as usize
}

/// The note to play for a key, after TRANSPOSE.
pub fn played_note(p: &ModuleParams, key: u8) -> f32 {
    (key as f32 + p.get(TRANSPOSE).round()).clamp(0.0, 127.0)
}

pub struct MidiNote {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    sr: f32,
    env: f32,
    /// Keys held on the MIDI cable, the last one's velocity, and how many
    /// notes the cable has played.
    held: Held,
    velocity: f32,
    played: u32,
}

impl MidiNote {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        Self {
            params,
            midi,
            sr: sample_rate,
            env: 0.0,
            held: Held::new(),
            velocity: 0.0,
            played: 0,
        }
    }
}

impl Processor for MidiNote {
    fn process(&mut self, _ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let mut key = self.midi.note(channel(p));
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                let [status, note, velocity] = e.data;
                let wanted = channel(p) == 0 || (status & 0x0F) as usize == channel(p) - 1;
                match note_event(e.data) {
                    Some(true) if wanted => {
                        self.held.press(note);
                        self.velocity = velocity as f32 / 127.0;
                        self.played = self.played.wrapping_add(1);
                    }
                    Some(false) if wanted => self.held.release(note),
                    _ => {}
                }
            }
            key.gate = !self.held.is_empty();
            key.velocity = self.velocity;
            if let Some(&note) = self.held.notes().last() {
                key.note = note;
            }
            key.seq = self.played;
        } else {
            self.held.clear();
        }
        // Times are to ~95% of the way (three time constants).
        let attack = time_coeff(p.get(ATTACK) / 3.0, self.sr);
        let release = time_coeff(p.get(RELEASE) / 3.0, self.sr);
        let (target, k) = if key.gate {
            (1.0, attack)
        } else {
            (0.0, release)
        };

        let mut peak = 0.0f32;
        let [env_out, vel_out] = outputs else {
            return;
        };
        for i in 0..n {
            self.env += (target - self.env) * k;
            let (env, vel) = (self.env * 2.0 - 1.0, self.env * key.velocity * 2.0 - 1.0);
            for (e, v) in env_out.iter_mut().zip(vel_out.iter_mut()) {
                e[i] = env;
                v[i] = vel;
            }
            peak = peak.max(self.env);
        }
        p.viz[ENV].set(self.env);
        p.viz[NOTE].set(if key.seq == 0 {
            -1.0
        } else {
            played_note(p, key.note)
        });
        p.viz[CABLE].set(if inputs.midi_connected { 1.0 } else { 0.0 });
        p.viz[SEQ].set((self.played % (1 << 24)) as f32);
        p.meter.fetch_max(peak);
    }
}
