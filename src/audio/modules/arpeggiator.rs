//! Arpeggiator: plays the notes being held one at a time, in time with the
//! global tempo, over one to four octaves. Notes come from its MIDI cable
//! or, with nothing plugged in, from the MIDI keyboard. LATCH keeps the
//! chord going after the keys are let go, until a new one is played.

use std::sync::Arc;

use super::steps::{block_end, Held, MAX_STEPS, Step, StepClock, Voice, note_event, publish_notes, roll, steps_per_beat, viz};
use super::{ModuleDef, ModuleParams, Rng};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit};

pub const RATE: usize = 0;
pub const OCTAVES: usize = 1;
pub const GATE: usize = 2;
pub const SWING: usize = 3;
pub const CHANCE: usize = 4;
pub const VELOCITY: usize = 5;

const PARAMS: [ParamSpec; 6] = [
    ParamSpec::lin("RATE", 0.0, 5.0, 3.0, Unit::Division),
    ParamSpec::lin("OCTAVES", 1.0, 4.0, 1.0, Unit::Integer),
    ParamSpec::lin("GATE", 0.05, 1.0, 0.6, Unit::Percent),
    ParamSpec::lin("SWING", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("CHANCE", 0.0, 1.0, 1.0, Unit::Percent),
    ParamSpec::lin("VELOCITY", 1.0, 127.0, 100.0, Unit::Integer),
];

pub const UP: usize = 0;
pub const DOWN: usize = 1;
pub const UP_DOWN: usize = 2;
pub const RANDOM: usize = 3;
pub const PLAYED: usize = 4;

pub const MODE_NAMES: [&str; 5] = ["Up", "Down", "Up & down", "Random", "As played"];

pub static DEF: ModuleDef = ModuleDef {
    name: "Arpeggiator",
    prefix: "ARP",
    hint: "held chord → MIDI notes",
    inputs: &[],
    outputs: &[],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["UP", "DOWN", "U+D", "RAND", "ORDER"],
    toggle: Some("LATCH"),
    toggle_default: false,
    map: false,
    midi_in: true,
    midi_out: true,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

/// Longest pattern: every held note in four octaves, up and back down.
const PATTERN_CAP: usize = Held::CAP * 4 * 2;

/// The notes of one pass, in playing order (random mode plays them shuffled).
fn pattern(chord: &[u8], mode: usize, octaves: usize, out: &mut [u8; PATTERN_CAP]) -> usize {
    // Random mode picks from the rising pattern.
    let mode = if mode == RANDOM { UP } else { mode };
    let mut sorted = [0u8; Held::CAP];
    let sorted = &mut sorted[..chord.len()];
    sorted.copy_from_slice(chord);
    if mode != PLAYED {
        sorted.sort_unstable();
    }
    let mut len = 0;
    for octave in 0..octaves {
        for &note in sorted.iter() {
            let n = note as usize + 12 * octave;
            if n <= 127 {
                out[len] = n as u8;
                len += 1;
            }
        }
    }
    match mode {
        DOWN => out[..len].reverse(),
        UP_DOWN if len > 2 => {
            // Back down without repeating the top and bottom notes.
            for i in (1..len - 1).rev() {
                out[len + (len - 2 - i)] = out[i];
            }
            len += len - 2;
        }
        _ => {}
    }
    len
}

pub struct Arpeggiator {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    cursor: u32,
    clock: StepClock,
    voice: Voice,
    rng: Rng,
    out: MidiBuf,
    /// Keys down now, and the chord being played (the same unless latched).
    keys: Held,
    chord: Held,
    latched: bool,
    cable: bool,
    /// Steps since the chord began.
    position: usize,
}

impl Arpeggiator {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>) -> Self {
        Self {
            params,
            cursor: midi.stream_head(),
            midi,
            clock: StepClock::new(),
            voice: Voice::new(),
            rng: Rng::new(0xA12F_0077),
            out: MidiBuf::default(),
            keys: Held::new(),
            chord: Held::new(),
            latched: false,
            cable: false,
            position: 0,
        }
    }

    fn key(&mut self, msg: [u8; 3]) {
        let note = msg[1];
        match note_event(msg) {
            Some(true) => {
                // The first key of a new chord replaces a latched one.
                if self.latched && self.keys.is_empty() {
                    self.chord.clear();
                }
                if self.chord.is_empty() {
                    self.position = 0;
                }
                self.keys.press(note);
                self.chord.press(note);
            }
            Some(false) => {
                self.keys.release(note);
                if !self.latched {
                    self.chord.release(note);
                }
            }
            None => {}
        }
    }
}

impl Processor for Arpeggiator {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, _outputs: &mut [Buf], n: usize) {
        self.out.clear();
        let p = self.params.clone();
        if ctx.restart {
            // Rewound: from the chord's first note again.
            self.clock = StepClock::new();
            self.voice.stop(&mut self.out, 0);
            self.position = 0;
        }

        // Switching between the cable and the keyboard drops held notes.
        if inputs.midi_connected != self.cable {
            self.cable = inputs.midi_connected;
            self.keys.clear();
            self.chord.clear();
        }
        let latch = p.toggled();
        if latch != self.latched {
            self.latched = latch;
            if !latch {
                self.chord = self.keys.clone();
            }
        }
        if self.cable {
            self.cursor = self.midi.stream_head();
            for e in inputs.midi.events() {
                self.key(e.data);
            }
        } else {
            let mut cursor = self.cursor;
            let midi = self.midi.clone();
            midi.read_stream(&mut cursor, |msg| self.key(msg));
            self.cursor = cursor;
        }

        let per_beat = steps_per_beat(p.get(RATE));
        let mut steps = [Step { frame: 0, beat: 0.0, index: 0 }; MAX_STEPS];
        let count = self.clock.steps(ctx, n, per_beat, p.get(SWING), &mut steps);
        let mut notes = [0u8; PATTERN_CAP];
        let octaves = (p.get(OCTAVES).round() as usize).clamp(1, 4);
        let len = pattern(self.chord.notes(), p.mode(), octaves, &mut notes);
        for step in &steps[..count] {
            self.voice.release_before(&mut self.out, ctx, n, step.beat);
            if len == 0 {
                continue;
            }
            let note = if p.mode() == RANDOM {
                notes[(self.rng.unit() * len as f32) as usize % len]
            } else {
                notes[self.position % len]
            };
            self.position += 1;
            if roll(&mut self.rng, p.get(CHANCE)) {
                let length = p.get(GATE) as f64 / per_beat;
                self.voice.play(&mut self.out, step, note, p.get(VELOCITY).round() as u8, length);
            }
        }
        self.voice.release_before(&mut self.out, ctx, n, block_end(ctx, n));
        publish_notes(&p, self.voice.note(), self.chord.notes());
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chord: &[u8], mode: usize, octaves: usize) -> Vec<u8> {
        let mut out = [0u8; PATTERN_CAP];
        let len = pattern(chord, mode, octaves, &mut out);
        out[..len].to_vec()
    }

    #[test]
    fn patterns() {
        let chord = [64, 60, 67];
        assert_eq!(run(&chord, UP, 1), [60, 64, 67]);
        assert_eq!(run(&chord, DOWN, 1), [67, 64, 60]);
        assert_eq!(run(&chord, UP_DOWN, 1), [60, 64, 67, 64]);
        assert_eq!(run(&chord, PLAYED, 1), [64, 60, 67]);
        assert_eq!(run(&chord, UP, 2), [60, 64, 67, 72, 76, 79]);
        assert_eq!(run(&chord, UP_DOWN, 2), [60, 64, 67, 72, 76, 79, 76, 72, 67, 64]);
        assert_eq!(run(&[60, 62], UP_DOWN, 1), [60, 62]);
        assert!(run(&[], UP, 3).is_empty());
    }
}
