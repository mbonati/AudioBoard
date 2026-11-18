//! Splitter: a sample slicer and granular glitch engine on the board's grid,
//! for fractured, minimal, very digital music: sine tones and pings,
//! clicks, data noise, stuttering micro-loops, frozen grains, sub pulses.
//!
//! **Sources.** A sample (WAV or AIFF, kept with the project: see
//! `audio::sample`) and eight built-in glitch atoms (see [`atoms`]): SINE,
//! PING, CLICK, SUB, NOISE, DATA, PULSE and CRACK, so the node plays with
//! nothing loaded. Up to 32 slices each take a region of one source.
//!
//! **Slices.** Each has its own settings (see [`slice`]): the region, its
//! pitch, gain and pan, reversed or not, how it plays (once, held, looping
//! while held), an attack and a release; a ratchet (REPEAT strikes at a
//! RATE on the grid, each FALL quieter and RAMP semitones away); a crusher
//! (BITS, and DOWN, a sample-and-hold); a granular mode (SIZE, DENSITY,
//! POS, SPRAY, JITTER, FREEZE, SYNC to the grid); a MIDI note; and how the
//! animation treats it (its probability, its Euclidean rhythm, a mute).
//! The slices are cut by hand on the panel, or in one go: an equal grid,
//! at the transients, or at random (see [`slicing`]).
//!
//! **Animation.** While RUN is on and the board plays, steps of RATE on the
//! board's grid play the slices: in ORDER, at RANDOM (LOCK repeats the
//! random choices every LENGTH steps), from the PATTERN (a slice or a rest
//! per step), by PROBability (each slice rolls its own dice every step), on
//! EUCLIDean rhythms (each slice its own, so they drift against each other)
//! or as STUTTERs (the pattern's slices rolled into ratchets, now and then
//! held). FILL thins every mode out, CHAOS varies the hits (reversed, an
//! octave or a fifth away, skipped into, doubled). The steps go out on MIDI
//! too. DICE (see [`dice`]) throws new slice settings, new rhythms or both,
//! and BACK takes a throw back.
//!
//! **MIDI.** Notes play the slices mapped to them (LEARN takes a slice's
//! note from the next one); a note-off ends a GATE or LOOP slice and its
//! remaining repeats. With CHROMA, unmapped notes play the selected slice
//! in pitch around C4 (note 60).

pub mod atoms;
pub mod dice;
pub mod slicing;
pub mod voice;

use std::sync::Arc;

use self::atoms::{ATOM_LEN, hash};
use self::voice::{Clock, Holder, Settings, Src, Variation, Voice};
use super::steps::{MAX_STEPS, Step, StepClock, roll};
use super::{ModuleDef, ModuleParams, Rng, soft_limit};
use crate::audio::engine::{BLOCK, Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::fastmath::exp2;
use crate::audio::params::{ParamSpec, Unit};
use crate::audio::sample::Sample;

/// Slices at most.
pub const MAX_SLICES: usize = 32;
/// Steps in a pattern at most.
pub const MAX_LEN: usize = 32;
/// Voices playing at once.
pub const VOICES: usize = 16;
const MAX_RATCHETS: usize = 24;
const MAX_HITS: usize = 128;

// The knobs.
pub const LEVEL: usize = 0;
/// Transposes every slice.
pub const PITCH: usize = 1;
/// The chance that a step (or a slice's hit) plays at all.
pub const FILL: usize = 2;
/// How often hits are varied.
pub const CHAOS: usize = 3;
/// Crushes every slice further.
pub const CRUSH: usize = 4;
/// Scatters hits and grains across the stereo field.
pub const SPREAD: usize = 5;
pub const SWING: usize = 6;
pub const PARAM_COUNT: usize = 7;

const PARAMS: [ParamSpec; PARAM_COUNT] = [
    ParamSpec::lin("LEVEL", 0.0, 1.0, 0.75, Unit::Percent),
    ParamSpec::lin("PITCH", -24.0, 24.0, 0.0, Unit::Semitones).bipolar(),
    ParamSpec::lin("FILL", 0.0, 1.0, 1.0, Unit::Percent),
    ParamSpec::lin("CHAOS", 0.0, 1.0, 0.12, Unit::Percent),
    ParamSpec::lin("CRUSH", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("SPREAD", 0.0, 1.0, 0.35, Unit::Percent),
    ParamSpec::lin("SWING", 0.0, 1.0, 0.0, Unit::Percent),
];

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// How the steps choose what to play.
pub const MODES: [&str; 6] = ["ORDER", "RANDOM", "PATTERN", "PROB", "EUCLID", "STUTTER"];
pub const ORDER: usize = 0;
pub const RANDOM: usize = 1;
pub const PATTERN: usize = 2;
pub const PROB: usize = 3;
pub const EUCLID: usize = 4;
pub const STUTTER: usize = 5;

/// Step lengths: name and steps per beat.
pub const RATES: [(&str, f64); 8] = [("1/2", 0.5), ("1/4", 1.0), ("1/8", 2.0), ("1/8T", 3.0), ("1/16", 4.0), ("1/16T", 6.0), ("1/32", 8.0), ("1/64", 16.0)];
/// A ratchet's rates: name and strikes per beat.
pub const REPEAT_RATES: [(&str, f64); 7] = [("1/8", 2.0), ("1/16", 4.0), ("1/16T", 6.0), ("1/32", 8.0), ("1/32T", 12.0), ("1/64", 16.0), ("1/128", 32.0)];
/// The panel's choices of repeats and bits.
pub const REPEATS: [f32; 8] = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0];
pub const BITS: [f32; 8] = [16.0, 12.0, 10.0, 8.0, 6.0, 4.0, 3.0, 2.0];
/// How a slice plays.
pub const PLAYS: [&str; 3] = ["ONE", "GATE", "LOOP"];
/// The sources: the sample, then the atoms.
pub const SOURCES: usize = 1 + atoms::ATOMS.len();

/// A slice's settings, in this order (see [`FIELDS`]).
pub mod slice {
    /// The region, as shares of the source (0…1).
    pub const START: usize = 0;
    pub const END: usize = 1;
    /// 0: the sample; 1…8: an atom.
    pub const SOURCE: usize = 2;
    /// Semitones.
    pub const PITCH: usize = 3;
    /// dB.
    pub const GAIN: usize = 4;
    pub const PAN: usize = 5;
    pub const REVERSE: usize = 6;
    /// ONE, GATE, LOOP.
    pub const PLAY: usize = 7;
    /// ms.
    pub const ATTACK: usize = 8;
    pub const RELEASE: usize = 9;
    /// The ratchet: strikes, their rate (an index into `REPEAT_RATES`),
    /// the level each loses (%), the semitones each moves.
    pub const REPEAT: usize = 10;
    pub const REP_RATE: usize = 11;
    pub const FALL: usize = 12;
    pub const RAMP: usize = 13;
    /// The crusher: bits, and samples held.
    pub const BITS: usize = 14;
    pub const DOWN: usize = 15;
    /// Granular: on, grain size (ms), grains a second, where the playhead
    /// starts (or stands, frozen), how far grains scatter from it, their
    /// pitch's scatter (semitones), frozen, on the grid.
    pub const GRAIN: usize = 16;
    pub const SIZE: usize = 17;
    pub const DENSITY: usize = 18;
    pub const POS: usize = 19;
    pub const SPRAY: usize = 20;
    pub const FREEZE: usize = 21;
    pub const SYNC: usize = 22;
    pub const JITTER: usize = 23;
    /// Its MIDI note (−1: none).
    pub const NOTE: usize = 24;
    /// PROB mode: its chance each step (%).
    pub const PROB: usize = 25;
    /// EUCLID mode: hits, steps, rotation.
    pub const HITS: usize = 26;
    pub const STEPS: usize = 27;
    pub const ROTATE: usize = 28;
    /// Left out of the animation.
    pub const MUTE: usize = 29;
    pub const LEN: usize = 30;
}

/// A slice setting's name, range and default.
pub struct Field {
    pub name: &'static str,
    pub lo: f32,
    pub hi: f32,
    pub default: f32,
}

const fn f(name: &'static str, lo: f32, hi: f32, default: f32) -> Field {
    Field { name, lo, hi, default }
}

pub const FIELDS: [Field; slice::LEN] = [
    f("START", 0.0, 1.0, 0.0),
    f("END", 0.0, 1.0, 0.25),
    f("SOURCE", 0.0, (SOURCES - 1) as f32, 0.0),
    f("PITCH", -48.0, 48.0, 0.0),
    f("GAIN", -40.0, 12.0, 0.0),
    f("PAN", -1.0, 1.0, 0.0),
    f("REVERSE", 0.0, 1.0, 0.0),
    f("PLAY", 0.0, 2.0, 0.0),
    f("ATTACK", 0.0, 2000.0, 0.5),
    f("RELEASE", 1.0, 4000.0, 30.0),
    f("REPEAT", 1.0, 16.0, 1.0),
    f("RATE", 0.0, (REPEAT_RATES.len() - 1) as f32, 3.0),
    f("FALL", 0.0, 100.0, 20.0),
    f("RAMP", -12.0, 12.0, 0.0),
    f("BITS", 1.0, 16.0, 16.0),
    f("DOWN", 1.0, 64.0, 1.0),
    f("GRAIN", 0.0, 1.0, 0.0),
    f("SIZE", 2.0, 500.0, 60.0),
    f("DENSITY", 1.0, 400.0, 30.0),
    f("POS", 0.0, 1.0, 0.0),
    f("SPRAY", 0.0, 1.0, 0.1),
    f("FREEZE", 0.0, 1.0, 0.0),
    f("SYNC", 0.0, 1.0, 0.0),
    f("JITTER", 0.0, 12.0, 0.0),
    f("NOTE", -1.0, 127.0, -1.0),
    f("PROB", 0.0, 100.0, 50.0),
    f("HITS", 0.0, 32.0, 0.0),
    f("STEPS", 1.0, 32.0, 16.0),
    f("ROTATE", 0.0, 31.0, 0.0),
    f("MUTE", 0.0, 1.0, 0.0),
];

/// What `ModuleParams::data` holds.
pub mod data {
    use super::{MAX_LEN, MAX_SLICES, slice};
    /// The panel's page: 0 SLICE, 1 SEQUENCE, 2 MIDI.
    pub const TAB: usize = 0;
    /// The slice being edited.
    pub const SELECTED: usize = 1;
    /// Slices in use.
    pub const COUNT: usize = 2;
    /// The animation plays with the transport.
    pub const RUN: usize = 3;
    pub const MODE: usize = 4;
    /// An index into `RATES`.
    pub const RATE: usize = 5;
    pub const LENGTH: usize = 6;
    /// How long a step holds a GATE or LOOP slice (and its MIDI note), in %
    /// of the step.
    pub const GATE: usize = 7;
    /// The random choices repeat every LENGTH steps.
    pub const LOCK: usize = 8;
    pub const SEED: usize = 9;
    /// 0 Omni, 1…16.
    pub const MIDI_CH: usize = 10;
    /// Waiting to learn slice n − 1's note (0: nothing).
    pub const LEARN: usize = 11;
    /// Unmapped notes play the selected slice in pitch.
    pub const CHROMA: usize = 12;
    /// The steps' hits go out as MIDI notes.
    pub const MIDI_OUT: usize = 13;
    /// How far a DICE throw goes, in %.
    pub const CHANCE: usize = 14;
    /// The display: 0 the whole source, 1 zoomed on the selected slice.
    pub const ZOOM: usize = 15;
    /// How readily the transient slicer cuts, in %.
    pub const SENS: usize = 16;
    /// The pattern: a slice per step, or −1 for a rest.
    pub const STEPS: usize = 24;
    /// The slices' settings, slice by slice.
    pub const SLICES: usize = STEPS + MAX_LEN;
    pub const LEN: usize = SLICES + MAX_SLICES * slice::LEN;
}

/// Where slice `s`'s setting `k` lives in `data`.
pub const fn at(s: usize, k: usize) -> usize {
    data::SLICES + s * slice::LEN + k
}

/// The kit it starts with, from the atoms: (source, settings).
const KIT: [(usize, &[(usize, f32)]); 8] = {
    use slice::*;
    [
        (4, &[(END, 0.35), (RELEASE, 160.0), (GAIN, -2.0), (HITS, 4.0), (NOTE, 36.0), (PROB, 70.0)]),
        (3, &[(END, 0.012), (RELEASE, 4.0), (GAIN, -4.0), (PAN, -0.3), (HITS, 7.0), (ROTATE, 2.0), (NOTE, 37.0), (PROB, 55.0)]),
        (2, &[(END, 0.25), (RELEASE, 120.0), (GAIN, -9.0), (PAN, 0.35), (HITS, 3.0), (STEPS, 11.0), (NOTE, 38.0), (PROB, 30.0)]),
        (1, &[(PITCH, 24.0), (END, 0.06), (ATTACK, 1.0), (RELEASE, 20.0), (GAIN, -18.0), (PAN, 0.55), (HITS, 2.0), (STEPS, 13.0), (ROTATE, 3.0), (NOTE, 39.0), (PROB, 25.0)]),
        (5, &[(END, 0.03), (RELEASE, 10.0), (BITS, 6.0), (GAIN, -13.0), (PAN, -0.5), (REPEAT, 2.0), (REP_RATE, 5.0), (HITS, 5.0), (ROTATE, 1.0), (NOTE, 40.0), (PROB, 35.0)]),
        (6, &[(END, 0.12), (RELEASE, 15.0), (DOWN, 2.0), (GAIN, -17.0), (HITS, 1.0), (STEPS, 12.0), (ROTATE, 6.0), (NOTE, 41.0), (PROB, 15.0)]),
        (8, &[(END, 1.0), (GRAIN, 1.0), (SIZE, 30.0), (DENSITY, 40.0), (SPRAY, 0.8), (RELEASE, 400.0), (GAIN, -8.0), (HITS, 1.0), (STEPS, 32.0), (NOTE, 42.0), (PROB, 10.0)]),
        (7, &[(PITCH, -12.0), (END, 0.08), (BITS, 4.0), (RELEASE, 30.0), (GAIN, -15.0), (HITS, 3.0), (STEPS, 7.0), (NOTE, 43.0), (PROB, 20.0)]),
    ]
};

/// The pattern it starts with (slices of the kit, −1 rests).
const PATTERN_STEPS: [f32; 16] = [0.0, -1.0, 1.0, -1.0, 4.0, -1.0, 1.0, 2.0, 0.0, -1.0, 1.0, 5.0, 4.0, 3.0, 1.0, -1.0];

static DATA: [f32; data::LEN] = {
    let mut d = [0.0; data::LEN];
    d[data::COUNT] = KIT.len() as f32;
    d[data::RUN] = 1.0;
    d[data::MODE] = EUCLID as f32;
    d[data::RATE] = 4.0; // 1/16
    d[data::LENGTH] = 16.0;
    d[data::GATE] = 50.0;
    d[data::SEED] = 1.0;
    d[data::MIDI_OUT] = 1.0;
    d[data::CHANCE] = 50.0;
    d[data::SENS] = 50.0;
    let mut i = 0;
    while i < MAX_LEN {
        d[data::STEPS + i] = if i < PATTERN_STEPS.len() { PATTERN_STEPS[i] } else { -1.0 };
        i += 1;
    }
    let mut s = 0;
    while s < MAX_SLICES {
        let mut k = 0;
        while k < slice::LEN {
            d[at(s, k)] = FIELDS[k].default;
            k += 1;
        }
        if s < KIT.len() {
            let (source, set) = KIT[s];
            d[at(s, slice::SOURCE)] = source as f32;
            let mut j = 0;
            while j < set.len() {
                d[at(s, set[j].0)] = set[j].1;
                j += 1;
            }
        }
        s += 1;
    }
    d
};

/// Layout of `ModuleParams::viz`.
pub mod viz {
    use super::{MAX_LEN, MAX_SLICES, VOICES};
    /// The step playing (−1: stopped).
    pub const STEP: usize = 0;
    /// Voices sounding.
    pub const VOICES_ON: usize = 1;
    /// Hits per slice so far (the panel flashes on a change).
    pub const HITS: usize = 2;
    /// The slice each step last played (−1: a rest), in the lane modes.
    pub const STEP_SLICE: usize = HITS + MAX_SLICES;
    /// Per voice: its slice (−1: silent), where it plays (a share of its
    /// source), its level.
    pub const HEADS: usize = STEP_SLICE + MAX_LEN;
    /// Grains sounding: where (a share of the source) and their slice.
    pub const GRAINS: usize = HEADS + 3 * VOICES;
    pub const SHOWN_GRAINS: usize = 24;
    /// From the panel: bumped to play the selected slice (PLAY), 1 while
    /// PLAY is held down.
    pub const AUDITION: usize = GRAINS + 2 * SHOWN_GRAINS;
    pub const HOLD: usize = AUDITION + 1;
    /// The last MIDI note received, and how many so far.
    pub const NOTE_IN: usize = HOLD + 1;
    pub const NOTES_IN: usize = NOTE_IN + 1;
    /// Steps since the start of the board's grid (each Euclidean ring
    /// counts it on its own length).
    pub const INDEX: usize = NOTES_IN + 1;
    pub const END: usize = INDEX + 1;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Splitter",
    prefix: "SPLITTER",
    hint: "sample slicer · grains · glitch on the grid",
    inputs: &[],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: true,
    midi_out: true,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::END,
};

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// A data value as a whole number in `lo..=hi`.
pub fn int(p: &ModuleParams, i: usize, lo: i32, hi: i32) -> i32 {
    let v = p.data[i].get();
    if v.is_nan() { lo } else { (v.round() as i32).clamp(lo, hi) }
}

pub fn on(p: &ModuleParams, i: usize) -> bool {
    p.data[i].get() > 0.5
}

pub fn toggle(p: &ModuleParams, i: usize) {
    p.data[i].set(if on(p, i) { 0.0 } else { 1.0 });
}

/// Slice `s`'s setting `k`, within its range.
pub fn get(p: &ModuleParams, s: usize, k: usize) -> f32 {
    let v = p.data[at(s.min(MAX_SLICES - 1), k)].get();
    let fl = &FIELDS[k];
    if v.is_nan() { fl.default } else { v.clamp(fl.lo, fl.hi) }
}

pub fn set(p: &ModuleParams, s: usize, k: usize, v: f32) {
    let fl = &FIELDS[k];
    p.data[at(s.min(MAX_SLICES - 1), k)].set(if v.is_nan() { fl.default } else { v.clamp(fl.lo, fl.hi) });
}

/// A slice's whole-number setting.
pub fn get_int(p: &ModuleParams, s: usize, k: usize) -> i32 {
    get(p, s, k).round() as i32
}

pub fn flag(p: &ModuleParams, s: usize, k: usize) -> bool {
    get(p, s, k) > 0.5
}

pub fn count(p: &ModuleParams) -> usize {
    int(p, data::COUNT, 0, MAX_SLICES as i32) as usize
}

/// The slice being edited (0 when there are none).
pub fn selected(p: &ModuleParams) -> usize {
    int(p, data::SELECTED, 0, count(p).max(1) as i32 - 1) as usize
}

pub fn mode(p: &ModuleParams) -> usize {
    int(p, data::MODE, 0, MODES.len() as i32 - 1) as usize
}

pub fn rate(p: &ModuleParams) -> usize {
    int(p, data::RATE, 0, RATES.len() as i32 - 1) as usize
}

pub fn length(p: &ModuleParams) -> usize {
    int(p, data::LENGTH, 1, MAX_LEN as i32) as usize
}

/// The pattern's step `i`: a slice, or a rest.
pub fn step(p: &ModuleParams, i: usize) -> Option<usize> {
    let v = int(p, data::STEPS + i.min(MAX_LEN - 1), -1, MAX_SLICES as i32 - 1);
    (v >= 0).then_some(v as usize)
}

pub fn set_step(p: &ModuleParams, i: usize, s: Option<usize>) {
    p.data[data::STEPS + i.min(MAX_LEN - 1)].set(s.map_or(-1.0, |s| s.min(MAX_SLICES - 1) as f32));
}

/// The channel received and sent on (None: Omni, which sends on 1).
pub fn midi_channel(p: &ModuleParams) -> Option<u8> {
    let c = int(p, data::MIDI_CH, 0, 16);
    (c > 0).then(|| (c - 1) as u8)
}

/// Slice `s`'s MIDI note, if it has one.
pub fn note(p: &ModuleParams, s: usize) -> Option<u8> {
    let n = get_int(p, s, slice::NOTE);
    (n >= 0).then_some(n as u8)
}

/// The note slice `s`'s hits go out as: its own, or C2 up by slice.
pub fn out_note(p: &ModuleParams, s: usize) -> u8 {
    note(p, s).unwrap_or((36 + s).min(127) as u8)
}

/// The Euclidean rhythm of `hits` over `steps`, turned by `rotate`: whether
/// step `i` plays. Hits fall as evenly as the steps allow.
pub fn euclid(hits: usize, steps: usize, rotate: usize, i: usize) -> bool {
    let n = steps.max(1);
    let k = hits.min(n);
    if k == 0 {
        return false;
    }
    let i = (i % n + n - rotate % n) % n;
    (i * k) % n < k
}

/// A slice's ratchet rate, in strikes per beat.
pub fn repeat_rate(p: &ModuleParams, s: usize) -> f64 {
    REPEAT_RATES[(get_int(p, s, slice::REP_RATE).max(0) as usize).min(REPEAT_RATES.len() - 1)].1
}

/// Slice `s` as a voice plays it, with the node's PITCH, CRUSH and SPREAD.
pub fn settings(p: &ModuleParams, s: usize) -> Settings {
    use slice::*;
    let crush = p.get(CRUSH).clamp(0.0, 1.0);
    Settings {
        source: get_int(p, s, SOURCE).max(0) as usize,
        start: get(p, s, START),
        end: get(p, s, END),
        pitch: get(p, s, PITCH) + p.get(PITCH),
        gain: exp2(get(p, s, GAIN) * 0.166_096_4),
        pan: get(p, s, PAN),
        reverse: flag(p, s, REVERSE),
        play: get_int(p, s, PLAY).max(0) as usize,
        attack: get(p, s, ATTACK) * 0.001,
        release: get(p, s, RELEASE) * 0.001,
        bits: get(p, s, BITS).min(16.0 - 13.0 * crush),
        down: get(p, s, DOWN) * exp2(crush * 3.6),
        grain: flag(p, s, GRAIN),
        size: get(p, s, SIZE) * 0.001,
        density: get(p, s, DENSITY),
        pos: get(p, s, POS),
        spray: get(p, s, SPRAY),
        freeze: flag(p, s, FREEZE),
        sync: flag(p, s, SYNC),
        jitter: get(p, s, JITTER),
        spread: p.get(SPREAD).clamp(0.0, 1.0),
    }
}

/// Slice `s` back to the plain settings (its region, source and note kept).
pub fn reset_slice(p: &ModuleParams, s: usize) {
    for (k, fl) in FIELDS.iter().enumerate() {
        if !matches!(k, slice::START | slice::END | slice::SOURCE | slice::NOTE) {
            set(p, s, k, fl.default);
        }
    }
}

/// A new slice of `source` from `start` to `end` (shares), selected; None
/// when all 32 are used.
pub fn add_slice(p: &ModuleParams, start: f32, end: f32, source: usize) -> Option<usize> {
    let s = count(p);
    if s >= MAX_SLICES {
        return None;
    }
    for (k, fl) in FIELDS.iter().enumerate() {
        set(p, s, k, fl.default);
    }
    set(p, s, slice::START, start.min(end));
    set(p, s, slice::END, start.max(end));
    set(p, s, slice::SOURCE, source as f32);
    p.data[data::COUNT].set((s + 1) as f32);
    p.data[data::SELECTED].set(s as f32);
    Some(s)
}

/// Removes slice `s`: the ones after it move down, the pattern follows.
pub fn delete_slice(p: &ModuleParams, s: usize) {
    let n = count(p);
    if s >= n {
        return;
    }
    for t in s..n - 1 {
        for k in 0..slice::LEN {
            p.data[at(t, k)].set(p.data[at(t + 1, k)].get());
        }
    }
    for (k, fl) in FIELDS.iter().enumerate() {
        set(p, n - 1, k, fl.default);
    }
    for i in 0..MAX_LEN {
        match step(p, i) {
            Some(t) if t == s => set_step(p, i, None),
            Some(t) if t > s => set_step(p, i, Some(t - 1)),
            _ => {}
        }
    }
    p.data[data::COUNT].set((n - 1) as f32);
    p.data[data::LEARN].set(0.0);
    let sel = selected(p);
    if sel >= s && sel > 0 {
        p.data[data::SELECTED].set((sel - 1) as f32);
    }
}

/// Copies slice `from`'s settings onto slice `to` (its note kept).
pub fn copy_slice(p: &ModuleParams, from: usize, to: usize) {
    for k in (0..slice::LEN).filter(|&k| k != slice::NOTE) {
        p.data[at(to, k)].set(p.data[at(from, k)].get());
    }
}

/// Maps the slices to notes from `first` up, one each.
pub fn map_chromatic(p: &ModuleParams, first: u8) {
    for s in 0..count(p) {
        set(p, s, slice::NOTE, (first as usize + s).min(127) as f32);
    }
}

// ---------------------------------------------------------------------------
// The processor
// ---------------------------------------------------------------------------

/// A ratchet: strikes on the grid after the first.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Roll {
    count: u8,
    per_beat: f64,
    ramp: f32,
    fall: f32,
}

#[derive(Clone, Copy, Debug)]
enum Event {
    Strike {
        slice: usize,
        velocity: f32,
        holder: Holder,
        var: Variation,
        /// From the steps (it goes out on MIDI).
        local: bool,
        roll: Option<Roll>,
        /// A ratchet's later strike (it starts no ratchet of its own).
        repeat: bool,
    },
    /// A key let go (or the panel's PLAY, as `Holder::Panel`).
    Release(Holder),
}

#[derive(Clone, Copy, Debug)]
struct Hit {
    frame: usize,
    event: Event,
}

/// This block's hits, in the order they were found.
struct HitList {
    hits: [Hit; MAX_HITS],
    len: usize,
}

impl HitList {
    fn new() -> Self {
        HitList { hits: [Hit { frame: 0, event: Event::Release(Holder::Panel) }; MAX_HITS], len: 0 }
    }

    fn push(&mut self, h: Hit) {
        if self.len < MAX_HITS {
            self.hits[self.len] = h;
            self.len += 1;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Ratchet {
    slice: usize,
    left: u8,
    k: u8,
    /// Samples between strikes, and until the next (from this block's
    /// start).
    interval: f64,
    countdown: f64,
    velocity: f32,
    holder: Holder,
    var: Variation,
    local: bool,
    ramp: f32,
    fall: f32,
}

/// MIDI notes sent last this long (a step's: its gate).
const NOTE_SECONDS: f32 = 0.03;
/// The panel's PLAY holds its note at least this long.
const PANEL_GATE: f32 = 0.05;

pub struct Splitter {
    params: Arc<ModuleParams>,
    sr: f32,
    sample: Option<Arc<Sample>>,
    seen: u32,
    voices: Box<[Voice; VOICES]>,
    ratchets: [Option<Ratchet>; MAX_RATCHETS],
    window: Vec<f32>,
    clock: StepClock,
    was_playing: bool,
    was_running: bool,
    rng: Rng,
    out: MidiBuf,
    /// Frames until each slice's sent note ends (−1: none), and the note.
    note_off: [i64; MAX_SLICES],
    sent: [u8; MAX_SLICES],
    hits: [u32; MAX_SLICES],
    auditioned: f32,
    /// Samples since PLAY was pressed (None: not held).
    panel_held: Option<u32>,
    age: u64,
    last: Option<usize>,
    notes_in: u32,
    level: f32,
}

impl Splitter {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        params.viz[viz::STEP].set(-1.0);
        // Takes the sample it finds (a seen version that cannot be current).
        let mut seen = params.sample.version().wrapping_add(1);
        let mut sample = None;
        params.sample.fetch(&mut seen, &mut sample);
        Self {
            auditioned: params.viz[viz::AUDITION].get(),
            level: params.get(LEVEL),
            params,
            sr: sample_rate,
            sample,
            seen,
            voices: Box::new(std::array::from_fn(|i| Voice::new(0x5711_7700 ^ (i as u32).wrapping_mul(0x9E37_79B9)))),
            ratchets: [None; MAX_RATCHETS],
            window: voice::window(),
            clock: StepClock::new(),
            was_playing: false,
            was_running: false,
            rng: Rng::new(0x0A1F_A207),
            out: MidiBuf::default(),
            note_off: [-1; MAX_SLICES],
            sent: [0; MAX_SLICES],
            hits: [0; MAX_SLICES],
            panel_held: None,
            age: 0,
            last: None,
            notes_in: 0,
        }
    }

    fn source_len(&self, source: usize) -> f64 {
        if source == 0 { self.sample.as_ref().map_or(0.0, |s| s.frames() as f64) } else { ATOM_LEN }
    }

    /// A slice struck: its last voice choked, a free (or the oldest) voice
    /// started.
    fn strike(&mut self, slice: usize, velocity: f32, holder: Holder, var: &Variation, clock: &Clock) {
        let s = settings(&self.params, slice);
        for v in self.voices.iter_mut().filter(|v| v.active && v.slice == slice) {
            v.choke(self.sr);
        }
        // A free voice, or the oldest (restarted at once).
        let i = self.voices.iter().position(|v| !v.active).unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
        self.age += 1;
        let src = src_of(&self.sample, s.source);
        self.voices[i].start(slice, &s, &src, self.sr, velocity, holder, var, clock, self.age);
        self.hits[slice] = self.hits[slice].wrapping_add(1);
    }

    /// The steps of this block (see the module's documentation).
    fn sequence(&mut self, ctx: &ProcessCtx, n: usize, list: &mut HitList) {
        let p = self.params.clone();
        let running = on(&p, data::RUN);
        let started = ctx.playing && !self.was_playing;
        if ctx.restart || started || (running && !self.was_running) {
            self.clock = StepClock::new();
            self.last = None;
        }
        self.was_playing = ctx.playing;
        self.was_running = running;
        if !(running && ctx.playing) {
            p.viz[viz::STEP].set(-1.0);
            return;
        }
        let mut steps = [Step { frame: 0, beat: 0.0, index: 0 }; MAX_STEPS];
        let per_beat = RATES[rate(&p)].1;
        let found = self.clock.steps(ctx, n, per_beat, p.get(SWING), &mut steps);
        if found == 0 {
            return;
        }
        let len = length(&p);
        let n_slices = count(&p);
        let mode = mode(&p);
        let lock = on(&p, data::LOCK);
        let seed = int(&p, data::SEED, 0, i32::MAX) as u32;
        let fill = p.get(FILL).clamp(0.0, 1.0);
        let chaos = p.get(CHAOS).clamp(0.0, 1.0);
        let step_samples = 1.0 / (per_beat * ctx.beats_per_sample.max(1e-12));
        let gate = ((step_samples * p.data[data::GATE].get().clamp(5.0, 100.0) as f64 / 100.0) as u32).max(1);
        let mut pool = [0usize; MAX_SLICES];
        let mut pooled = 0;
        for s in (0..n_slices).filter(|&s| !flag(&p, s, slice::MUTE)) {
            pool[pooled] = s;
            pooled += 1;
        }
        let pool = &pool[..pooled];
        for st in &steps[..found] {
            let i = st.index.rem_euclid(len as i64) as usize;
            p.viz[viz::STEP].set(i as f32);
            p.viz[viz::INDEX].set(st.index.max(0) as f32);
            let mut rng = if lock { Rng::new(hash(seed.wrapping_mul(0x9E37_79B9) ^ (i as u32 + 1).wrapping_mul(0x85EB_CA6B)) | 1) } else { Rng::new(self.rng.next_u32()) };
            let strike = |slice: usize, var: Variation, roll: Option<Roll>| Hit {
                frame: st.frame,
                event: Event::Strike { slice, velocity: 0.9, holder: Holder::Timed(gate), var, local: true, roll, repeat: false },
            };
            let mut lane: Option<usize> = None;
            match mode {
                ORDER | RANDOM | PATTERN | STUTTER => {
                    let mut pick = match mode {
                        ORDER => (!pool.is_empty()).then(|| pool[i % pool.len()]),
                        RANDOM if !pool.is_empty() => {
                            let mut k = (rng.unit() * pool.len() as f32) as usize % pool.len();
                            if !lock && pool.len() > 1 && Some(pool[k]) == self.last {
                                k = (k + 1 + (rng.unit() * (pool.len() - 1) as f32) as usize) % pool.len();
                            }
                            Some(pool[k])
                        }
                        PATTERN => step(&p, i).filter(|&s| s < n_slices && !flag(&p, s, slice::MUTE)),
                        STUTTER => {
                            let from_pattern = step(&p, i).filter(|&s| s < n_slices && !flag(&p, s, slice::MUTE));
                            let any = (0..len).any(|j| step(&p, j).is_some_and(|s| s < n_slices));
                            if any { from_pattern } else { (!pool.is_empty()).then(|| pool[i % pool.len()]) }
                        }
                        _ => None,
                    };
                    if !roll(&mut rng, fill) {
                        pick = None;
                    }
                    let mut stutter = None;
                    if mode == STUTTER && pick.is_some() {
                        // Now and then the last slice is held instead.
                        if self.last.is_some() && rng.unit() < chaos * 0.5 {
                            pick = self.last;
                        }
                        if rng.unit() < 0.4 + 0.6 * chaos {
                            stutter = Some(stutter_roll(&mut rng, chaos));
                        }
                    }
                    if let Some(s) = pick {
                        let (var, extra) = vary(&mut rng, chaos);
                        list.push(strike(s, var, stutter.or(extra)));
                        self.last = Some(s);
                    }
                    lane = Some(pick.unwrap_or(usize::MAX));
                }
                PROB => {
                    for &s in pool {
                        if rng.unit() < get(&p, s, slice::PROB) * 0.01 * fill {
                            let (var, extra) = vary(&mut rng, chaos);
                            list.push(strike(s, var, extra));
                        }
                    }
                }
                _ => {
                    for &s in pool {
                        let k = get_int(&p, s, slice::HITS).max(0) as usize;
                        let steps_n = get_int(&p, s, slice::STEPS).max(1) as usize;
                        let rot = get_int(&p, s, slice::ROTATE).max(0) as usize;
                        let at_step = st.index.rem_euclid(steps_n as i64) as usize;
                        if euclid(k, steps_n, rot, at_step) && roll(&mut rng, fill) {
                            let (var, extra) = vary(&mut rng, chaos);
                            list.push(strike(s, var, extra));
                        }
                    }
                }
            }
            if let Some(l) = lane {
                p.viz[viz::STEP_SLICE + i.min(MAX_LEN - 1)].set(if l == usize::MAX { -1.0 } else { l as f32 });
            }
        }
    }
}

/// What slices of `source` read.
fn src_of(sample: &Option<Arc<Sample>>, source: usize) -> Src<'_> {
    if source == 0 {
        match sample {
            Some(s) if s.frames() >= 2 => Src::Sample { l: &s.left, r: s.right(), rate: s.rate },
            _ => Src::Empty,
        }
    } else {
        Src::Atom((source - 1).min(atoms::ATOMS.len() - 1))
    }
}

/// CHAOS: now and then a hit is played another way.
fn vary(rng: &mut Rng, chaos: f32) -> (Variation, Option<Roll>) {
    let mut var = Variation::NONE;
    if chaos <= 0.0 || rng.unit() >= chaos * 0.5 {
        return (var, None);
    }
    match (rng.unit() * 6.0) as usize {
        0 => var.flip = true,
        1 => var.pitch = if rng.unit() < 0.5 { 12.0 } else { -12.0 },
        2 => var.pitch = if rng.unit() < 0.5 { 7.0 } else { -5.0 },
        3 => var.skip = 0.25 + 0.4 * rng.unit(),
        4 => var.gain = 0.5,
        _ => return (var, Some(Roll { count: 2 + (rng.unit() * 2.0) as u8, per_beat: if rng.unit() < 0.5 { 8.0 } else { 16.0 }, ramp: 0.0, fall: 0.25 })),
    }
    (var, None)
}

/// The STUTTER mode's roll: a burst of repeats on the grid, maybe climbing
/// or falling in pitch.
fn stutter_roll(rng: &mut Rng, chaos: f32) -> Roll {
    const COUNTS: [u8; 6] = [2, 3, 4, 6, 8, 12];
    const PER_BEAT: [f64; 5] = [8.0, 12.0, 16.0, 16.0, 32.0];
    const RAMPS: [f32; 8] = [0.0, 0.0, 0.0, 12.0, -12.0, 7.0, -5.0, 1.0];
    let pick = |rng: &mut Rng, n: usize, reach: f32| ((rng.unit() * (n as f32) * reach) as usize).min(n - 1);
    let reach = 0.5 + 0.5 * chaos;
    Roll {
        count: COUNTS[pick(rng, COUNTS.len(), reach)],
        per_beat: PER_BEAT[pick(rng, PER_BEAT.len(), 1.0)],
        ramp: RAMPS[pick(rng, RAMPS.len(), reach)],
        fall: 0.1 + 0.3 * rng.unit(),
    }
}

impl Processor for Splitter {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        self.out.clear();
        let p = self.params.clone();

        // A new sample from the panel: voices on the old one stop.
        let mut sample = self.sample.take();
        if p.sample.fetch(&mut self.seen, &mut sample) {
            for v in self.voices.iter_mut().filter(|v| v.source == 0) {
                v.active = false;
            }
        }
        self.sample = sample;

        let mut list = HitList::new();
        let n_slices = count(&p);

        // MIDI in.
        if inputs.midi_connected {
            let listen = midi_channel(&p);
            for e in inputs.midi.events() {
                let [status, d1, d2] = e.data;
                if !(0x80..0xF0).contains(&status) || listen.is_some_and(|c| c != status & 0x0F) {
                    continue;
                }
                let frame = (e.frame as usize).min(n.saturating_sub(1));
                match status & 0xF0 {
                    0x90 if d2 > 0 => {
                        self.notes_in = self.notes_in.wrapping_add(1);
                        p.viz[viz::NOTE_IN].set(d1 as f32);
                        let learn = int(&p, data::LEARN, 0, MAX_SLICES as i32);
                        if learn > 0 && (learn as usize) <= n_slices {
                            set(&p, learn as usize - 1, slice::NOTE, d1 as f32);
                            p.data[data::LEARN].set(0.0);
                        }
                        let velocity = d2 as f32 / 127.0;
                        let mut mapped = false;
                        for s in (0..n_slices).filter(|&s| note(&p, s) == Some(d1)) {
                            mapped = true;
                            list.push(Hit { frame, event: Event::Strike { slice: s, velocity, holder: Holder::Key(d1), var: Variation::NONE, local: false, roll: None, repeat: false } });
                        }
                        if !mapped && on(&p, data::CHROMA) && n_slices > 0 {
                            let var = Variation { pitch: d1 as f32 - 60.0, ..Variation::NONE };
                            list.push(Hit { frame, event: Event::Strike { slice: selected(&p), velocity, holder: Holder::Key(d1), var, local: false, roll: None, repeat: false } });
                        }
                    }
                    0x80 | 0x90 => list.push(Hit { frame, event: Event::Release(Holder::Key(d1)) }),
                    _ => {}
                }
            }
        }
        p.viz[viz::NOTES_IN].set(self.notes_in as f32);

        // The panel's PLAY.
        let a = p.viz[viz::AUDITION].get();
        if a != self.auditioned {
            self.auditioned = a;
            if n_slices > 0 {
                list.push(Hit { frame: 0, event: Event::Strike { slice: selected(&p), velocity: 0.9, holder: Holder::Panel, var: Variation::NONE, local: false, roll: None, repeat: false } });
                self.panel_held = Some(0);
            }
        }
        if let Some(t) = &mut self.panel_held {
            *t = t.saturating_add(n as u32);
            if p.viz[viz::HOLD].get() < 0.5 && *t as f32 >= PANEL_GATE * self.sr {
                list.push(Hit { frame: 0, event: Event::Release(Holder::Panel) });
                self.panel_held = None;
            }
        }

        // The animation.
        self.sequence(ctx, n, &mut list);

        // Ratchets: new ones from this block's strikes, then every ratchet's
        // strikes that land in this block.
        let bps = ctx.beats_per_sample.max(1e-12);
        for i in 0..list.len {
            let h = list.hits[i];
            let Event::Strike { slice: s, velocity, holder, var, local, roll, repeat: false } = h.event else { continue };
            let own = get_int(&p, s, slice::REPEAT).max(1) as u8;
            let roll = roll.or((own > 1).then(|| Roll { count: own, per_beat: repeat_rate(&p, s), ramp: get(&p, s, slice::RAMP), fall: get(&p, s, slice::FALL) * 0.01 }));
            let Some(roll) = roll.filter(|r| r.count > 1) else { continue };
            let interval = 1.0 / (roll.per_beat * bps);
            if let Some(slot) = self.ratchets.iter_mut().find(|r| r.is_none()) {
                *slot = Some(Ratchet { slice: s, left: roll.count - 1, k: 1, interval, countdown: h.frame as f64 + interval, velocity, holder, var, local, ramp: roll.ramp, fall: roll.fall });
            }
        }
        for slot in self.ratchets.iter_mut() {
            let Some(r) = slot else { continue };
            while r.left > 0 && r.countdown < n as f64 {
                let k = r.k as f32;
                let var = Variation { pitch: r.var.pitch + r.ramp * k, gain: r.var.gain * (1.0 - r.fall).max(0.0).powf(k), ..r.var };
                list.push(Hit { frame: r.countdown.max(0.0) as usize, event: Event::Strike { slice: r.slice, velocity: r.velocity, holder: r.holder, var, local: r.local, roll: None, repeat: true } });
                r.k += 1;
                r.left -= 1;
                r.countdown += r.interval;
            }
            r.countdown -= n as f64;
            if r.left == 0 {
                *slot = None;
            }
        }
        // A key let go takes the rest of its held slices' repeats with it.
        for h in &list.hits[..list.len] {
            if let Event::Release(holder) = h.event {
                for slot in self.ratchets.iter_mut() {
                    if slot.is_some_and(|r| r.holder == holder && get_int(&p, r.slice, slice::PLAY) != voice::ONE as i32) {
                        *slot = None;
                    }
                }
            }
        }
        let hits = &mut list.hits[..list.len];
        hits.sort_by_key(|h| h.frame);

        // MIDI out: the steps' strikes. Pending note-offs count from the
        // last block's start: from this one's now.
        let out_ch = midi_channel(&p).unwrap_or(0);
        let send = on(&p, data::MIDI_OUT);
        for (s, off) in self.note_off.iter_mut().enumerate() {
            if *off >= 0 {
                *off -= n as i64;
                if *off < n as i64 {
                    self.out.push((*off).max(0) as usize, [0x80 | out_ch, self.sent[s], 0]);
                    *off = -1;
                }
            }
        }
        if send {
            for h in hits.iter() {
                let Event::Strike { slice: s, velocity, holder, local: true, .. } = h.event else { continue };
                if self.note_off[s] >= 0 {
                    self.out.push(h.frame, [0x80 | out_ch, self.sent[s], 0]);
                }
                self.sent[s] = out_note(&p, s);
                self.out.push(h.frame, [0x90 | out_ch, self.sent[s], ((velocity * 127.0).round() as u8).clamp(1, 127)]);
                let length = if let Holder::Timed(g) = holder { g as i64 } else { (NOTE_SECONDS * self.sr) as i64 };
                self.note_off[s] = h.frame as i64 + length.max(1);
            }
            // Notes so short they end in this block.
            for (s, off) in self.note_off.iter_mut().enumerate() {
                if *off >= 0 && *off < n as i64 {
                    self.out.push(*off as usize, [0x80 | out_ch, self.sent[s], 0]);
                    *off = -1;
                }
            }
        }

        // The voices follow their slices' settings (block rate).
        let clock = Clock { beat: ctx.beat, beats_per_sample: bps, playing: ctx.playing };
        let mut cache: [Option<Settings>; MAX_SLICES] = [None; MAX_SLICES];
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let cfg = *cache[v.slice].get_or_insert_with(|| settings(&p, v.slice));
            let src = src_of(&self.sample, cfg.source);
            v.update(&cfg, &src, self.sr);
            v.sync_grains(&clock, self.sr);
        }

        // Sound, in stretches between the hits.
        let idle = hits.is_empty() && self.voices.iter().all(|v| !v.active);
        let mut acc_l = [0.0f32; BLOCK];
        let mut acc_r = [0.0f32; BLOCK];
        if !idle {
            let mut next = 0;
            let mut from = 0;
            while from < n {
                while next < hits.len() && hits[next].frame <= from {
                    match hits[next].event {
                        Event::Strike { slice: s, velocity, holder, var, .. } => {
                            let hit_clock = Clock { beat: ctx.beat + bps * from as f64, ..clock };
                            self.strike(s, velocity, holder, &var, &hit_clock);
                        }
                        Event::Release(holder) => {
                            for v in self.voices.iter_mut().filter(|v| v.active && v.holder == holder) {
                                if get_int(&p, v.slice, slice::PLAY) != voice::ONE as i32 {
                                    v.release();
                                }
                            }
                        }
                    }
                    next += 1;
                }
                let to = if next < hits.len() { hits[next].frame.clamp(from + 1, n) } else { n };
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    let src = src_of(&self.sample, v.source);
                    v.render(&src, &mut acc_l[from..to], &mut acc_r[from..to], &self.window);
                }
                from = to;
            }
        }
        let target = p.get(LEVEL).clamp(0.0, 1.0);
        let mut peak = 0.0f32;
        for i in 0..n {
            self.level += (target - self.level) * ctx.smooth;
            let l = soft_limit(acc_l[i] * self.level);
            let r = soft_limit(acc_r[i] * self.level);
            outputs[0][0][i] = l;
            outputs[0][1][i] = r;
            peak = peak.max(l.abs().max(r.abs()));
        }
        p.meter.fetch_max(peak);

        // What the panel shows.
        let mut on_count = 0;
        let mut grains = [0.0f32; viz::SHOWN_GRAINS];
        let mut shown = 0;
        for (i, v) in self.voices.iter().enumerate() {
            let base = viz::HEADS + 3 * i;
            if v.active {
                on_count += 1;
                let len = self.source_len(v.source);
                p.viz[base].set(v.slice as f32);
                p.viz[base + 1].set(v.position(len));
                p.viz[base + 2].set(v.level);
                if shown < viz::SHOWN_GRAINS {
                    let got = v.grain_positions(len, &mut grains[shown..]);
                    for g in &grains[shown..shown + got] {
                        p.viz[viz::GRAINS + 2 * shown].set(*g);
                        p.viz[viz::GRAINS + 2 * shown + 1].set(v.slice as f32);
                        shown += 1;
                    }
                }
            } else {
                p.viz[base].set(-1.0);
            }
        }
        for g in shown..viz::SHOWN_GRAINS {
            p.viz[viz::GRAINS + 2 * g + 1].set(-1.0);
        }
        p.viz[viz::VOICES_ON].set(on_count as f32);
        for (s, h) in self.hits.iter().enumerate() {
            p.viz[viz::HITS + s].set(*h as f32);
        }
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}
