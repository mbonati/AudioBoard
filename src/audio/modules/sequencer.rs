//! Step Sequencer: up to sixteen steps of notes on a scale, in time with the
//! global tempo, sent out on a MIDI cable (to a VST instrument, an
//! arpeggiator or a MIDI Note node). Each step is on or off and holds a
//! scale degree; RATE sets the step length, SWING delays the off-beats and
//! CHANCE skips steps at random.

use std::sync::Arc;

use super::steps::{block_end, MAX_STEPS, Step, StepClock, Voice, roll, steps_per_beat};
use super::{ModuleDef, ModuleParams, Rng};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::params::{ParamSpec, Unit};

pub const RATE: usize = 0;
pub const STEPS: usize = 1;
pub const GATE: usize = 2;
pub const SWING: usize = 3;
pub const CHANCE: usize = 4;
pub const VELOCITY: usize = 5;
pub const ROOT: usize = 6;
pub const OCTAVE: usize = 7;

const PARAMS: [ParamSpec; 8] = [
    ParamSpec::lin("RATE", 0.0, 5.0, 3.0, Unit::Division),
    ParamSpec::lin("STEPS", 1.0, 16.0, 16.0, Unit::Integer),
    ParamSpec::lin("GATE", 0.05, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("SWING", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("CHANCE", 0.0, 1.0, 1.0, Unit::Percent),
    ParamSpec::lin("VELOCITY", 1.0, 127.0, 100.0, Unit::Integer),
    ParamSpec::lin("ROOT", 0.0, 11.0, 0.0, Unit::NoteName),
    ParamSpec::lin("OCTAVE", 1.0, 6.0, 3.0, Unit::Integer),
];

/// (name, semitones of each degree) for the mode selector.
pub const SCALES: [(&str, &[u8]); 5] = [
    ("Major", &[0, 2, 4, 5, 7, 9, 11]),
    ("Minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("Dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("Minor pentatonic", &[0, 3, 5, 7, 10]),
    ("Chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
];

/// Steps in the pattern (STEPS plays the first ones).
pub const LEN: usize = 16;
/// Highest degree a step can hold: two octaves of a seven-note scale.
pub const DEGREE_MAX: usize = 14;

/// `data[..LEN]`: each step's degree; `data[LEN..]`: 1 if the step plays.
const DATA: [f32; 2 * LEN] = [
    0.0, 2.0, 4.0, 2.0, 5.0, 4.0, 2.0, 1.0, 0.0, 2.0, 4.0, 6.0, 7.0, 6.0, 4.0, 2.0, //
    1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0,
];

/// `viz[STEP]`: the step playing, or −1 when stopped.
pub const STEP: usize = 0;
/// `viz[NOTE]`: the note sounding, or −1.
pub const NOTE: usize = 1;

pub static DEF: ModuleDef = ModuleDef {
    name: "Step Sequencer",
    prefix: "SEQ",
    hint: "notes in time → MIDI",
    inputs: &[],
    outputs: &[],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["MAJ", "MIN", "DOR", "PENT", "CHR"],
    toggle: Some("PLAY"),
    toggle_default: true,
    map: false,
    midi_in: false,
    midi_out: true,
    data: &DATA,
    display_h: 100.0,
    controls_h: 0.0,
    viz: 2,
};

/// Steps played (the STEPS knob).
pub fn length(p: &ModuleParams) -> usize {
    (p.get(STEPS).round() as usize).clamp(1, LEN)
}

pub fn degree(p: &ModuleParams, step: usize) -> usize {
    (p.data[step].get().round().max(0.0) as usize).min(DEGREE_MAX)
}

pub fn is_on(p: &ModuleParams, step: usize) -> bool {
    p.data[LEN + step].get() > 0.5
}

pub fn set_degree(p: &ModuleParams, step: usize, degree: usize) {
    p.data[step].set(degree.min(DEGREE_MAX) as f32);
}

pub fn set_on(p: &ModuleParams, step: usize, on: bool) {
    p.data[LEN + step].set(if on { 1.0 } else { 0.0 });
}

/// The MIDI note for a degree of the chosen scale, from ROOT and OCTAVE.
pub fn note_of(p: &ModuleParams, degree: usize) -> u8 {
    let scale = SCALES[p.mode().min(SCALES.len() - 1)].1;
    let root = 12 * (p.get(OCTAVE).round() as usize + 1) + p.get(ROOT).round() as usize % 12;
    let note = root + 12 * (degree / scale.len()) + scale[degree % scale.len()] as usize;
    note.min(127) as u8
}

pub struct Sequencer {
    params: Arc<ModuleParams>,
    clock: StepClock,
    voice: Voice,
    rng: Rng,
    out: MidiBuf,
}

impl Sequencer {
    pub fn new(params: Arc<ModuleParams>) -> Self {
        Self {
            params,
            clock: StepClock::new(),
            voice: Voice::new(),
            rng: Rng::new(0x5EC0_1234),
            out: MidiBuf::default(),
        }
    }
}

impl Processor for Sequencer {
    fn process(&mut self, ctx: &ProcessCtx, _inputs: &Inputs, _outputs: &mut [Buf], n: usize) {
        self.out.clear();
        if ctx.restart {
            // Rewound: from step 1 again, ending the note held.
            self.clock = StepClock::new();
            self.voice.stop(&mut self.out, 0);
        }
        let p = &self.params;
        // Off, or the transport stopped: end the note held.
        if !p.toggled() || !ctx.playing {
            self.voice.stop(&mut self.out, 0);
            p.viz[STEP].set(-1.0);
            p.viz[NOTE].set(-1.0);
            return;
        }
        let per_beat = steps_per_beat(p.get(RATE));
        let mut steps = [Step { frame: 0, beat: 0.0, index: 0 }; MAX_STEPS];
        let count = self.clock.steps(ctx, n, per_beat, p.get(SWING), &mut steps);
        for step in &steps[..count] {
            self.voice.release_before(&mut self.out, ctx, n, step.beat);
            let i = step.index.rem_euclid(length(p) as i64) as usize;
            p.viz[STEP].set(i as f32);
            if is_on(p, i) && roll(&mut self.rng, p.get(CHANCE)) {
                let note = note_of(p, degree(p, i));
                let length = p.get(GATE) as f64 / per_beat;
                self.voice.play(&mut self.out, step, note, p.get(VELOCITY).round() as u8, length);
            }
        }
        self.voice.release_before(&mut self.out, ctx, n, block_end(ctx, n));
        p.viz[NOTE].set(self.voice.note().map_or(-1.0, f32::from));
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}
