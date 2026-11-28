//! Quartz: an eight-voice, four-operator FM synthesizer after the Yamaha
//! TX81Z, built from its owner's manual.
//!
//! The voice is the TX's ([`super::fm`]): four operators with eight
//! waveforms, ratio or fixed frequencies, detune, the five-rate envelopes
//! with EG SHIFT, rate and level scaling, key velocity and EG bias
//! sensitivity, amplitude modulation enable, eight algorithms and feedback
//! on operator 4. Around it, the voice's other pages: the LFO (saw up,
//! square, triangle, S/Hold; speed, delay, pitch and amplitude modulation
//! depth, sync) and the sensitivities (PMS, AMS); the FUNCTION page (poly
//! or mono, pitch bend range, full-time or fingered portamento and its
//! time, the mod wheel's and the breath controller's pitch and amplitude,
//! breath pitch and EG bias, transpose, the pseudo-reverb); one of the
//! performance effects, worked in the note domain as on the TX (the
//! transposed DELAY replays the notes, PAN throws each note between the
//! outputs by the LFO, velocity or key, CHORD plays a chord from one key);
//! the microtonal scales (equal, seven historical temperaments, quarter
//! and eighth tones, on any key). Ours: up to eight voices in one
//! instrument, and a bank of factory voices of our own.
//!
//! An independent instrument written for the board: the same way of working
//! and the same parameters, its own sounds and name.

use std::sync::Arc;

use super::fm::{self, FmPatch, FmVoice, Lfo, Mods, OPS, OpPatch, SEG};
use super::{ModuleDef, ModuleParams, Rng, pan_gains, soft_limit};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit};

/// Voices, as on the TX.
pub const MAX_VOICES: usize = 8;

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// One operator's parameters: [`OP_LEN`] of them from `op(k, field)`.
pub mod op {
    pub const ON: usize = 0;
    pub const FIXED: usize = 1;
    pub const CRS: usize = 2;
    pub const FINE: usize = 3;
    pub const RANGE: usize = 4;
    pub const DET: usize = 5;
    pub const WAVE: usize = 6;
    pub const OUT: usize = 7;
    pub const AR: usize = 8;
    pub const D1R: usize = 9;
    pub const D1L: usize = 10;
    pub const D2R: usize = 11;
    pub const RR: usize = 12;
    pub const SHIFT: usize = 13;
    pub const RS: usize = 14;
    pub const LS: usize = 15;
    pub const KVS: usize = 16;
    pub const AME: usize = 17;
    pub const EBS: usize = 18;
}
pub const OP_LEN: usize = 19;

/// Operator `k`'s (0 = OP1) parameter `field`.
pub const fn op(k: usize, field: usize) -> usize {
    k * OP_LEN + field
}

const G: usize = OPS * OP_LEN;
pub const ALG: usize = G;
pub const FEEDBACK: usize = G + 1;
pub const LFO_WAVE: usize = G + 2;
pub const LFO_SPEED: usize = G + 3;
pub const LFO_DELAY: usize = G + 4;
pub const PMD: usize = G + 5;
pub const AMD: usize = G + 6;
pub const LFO_SYNC: usize = G + 7;
pub const PMS: usize = G + 8;
pub const AMS: usize = G + 9;
pub const MONO: usize = G + 10;
pub const PB_RANGE: usize = G + 11;
pub const PORTA_MODE: usize = G + 12;
pub const PORTA_TIME: usize = G + 13;
pub const MW_PITCH: usize = G + 14;
pub const MW_AMP: usize = G + 15;
pub const BC_PITCH: usize = G + 16;
pub const BC_AMP: usize = G + 17;
pub const BC_PBIAS: usize = G + 18;
pub const BC_EBIAS: usize = G + 19;
pub const TRANSPOSE: usize = G + 20;
pub const REVERB: usize = G + 21;
pub const NOTES: usize = G + 22;
pub const EFFECT: usize = G + 23;
pub const DLY_TIME: usize = G + 24;
pub const DLY_SHIFT: usize = G + 25;
pub const DLY_FB: usize = G + 26;
pub const DLY_LEVEL: usize = G + 27;
pub const PAN_SRC: usize = G + 28;
pub const PAN_DIR: usize = G + 29;
pub const PAN_RANGE: usize = G + 30;
pub const CHORD: usize = G + 31;
pub const MICRO: usize = G + 32;
pub const MICRO_KEY: usize = G + 33;
pub const VOLUME: usize = G + 34;
pub const MOD_WHEEL: usize = G + 35;
pub const BREATH: usize = G + 36;
pub const PARAM_COUNT: usize = G + 37;

pub const EFFECTS: [&str; 4] = ["OFF", "DELAY", "PAN", "CHORD"];
pub const PAN_SOURCES: [&str; 3] = ["LFO", "VELOCITY", "NOTE"];
pub const PAN_DIRS: [&str; 3] = ["I→II", "II→I", "I↔II"];
/// Chords from one key (semitones; the TX keeps one per key, we keep one).
pub const CHORDS: [(&str, &[i32]); 8] = [
    ("MAJOR", &[0, 4, 7]),
    ("MINOR", &[0, 3, 7]),
    ("SEVENTH", &[0, 4, 7, 10]),
    ("MAJ7", &[0, 4, 7, 11]),
    ("MIN7", &[0, 3, 7, 10]),
    ("SUS4", &[0, 5, 7]),
    ("FIFTHS", &[0, 7, 12]),
    ("OCTAVES", &[0, 12, -12]),
];

const fn int(name: &'static str, min: f32, max: f32, default: f32) -> ParamSpec {
    ParamSpec::lin(name, min, max, default, Unit::Integer)
}

const PARAMS: [ParamSpec; PARAM_COUNT] = {
    let mut p = [int("", 0.0, 1.0, 0.0); PARAM_COUNT];
    const NAMES: [[&str; OP_LEN]; OPS] = [
        [
            "OP1 ON", "OP1 FIX", "OP1 CRS", "OP1 FINE", "OP1 FIX RANGE", "OP1 DET", "OP1 WAVE",
            "OP1 OUT", "OP1 AR", "OP1 D1R", "OP1 D1L", "OP1 D2R", "OP1 RR", "OP1 SHIFT", "OP1 RS",
            "OP1 LS", "OP1 KVS", "OP1 AME", "OP1 EBS",
        ],
        [
            "OP2 ON", "OP2 FIX", "OP2 CRS", "OP2 FINE", "OP2 FIX RANGE", "OP2 DET", "OP2 WAVE",
            "OP2 OUT", "OP2 AR", "OP2 D1R", "OP2 D1L", "OP2 D2R", "OP2 RR", "OP2 SHIFT", "OP2 RS",
            "OP2 LS", "OP2 KVS", "OP2 AME", "OP2 EBS",
        ],
        [
            "OP3 ON", "OP3 FIX", "OP3 CRS", "OP3 FINE", "OP3 FIX RANGE", "OP3 DET", "OP3 WAVE",
            "OP3 OUT", "OP3 AR", "OP3 D1R", "OP3 D1L", "OP3 D2R", "OP3 RR", "OP3 SHIFT", "OP3 RS",
            "OP3 LS", "OP3 KVS", "OP3 AME", "OP3 EBS",
        ],
        [
            "OP4 ON", "OP4 FIX", "OP4 CRS", "OP4 FINE", "OP4 FIX RANGE", "OP4 DET", "OP4 WAVE",
            "OP4 OUT", "OP4 AR", "OP4 D1R", "OP4 D1L", "OP4 D2R", "OP4 RR", "OP4 SHIFT", "OP4 RS",
            "OP4 LS", "OP4 KVS", "OP4 AME", "OP4 EBS",
        ],
    ];
    // An init voice: one sine carrier with a sine modulator an octave up.
    const OUT: [f32; OPS] = [90.0, 70.0, 0.0, 0.0];
    let mut k = 0;
    while k < OPS {
        let n = NAMES[k];
        p[op(k, op::ON)] = int(n[0], 0.0, 1.0, 1.0);
        p[op(k, op::FIXED)] = int(n[1], 0.0, 1.0, 0.0);
        p[op(k, op::CRS)] = int(n[2], 0.0, 63.0, if k == 1 { 8.0 } else { 4.0 });
        p[op(k, op::FINE)] = int(n[3], 0.0, 15.0, 0.0);
        p[op(k, op::RANGE)] = int(n[4], 0.0, 7.0, 0.0);
        p[op(k, op::DET)] = int(n[5], -3.0, 3.0, 0.0).bipolar();
        p[op(k, op::WAVE)] = int(n[6], 0.0, 7.0, 0.0);
        p[op(k, op::OUT)] = int(n[7], 0.0, 99.0, OUT[k]);
        p[op(k, op::AR)] = int(n[8], 0.0, 31.0, 31.0);
        p[op(k, op::D1R)] = int(n[9], 0.0, 31.0, 12.0);
        p[op(k, op::D1L)] = int(n[10], 0.0, 15.0, 12.0);
        p[op(k, op::D2R)] = int(n[11], 0.0, 31.0, 0.0);
        p[op(k, op::RR)] = int(n[12], 1.0, 15.0, 7.0);
        p[op(k, op::SHIFT)] = int(n[13], 0.0, 3.0, 0.0);
        p[op(k, op::RS)] = int(n[14], 0.0, 3.0, 1.0);
        p[op(k, op::LS)] = int(n[15], 0.0, 99.0, 0.0);
        p[op(k, op::KVS)] = int(n[16], 0.0, 7.0, 2.0);
        p[op(k, op::AME)] = int(n[17], 0.0, 1.0, 0.0);
        p[op(k, op::EBS)] = int(n[18], 0.0, 7.0, 0.0);
        k += 1;
    }
    p[ALG] = int("ALGORITHM", 0.0, 7.0, 0.0);
    p[FEEDBACK] = int("FEEDBACK", 0.0, 7.0, 0.0);
    p[LFO_WAVE] = int("LFO WAVE", 0.0, 3.0, 2.0);
    p[LFO_SPEED] = int("LFO SPEED", 0.0, 99.0, 35.0);
    p[LFO_DELAY] = int("LFO DELAY", 0.0, 99.0, 0.0);
    p[PMD] = int("PMD", 0.0, 99.0, 0.0);
    p[AMD] = int("AMD", 0.0, 99.0, 0.0);
    p[LFO_SYNC] = int("LFO SYNC", 0.0, 1.0, 0.0);
    p[PMS] = int("PMS", 0.0, 7.0, 3.0);
    p[AMS] = int("AMS", 0.0, 3.0, 0.0);
    p[MONO] = int("MONO", 0.0, 1.0, 0.0);
    p[PB_RANGE] = int("PB RANGE", 0.0, 12.0, 2.0);
    p[PORTA_MODE] = int("PORTA MODE", 0.0, 1.0, 0.0);
    p[PORTA_TIME] = int("PORTA TIME", 0.0, 99.0, 0.0);
    p[MW_PITCH] = int("MW PITCH", 0.0, 99.0, 50.0);
    p[MW_AMP] = int("MW AMPLITUDE", 0.0, 99.0, 0.0);
    p[BC_PITCH] = int("BC PITCH", 0.0, 99.0, 0.0);
    p[BC_AMP] = int("BC AMPLITUDE", 0.0, 99.0, 0.0);
    p[BC_PBIAS] = int("BC PITCH BIAS", -50.0, 50.0, 0.0).bipolar();
    p[BC_EBIAS] = int("BC EG BIAS", 0.0, 99.0, 0.0);
    p[TRANSPOSE] = int("TRANSPOSE", -24.0, 24.0, 0.0).bipolar();
    p[REVERB] = int("REVERB RATE", 0.0, 7.0, 0.0);
    p[NOTES] = int("MAX NOTES", 1.0, MAX_VOICES as f32, MAX_VOICES as f32);
    p[EFFECT] = int("EFFECT", 0.0, 3.0, 0.0);
    p[DLY_TIME] = int("DELAY TIME", 0.0, 127.0, 24.0);
    p[DLY_SHIFT] = int("DELAY SHIFT", -24.0, 24.0, 0.0).bipolar();
    p[DLY_FB] = int("DELAY FEEDBACK", 0.0, 7.0, 2.0);
    p[DLY_LEVEL] = int("DELAY LEVEL", 0.0, 99.0, 73.0);
    p[PAN_SRC] = int("PAN SOURCE", 0.0, 2.0, 0.0);
    p[PAN_DIR] = int("PAN DIRECTION", 0.0, 2.0, 2.0);
    p[PAN_RANGE] = int("PAN RANGE", 0.0, 99.0, 50.0);
    p[CHORD] = int("CHORD", 0.0, (CHORDS.len() - 1) as f32, 0.0);
    p[MICRO] = int("MICROTUNE", 0.0, (fm::MICRO_NAMES.len() - 1) as f32, 0.0);
    p[MICRO_KEY] = ParamSpec::lin("MICRO KEY", 0.0, 11.0, 0.0, Unit::NoteName);
    p[VOLUME] = int("VOLUME", 0.0, 99.0, 80.0);
    p[MOD_WHEEL] = ParamSpec::lin("MOD WHEEL", 0.0, 1.0, 0.0, Unit::Percent);
    p[BREATH] = ParamSpec::lin("BREATH", 0.0, 1.0, 1.0, Unit::Percent);
    p
};

pub mod data {
    /// The factory voice last loaded.
    pub const VOICE: usize = 0;
    pub const LEN: usize = 1;
}

pub mod viz {
    use super::{MAX_VOICES, OPS};
    /// Each voice's level, note (−1 idle) and gate.
    pub const LEVEL: usize = 0;
    pub const NOTE: usize = LEVEL + MAX_VOICES;
    pub const GATE: usize = NOTE + MAX_VOICES;
    /// The newest voice's operator envelope levels, 0..1.
    pub const OP_LEVEL: usize = GATE + MAX_VOICES;
    /// The LFO, −1..1.
    pub const LFO: usize = OP_LEVEL + OPS;
    /// Written by the panel: the key its keyboard holds + 1 (0: none).
    pub const PANEL_KEY: usize = LFO + 1;
    /// Bumped by the panel: all notes off.
    pub const PANIC: usize = PANEL_KEY + 1;
    pub const LEN: usize = PANIC + 1;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Quartz",
    prefix: "QUARTZ",
    hint: "4-operator FM synth · 8 waveforms · TX style",
    outputs: &["OUT"],
    params: &PARAMS,
    midi_in: true,
    data: &[0.0; data::LEN],
    viz: viz::LEN,
    ..ModuleDef::EMPTY
};

pub fn spec(i: usize) -> &'static ParamSpec {
    &DEF.params[i]
}

pub fn int_of(p: &ModuleParams, i: usize) -> i32 {
    let s = spec(i);
    p.get(i).round().clamp(s.min, s.max) as i32
}

// ---------------------------------------------------------------------------
// Factory voices
// ---------------------------------------------------------------------------

/// An operator of a factory voice: wave, CRS, FINE, OUT, AR, D1R, D1L, D2R,
/// RR, KVS, DET.
type OpDef = (u8, u8, u8, u8, u8, u8, u8, u8, u8, u8, i8);

pub struct Voice {
    pub name: &'static str,
    pub alg: u8,
    pub feedback: u8,
    pub ops: [OpDef; OPS],
    /// Other settings: (parameter, value).
    pub extra: &'static [(usize, f32)],
}

pub const VOICES: [Voice; 16] = [
    Voice {
        name: "Init Voice",
        alg: 0,
        feedback: 0,
        ops: [
            (0, 4, 0, 90, 31, 12, 12, 0, 7, 2, 0),
            (0, 8, 0, 70, 31, 12, 12, 0, 7, 2, 0),
            (0, 4, 0, 0, 31, 0, 15, 0, 7, 0, 0),
            (0, 4, 0, 0, 31, 0, 15, 0, 7, 0, 0),
        ],
        extra: &[],
    },
    Voice {
        name: "Solid Bass",
        alg: 4,
        feedback: 5,
        ops: [
            (0, 0, 0, 92, 31, 14, 8, 6, 9, 2, 0),
            (1, 0, 0, 72, 31, 18, 4, 8, 9, 4, 0),
            (0, 0, 0, 85, 31, 15, 7, 6, 9, 2, 1),
            (2, 0, 0, 68, 31, 20, 3, 8, 9, 3, 0),
        ],
        extra: &[(MONO, 1.0), (PORTA_TIME, 18.0), (PORTA_MODE, 1.0)],
    },
    Voice {
        name: "Glass Piano",
        alg: 4,
        feedback: 2,
        ops: [
            (0, 4, 0, 94, 31, 11, 6, 3, 8, 3, 1),
            (0, 4, 0, 75, 31, 13, 3, 5, 8, 4, 0),
            (0, 4, 0, 82, 31, 12, 5, 4, 8, 3, -1),
            (0, 31, 0, 60, 31, 18, 0, 6, 8, 5, 0),
        ],
        extra: &[(REVERB, 3.0)],
    },
    Voice {
        name: "Tine Keys",
        alg: 4,
        feedback: 0,
        ops: [
            (0, 4, 0, 94, 31, 10, 6, 3, 7, 3, 0),
            (0, 49, 0, 64, 31, 20, 0, 8, 7, 5, 0),
            (0, 4, 0, 88, 31, 9, 7, 3, 7, 3, 2),
            (0, 4, 0, 70, 31, 13, 4, 5, 7, 4, 0),
        ],
        extra: &[(LFO_SPEED, 40.0), (AMD, 12.0), (AMS, 1.0)],
    },
    Voice {
        name: "Bright Brass",
        alg: 1,
        feedback: 4,
        ops: [
            (0, 4, 0, 94, 20, 8, 12, 2, 8, 2, 0),
            (0, 4, 0, 78, 18, 9, 10, 3, 8, 3, 1),
            (0, 4, 0, 60, 24, 10, 8, 3, 8, 3, -1),
            (0, 8, 0, 66, 17, 7, 10, 2, 8, 3, 0),
        ],
        extra: &[(LFO_DELAY, 50.0), (PMD, 8.0), (PMS, 4.0)],
    },
    Voice {
        name: "Warm Strings",
        alg: 6,
        feedback: 3,
        ops: [
            (0, 4, 0, 86, 14, 6, 13, 2, 6, 1, 2),
            (0, 4, 0, 86, 13, 6, 13, 2, 6, 1, -2),
            (0, 4, 0, 82, 15, 6, 13, 2, 6, 1, 0),
            (0, 4, 0, 62, 16, 6, 12, 2, 6, 1, 0),
        ],
        extra: &[
            (LFO_SPEED, 30.0),
            (LFO_DELAY, 40.0),
            (PMD, 6.0),
            (PMS, 4.0),
            (REVERB, 2.0),
        ],
    },
    Voice {
        name: "Bell Tower",
        alg: 4,
        feedback: 0,
        ops: [
            (0, 4, 0, 92, 31, 8, 0, 3, 5, 3, 0),
            (0, 16, 4, 74, 31, 9, 0, 3, 5, 3, 0),
            (0, 4, 0, 82, 31, 7, 0, 3, 5, 3, 1),
            (0, 23, 0, 66, 31, 10, 0, 3, 5, 3, 0),
        ],
        extra: &[(REVERB, 1.0)],
    },
    Voice {
        name: "Square Lead",
        alg: 3,
        feedback: 6,
        ops: [
            (1, 4, 0, 92, 31, 10, 13, 2, 8, 1, 0),
            (0, 8, 0, 64, 31, 12, 10, 3, 8, 2, 0),
            (0, 4, 0, 60, 31, 12, 10, 3, 8, 2, 1),
            (0, 4, 0, 72, 31, 10, 12, 2, 8, 2, 0),
        ],
        extra: &[
            (MONO, 1.0),
            (PORTA_TIME, 25.0),
            (LFO_DELAY, 45.0),
            (PMD, 10.0),
        ],
    },
    Voice {
        name: "Hollow Organ",
        alg: 7,
        feedback: 3,
        ops: [
            (0, 4, 0, 86, 31, 0, 15, 0, 9, 0, 0),
            (0, 8, 0, 80, 31, 0, 15, 0, 9, 0, 0),
            (2, 13, 0, 74, 31, 0, 15, 0, 9, 0, 0),
            (0, 0, 0, 78, 31, 0, 15, 0, 9, 0, 0),
        ],
        extra: &[(LFO_SPEED, 62.0), (AMD, 18.0), (AMS, 1.0)],
    },
    Voice {
        name: "Pluck Wire",
        alg: 0,
        feedback: 5,
        ops: [
            (0, 4, 0, 94, 31, 15, 0, 6, 8, 3, 0),
            (4, 4, 0, 74, 31, 17, 0, 8, 8, 4, 0),
            (0, 12, 0, 64, 31, 20, 0, 9, 8, 5, 0),
            (0, 4, 0, 60, 31, 22, 0, 10, 8, 5, 0),
        ],
        extra: &[],
    },
    Voice {
        name: "Breath Flute",
        alg: 3,
        feedback: 4,
        ops: [
            (0, 4, 0, 92, 15, 6, 13, 2, 7, 1, 0),
            (0, 4, 0, 52, 18, 8, 10, 3, 7, 2, 0),
            (0, 8, 0, 48, 16, 8, 10, 3, 7, 2, 0),
            (0, 4, 0, 60, 20, 10, 8, 3, 7, 2, 0),
        ],
        extra: &[(LFO_DELAY, 55.0), (PMD, 7.0), (PMS, 4.0), (LFO_SPEED, 33.0)],
    },
    Voice {
        name: "Metal Drop",
        alg: 1,
        feedback: 7,
        ops: [
            (0, 4, 0, 94, 31, 13, 0, 5, 7, 4, 0),
            (5, 17, 0, 76, 31, 15, 0, 6, 7, 5, 0),
            (6, 26, 0, 70, 31, 16, 0, 7, 7, 5, 0),
            (7, 11, 0, 66, 31, 18, 0, 8, 7, 5, 0),
        ],
        extra: &[],
    },
    Voice {
        name: "Space Pad",
        alg: 4,
        feedback: 4,
        ops: [
            (0, 4, 0, 86, 9, 4, 13, 2, 5, 1, 3),
            (2, 8, 0, 62, 8, 4, 11, 2, 5, 1, 0),
            (0, 4, 0, 86, 9, 4, 13, 2, 5, 1, -3),
            (6, 4, 3, 58, 8, 5, 10, 2, 5, 1, 0),
        ],
        extra: &[
            (LFO_SPEED, 18.0),
            (PMD, 4.0),
            (AMD, 10.0),
            (AMS, 1.0),
            (REVERB, 1.0),
            (EFFECT, 1.0),
            (DLY_TIME, 40.0),
            (DLY_SHIFT, 12.0),
            (DLY_FB, 2.0),
        ],
    },
    Voice {
        name: "Wood Mallet",
        alg: 5,
        feedback: 2,
        ops: [
            (0, 4, 0, 92, 31, 16, 0, 8, 8, 4, 0),
            (0, 13, 0, 70, 31, 20, 0, 10, 8, 4, 0),
            (0, 8, 0, 66, 31, 18, 0, 9, 8, 4, 0),
            (0, 37, 0, 62, 31, 24, 0, 12, 8, 6, 0),
        ],
        extra: &[],
    },
    Voice {
        name: "Snare Fix",
        alg: 3,
        feedback: 7,
        ops: [
            (0, 4, 0, 94, 31, 18, 0, 12, 10, 3, 0),
            (1, 4, 0, 80, 31, 20, 0, 14, 10, 3, 0),
            (0, 4, 0, 84, 31, 22, 0, 14, 10, 3, 0),
            (0, 4, 0, 90, 31, 20, 0, 13, 10, 3, 0),
        ],
        extra: &[(op(2, op::FIXED), 1.0), (op(2, op::RANGE), 2.0)],
    },
    Voice {
        name: "Thin Clav",
        alg: 2,
        feedback: 5,
        ops: [
            (3, 4, 0, 92, 31, 14, 4, 6, 9, 4, 0),
            (0, 12, 0, 70, 31, 16, 2, 8, 9, 5, 0),
            (0, 8, 0, 62, 31, 18, 0, 9, 9, 5, 0),
            (0, 20, 0, 58, 31, 20, 0, 10, 9, 5, 0),
        ],
        extra: &[(EFFECT, 2.0), (PAN_SRC, 2.0)],
    },
];

/// Factory voice `k` as an engine patch (Stormo's nodes play these).
pub fn voice_patch(k: usize) -> FmPatch {
    let v = &VOICES[k % VOICES.len()];
    let value = |i: usize| {
        v.extra
            .iter()
            .rev()
            .find(|e| e.0 == i)
            .map_or(spec(i).default, |e| e.1)
    };
    let mut patch = FmPatch {
        alg: v.alg as usize,
        feedback: v.feedback as usize,
        reverb: value(REVERB),
        ..FmPatch::default()
    };
    for (k, o) in v.ops.iter().enumerate() {
        let (wave, crs, fine, out, ar, d1r, d1l, d2r, rr, kvs, det) = *o;
        let fixed = value(op(k, op::FIXED)) > 0.5;
        patch.ops[k] = OpPatch {
            on: true,
            fixed,
            freq: if fixed {
                fm::fixed_hz(crs as usize, fine as usize, value(op(k, op::RANGE)) as usize)
            } else {
                fm::ratio(crs as usize, fine as usize)
            },
            detune_cents: det as f32 * 0.87,
            wave: wave as usize,
            out: out as f32,
            ar: ar as f32,
            d1r: d1r as f32,
            d1l: d1l as f32,
            d2r: d2r as f32,
            rr: rr as f32,
            shift: 0,
            rs: value(op(k, op::RS)),
            ls: 0.0,
            kvs: kvs as f32,
            ame: false,
            ebs: 0.0,
        };
    }
    patch
}

/// Loads factory voice `k` into the panel (every voice parameter; the
/// master section stays).
pub fn load_voice(p: &ModuleParams, k: usize) {
    let v = &VOICES[k % VOICES.len()];
    for i in 0..PARAM_COUNT {
        if matches!(i, VOLUME | MOD_WHEEL | BREATH | NOTES | MICRO | MICRO_KEY) {
            continue;
        }
        p.set(i, spec(i).default);
    }
    p.set(ALG, v.alg as f32);
    p.set(FEEDBACK, v.feedback as f32);
    for (k, o) in v.ops.iter().enumerate() {
        let (wave, crs, fine, out, ar, d1r, d1l, d2r, rr, kvs, det) = *o;
        for (f, val) in [
            (op::WAVE, wave as f32),
            (op::CRS, crs as f32),
            (op::FINE, fine as f32),
            (op::OUT, out as f32),
            (op::AR, ar as f32),
            (op::D1R, d1r as f32),
            (op::D1L, d1l as f32),
            (op::D2R, d2r as f32),
            (op::RR, rr as f32),
            (op::KVS, kvs as f32),
            (op::DET, det as f32),
        ] {
            p.set(op(k, f), val);
        }
    }
    for &(i, val) in v.extra {
        p.set(i, val);
    }
    p.data[data::VOICE].set(k as f32);
}

// ---------------------------------------------------------------------------
// The processor
// ---------------------------------------------------------------------------

const EVENTS: usize = 256;

/// The transposed delay's notes waiting their turn.
#[derive(Clone, Copy, Default)]
struct Pending {
    /// Frames to go.
    wait: u32,
    data: [u8; 3],
}
const PENDING: usize = 128;

#[derive(Clone, Copy, Default)]
struct Voice8 {
    fm: FmVoice,
    note: u8,
    /// The pitch now (portamento) and where it is going.
    pitch: f32,
    target: f32,
    vel: f32,
    pan: f32,
    started: u64,
    sustained: bool,
    /// Which key started it (chords and delays play several notes per key).
    key: u8,
}

pub struct Quartz {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    sr: f32,
    voices: [Voice8; MAX_VOICES],
    patch: FmPatch,
    lfo: Lfo,
    pan_lfo: f32,
    rng: Rng,
    clock: u64,
    held: Vec<u8>,
    pedal: bool,
    last_pitch: Option<f32>,
    wheel: f32,
    wheel_param: f32,
    breath: f32,
    breath_param: f32,
    bend: f32,
    porta_on: bool,
    /// This block's events, in frame order; `true` for the delay's own.
    events: Vec<(usize, [u8; 3], bool)>,
    pending: [Pending; PENDING],
    panel_key: u8,
    panic_seen: f32,
    mono: Box<[f32; SEG]>,
    dc: [(f32, f32); 2],
    volume: f32,
}

impl Quartz {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        Self {
            midi_cursor: midi.stream_head(),
            midi,
            sr: sample_rate,
            voices: [Voice8::default(); MAX_VOICES],
            patch: FmPatch::default(),
            lfo: Lfo::default(),
            pan_lfo: 0.0,
            rng: Rng::new(0x0A27_812A),
            clock: 0,
            held: Vec::with_capacity(32),
            pedal: false,
            last_pitch: None,
            wheel: params.get(MOD_WHEEL),
            wheel_param: params.get(MOD_WHEEL),
            breath: params.get(BREATH),
            breath_param: params.get(BREATH),
            bend: 0.0,
            porta_on: true,
            events: Vec::with_capacity(EVENTS),
            pending: [Pending::default(); PENDING],
            panel_key: 0,
            panic_seen: params.viz[viz::PANIC].get(),
            mono: Box::new([0.0; SEG]),
            dc: [(0.0, 0.0); 2],
            volume: 0.0,
            params,
        }
    }

    fn get(&self, i: usize) -> f32 {
        self.params.get(i)
    }

    fn geti(&self, i: usize) -> i32 {
        int_of(&self.params, i)
    }

    /// The knobs into the engine's patch.
    fn update_patch(&mut self) {
        for k in 0..OPS {
            let g = |f: usize| self.get(op(k, f));
            let gi = |f: usize| int_of(&self.params, op(k, f));
            let fixed = gi(op::FIXED) > 0;
            let (crs, fine) = (gi(op::CRS) as usize, gi(op::FINE) as usize);
            self.patch.ops[k] = OpPatch {
                on: gi(op::ON) > 0,
                fixed,
                freq: if fixed {
                    fm::fixed_hz(crs, fine, gi(op::RANGE) as usize)
                } else {
                    fm::ratio(crs, fine)
                },
                detune_cents: g(op::DET).round() * 0.87,
                wave: gi(op::WAVE) as usize,
                out: g(op::OUT).round(),
                ar: g(op::AR).round(),
                d1r: g(op::D1R).round(),
                d1l: g(op::D1L).round(),
                d2r: g(op::D2R).round(),
                rr: g(op::RR).round(),
                shift: gi(op::SHIFT) as usize,
                rs: g(op::RS).round(),
                ls: g(op::LS).round(),
                kvs: g(op::KVS).round(),
                ame: gi(op::AME) > 0,
                ebs: g(op::EBS).round(),
            };
        }
        self.patch.alg = self.geti(ALG) as usize;
        self.patch.feedback = self.geti(FEEDBACK) as usize;
        self.patch.reverb = self.geti(REVERB) as f32;
    }

    fn all_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.fm.kill();
            v.sustained = false;
        }
        self.held.clear();
        self.pending = [Pending::default(); PENDING];
    }

    fn schedule(&mut self, wait: u32, data: [u8; 3]) {
        if let Some(slot) = self.pending.iter_mut().find(|e| e.wait == 0) {
            *slot = Pending {
                wait: wait.max(1),
                data,
            };
        }
    }

    /// A key from MIDI, the panel or the delay: the effects first.
    fn key(&mut self, note: u8, vel: Option<f32>, delayed: bool) {
        let effect = self.geti(EFFECT);
        // The transposed delay replays the key, each time moved and quieter.
        if effect == 1 && !delayed {
            let step = ((self.geti(DLY_TIME) as f32 + 1.0) * 0.01 * self.sr) as u32;
            let shift = self.geti(DLY_SHIFT);
            let repeats = 1 + self.geti(DLY_FB) as u32;
            let level = self.get(DLY_LEVEL) / 99.0;
            for r in 1..=repeats {
                let n = note as i32 + shift * r as i32;
                if !(0..128).contains(&n) {
                    break;
                }
                let data = match vel {
                    Some(v) => {
                        let v = v * level.powi(r as i32);
                        if v < 0.02 {
                            break;
                        }
                        [0x90, n as u8, (v * 127.0).round().clamp(1.0, 127.0) as u8]
                    }
                    None => [0x80, n as u8, 0],
                };
                self.schedule(step * r, data);
            }
        }
        if effect == 3 {
            let (_, chord) = CHORDS[self.geti(CHORD).clamp(0, 7) as usize];
            for &iv in chord {
                let n = note as i32 + iv;
                if (0..128).contains(&n) {
                    match vel {
                        Some(v) => self.note_on(n as u8, v, note),
                        None => self.note_off(n as u8),
                    }
                }
            }
        } else {
            match vel {
                Some(v) => self.note_on(note, v, note),
                None => self.note_off(note),
            }
        }
    }

    fn note_pitch(&self, note: u8) -> f32 {
        let n = note as f32 + self.geti(TRANSPOSE) as f32;
        fm::micro_pitch(n, self.geti(MICRO) as usize, self.geti(MICRO_KEY) as usize)
    }

    fn pan_of(&mut self, note: u8, vel: f32) -> f32 {
        if self.geti(EFFECT) != 2 {
            return 0.0;
        }
        let range = self.get(PAN_RANGE) / 99.0;
        let x = match self.geti(PAN_SRC) {
            0 => self.pan_lfo,
            1 => vel * 2.0 - 1.0,
            _ => (note as f32 - 60.0) / 36.0,
        }
        .clamp(-1.0, 1.0);
        let x = match self.geti(PAN_DIR) {
            0 => x,
            1 => -x,
            _ => {
                // Alternating: each new note to the other side.
                if self.clock.is_multiple_of(2) { x.abs() } else { -x.abs() }
            }
        };
        x * range
    }

    fn note_on(&mut self, note: u8, vel: f32, key: u8) {
        self.clock += 1;
        if !self.held.contains(&note) {
            self.held.push(note);
        }
        let pitch = self.note_pitch(note);
        let mono = self.geti(MONO) > 0;
        let porta_time = self.get(PORTA_TIME);
        let fingered = self.geti(PORTA_MODE) > 0;
        let legato = self.voices.iter().any(|v| v.fm.gate());
        let glide = porta_time > 0.0
            && self.porta_on
            && (!fingered || (mono && legato))
            && self.last_pitch.is_some();
        if self.geti(LFO_SYNC) > 0 || !legato {
            self.lfo.restart(self.geti(LFO_SYNC) > 0);
        }
        let pan = self.pan_of(note, vel);
        let count = if mono {
            1
        } else {
            self.geti(NOTES).clamp(1, MAX_VOICES as i32) as usize
        };
        let patch = self.patch;
        let pick = if mono {
            0
        } else {
            let vs = &self.voices[..count];
            vs.iter()
                .position(|v| v.note == note && v.fm.active(&patch))
                .or_else(|| vs.iter().position(|v| !v.fm.active(&patch)))
                .or_else(|| {
                    (0..count)
                        .filter(|&i| !vs[i].fm.gate())
                        .min_by(|&a, &b| vs[a].fm.level(&patch).total_cmp(&vs[b].fm.level(&patch)))
                })
                .unwrap_or_else(|| (0..count).min_by_key(|&i| vs[i].started).unwrap_or(0))
        };
        let from = self.last_pitch.unwrap_or(pitch);
        let clock = self.clock;
        let v = &mut self.voices[pick];
        let was_gate = v.fm.gate();
        v.note = note;
        v.key = key;
        v.target = pitch;
        v.pitch = if glide {
            if mono && was_gate { v.pitch } else { from }
        } else {
            pitch
        };
        v.vel = vel;
        v.pan = pan;
        v.started = clock;
        v.sustained = false;
        // Mono legato with fingered portamento keeps the envelopes going.
        if !(mono && was_gate && fingered) {
            v.fm.key_on(pitch, vel, true);
        }
        self.last_pitch = Some(pitch);
    }

    fn note_off(&mut self, note: u8) {
        self.held.retain(|&n| n != note);
        let mono = self.geti(MONO) > 0;
        if mono {
            // Back to the last key still held.
            if let Some(&back) = self.held.last() {
                let pitch = self.note_pitch(back);
                let no_glide = self.get(PORTA_TIME) <= 0.0;
                let v = &mut self.voices[0];
                if v.note == note {
                    v.note = back;
                    v.target = pitch;
                    if no_glide {
                        v.pitch = pitch;
                    }
                    self.last_pitch = Some(pitch);
                }
                return;
            }
        }
        for v in self.voices.iter_mut() {
            if v.note == note && v.fm.gate() {
                if self.pedal {
                    v.sustained = true;
                } else {
                    v.fm.key_off();
                }
            }
        }
    }

    fn handle(&mut self, data: [u8; 3]) {
        let [status, d1, d2] = data;
        match status & 0xF0 {
            0x90 if d2 > 0 => self.key(d1 & 0x7F, Some(d2 as f32 / 127.0), false),
            0x80 | 0x90 => self.key(d1 & 0x7F, None, false),
            0xB0 => match d1 {
                1 => self.wheel = d2 as f32 / 127.0,
                2 => self.breath = d2 as f32 / 127.0,
                5 => self.params.set(PORTA_TIME, (d2 as f32 / 127.0 * 99.0).round()),
                7 => self.params.set(VOLUME, (d2 as f32 / 127.0 * 99.0).round()),
                64 => {
                    self.pedal = d2 >= 64;
                    if !self.pedal {
                        for v in self.voices.iter_mut().filter(|v| v.sustained) {
                            v.sustained = false;
                            v.fm.key_off();
                        }
                    }
                }
                65 => self.porta_on = d2 >= 64,
                120 => self.all_off(),
                123 => {
                    for v in self.voices.iter_mut() {
                        v.fm.key_off();
                    }
                    self.held.clear();
                }
                _ => {}
            },
            0xD0 => self.breath = d1 as f32 / 127.0,
            0xE0 => {
                let v = ((d2 as i32) << 7 | d1 as i32) - 8192;
                self.bend = (v as f32 / 8192.0).clamp(-1.0, 1.0);
            }
            0xC0 => load_voice(&self.params, d1 as usize % VOICES.len()),
            _ => {}
        }
    }

    fn handle_delayed(&mut self, data: [u8; 3]) {
        let [status, d1, d2] = data;
        if status & 0xF0 == 0x90 && d2 > 0 {
            self.key(d1, Some(d2 as f32 / 127.0), true);
        } else {
            self.key(d1, None, true);
        }
    }

    fn publish(&self) {
        let p = &self.params;
        let mut newest = None;
        for (i, v) in self.voices.iter().enumerate() {
            let active = v.fm.active(&self.patch);
            p.viz[viz::LEVEL + i].set(if active { v.fm.level(&self.patch) } else { 0.0 });
            p.viz[viz::NOTE + i].set(if active { v.note as f32 } else { -1.0 });
            p.viz[viz::GATE + i].set(if v.fm.gate() { 1.0 } else { 0.0 });
            if active && newest.is_none_or(|(s, _)| v.started > s) {
                newest = Some((v.started, i));
            }
        }
        for k in 0..OPS {
            let l = newest.map_or(0.0, |(_, i)| self.voices[i].fm.op_level(k));
            p.viz[viz::OP_LEVEL + k].set(l);
        }
        p.viz[viz::LFO].set(self.lfo.value);
    }
}

/// Pitch modulation sensitivity 0–7: the LFO's reach at full depth, in
/// semitones.
const PMS_SEMIS: [f32; 8] = [0.0, 0.08, 0.16, 0.32, 0.65, 1.3, 4.0, 7.0];
/// Amplitude modulation sensitivity 0–3: dB at full depth.
const AMS_DB: [f32; 4] = [0.0, 6.0, 16.0, 40.0];

impl Processor for Quartz {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        self.update_patch();
        let p = self.params.clone();
        let panic = p.viz[viz::PANIC].get();
        if panic != self.panic_seen {
            self.panic_seen = panic;
            self.all_off();
        }
        // The panel's wheel and breath when they move, else MIDI's.
        let w = p.get(MOD_WHEEL);
        if (w - self.wheel_param).abs() > 1e-6 {
            self.wheel = w;
            self.wheel_param = w;
        }
        let b = p.get(BREATH);
        if (b - self.breath_param).abs() > 1e-6 {
            self.breath = b;
            self.breath_param = b;
        }

        // This block's events: MIDI, the panel's key, the delay's notes.
        self.events.clear();
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                self.events.push((e.frame as usize, e.data, false));
            }
            self.midi_cursor = self.midi.stream_head();
        } else {
            let ev = &mut self.events;
            self.midi.read_stream(&mut self.midi_cursor, |m| ev.push((0, m, false)));
        }
        let panel_key = p.viz[viz::PANEL_KEY].get().round().clamp(0.0, 128.0) as u8;
        if panel_key != self.panel_key {
            if self.panel_key > 0 {
                self.events.push((0, [0x80, self.panel_key - 1, 0], false));
            }
            if panel_key > 0 {
                self.events.push((0, [0x90, panel_key - 1, 100], false));
            }
            self.panel_key = panel_key;
        }
        for e in self.pending.iter_mut().filter(|e| e.wait > 0) {
            if (e.wait as usize) <= n {
                self.events
                    .push(((e.wait as usize).saturating_sub(1), e.data, true));
                e.wait = 0;
            } else {
                e.wait -= n as u32;
            }
        }
        self.events.sort_by_key(|e| e.0);

        let mut ei = 0;
        let mut f = 0;
        let sr = self.sr;
        let porta = self.get(PORTA_TIME);
        // Portamento: semitones a second (99 ≈ 30 s for three octaves).
        let glide_rate = if porta > 0.0 {
            36.0 / (0.002 * fastmath::exp2(porta / 99.0 * 13.9))
        } else {
            f32::INFINITY
        };
        let pb = self.get(PB_RANGE).round();
        let lfo_wave = self.geti(LFO_WAVE) as usize;
        let (speed, delay) = (self.get(LFO_SPEED), self.get(LFO_DELAY));
        let pms = PMS_SEMIS[self.geti(PMS).clamp(0, 7) as usize];
        let ams = AMS_DB[self.geti(AMS).clamp(0, 3) as usize];
        let (pmd, amd) = (self.get(PMD) / 99.0, self.get(AMD) / 99.0);
        let (mwp, mwa) = (self.get(MW_PITCH) / 99.0, self.get(MW_AMP) / 99.0);
        let (bcp, bca) = (self.get(BC_PITCH) / 99.0, self.get(BC_AMP) / 99.0);
        let bc_bias = self.get(BC_PBIAS) / 50.0;
        let bc_eg = self.get(BC_EBIAS) / 99.0;
        let vol_t = {
            let v = self.get(VOLUME) / 99.0;
            v * v
        };
        let out_gain = 0.32;
        let dc_k = 1.0 - (-std::f32::consts::TAU * 12.0 / sr).exp();
        let mut peak = 0.0f32;
        while f < n {
            while ei < self.events.len() && self.events[ei].0 <= f {
                let (_, d, delayed) = self.events[ei];
                ei += 1;
                if delayed {
                    self.handle_delayed(d);
                } else {
                    self.handle(d);
                }
            }
            let next = self.events.get(ei).map_or(n, |e| e.0.max(f + 1));
            let m = (next - f).min(SEG).min(n - f);
            let dt = m as f32 / sr;

            // The LFO and what moves every voice.
            let rnd = self.rng.bipolar();
            let lfo = self.lfo.tick(lfo_wave, speed, delay, dt, rnd);
            self.pan_lfo = lfo;
            let pm_depth = (pmd + self.wheel * mwp + self.breath * bcp).min(1.0);
            let am_depth = (amd + self.wheel * mwa + self.breath * bca).min(1.0);
            let mods = Mods {
                pitch: self.bend * pb
                    + lfo * pms * pm_depth
                    + bc_bias * (1.0 - self.breath) * -2.0 * bcp.max(0.25),
                am_db: (lfo * 0.5 + 0.5) * am_depth * ams,
                eg_bias: (1.0 - self.breath) * bc_eg,
            };

            let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
            let patch = self.patch;
            for v in self.voices.iter_mut() {
                if !v.fm.active(&patch) {
                    continue;
                }
                let step = glide_rate * dt;
                let d = v.target - v.pitch;
                v.pitch = if d.abs() <= step { v.target } else { v.pitch + step * d.signum() };
                let buf = &mut self.mono[..m];
                buf.fill(0.0);
                v.fm.render(&patch, &mods, v.pitch, 1.0, sr, buf);
                let (gl, gr) = pan_gains(v.pan);
                let a = 0.35 + 0.65 * v.vel;
                for i in 0..m {
                    l[i] += buf[i] * gl * a;
                    r[i] += buf[i] * gr * a;
                }
            }
            for i in 0..m {
                self.volume += (vol_t - self.volume) * ctx.smooth;
                let g = self.volume * out_gain * std::f32::consts::SQRT_2;
                let (x, y) = (l[i] * g, r[i] * g);
                // AC coupling: the half waves carry DC.
                self.dc[0].0 += (x - self.dc[0].0) * dc_k;
                self.dc[1].0 += (y - self.dc[1].0) * dc_k;
                let yl = soft_limit(x - self.dc[0].0);
                let yr = soft_limit(y - self.dc[1].0);
                outputs[0][0][f + i] = yl;
                outputs[0][1][f + i] = yr;
                peak = peak.max(yl.abs()).max(yr.abs());
            }
            f += m;
        }
        p.meter.fetch_max(peak);
        self.publish();
    }
}
