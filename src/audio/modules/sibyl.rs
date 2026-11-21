//! Sibyl: a sixteen-voice analog polysynth after the Sequential Prophet-5
//! (rev 3) and u-he's Repro-5, its circuit-minded software successor.
//!
//! Every voice is the classic signal path ([`voice`]): two oscillators (A:
//! saw and pulse with hard sync; B: saw, triangle and pulse, LO FREQ and
//! keyboard tracking), a mixer with noise (or feedback from behind the
//! amplifier), a four-pole low-pass of the CEM3320 kind that saturates and
//! self-oscillates, and two CEM3310-style envelopes for the filter and the
//! amplifier. POLY-MOD sends each voice's filter envelope and oscillator B to
//! oscillator A's pitch and pulse width and to the cutoff, at audio rate
//! (sync sweeps, metallic FM, growls); WHEEL-MOD mixes the global LFO with
//! noise under the mod wheel's limits. Voices drift and differ like real
//! ones (VOICE DETUNE, the oscillator and envelope models); UNISON stacks
//! them on one note with a note priority and GLIDE.
//!
//! As in Repro-5 there is more behind the panel: the TWEAKS (jumpers: LFO
//! DC and inversion, S&H wheel noise, inverted saw, note priority, voice
//! reallocation, mixer feedback, inverted filter envelope, cutoff following
//! the bend; selectors: oscillator, filter and envelope models, the bend's
//! range; a pan per voice), a four-slot modulation matrix with curve,
//! rectifier, quantizer, sample & hold and slew ([`mods`]), a distortion
//! per voice and a chain of five reorderable effects ([`fx`]): tape
//! saturation, a bucket-brigade delay, a resonator/EQ, a plate and a
//! conditioner. Ours on top: up to sixteen voices, an arpeggiator on the
//! board's clock ([`arp`]), and forty programs in five banks of eight, as on
//! the original, filled with our own sounds ([`bank`]) and overwritten with
//! RECORD.
//!
//! Performance: voices are worked out one at a time over short segments
//! (at most 16 frames, split at every MIDI event), so everything slow
//! (envelopes, glide, the matrix, pitches, the filter's base cutoff) is
//! computed once per segment and ramped, and only the oscillators, the
//! filter and the amplifier run per sample. Idle voices cost nothing. HQ runs
//! the voices at twice the rate.
//!
//! An independent instrument written for the board: the same way of working
//! and the same controls, its own engine, sounds and names.

pub mod arp;
pub mod bank;
pub mod fx;
pub mod mods;
pub mod voice;

use std::sync::Arc;

use self::arp::Arp;
use self::fx::Chain;
use self::mods::{Lfo, Matrix, SLOTS};
use self::voice::{FilterTable, Patch, Voice};
use super::oversample::Oversampler;
use super::{ModuleDef, ModuleParams, Rng, silence, soft_limit};
use crate::audio::engine::{BLOCK, Buf, Inputs, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit};

/// Most voices.
pub const MAX_VOICES: usize = 16;
/// Programs: banks of eight, as on the original (five there; a sixth here).
pub const BANKS: usize = 6;
pub const PROGRAMS: usize = BANKS * 8;

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

// POLY-MOD (VOICE MOD): the filter envelope and oscillator B, to three places.
pub const PM_FENV: usize = 0;
pub const PM_OSCB: usize = 1;
pub const PM_FREQ_A: usize = 2;
pub const PM_PW_A: usize = 3;
pub const PM_FILTER: usize = 4;
// LFO.
pub const LFO_RATE: usize = 5;
pub const LFO_SYNC: usize = 6;
pub const LFO_DIV: usize = 7;
pub const LFO_SAW: usize = 8;
pub const LFO_TRI: usize = 9;
pub const LFO_SQR: usize = 10;
// WHEEL-MOD: LFO and noise mixed, to five places, under the mod wheel.
pub const WM_MIX: usize = 11;
pub const WM_FREQ_A: usize = 12;
pub const WM_FREQ_B: usize = 13;
pub const WM_PW_A: usize = 14;
pub const WM_PW_B: usize = 15;
pub const WM_FILTER: usize = 16;
// OSCILLATOR A.
pub const A_OCT: usize = 17;
pub const A_FREQ: usize = 18;
pub const A_FINE: usize = 19;
pub const A_SAW: usize = 20;
pub const A_PULSE: usize = 21;
pub const A_PW: usize = 22;
pub const A_SYNC: usize = 23;
// OSCILLATOR B.
pub const B_OCT: usize = 24;
pub const B_FREQ: usize = 25;
pub const B_FINE: usize = 26;
pub const B_SAW: usize = 27;
pub const B_TRI: usize = 28;
pub const B_PULSE: usize = 29;
pub const B_PW: usize = 30;
pub const B_LO: usize = 31;
pub const B_KBD: usize = 32;
// MIXER.
pub const MIX_A: usize = 33;
pub const MIX_B: usize = 34;
pub const MIX_NOISE: usize = 35;
// FILTER.
pub const CUTOFF: usize = 36;
pub const RESO: usize = 37;
pub const ENV_AMT: usize = 38;
pub const KEY_AMT: usize = 39;
pub const FENV_VEL: usize = 40;
pub const F_ATTACK: usize = 41;
pub const F_DECAY: usize = 42;
pub const F_SUSTAIN: usize = 43;
pub const F_RELEASE: usize = 44;
// AMPLIFIER.
pub const A_ATTACK: usize = 45;
pub const A_DECAY: usize = 46;
pub const A_SUSTAIN: usize = 47;
pub const A_RELEASE: usize = 48;
pub const AENV_VEL: usize = 49;
// Unison, glide, voices, master.
pub const UNISON: usize = 50;
pub const GLIDE: usize = 51;
pub const DETUNE: usize = 52;
pub const VOICES: usize = 53;
pub const UNI_VOICES: usize = 54;
pub const TUNE: usize = 55;
pub const VOLUME: usize = 56;
pub const OUTPUT: usize = 57;
pub const HQ: usize = 58;
pub const A440: usize = 59;
// Wheels.
pub const MOD_WHEEL: usize = 60;
pub const WM_LOW: usize = 61;
pub const WM_HIGH: usize = 62;
pub const PB_UP: usize = 63;
pub const PB_DOWN: usize = 64;
// TWEAKS: jumpers.
pub const LFO_DC: usize = 65;
pub const LFO_INV: usize = 66;
pub const WM_SH: usize = 67;
pub const B_INV: usize = 68;
pub const NOTE_PRIO: usize = 69;
pub const REALLOC: usize = 70;
pub const MIX_FB: usize = 71;
pub const FENV_INV: usize = 72;
pub const KEY_PB: usize = 73;
// TWEAKS: selectors.
pub const A_MODEL: usize = 74;
pub const B_MODEL: usize = 75;
pub const FILTER_MODEL: usize = 76;
pub const ENV_MODEL: usize = 77;
/// A pan per voice: `PAN + v`.
pub const PAN: usize = 78;
// DISTORTION (one per voice).
pub const DIST_ON: usize = PAN + MAX_VOICES;
pub const DIST_TYPE: usize = DIST_ON + 1;
pub const DIST_AMOUNT: usize = DIST_ON + 2;
pub const DIST_TONE: usize = DIST_ON + 3;
pub const DIST_MIX: usize = DIST_ON + 4;
// TAPE.
pub const TAPE_ON: usize = DIST_ON + 5;
pub const TAPE_FLAVOR: usize = TAPE_ON + 1;
pub const TAPE_GAIN: usize = TAPE_ON + 2;
pub const TAPE_HUSH: usize = TAPE_ON + 3;
// ECHO (bucket brigade).
pub const ECHO_ON: usize = TAPE_ON + 4;
pub const ECHO_RANGE: usize = ECHO_ON + 1;
pub const ECHO_FLAVOR: usize = ECHO_ON + 2;
pub const ECHO_MODE: usize = ECHO_ON + 3;
pub const ECHO_MOD: usize = ECHO_ON + 4;
pub const ECHO_TIME: usize = ECHO_ON + 5;
pub const ECHO_MIX: usize = ECHO_ON + 6;
pub const ECHO_REGEN: usize = ECHO_ON + 7;
// RES·EQ: resonator or equalizer.
pub const RESEQ_ON: usize = ECHO_ON + 8;
pub const RESEQ_MODE: usize = RESEQ_ON + 1;
pub const RESEQ_LOW_F: usize = RESEQ_ON + 2;
pub const RESEQ_LOW: usize = RESEQ_ON + 3;
pub const RESEQ_MID_F: usize = RESEQ_ON + 4;
pub const RESEQ_MID: usize = RESEQ_ON + 5;
pub const RESEQ_HIGH_F: usize = RESEQ_ON + 6;
pub const RESEQ_HIGH: usize = RESEQ_ON + 7;
pub const RESEQ_Q: usize = RESEQ_ON + 8;
// PLATE reverb.
pub const PLATE_ON: usize = RESEQ_ON + 9;
pub const PLATE_PRE: usize = PLATE_ON + 1;
pub const PLATE_DECAY: usize = PLATE_ON + 2;
pub const PLATE_TONE: usize = PLATE_ON + 3;
pub const PLATE_MIX: usize = PLATE_ON + 4;
// CONDITIONER.
pub const COND_ON: usize = PLATE_ON + 5;
pub const COND_GAIN: usize = COND_ON + 1;
pub const COND_PUNCH: usize = COND_ON + 2;
pub const COND_WIDTH: usize = COND_ON + 3;
// ARPEGGIATOR.
pub const ARP_ON: usize = COND_ON + 4;
pub const ARP_MODE: usize = ARP_ON + 1;
pub const ARP_RATE: usize = ARP_ON + 2;
pub const ARP_OCT: usize = ARP_ON + 3;
pub const ARP_GATE: usize = ARP_ON + 4;
pub const ARP_SWING: usize = ARP_ON + 5;
pub const ARP_LATCH: usize = ARP_ON + 6;
/// The matrix: [`SLOT_LEN`] parameters a slot from `MATRIX + k * SLOT_LEN`.
pub const MATRIX: usize = ARP_ON + 7;
pub const SLOT_LEN: usize = 8;
pub mod slot {
    pub const SOURCE: usize = 0;
    pub const DEST: usize = 1;
    pub const DEPTH: usize = 2;
    pub const CURVE: usize = 3;
    pub const RECTIFY: usize = 4;
    pub const QUANTIZE: usize = 5;
    pub const SH: usize = 6;
    pub const SLEW: usize = 7;
}
/// The controller numbers read by the Control A and Control B sources.
pub const CC_A: usize = MATRIX + SLOTS * SLOT_LEN;
pub const CC_B: usize = CC_A + 1;
pub const PARAM_COUNT: usize = CC_B + 1;

/// A matrix slot's parameter.
pub const fn sp(k: usize, field: usize) -> usize {
    MATRIX + k * SLOT_LEN + field
}

const fn sw(name: &'static str, on: bool) -> ParamSpec {
    ParamSpec::lin(name, 0.0, 1.0, if on { 1.0 } else { 0.0 }, Unit::Integer)
}

const fn sel(name: &'static str, count: usize, default: usize) -> ParamSpec {
    ParamSpec::lin(name, 0.0, (count - 1) as f32, default as f32, Unit::Integer)
}

const fn pct(name: &'static str, default: f32) -> ParamSpec {
    ParamSpec::lin(name, 0.0, 1.0, default, Unit::Percent)
}

const fn time(name: &'static str, max: f32, default: f32) -> ParamSpec {
    ParamSpec::log(name, 0.002, max, default, Unit::Seconds)
}

/// The LFO's synced periods, slowest first (in beats), and their names.
pub const LFO_DIVS: [(&str, f64); 19] = [
    ("8/1", 32.0),
    ("4/1", 16.0),
    ("2/1", 8.0),
    ("1/1", 4.0),
    ("1/2D", 3.0),
    ("1/2", 2.0),
    ("1/2T", 4.0 / 3.0),
    ("1/4D", 1.5),
    ("1/4", 1.0),
    ("1/4T", 2.0 / 3.0),
    ("1/8D", 0.75),
    ("1/8", 0.5),
    ("1/8T", 1.0 / 3.0),
    ("1/16D", 0.375),
    ("1/16", 0.25),
    ("1/16T", 1.0 / 6.0),
    ("1/32", 0.125),
    ("1/32T", 1.0 / 12.0),
    ("1/64", 0.0625),
];

/// The bend's ranges: 0 to 24 semitones, then three and four octaves.
pub const BEND_RANGES: usize = 27;
pub fn bend_semitones(index: f32) -> f32 {
    match index.round() as i32 {
        i @ ..=24 => i.max(0) as f32,
        25 => 36.0,
        _ => 48.0,
    }
}

pub const NOTE_PRIOS: [&str; 3] = ["LOW", "HIGH", "LAST"];
pub const A_MODELS: [&str; 4] = ["P5", "P1", "IDEAL", "P5 OLD"];
pub const B_MODELS: [&str; 5] = ["P5", "P1", "IDEAL", "P5 OLD", "BOTTOM"];
pub const FILTER_MODELS: [&str; 4] = ["CRISPY", "ROUNDED", "DRIVEN", "POLY"];
pub const ENV_MODELS: [&str; 6] = [
    "IDEAL", "ANALOG", "HIGH SUS", "ONE SHOT", "PIANO 1", "PIANO 2",
];
pub const DIST_TYPES: [&str; 4] = ["SOFT CLIP", "HARD CLIP", "FOLDBACK", "CORRODE"];
pub const TAPE_FLAVORS: [&str; 4] = ["SMOOTH", "WARM", "DRIVE", "WORN"];
pub const ECHO_RANGES: [&str; 4] = ["CHORUS", "LONG", "SYNC 1/16", "SYNC 1/4"];
pub const ECHO_FLAVORS: [&str; 3] = ["CLEAN", "BRIGHT", "DARK"];
pub const ECHO_MODES: [&str; 4] = ["ECHO", "PPONG", "SWING", "GROOVE"];
pub const ECHO_MODS: [&str; 4] = ["OFF", "MIN", "MED", "MAX"];
pub const ARP_MODES: [&str; 7] = ["UP", "DOWN", "UP/DN", "UP+DN", "ORDER", "RANDOM", "CHORD"];
/// The arpeggiator's note values (beats).
pub const ARP_RATES: [(&str, f64); 8] = [
    ("1/4", 1.0),
    ("1/8D", 0.75),
    ("1/4T", 2.0 / 3.0),
    ("1/8", 0.5),
    ("1/8T", 1.0 / 3.0),
    ("1/16", 0.25),
    ("1/16T", 1.0 / 6.0),
    ("1/32", 0.125),
];

/// A gentle spread to start from: voices alternate left and right, wider
/// as they go.
const fn pan_default(v: usize) -> f32 {
    let side = if v.is_multiple_of(2) { -1.0 } else { 1.0 };
    side * (0.12 + 0.05 * (v / 2) as f32)
}

const PARAMS: [ParamSpec; PARAM_COUNT] = {
    let mut p = [pct("", 0.0); PARAM_COUNT];
    p[PM_FENV] = pct("PM FILT ENV", 0.0);
    p[PM_OSCB] = pct("PM OSC B", 0.0);
    p[PM_FREQ_A] = sw("PM FREQ A", false);
    p[PM_PW_A] = sw("PM PW A", false);
    p[PM_FILTER] = sw("PM FILTER", false);
    p[LFO_RATE] = ParamSpec::log("LFO RATE", 0.03, 27.5, 4.0, Unit::Hz);
    p[LFO_SYNC] = sw("LFO SYNC", false);
    p[LFO_DIV] = sel("LFO DIV", LFO_DIVS.len(), 8);
    p[LFO_SAW] = sw("LFO SAW", false);
    p[LFO_TRI] = sw("LFO TRI", true);
    p[LFO_SQR] = sw("LFO SQUARE", false);
    p[WM_MIX] = pct("WM SOURCE MIX", 0.0);
    p[WM_FREQ_A] = sw("WM FREQ A", true);
    p[WM_FREQ_B] = sw("WM FREQ B", true);
    p[WM_PW_A] = sw("WM PW A", false);
    p[WM_PW_B] = sw("WM PW B", false);
    p[WM_FILTER] = sw("WM FILTER", false);
    p[A_OCT] = sel("OSC A OCTAVE", 4, 1);
    p[A_FREQ] = ParamSpec::lin("OSC A FREQ", -12.0, 12.0, 0.0, Unit::Semitones).bipolar();
    p[A_FINE] = ParamSpec::lin("OSC A FINE", -20.0, 20.0, 0.0, Unit::Cents).bipolar();
    p[A_SAW] = sw("OSC A SAW", true);
    p[A_PULSE] = sw("OSC A PULSE", false);
    p[A_PW] = pct("OSC A PW", 0.5);
    p[A_SYNC] = sw("OSC A SYNC", false);
    p[B_OCT] = sel("OSC B OCTAVE", 4, 1);
    p[B_FREQ] = ParamSpec::lin("OSC B FREQ", -12.0, 12.0, 0.0, Unit::Semitones).bipolar();
    p[B_FINE] = ParamSpec::lin("OSC B FINE", -100.0, 100.0, 7.0, Unit::Cents).bipolar();
    p[B_SAW] = sw("OSC B SAW", true);
    p[B_TRI] = sw("OSC B TRI", false);
    p[B_PULSE] = sw("OSC B PULSE", false);
    p[B_PW] = pct("OSC B PW", 0.5);
    p[B_LO] = sw("OSC B LO FREQ", false);
    p[B_KBD] = sw("OSC B KEYBOARD", true);
    p[MIX_A] = pct("MIX OSC A", 0.8);
    p[MIX_B] = pct("MIX OSC B", 0.65);
    p[MIX_NOISE] = pct("MIX NOISE", 0.0);
    p[CUTOFF] = ParamSpec::log("CUTOFF", 10.0, 20_000.0, 1800.0, Unit::Hz);
    p[RESO] = pct("RESONANCE", 0.12);
    p[ENV_AMT] = pct("ENV AMOUNT", 0.35);
    p[KEY_AMT] = ParamSpec::lin("KEY AMOUNT", 0.0, 1.33, 0.5, Unit::Percent);
    p[FENV_VEL] = pct("FILT ENV VEL", 0.0);
    p[F_ATTACK] = time("FILT ATTACK", 15.0, 0.002);
    p[F_DECAY] = time("FILT DECAY", 15.0, 0.7);
    p[F_SUSTAIN] = pct("FILT SUSTAIN", 0.3);
    p[F_RELEASE] = time("FILT RELEASE", 55.0, 0.5);
    p[A_ATTACK] = time("AMP ATTACK", 15.0, 0.003);
    p[A_DECAY] = time("AMP DECAY", 15.0, 0.7);
    p[A_SUSTAIN] = pct("AMP SUSTAIN", 0.8);
    p[A_RELEASE] = time("AMP RELEASE", 55.0, 0.45);
    p[AENV_VEL] = pct("AMP ENV VEL", 0.25);
    p[UNISON] = sw("UNISON", false);
    p[GLIDE] = pct("GLIDE", 0.0);
    p[DETUNE] = pct("VOICE DETUNE", 0.25);
    p[VOICES] = ParamSpec::lin(
        "VOICES",
        1.0,
        MAX_VOICES as f32,
        MAX_VOICES as f32,
        Unit::Integer,
    );
    p[UNI_VOICES] = ParamSpec::lin("UNISON VOICES", 1.0, MAX_VOICES as f32, 6.0, Unit::Integer);
    p[TUNE] = ParamSpec::lin("TUNE", -12.0, 12.0, 0.0, Unit::Semitones).bipolar();
    p[VOLUME] = pct("VOLUME", 0.7);
    p[OUTPUT] = ParamSpec::lin("OUTPUT", 0.0, 2.0, 1.0, Unit::Percent);
    p[HQ] = sw("HQ", false);
    p[A440] = sw("A-440", false);
    p[MOD_WHEEL] = pct("MOD WHEEL", 0.0);
    p[WM_LOW] = pct("WHEEL LOW LIMIT", 0.0);
    p[WM_HIGH] = pct("WHEEL HIGH LIMIT", 1.0);
    p[PB_UP] = sel("BEND UP", BEND_RANGES, 2);
    p[PB_DOWN] = sel("BEND DOWN", BEND_RANGES, 2);
    p[LFO_DC] = sw("J LFO DC", false);
    p[LFO_INV] = sw("J LFO INVERT", false);
    p[WM_SH] = sw("J WHEEL S&H", false);
    p[B_INV] = sw("J OSC B INVERT", false);
    p[NOTE_PRIO] = sel("J NOTE PRIORITY", NOTE_PRIOS.len(), 0);
    p[REALLOC] = sw("J REALLOCATE", true);
    p[MIX_FB] = sw("J MIXER FEEDBACK", false);
    p[FENV_INV] = sw("J FILT ENV INVERT", false);
    p[KEY_PB] = sw("J KEY + BEND", false);
    p[A_MODEL] = sel("OSC A MODEL", A_MODELS.len(), 0);
    p[B_MODEL] = sel("OSC B MODEL", B_MODELS.len(), 0);
    p[FILTER_MODEL] = sel("FILTER MODEL", FILTER_MODELS.len(), 3);
    p[ENV_MODEL] = sel("ENV MODEL", ENV_MODELS.len(), 1);
    const PAN_NAMES: [&str; MAX_VOICES] = [
        "PAN 1", "PAN 2", "PAN 3", "PAN 4", "PAN 5", "PAN 6", "PAN 7", "PAN 8", "PAN 9", "PAN 10",
        "PAN 11", "PAN 12", "PAN 13", "PAN 14", "PAN 15", "PAN 16",
    ];
    let mut v = 0;
    while v < MAX_VOICES {
        p[PAN + v] = ParamSpec::lin(PAN_NAMES[v], -1.0, 1.0, pan_default(v), Unit::Plain).bipolar();
        v += 1;
    }
    p[DIST_ON] = sw("DIST ON", false);
    p[DIST_TYPE] = sel("DIST TYPE", DIST_TYPES.len(), 0);
    p[DIST_AMOUNT] = ParamSpec::lin("DIST AMOUNT", 0.0, 60.0, 30.0, Unit::Db);
    p[DIST_TONE] = ParamSpec::lin("DIST TONE", -1.0, 1.0, 0.0, Unit::Plain).bipolar();
    p[DIST_MIX] = pct("DIST MIX", 1.0);
    p[TAPE_ON] = sw("TAPE ON", false);
    p[TAPE_FLAVOR] = sel("TAPE FLAVOR", TAPE_FLAVORS.len(), 0);
    p[TAPE_GAIN] = ParamSpec::lin("TAPE GAIN", -12.0, 24.0, 0.0, Unit::Db).bipolar();
    p[TAPE_HUSH] = sw("TAPE HISS MUTE", false);
    p[ECHO_ON] = sw("ECHO ON", false);
    p[ECHO_RANGE] = sel("ECHO RANGE", ECHO_RANGES.len(), 2);
    p[ECHO_FLAVOR] = sel("ECHO FLAVOR", ECHO_FLAVORS.len(), 0);
    p[ECHO_MODE] = sel("ECHO MODE", ECHO_MODES.len(), 1);
    p[ECHO_MOD] = sel("ECHO MODULATION", ECHO_MODS.len(), 1);
    p[ECHO_TIME] = pct("ECHO TIME", 0.43);
    p[ECHO_MIX] = pct("ECHO MIX", 0.25);
    p[ECHO_REGEN] = pct("ECHO REGEN", 0.4);
    p[RESEQ_ON] = sw("RESEQ ON", false);
    p[RESEQ_MODE] = sel("RESEQ MODE", 2, 0);
    p[RESEQ_LOW_F] = ParamSpec::log("RESEQ LOW FREQ", 45.0, 3000.0, 120.0, Unit::Hz);
    p[RESEQ_LOW] = pct("RESEQ LOW", 0.5).bipolar();
    p[RESEQ_MID_F] = ParamSpec::log("RESEQ MID FREQ", 55.0, 9000.0, 900.0, Unit::Hz);
    p[RESEQ_MID] = pct("RESEQ MID", 0.5).bipolar();
    p[RESEQ_HIGH_F] = ParamSpec::log("RESEQ HIGH FREQ", 130.0, 10_000.0, 4000.0, Unit::Hz);
    p[RESEQ_HIGH] = pct("RESEQ HIGH", 0.5).bipolar();
    p[RESEQ_Q] = pct("RESEQ Q", 0.3);
    p[PLATE_ON] = sw("PLATE ON", false);
    p[PLATE_PRE] = ParamSpec::lin("PLATE PREDELAY", 0.0, 0.25, 0.02, Unit::Seconds);
    p[PLATE_DECAY] = pct("PLATE DECAY", 0.55);
    p[PLATE_TONE] = ParamSpec::lin("PLATE TONE", -1.0, 1.0, 0.0, Unit::Plain).bipolar();
    p[PLATE_MIX] = pct("PLATE MIX", 0.25);
    p[COND_ON] = sw("COND ON", false);
    p[COND_GAIN] = ParamSpec::lin("COND GAIN", -1.0, 1.0, 0.0, Unit::Plain).bipolar();
    p[COND_PUNCH] = ParamSpec::lin("COND TRANSIENT", -1.0, 1.0, 0.0, Unit::Plain).bipolar();
    p[COND_WIDTH] = ParamSpec::lin("COND WIDTH", 0.0, 2.0, 1.0, Unit::Percent);
    p[ARP_ON] = sw("ARP ON", false);
    p[ARP_MODE] = sel("ARP MODE", ARP_MODES.len(), 0);
    p[ARP_RATE] = sel("ARP RATE", ARP_RATES.len(), 5);
    p[ARP_OCT] = ParamSpec::lin("ARP OCTAVES", 1.0, 4.0, 1.0, Unit::Integer);
    p[ARP_GATE] = ParamSpec::lin("ARP GATE", 0.05, 1.0, 0.5, Unit::Percent);
    p[ARP_SWING] = ParamSpec::lin("ARP SWING", 0.0, 0.75, 0.0, Unit::Percent);
    p[ARP_LATCH] = sw("ARP LATCH", false);
    const SLOT_NAMES: [[&str; SLOT_LEN]; SLOTS] = [
        [
            "MM1 SOURCE",
            "MM1 DEST",
            "MM1 DEPTH",
            "MM1 CURVE",
            "MM1 RECTIFY",
            "MM1 QUANTIZE",
            "MM1 S&H",
            "MM1 SLEW",
        ],
        [
            "MM2 SOURCE",
            "MM2 DEST",
            "MM2 DEPTH",
            "MM2 CURVE",
            "MM2 RECTIFY",
            "MM2 QUANTIZE",
            "MM2 S&H",
            "MM2 SLEW",
        ],
        [
            "MM3 SOURCE",
            "MM3 DEST",
            "MM3 DEPTH",
            "MM3 CURVE",
            "MM3 RECTIFY",
            "MM3 QUANTIZE",
            "MM3 S&H",
            "MM3 SLEW",
        ],
        [
            "MM4 SOURCE",
            "MM4 DEST",
            "MM4 DEPTH",
            "MM4 CURVE",
            "MM4 RECTIFY",
            "MM4 QUANTIZE",
            "MM4 S&H",
            "MM4 SLEW",
        ],
    ];
    let mut k = 0;
    while k < SLOTS {
        let n = SLOT_NAMES[k];
        p[sp(k, slot::SOURCE)] = sel(n[0], mods::SOURCES.len(), 0);
        p[sp(k, slot::DEST)] = sel(n[1], mods::DESTS.len(), 0);
        p[sp(k, slot::DEPTH)] = ParamSpec::lin(n[2], -1.0, 1.0, 0.0, Unit::Percent).bipolar();
        p[sp(k, slot::CURVE)] = sel(n[3], mods::CURVES.len(), 0);
        p[sp(k, slot::RECTIFY)] = sel(n[4], mods::RECTIFIERS.len(), 0);
        p[sp(k, slot::QUANTIZE)] = sel(n[5], mods::QUANTIZERS.len(), 0);
        p[sp(k, slot::SH)] = sel(n[6], mods::SH_SOURCES.len(), 0);
        p[sp(k, slot::SLEW)] = sel(n[7], mods::SLEWS.len(), 0);
        k += 1;
    }
    p[CC_A] = ParamSpec::lin("CONTROL A CC", 0.0, 127.0, 2.0, Unit::Integer);
    p[CC_B] = ParamSpec::lin("CONTROL B CC", 0.0, 127.0, 11.0, Unit::Integer);
    p
};

/// Kept with the panel, not with a program: the master section, the bend's
/// controllers and the CPU-related settings.
pub fn is_global(k: usize) -> bool {
    matches!(
        k,
        TUNE | OUTPUT | HQ | A440 | MOD_WHEEL | VOICES | CC_A | CC_B
    )
}

/// Switches and selectors (whole numbers).
pub fn is_switch(k: usize) -> bool {
    DEF.params[k].unit == Unit::Integer
}

/// Snaps a switch or selector to a whole number, keeps the rest.
pub fn snap(k: usize, v: f32) -> f32 {
    let s = &DEF.params[k];
    let v = v.clamp(s.min, s.max);
    if is_switch(k) { v.round() } else { v }
}

// ---------------------------------------------------------------------------
// Extra settings and display values
// ---------------------------------------------------------------------------

pub mod data {
    use super::{PARAM_COUNT, PROGRAMS};
    /// The lower panel: 0 keys and arpeggiator, 1 effects.
    pub const LOWER: usize = 0;
    /// The upper panel: 0 the synth, 1 the tweaks.
    pub const VIEW: usize = 1;
    /// The program last loaded or stored.
    pub const PROGRAM: usize = 2;
    /// The matrix page shown: 0 slots 1–2, 1 slots 3–4.
    pub const MM_PAGE: usize = 3;
    /// The effects chain's order: five indices into [`super::fx::EFFECTS`].
    pub const FX_ORDER: usize = 4;
    /// 1: the program was recorded over (its values are in `STORE`).
    pub const USER: usize = FX_ORDER + 5;
    /// The recorded programs: every parameter, `STORE + k * PARAM_COUNT`.
    pub const STORE: usize = USER + PROGRAMS;
    /// The sound's name, a character a slot ([`super::NAME_LEN`], 0 ends it).
    pub const NAME: usize = STORE + PROGRAMS * PARAM_COUNT;
    /// The recorded programs' names: `PROG_NAME + k * NAME_LEN`.
    pub const PROG_NAME: usize = NAME + super::NAME_LEN;
    /// The recorded programs' effects order: `PROG_FX + k * 5`.
    pub const PROG_FX: usize = PROG_NAME + PROGRAMS * super::NAME_LEN;
    pub const LEN: usize = PROG_FX + PROGRAMS * 5;
}

/// Characters a name keeps.
pub const NAME_LEN: usize = 24;

static DATA: [f32; data::LEN] = {
    let mut d = [0.0; data::LEN];
    let mut i = 0;
    while i < 5 {
        d[data::FX_ORDER + i] = i as f32;
        i += 1;
    }
    d
};

pub mod viz {
    use super::MAX_VOICES;
    /// Each voice's amplifier level.
    pub const LEVEL: usize = 0;
    /// Each voice's note, or −1 while idle.
    pub const NOTE: usize = LEVEL + MAX_VOICES;
    /// 1 while the voice's key (or the pedal) holds it.
    pub const GATE: usize = NOTE + MAX_VOICES;
    /// The LFO, −1..1.
    pub const LFO: usize = GATE + MAX_VOICES;
    /// The mod wheel and the bend as they are now (MIDI or panel).
    pub const WHEEL: usize = LFO + 1;
    pub const BEND: usize = WHEEL + 1;
    /// Written by the panel: the key its keyboard holds + 1 (0: none).
    pub const PANEL_KEY: usize = BEND + 1;
    /// Written by the panel: the bend wheel while dragged, −1..1, else 0.
    pub const PANEL_BEND: usize = PANEL_KEY + 1;
    /// The arpeggiator's note, or −1.
    pub const ARP_NOTE: usize = PANEL_BEND + 1;
    /// How hard the conditioner saturates, 0..1.
    pub const COND_LOAD: usize = ARP_NOTE + 1;
    /// Keys held (or latched by the arpeggiator).
    pub const HELD: usize = COND_LOAD + 1;
    /// Bumped by the panel to let go of the arpeggiator's latched keys.
    pub const UNLATCH: usize = HELD + 1;
    /// Bumped by the panel to silence every voice at once (panic).
    pub const PANIC: usize = UNLATCH + 1;
    pub const LEN: usize = PANIC + 1;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Sibyl",
    prefix: "SIBYL",
    hint: "16-voice analog polysynth · poly-mod · arp",
    outputs: &["OUT"],
    params: &PARAMS,
    midi_in: true,
    data: &DATA,
    viz: viz::LEN,
    ..ModuleDef::EMPTY
};

// ---------------------------------------------------------------------------
// Programs
// ---------------------------------------------------------------------------

/// Program `k`'s value for parameter `i`: what RECORD stored there, else the
/// factory's.
pub fn program_value(p: &ModuleParams, k: usize, i: usize) -> f32 {
    let k = k.min(PROGRAMS - 1);
    if p.data[data::USER + k].get() > 0.5 {
        p.data[data::STORE + k * PARAM_COUNT + i].get()
    } else {
        bank::value(k, i)
    }
}

/// Loads program `k` into the panel (the master section stays).
pub fn load_program(p: &ModuleParams, k: usize) {
    let k = k.min(PROGRAMS - 1);
    for i in (0..PARAM_COUNT).filter(|&i| !is_global(i)) {
        p.set(i, program_value(p, k, i));
    }
    let user = p.data[data::USER + k].get() > 0.5;
    let order = if user { std::array::from_fn(|n| p.data[data::PROG_FX + k * 5 + n].get().round() as usize) } else { bank::fx_order(k) };
    for (n, &e) in order.iter().enumerate() {
        p.data[data::FX_ORDER + n].set(e as f32);
    }
    p.data[data::PROGRAM].set(k as f32);
    set_sound_name(p, &program_name(p, k));
}

/// Stores the panel, with its name and effects order, as program `k` (RECORD).
pub fn store_program(p: &ModuleParams, k: usize) {
    let k = k.min(PROGRAMS - 1);
    for i in 0..PARAM_COUNT {
        p.data[data::STORE + k * PARAM_COUNT + i].set(p.get(i));
    }
    for (n, e) in fx_order(p).into_iter().enumerate() {
        p.data[data::PROG_FX + k * 5 + n].set(e as f32);
    }
    let name = sound_name(p);
    write_name(p, data::PROG_NAME + k * NAME_LEN, &name);
    p.data[data::USER + k].set(1.0);
    p.data[data::PROGRAM].set(k as f32);
}

/// Puts the factory's sound back in slot `k`.
pub fn restore_program(p: &ModuleParams, k: usize) {
    let k = k.min(PROGRAMS - 1);
    p.data[data::USER + k].set(0.0);
    write_name(p, data::PROG_NAME + k * NAME_LEN, "");
}

/// The panel differs from the program it came from.
pub fn edited(p: &ModuleParams) -> bool {
    let k = program(p);
    (0..PARAM_COUNT).filter(|&i| !is_global(i)).any(|i| (p.get(i) - program_value(p, k, i)).abs() > 1e-4)
}

pub fn program(p: &ModuleParams) -> usize {
    (p.data[data::PROGRAM].get().round().max(0.0) as usize).min(PROGRAMS - 1)
}

/// "23": bank 2, program 3, as on the original's display.
pub fn program_label(k: usize) -> String {
    format!("{}{}", k / 8 + 1, k % 8 + 1)
}

/// Program `k`'s name: the one it was recorded with, else the factory's.
pub fn program_name(p: &ModuleParams, k: usize) -> String {
    let k = k.min(PROGRAMS - 1);
    if p.data[data::USER + k].get() > 0.5 {
        let name = read_name(p, data::PROG_NAME + k * NAME_LEN);
        if name.is_empty() { "User program".to_owned() } else { name }
    } else {
        bank::name(k).to_owned()
    }
}

/// The sound's name (the program's, a preset's, or the one it was saved as).
pub fn sound_name(p: &ModuleParams) -> String {
    let name = read_name(p, data::NAME);
    if name.is_empty() { program_name(p, program(p)) } else { name }
}

pub fn set_sound_name(p: &ModuleParams, name: &str) {
    write_name(p, data::NAME, name);
}

/// A name into `NAME_LEN` slots from `at`: a character each, 0 after the end.
fn write_name(p: &ModuleParams, at: usize, name: &str) {
    let mut chars = name.trim().chars();
    for slot in &p.data[at..at + NAME_LEN] {
        slot.set(chars.next().map_or(0.0, |c| c as u32 as f32));
    }
}

fn read_name(p: &ModuleParams, at: usize) -> String {
    p.data[at..at + NAME_LEN].iter().map(|c| c.get()).take_while(|&c| c >= 1.0).filter_map(|c| char::from_u32(c as u32)).collect()
}

/// The effects chain's order (each effect once).
pub fn fx_order(p: &ModuleParams) -> [usize; 5] {
    let mut order = [0usize; 5];
    let mut seen = [false; 5];
    for (n, slot) in order.iter_mut().enumerate() {
        let e = (p.data[data::FX_ORDER + n].get().round().max(0.0) as usize).min(4);
        *slot = if seen[e] {
            (0..5).find(|&x| !seen[x]).unwrap_or(0)
        } else {
            e
        };
        seen[*slot] = true;
    }
    order
}

// ---------------------------------------------------------------------------
// The processor
// ---------------------------------------------------------------------------

/// Note events a block can hold (MIDI and the arpeggiator's).
const EVENTS: usize = 256;
/// Longest stretch, in frames at the board's rate, between control updates.
pub const TICK: usize = 16;

/// A note or controller at a frame of the block.
#[derive(Clone, Copy, Default)]
pub struct Event {
    pub frame: usize,
    pub data: [u8; 3],
}

/// The keys held, the sustain pedal, the unison stack and the voices'
/// allocation.
struct Keys {
    /// Keys held, in press order, with their velocities.
    held: [(u8, f32); 32],
    held_len: usize,
    /// Keys let go while the pedal holds.
    pedal: bool,
    sustained: [bool; 128],
    /// Allocation clock.
    clock: u64,
    /// Round-robin position.
    next: usize,
}

impl Keys {
    fn new() -> Self {
        Self {
            held: [(0, 0.0); 32],
            held_len: 0,
            pedal: false,
            sustained: [false; 128],
            clock: 0,
            next: 0,
        }
    }

    fn press(&mut self, note: u8, vel: f32) {
        self.release(note);
        if self.held_len < self.held.len() {
            self.held[self.held_len] = (note, vel);
            self.held_len += 1;
        }
        self.sustained[note as usize] = false;
    }

    fn release(&mut self, note: u8) {
        if let Some(i) = self.held[..self.held_len].iter().position(|h| h.0 == note) {
            self.held.copy_within(i + 1..self.held_len, i);
            self.held_len -= 1;
        }
    }

    fn held(&self) -> &[(u8, f32)] {
        &self.held[..self.held_len]
    }

    /// The note unison plays, by priority.
    fn priority(&self, prio: usize) -> Option<(u8, f32)> {
        let h = self.held();
        match prio {
            0 => h.iter().copied().min_by_key(|x| x.0),
            1 => h.iter().copied().max_by_key(|x| x.0),
            _ => h.last().copied(),
        }
    }
}

pub struct Sibyl {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    sr: f32,
    voices: Box<[Voice; MAX_VOICES]>,
    keys: Keys,
    arp: Arp,
    lfo: Lfo,
    matrix: Matrix,
    patch: Box<Patch>,
    table: Box<FilterTable>,
    table_hq: Box<FilterTable>,
    chain: Box<Chain>,
    down: Oversampler,
    hq: bool,
    rng: Rng,
    /// This block's MIDI (and panel) events, then the arpeggiator's.
    events: Box<[Event; EVENTS]>,
    arp_events: Box<[Event; 64]>,
    /// Shared noise for the voices' mixers (white), a block at a time.
    noise: Box<[f32; 2 * BLOCK]>,
    /// A group's oscillators' mix and cutoff, sample by sample, for the filter.
    pre: Box<voice::LaneBuf>,
    cut: Box<voice::LaneBuf>,
    lanes_out: Box<voice::LaneBuf>,
    /// The voices' sum (twice as long for HQ).
    bus: Box<[[f32; 2 * BLOCK]; 2]>,
    /// AC coupling after the voices.
    dc: [(f32, f32); 2],
    // Controllers.
    wheel: f32,
    wheel_param: f32,
    bend: f32,
    bend_panel: f32,
    pressure: f32,
    cc_a: f32,
    cc_b: f32,
    panel_key: u8,
    unlatch_seen: f32,
    panic_seen: f32,
    /// The board's beat while the transport is stopped (the arpeggiator's clock).
    free_beat: f64,
    /// Program changes waiting for the panel thread are applied here.
    a440_phase: f32,
    /// Frames since anything was heard (rests when long).
    silent: usize,
    /// Smoothed knob values (the continuous ones).
    smooth: Box<[f32; PARAM_COUNT]>,
    fresh: bool,
}

impl Sibyl {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        let mut rng = Rng::new(0x51B1_1E55);
        let voices = Box::new(std::array::from_fn(|v| Voice::new(v, &mut rng)));
        let smooth = Box::new(std::array::from_fn(|k| params.get(k)));
        Self {
            midi_cursor: midi.stream_head(),
            midi,
            sr: sample_rate,
            voices,
            keys: Keys::new(),
            arp: Arp::new(),
            lfo: Lfo::new(),
            matrix: Matrix::default(),
            patch: Box::default(),
            table: Box::new(FilterTable::new(sample_rate)),
            table_hq: Box::new(FilterTable::new(sample_rate * 2.0)),
            chain: Box::new(Chain::new(sample_rate)),
            down: Oversampler::new(),
            hq: false,
            rng,
            events: Box::new([Event::default(); EVENTS]),
            arp_events: Box::new([Event::default(); 64]),
            noise: Box::new([0.0; 2 * BLOCK]),
            pre: Box::new([voice::F4::default(); voice::SEG]),
            cut: Box::new([voice::F4::splat(60.0); voice::SEG]),
            lanes_out: Box::new([voice::F4::default(); voice::SEG]),
            bus: Box::new([[0.0; 2 * BLOCK]; 2]),
            dc: [(0.0, 0.0); 2],
            wheel: 0.0,
            wheel_param: params.get(MOD_WHEEL),
            bend: 0.0,
            bend_panel: 0.0,
            pressure: 0.0,
            cc_a: 0.0,
            cc_b: 0.0,
            panel_key: 0,
            unlatch_seen: params.viz[viz::UNLATCH].get(),
            panic_seen: params.viz[viz::PANIC].get(),
            free_beat: 0.0,
            a440_phase: 0.0,
            silent: usize::MAX / 2,
            smooth,
            fresh: true,
            params,
        }
    }

    /// The knobs, smoothed over ~10 ms where a jump would click.
    fn read_params(&mut self, ctx: &ProcessCtx, n: usize) {
        let p = &self.params;
        let k = 1.0 - (1.0 - ctx.smooth).powi(n as i32);
        for (i, s) in self.smooth.iter_mut().enumerate() {
            let target = p.get(i);
            if self.fresh || !SMOOTHED[i] {
                *s = target;
            } else {
                *s += (target - *s) * k;
                if (target - *s).abs() < 1e-6 {
                    *s = target;
                }
            }
        }
        self.fresh = false;
    }

    fn all_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.kill();
        }
        self.keys = Keys::new();
        self.arp.clear();
    }

    /// A key from MIDI or the panel: to the arpeggiator when it runs, else
    /// to the voices.
    fn key(&mut self, note: u8, vel: Option<f32>) {
        if self.patch.arp_on {
            match vel {
                Some(v) => self.arp.press(note, v),
                None => self.arp.release(note),
            }
        } else {
            match vel {
                Some(v) => self.note_on(note, v),
                None => self.note_off(note),
            }
        }
    }

    fn note_on(&mut self, note: u8, vel: f32) {
        self.keys.press(note, vel);
        let patch = &*self.patch;
        if patch.unison {
            self.unison_update(true);
            return;
        }
        let count = patch.voices.clamp(1, MAX_VOICES);
        self.keys.clock += 1;
        let clock = self.keys.clock;
        let voices = &mut self.voices[..count];
        // The same note's voice comes back (REALLOCATE), else a free one in
        // turn, else the quietest released one, else the oldest.
        let same = patch
            .realloc
            .then(|| voices.iter().position(|v| v.active() && v.note == note))
            .flatten();
        let start = self.keys.next % count;
        let free = || {
            (0..count)
                .map(|i| (start + i) % count)
                .find(|&i| !voices[i].active())
        };
        let pick = same
            .or_else(|| {
                if patch.realloc {
                    // A free voice that last played this note keeps its character.
                    (0..count)
                        .find(|&i| !voices[i].active() && voices[i].note == note)
                        .or_else(free)
                } else {
                    free()
                }
            })
            .or_else(|| {
                (0..count)
                    .filter(|&i| !voices[i].gate)
                    .min_by(|&a, &b| voices[a].level().total_cmp(&voices[b].level()))
            })
            .unwrap_or_else(|| (0..count).min_by_key(|&i| voices[i].started).unwrap_or(0));
        self.keys.next = pick + 1;
        let v = &mut voices[pick];
        v.unison = 0.0;
        v.trigger(note, vel, clock, patch);
    }

    fn note_off(&mut self, note: u8) {
        self.keys.release(note);
        if self.patch.unison {
            self.unison_update(false);
            return;
        }
        if self.keys.pedal {
            self.keys.sustained[note as usize] = true;
            return;
        }
        for v in self.voices.iter_mut().filter(|v| v.gate && v.note == note) {
            v.release(&self.patch);
        }
    }

    /// Unison: every stacked voice on the note the priority picks; a new key
    /// retriggers, going back to a held one only moves the pitch.
    fn unison_update(&mut self, pressed: bool) {
        let patch = &*self.patch;
        let count = patch.uni_voices.clamp(1, MAX_VOICES);
        match self.keys.priority(patch.prio) {
            Some((note, vel)) => {
                self.keys.clock += 1;
                let clock = self.keys.clock;
                for (i, v) in self.voices.iter_mut().enumerate() {
                    if i >= count {
                        if v.gate {
                            v.release(patch);
                        }
                        continue;
                    }
                    v.unison = if count > 1 {
                        i as f32 / (count - 1) as f32 * 2.0 - 1.0
                    } else {
                        0.0
                    };
                    if pressed || !v.gate {
                        v.trigger(note, vel, clock, patch);
                    } else {
                        v.glide_to(note, patch);
                    }
                }
            }
            None if !self.keys.pedal => {
                for v in self.voices.iter_mut().filter(|v| v.gate) {
                    v.release(patch);
                }
            }
            None => {}
        }
    }

    fn pedal(&mut self, down: bool) {
        self.keys.pedal = down;
        if !down {
            let held: [bool; 128] =
                std::array::from_fn(|n| self.keys.held().iter().any(|h| h.0 as usize == n));
            if self.patch.unison {
                if self.keys.held_len == 0 {
                    for v in self.voices.iter_mut().filter(|v| v.gate) {
                        v.release(&self.patch);
                    }
                }
            } else {
                for v in self
                    .voices
                    .iter_mut()
                    .filter(|v| v.gate && !held[v.note as usize])
                {
                    v.release(&self.patch);
                }
            }
            self.keys.sustained = [false; 128];
        }
    }

    fn handle(&mut self, data: [u8; 3]) {
        let [status, d1, d2] = data;
        match status & 0xF0 {
            0x90 if d2 > 0 => self.key(d1 & 0x7F, Some(d2 as f32 / 127.0)),
            0x80 | 0x90 => self.key(d1 & 0x7F, None),
            0xB0 => match d1 {
                1 => {
                    self.wheel = d2 as f32 / 127.0;
                }
                64 => self.pedal(d2 >= 64),
                120 | 123 => {
                    if d1 == 120 {
                        self.all_off();
                    } else {
                        for n in 0..128u8 {
                            self.key(n, None);
                        }
                    }
                }
                _ => {
                    if d1 as f32 == self.patch.cc_a {
                        self.cc_a = d2 as f32 / 127.0;
                    }
                    if d1 as f32 == self.patch.cc_b {
                        self.cc_b = d2 as f32 / 127.0;
                    }
                }
            },
            0xE0 => {
                let v = ((d2 as i32) << 7 | d1 as i32) - 8192;
                self.bend = (v as f32 / 8192.0).clamp(-1.0, 1.0);
            }
            0xD0 => self.pressure = d1 as f32 / 127.0,
            0xC0 => {
                load_program(&self.params, d1 as usize % PROGRAMS);
            }
            _ => {}
        }
    }

    /// Publishes what the panel shows.
    fn publish(&self) {
        let p = &self.params;
        for (i, v) in self.voices.iter().enumerate() {
            p.viz[viz::LEVEL + i].set(v.level());
            p.viz[viz::NOTE + i].set(if v.active() { v.note as f32 } else { -1.0 });
            p.viz[viz::GATE + i].set(if v.gate { 1.0 } else { 0.0 });
        }
        p.viz[viz::LFO].set(self.lfo.value);
        p.viz[viz::WHEEL].set(self.wheel);
        p.viz[viz::BEND].set(if self.bend_panel != 0.0 {
            self.bend_panel
        } else {
            self.bend
        });
        p.viz[viz::ARP_NOTE].set(self.arp.sounding().map_or(-1.0, f32::from));
        let held = if self.patch.arp_on {
            self.arp.held_count()
        } else {
            self.keys.held_len
        };
        p.viz[viz::HELD].set(held as f32);
    }
}

/// The continuous knobs whose jumps are smoothed.
static SMOOTHED: [bool; PARAM_COUNT] = {
    let mut s = [false; PARAM_COUNT];
    let list = [
        PM_FENV,
        PM_OSCB,
        WM_MIX,
        A_PW,
        B_PW,
        MIX_A,
        MIX_B,
        MIX_NOISE,
        CUTOFF,
        RESO,
        ENV_AMT,
        KEY_AMT,
        VOLUME,
        OUTPUT,
        A_FINE,
        B_FINE,
        TUNE,
        DIST_AMOUNT,
        DIST_TONE,
        DIST_MIX,
        TAPE_GAIN,
        ECHO_MIX,
        ECHO_REGEN,
        RESEQ_LOW,
        RESEQ_MID,
        RESEQ_HIGH,
        PLATE_MIX,
        PLATE_TONE,
        COND_GAIN,
        COND_WIDTH,
        MOD_WHEEL,
    ];
    let mut i = 0;
    while i < list.len() {
        s[list[i]] = true;
        i += 1;
    }
    s
};

impl Processor for Sibyl {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        self.read_params(ctx, n);

        // HQ switches the voices' rate: start them clean.
        let hq = self.smooth[HQ] > 0.5;
        if hq != self.hq {
            self.hq = hq;
            self.down.reset();
            for v in self.voices.iter_mut() {
                v.reset_filter();
            }
        }
        let os = if hq { 2 } else { 1 };
        let vsr = self.sr * os as f32;
        self.patch.update(&self.smooth, &p, vsr);
        self.matrix.update(&self.smooth);

        // Panic, unlatch.
        let panic = p.viz[viz::PANIC].get();
        if panic != self.panic_seen {
            self.panic_seen = panic;
            self.all_off();
        }
        let unlatch = p.viz[viz::UNLATCH].get();
        if unlatch != self.unlatch_seen {
            self.unlatch_seen = unlatch;
            self.arp.unlatch();
        }

        // The arpeggiator switched off: let its notes go, and the keys it held.
        if !self.patch.arp_on
            && self.arp.running()
            && let Some(n) = self.arp.stop()
        {
            self.note_off(n);
        }

        // The mod wheel: the knob when it moves, else MIDI.
        let wheel_param = self.smooth[MOD_WHEEL];
        if (wheel_param - self.wheel_param).abs() > 1e-6 {
            self.wheel = wheel_param;
            self.wheel_param = wheel_param;
        }
        let panel_bend = p.viz[viz::PANEL_BEND].get();
        if panel_bend != self.bend_panel {
            self.bend_panel = panel_bend.clamp(-1.0, 1.0);
            if self.bend_panel == 0.0 {
                self.bend = 0.0;
            }
        }

        // Events: MIDI (the cable if patched, else the keyboards) and the panel's key.
        let mut count = 0usize;
        {
            let events = &mut *self.events;
            let mut push = |frame: usize, data: [u8; 3]| {
                if count < EVENTS {
                    events[count] = Event {
                        frame: frame.min(n.saturating_sub(1)),
                        data,
                    };
                    count += 1;
                }
            };
            if inputs.midi_connected {
                for e in inputs.midi.events() {
                    push(e.frame as usize, e.data);
                }
                self.midi_cursor = self.midi.stream_head();
            } else {
                self.midi
                    .read_stream(&mut self.midi_cursor, |msg| push(0, msg));
            }
            let panel_key = p.viz[viz::PANEL_KEY].get().round().clamp(0.0, 128.0) as u8;
            if panel_key != self.panel_key {
                if self.panel_key > 0 {
                    push(0, [0x80, self.panel_key - 1, 0]);
                }
                if panel_key > 0 {
                    push(0, [0x90, panel_key - 1, 100]);
                }
                self.panel_key = panel_key;
            }
        }
        if ctx.restart {
            self.arp.restart();
        }
        // In frame order, keeping the order of events at the same frame
        // (insertion sort: no allocation, and blocks hold few events).
        let midi_events = &mut self.events[..count];
        for i in 1..midi_events.len() {
            let mut j = i;
            while j > 0 && midi_events[j - 1].frame > midi_events[j].frame {
                midi_events.swap(j - 1, j);
                j -= 1;
            }
        }

        // Keys go through the arpeggiator first, which answers with its own notes.
        let beat0 = if ctx.playing {
            ctx.beat
        } else {
            self.free_beat
        };
        let bps = ctx.beats_per_sample.max(1e-9);
        self.free_beat = beat0 + bps * n as f64;
        let mut arp_count = 0;
        if self.patch.arp_on {
            let arp_events = &mut *self.arp_events;
            let mut push_arp = |frame: usize, data: [u8; 3]| {
                if arp_count < arp_events.len() {
                    arp_events[arp_count] = Event { frame, data };
                    arp_count += 1;
                }
            };
            self.arp.plan(
                &self.patch,
                beat0,
                bps,
                n,
                &self.events[..count],
                &mut push_arp,
            );
        }

        // Resting: nothing sounds, nothing comes, the effects have died away.
        let a440 = self.smooth[A440] > 0.5;
        let any = self.voices.iter().any(|v| v.active());
        if !any && count == 0 && arp_count == 0 && !a440 && self.silent > (self.sr * 0.5) as usize {
            self.silent = self.silent.saturating_add(n);
            silence(outputs, n);
            self.publish();
            return;
        }

        // Render: segments up to TICK frames, split at each event.
        let len = n * os;
        for ch in self.bus.iter_mut() {
            ch[..len].fill(0.0);
        }
        for x in self.noise[..len].iter_mut() {
            *x = self.rng.bipolar();
        }
        let (mut mi, mut ai) = (0usize, 0usize);
        let mut pos = 0usize;
        while pos < n {
            // Events at this frame.
            while mi < count && self.events[mi].frame <= pos {
                let e = self.events[mi];
                mi += 1;
                if self.patch.arp_on && matches!(e.data[0] & 0xF0, 0x80 | 0x90) {
                    continue; // the arpeggiator took them in `plan`
                }
                self.handle(e.data);
            }
            while ai < arp_count && self.arp_events[ai].frame <= pos {
                let e = self.arp_events[ai];
                ai += 1;
                match e.data[0] & 0xF0 {
                    0x90 => self.note_on(e.data[1], e.data[2] as f32 / 127.0),
                    _ => self.note_off(e.data[1]),
                }
            }
            let next_event = [
                if mi < count { self.events[mi].frame } else { n },
                if ai < arp_count {
                    self.arp_events[ai].frame
                } else {
                    n
                },
                (pos / TICK + 1) * TICK,
            ]
            .into_iter()
            .min()
            .unwrap_or(n)
            .min(n)
            .max(pos + 1);
            let seg = next_event - pos;
            let beat = beat0 + bps * pos as f64;

            // Global modulation for this segment.
            let bend = if self.bend_panel != 0.0 {
                self.bend_panel
            } else {
                self.bend
            };
            let mut g = mods::Globals {
                wheel: self.wheel,
                bend,
                pressure: self.pressure,
                cc_a: self.cc_a,
                cc_b: self.cc_b,
                lfo: 0.0,
                wm: 0.0,
                wm_noise: 0.0,
            };
            let lfo_mod =
                self.matrix
                    .global(mods::dest::LFO_RATE, &g, &self.voices, self.keys.clock);
            self.lfo.advance(
                &self.patch,
                seg as f32 / self.sr,
                beat,
                bps * self.sr as f64,
                ctx.playing,
                lfo_mod,
                &mut self.rng,
            );
            g.lfo = self.lfo.value;
            g.wm_noise = self.lfo.noise;
            let mix = (self.patch.wm_mix
                + self
                    .matrix
                    .global(mods::dest::WM_MIX, &g, &self.voices, self.keys.clock))
            .clamp(0.0, 1.0);
            let low = (self.patch.wm_low
                + self
                    .matrix
                    .global(mods::dest::WM_LOW, &g, &self.voices, self.keys.clock))
            .clamp(0.0, 1.0);
            let high = (self.patch.wm_high
                + self
                    .matrix
                    .global(mods::dest::WM_HIGH, &g, &self.voices, self.keys.clock))
            .clamp(0.0, 1.0);
            let depth = low + (high - low) * self.wheel;
            g.wm = depth * (g.lfo * (1.0 - mix) + g.wm_noise * mix);

            let from = pos * os;
            let to = next_event * os;
            let len = to - from;
            let mut active = [0usize; MAX_VOICES];
            let mut count_active = 0;
            for (i, v) in self.voices.iter_mut().enumerate() {
                if v.active() {
                    v.control(&self.patch, &self.matrix, &g, len as f32);
                    active[count_active] = i;
                    count_active += 1;
                }
            }
            let table: &FilterTable = if hq { &self.table_hq } else { &self.table };
            for group in active[..count_active].chunks(voice::LANES) {
                for (l, &i) in group.iter().enumerate() {
                    self.voices[i].osc(&self.patch, len, l, &mut self.pre, &mut self.cut);
                }
                for l in group.len()..voice::LANES {
                    for i in 0..len {
                        self.pre[i].0[l] = 0.0;
                        self.cut[i].0[l] = 60.0;
                    }
                }
                let [bus_l, bus_r] = &mut *self.bus;
                voice::filter_group(
                    &mut self.voices[..],
                    group,
                    &self.patch,
                    table,
                    &self.noise[from..to],
                    &self.pre,
                    &self.cut,
                    &mut self.lanes_out,
                    len,
                    &mut bus_l[from..to],
                    &mut bus_r[from..to],
                );
            }
            pos = next_event;
        }
        // Events left at the block's end.
        while mi < count {
            let e = self.events[mi];
            mi += 1;
            if !(self.patch.arp_on && matches!(e.data[0] & 0xF0, 0x80 | 0x90)) {
                self.handle(e.data);
            }
        }
        while ai < arp_count {
            let e = self.arp_events[ai];
            ai += 1;
            match e.data[0] & 0xF0 {
                0x90 => self.note_on(e.data[1], e.data[2] as f32 / 127.0),
                _ => self.note_off(e.data[1]),
            }
        }

        // Back to the board's rate, AC-coupled.
        let [out_l, out_r] = &mut outputs[0];
        let hp = 1.0 - (-std::f32::consts::TAU * 8.0 / self.sr).exp();
        let (mut dl, mut dr) = (self.dc[0].0, self.dc[1].0);
        if hq {
            for i in 0..n {
                let y = self.down.down([
                    [self.bus[0][2 * i], self.bus[1][2 * i]],
                    [self.bus[0][2 * i + 1], self.bus[1][2 * i + 1]],
                ]);
                dl += (y[0] - dl) * hp;
                dr += (y[1] - dr) * hp;
                out_l[i] = y[0] - dl;
                out_r[i] = y[1] - dr;
            }
        } else {
            for i in 0..n {
                let (l, r) = (self.bus[0][i], self.bus[1][i]);
                dl += (l - dl) * hp;
                dr += (r - dr) * hp;
                out_l[i] = l - dl;
                out_r[i] = r - dr;
            }
        }
        let tidy = |v: f32| if v.abs() < 1e-15 { 0.0 } else { v };
        self.dc = [(tidy(dl), 0.0), (tidy(dr), 0.0)];

        // The effects, in their order; then OUTPUT.
        let g = mods::Globals {
            wheel: self.wheel,
            bend: self.bend,
            pressure: self.pressure,
            cc_a: self.cc_a,
            cc_b: self.cc_b,
            lfo: self.lfo.value,
            wm: 0.0,
            wm_noise: 0.0,
        };
        let fx_mod = self.matrix.fx_mods(&g, &self.voices, self.keys.clock);
        let order = fx_order(&p);
        let load = self
            .chain
            .process(&self.smooth, &order, &fx_mod, ctx, out_l, out_r, n);
        p.viz[viz::COND_LOAD].set(load);
        let gain = self.smooth[OUTPUT];
        for i in 0..n {
            if a440 {
                self.a440_phase = (self.a440_phase + 440.0 / self.sr).fract();
                let y = 0.25 * crate::audio::fastmath::sin_turns(self.a440_phase);
                out_l[i] = y;
                out_r[i] = y;
            } else {
                out_l[i] = soft_limit(out_l[i] * gain);
                out_r[i] = soft_limit(out_r[i] * gain);
            }
        }

        // How long it has been silent (it rests after half a second).
        let peak = out_l[..n]
            .iter()
            .chain(&out_r[..n])
            .fold(0.0f32, |m, x| m.max(x.abs()));
        let any = self.voices.iter().any(|v| v.active());
        self.silent = if any || peak > 1e-5 {
            0
        } else {
            self.silent.saturating_add(n)
        };
        self.publish();
    }
}
