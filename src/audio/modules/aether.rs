//! Aether: warm drones and slowly turning pads for ambient music, moved by
//! fractal motion, with glitches in tune.
//!
//! Five voices hold a chord: a CHORD shape on ROOT, or the keys on the MIDI
//! input (one key plays the shape on it, more keys play themselves). FREE
//! imposes no chord: each key gets a voice of its own, as on a plain pad,
//! and with no keys ROOT sounds alone, in unison. With GLIDE the voices
//! glide to their new notes, else they step there. Each
//! voice is two detuned layers of eight partials, built additively, so
//! every partial can move on its own. All the motion is fractal
//! ([`fractal`]): one curve at six time scales, from a whole EVOLVE period
//! (in bars when synced, else in seconds) down to 1/32 of it. It morphs the
//! timbre between four spectra (COLOR, how far: MORPH), opens and closes
//! the tone, drifts each voice's pitch, level and place, breathes the
//! detune, and ripples each partial's level and tuning (SHIMMER) — the
//! higher partials faster: the same motion, scaled.
//!
//! On the board's grid, glitches land on harmonics of the chord tones
//! ([`glitch`]): glassy pings and skips that loop in tune, ratcheted in
//! time.
//!
//! The sound then goes through soft saturation, a chorus and a wide,
//! diffuse space with air ([`space`]). It plays while the transport does
//! (or while keys are held), swelling in and out like breath.

pub mod fractal;
pub mod glitch;
pub mod space;

use std::sync::Arc;

use self::fractal::{Lane, OCTAVES, PARTIALS, spectrum, spectrum_name};
use self::glitch::{Fire, Glitch, HARMONICS, QUEUE, Settings};
use self::space::{Chorus, Space, Warmth};
use super::steps::{Held, MAX_STEPS, Step, StepClock, note_event, steps_per_beat};
use super::{ModuleDef, ModuleParams, Quiet, Rng, approach, pan_gains, silence, soft_limit, time_coeff};
use crate::audio::engine::{BLOCK, Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::midi::{MidiState, note_hz};
use crate::audio::params::{DIVISIONS, ParamSpec, Unit, format_db, gain_to_db, note_label};

pub const ROOT: usize = 0;
pub const DETUNE: usize = 1;
pub const COLOR: usize = 2;
pub const TONE: usize = 3;
pub const MORPH: usize = 4;
pub const SHIMMER: usize = 5;
pub const EVOLVE: usize = 6;
pub const ROUGH: usize = 7;
pub const DRIVE: usize = 8;
pub const CHORUS: usize = 9;
pub const SPACE: usize = 10;
pub const AIR: usize = 11;
pub const WIDTH: usize = 12;
pub const LEVEL: usize = 13;
pub const GLITCH: usize = 14;
pub const HARM: usize = 15;
pub const REPEAT: usize = 16;
pub const SKIP: usize = 17;
pub const DECAY: usize = 18;
pub const SPARK: usize = 19;

const PARAMS: [ParamSpec; 20] = [
    ParamSpec::lin("ROOT", 24.0, 72.0, 45.0, Unit::Note),
    ParamSpec::lin("DETUNE", 0.0, 40.0, 11.0, Unit::Cents),
    ParamSpec::lin("COLOR", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("TONE", 0.0, 1.0, 0.42, Unit::Percent),
    ParamSpec::lin("MORPH", 0.0, 1.0, 0.55, Unit::Percent),
    ParamSpec::lin("SHIMMER", 0.0, 1.0, 0.45, Unit::Percent),
    ParamSpec::lin("EVOLVE", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("ROUGH", 0.0, 1.0, 0.4, Unit::Percent),
    ParamSpec::lin("DRIVE", 0.0, 1.0, 0.35, Unit::Percent),
    ParamSpec::lin("CHORUS", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("SPACE", 0.0, 1.0, 0.55, Unit::Percent),
    ParamSpec::lin("AIR", 0.0, 1.0, 0.35, Unit::Percent),
    ParamSpec::lin("WIDTH", 0.0, 1.0, 0.75, Unit::Percent),
    ParamSpec::lin("LEVEL", 0.0, 1.0, 0.6, Unit::Decibel),
    ParamSpec::lin("GLITCH", 0.0, 1.0, 0.12, Unit::Percent),
    ParamSpec::lin("HARM", 2.0, 16.0, 8.0, Unit::Integer),
    ParamSpec::lin("REPEAT", 0.0, 1.0, 0.35, Unit::Percent),
    ParamSpec::lin("SKIP", 0.0, 1.0, 0.25, Unit::Percent),
    ParamSpec::log("DECAY", 0.03, 2.0, 0.4, Unit::Seconds),
    ParamSpec::lin("SPARK", 0.0, 1.0, 0.5, Unit::Percent),
];

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

pub const VOICES: usize = 5;

/// The chord shapes (the mode): name, what it is, semitones per voice.
pub const CHORDS: [(&str, &str, [i32; VOICES]); 8] = [
    ("5TH", "Fifths and octaves: the plainest drone", [0, 7, 12, 19, 24]),
    ("SUS", "Suspended: open, never resolving", [0, 7, 14, 17, 24]),
    ("MAJ9", "Major ninth: warm and bright", [0, 7, 14, 16, 23]),
    ("MIN9", "Minor ninth: warm and dark", [0, 7, 14, 15, 22]),
    ("MIN11", "Minor eleventh: modal, unhurried", [0, 7, 10, 15, 17]),
    ("4THS", "Stacked fourths: hollow, floating", [0, 5, 10, 15, 20]),
    ("LYD", "Lydian: major seventh and raised eleventh, weightless", [0, 7, 14, 18, 23]),
    ("FREE", "No chord, no scale: the keys as you play them, a voice each (a plain pad).\nWith no keys, ROOT alone in unison", [0; VOICES]),
];
/// The shape that imposes nothing.
pub const FREE: usize = 7;

/// Settings beyond the knobs.
pub mod data {
    /// The glitch grid: an index into `DIVISIONS`.
    pub const DIV: usize = 0;
    /// Keys on the MIDI input: 0 hold the chord they play (LATCH), 1
    /// sound only while held (GATE).
    pub const KEYS: usize = 1;
    /// The motion's seed: RESEED picks another evolution.
    pub const SEED: usize = 2;
    /// 1: a voice glides to its new note (portamento); 0: it steps there.
    pub const GLIDE: usize = 3;
    pub const LEN: usize = 4;
}

const DATA: [f32; data::LEN] = [3.0, 0.0, 1.0, 0.0];

/// What the processor shows the panel.
pub mod viz {
    use super::{PARTIALS, VOICES};
    /// Each voice's pitch now, as a MIDI note (drift included).
    pub const NOTE: usize = 0;
    /// Each voice's partial levels, voice by voice (after all the motion).
    pub const AMP: usize = NOTE + VOICES;
    /// The motion's position, in EVOLVE periods.
    pub const POS: usize = AMP + VOICES * PARTIALS;
    /// The colour and brightness now, after the motion (0…1).
    pub const COLOR: usize = POS + 1;
    pub const BRIGHT: usize = COLOR + 1;
    /// How far the sound has swelled in (0…1).
    pub const RUN: usize = BRIGHT + 1;
    /// How far each voice has faded in (in FREE, voices without a key
    /// fade out; else all are in).
    pub const LIVE: usize = RUN + 1;
    /// Where the chord comes from: 0 the knobs, 1 keys held, 2 keys latched.
    pub const SOURCE: usize = LIVE + VOICES;
    /// The glitch grid's step now (−1: not running).
    pub const STEP: usize = SOURCE + 1;
    /// Glitches so far, and the latest ones: a ring of [`EVENTS`] of
    /// (count, frequency in Hz, kind: `glitch::PING` or `SKIP`).
    pub const FIRED: usize = STEP + 1;
    pub const EVENT: usize = FIRED + 1;
    pub const EVENTS: usize = 8;
    /// The panel bumps this to let go of latched keys.
    pub const UNLATCH: usize = EVENT + 3 * EVENTS;
    pub const END: usize = UNLATCH + 1;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Aether",
    prefix: "AETHER",
    hint: "fractal pads · harmonic glitch",
    inputs: &[],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["5TH", "SUS", "MAJ9", "MIN9", "MIN11", "4THS", "LYD", "FREE"],
    toggle: Some("SYNC"),
    toggle_default: true,
    map: false,
    midi_in: true,
    midi_out: false,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::END,
};

pub fn chord(p: &ModuleParams) -> usize {
    p.mode().min(CHORDS.len() - 1)
}

/// FREE: no chord imposed.
pub fn free(p: &ModuleParams) -> bool {
    chord(p) == FREE
}

/// The glitch grid (an index into `DIVISIONS`).
pub fn division(p: &ModuleParams) -> usize {
    (p.data[data::DIV].get().round().max(0.0) as usize).min(DIVISIONS.len() - 1)
}

/// Keys sound only while held.
pub fn gated(p: &ModuleParams) -> bool {
    p.data[data::KEYS].get() > 0.5
}

/// Voices glide to their new notes, rather than step there.
pub fn glides(p: &ModuleParams) -> bool {
    p.data[data::GLIDE].get() > 0.5
}

pub fn seed(p: &ModuleParams) -> u32 {
    p.data[data::SEED].get().max(0.0) as u32
}

/// A new evolution (kept to 24 bits: the setting is a float).
pub fn reseed(p: &ModuleParams, seed: u32) {
    p.data[data::SEED].set((seed & 0xFF_FFFF).max(1) as f32);
}

/// EVOLVE's period synced: 1 to 64 bars, doubling.
pub fn evolve_bars(v: f32) -> f32 {
    2f32.powi((v.clamp(0.0, 1.0) * 6.0).round() as i32)
}

/// EVOLVE's period free: 4 s to 4 minutes.
pub fn evolve_seconds(v: f32) -> f32 {
    4.0 * 60f32.powf(v.clamp(0.0, 1.0))
}

/// The chord the knobs play, as MIDI notes.
pub fn knob_chord(p: &ModuleParams) -> [f32; VOICES] {
    let root = p.get(ROOT).round();
    CHORDS[chord(p)].2.map(|s| root + s as f32)
}

/// The keys, each once, low to high.
fn sorted_keys(keys: &[u8]) -> ([u8; Held::CAP], usize) {
    let mut sorted = [0u8; Held::CAP];
    let mut len = 0;
    for &k in keys.iter().take(Held::CAP) {
        if !sorted[..len].contains(&k) {
            sorted[len] = k;
            len += 1;
        }
    }
    sorted[..len].sort_unstable();
    (sorted, len)
}

/// `VOICES` of `n` sorted notes, evenly spread (the lowest and the highest
/// kept).
fn thinned(notes: &[u8]) -> [f32; VOICES] {
    let n = notes.len();
    std::array::from_fn(|v| notes[(v * (n - 1) + (VOICES - 1) / 2) / (VOICES - 1)] as f32)
}

/// The voices for the keys held: one key plays `shape` on it; more keys
/// play themselves, doubled an octave up (then two) to fill the voices, or
/// thinned evenly when there are more than voices.
pub fn voicing(keys: &[u8], shape: &[i32; VOICES]) -> [f32; VOICES] {
    let (sorted, len) = sorted_keys(keys);
    let notes = &sorted[..len];
    match len {
        0 => [60.0; VOICES],
        1 => shape.map(|s| (notes[0] as i32 + s).clamp(12, 108) as f32),
        n if n >= VOICES => thinned(notes),
        n => {
            let mut out: [f32; VOICES] = std::array::from_fn(|v| {
                let octave = v / n;
                (notes[v % n] as i32 + 12 * octave as i32).min(108) as f32
            });
            out.sort_unstable_by(f32::total_cmp);
            out
        }
    }
}

/// FREE: the notes to play and how many — the keys as they are (thinned
/// evenly past five), or with none ROOT on every voice, in unison.
pub fn free_notes(keys: &[u8], root: f32) -> ([f32; VOICES], usize) {
    let (sorted, len) = sorted_keys(keys);
    let notes = &sorted[..len];
    match len {
        0 => ([root.round(); VOICES], VOICES),
        n if n >= VOICES => (thinned(notes), VOICES),
        n => (std::array::from_fn(|v| notes[v.min(n - 1)] as f32), n),
    }
}

/// A knob's value as the panel shows it.
pub fn format(p: &ModuleParams, i: usize) -> String {
    let v = p.get(i);
    match i {
        ROOT => note_label(v.round() as i32),
        DETUNE => format!("{v:.0} ct"),
        COLOR => spectrum_name(v).to_owned(),
        EVOLVE if p.toggled() => match evolve_bars(v) as u32 {
            1 => "1 bar".to_owned(),
            bars => format!("{bars} bars"),
        },
        EVOLVE => {
            let s = evolve_seconds(v);
            if s < 60.0 { format!("{s:.0} s") } else { format!("{:.1} min", s / 60.0) }
        }
        HARM => format!("×{}", HARMONICS[glitch::allowed(v) - 1]),
        DECAY if v < 1.0 => format!("{:.0} ms", v * 1000.0),
        DECAY => format!("{v:.2} s"),
        LEVEL => format_db(gain_to_db(v)),
        _ => format!("{:.0}%", v * 100.0),
    }
}

/// What RANDOMIZE draws each knob from: the part of its range that stays
/// warm and musical.
const TIMBRE_RANGES: [(usize, f32, f32); 12] = [
    (DETUNE, 3.0, 24.0),
    (COLOR, 0.0, 1.0),
    (TONE, 0.2, 0.7),
    (MORPH, 0.2, 0.85),
    (SHIMMER, 0.15, 0.8),
    (EVOLVE, 0.2, 0.9),
    (ROUGH, 0.1, 0.75),
    (DRIVE, 0.1, 0.65),
    (CHORUS, 0.2, 0.85),
    (SPACE, 0.3, 0.85),
    (AIR, 0.1, 0.6),
    (WIDTH, 0.5, 1.0),
];
const GLITCH_RANGES: [(usize, f32, f32); 3] = [(REPEAT, 0.0, 0.7), (SKIP, 0.0, 0.6), (SPARK, 0.3, 0.8)];

/// RANDOMIZE: a new sound — the chord shape (FREE stays FREE), the timbre,
/// the motion and its seed, the warm stages and the glitches. ROOT, LEVEL,
/// SYNC and the keys' mode stay. `timbre_only` also keeps the chord, the
/// seed and the glitches.
pub fn randomize(p: &ModuleParams, seed: u32, timbre_only: bool) {
    let mut rng = Rng::new(seed);
    for (i, lo, hi) in TIMBRE_RANGES {
        p.set(i, lo + (hi - lo) * rng.unit());
    }
    if timbre_only {
        return;
    }
    for (i, lo, hi) in GLITCH_RANGES {
        p.set(i, lo + (hi - lo) * rng.unit());
    }
    // Mostly sparse: glitches are a spice.
    p.set(GLITCH, 0.4 * rng.unit().powi(2));
    p.set(HARM, (4.0 + 10.0 * rng.unit()).round());
    p.set(DECAY, 0.08 * (1.5f32 / 0.08).powf(rng.unit()));
    // Eighths to sixteenth triplets.
    p.data[data::DIV].set(1.0 + (rng.unit() * 4.0).floor());
    if !free(p) {
        p.set_mode((rng.unit() * FREE as f32) as usize % FREE);
    }
    reseed(p, (rng.unit() * 16_777_216.0) as u32);
}

/// A built-in sound: its name, chord shape, glitch grid (an index into
/// `DIVISIONS`), and the knobs it sets away from their defaults.
pub type Sound = (&'static str, usize, usize, &'static [(usize, f32)]);

pub const FACTORY: [Sound; 7] = [
    ("Default", 0, 3, &[]),
    ("Plain pad", FREE, 3, &[
        (DETUNE, 14.0), (COLOR, 0.33), (TONE, 0.45), (MORPH, 0.3), (SHIMMER, 0.3), (ROUGH, 0.25),
        (DRIVE, 0.3), (CHORUS, 0.65), (SPACE, 0.5), (AIR, 0.25), (GLITCH, 0.0),
    ]),
    ("Glass choir", 2, 1, &[
        (COLOR, 1.0), (TONE, 0.6), (SHIMMER, 0.65), (AIR, 0.6), (SPACE, 0.75),
        (GLITCH, 0.22), (HARM, 12.0), (REPEAT, 0.25), (SKIP, 0.05), (DECAY, 1.2), (SPARK, 0.55),
    ]),
    ("Deep drone", 0, 3, &[
        (DETUNE, 8.0), (COLOR, 0.0), (TONE, 0.25), (MORPH, 0.45), (EVOLVE, 0.84), (ROUGH, 0.3),
        (DRIVE, 0.6), (CHORUS, 0.35), (SPACE, 0.7), (AIR, 0.12), (GLITCH, 0.04),
    ]),
    ("Hollow fourths", 5, 3, &[
        (COLOR, 0.66), (TONE, 0.5), (SHIMMER, 0.7), (ROUGH, 0.6), (CHORUS, 0.7), (WIDTH, 1.0),
        (GLITCH, 0.15), (HARM, 9.0),
    ]),
    ("Shards", 4, 4, &[
        (TONE, 0.5), (EVOLVE, 0.33), (ROUGH, 0.75),
        (GLITCH, 0.45), (REPEAT, 0.7), (SKIP, 0.55), (DECAY, 0.15), (SPARK, 0.65),
    ]),
    ("Night air", 3, 3, &[
        (COLOR, 0.4), (TONE, 0.35), (MORPH, 0.65), (SHIMMER, 0.5), (EVOLVE, 1.0),
        (SPACE, 0.85), (AIR, 0.7), (WIDTH, 0.95), (GLITCH, 0.08), (SKIP, 0.0), (DECAY, 1.6),
    ]),
];

/// Built-in sound `k`, from the defaults. It keeps ROOT, LEVEL, GLIDE and
/// the keys' mode: they are the player's.
pub fn load_factory(p: &ModuleParams, k: usize) {
    let (_, shape, div, knobs) = FACTORY[k.min(FACTORY.len() - 1)];
    for (i, spec) in PARAMS.iter().enumerate().filter(|(i, _)| ![ROOT, LEVEL].contains(i)) {
        p.set(i, spec.default);
    }
    for &(i, v) in knobs {
        p.set(i, v);
    }
    p.set_mode(shape);
    p.set_toggle(DEF.toggle_default);
    p.data[data::DIV].set(div as f32);
    p.data[data::SEED].set(DATA[data::SEED]);
}

/// Oscillator banks: one per layer of a voice, a lane per partial.
const BANKS: usize = VOICES * 2;
/// Partials quieter than this (−94 dBFS) are not worked out.
const SILENT: f32 = 2e-5;
/// Each voice's level, low to high, and its place in the stereo field.
const VOICE_GAIN: [f32; VOICES] = [1.0, 0.82, 0.72, 0.62, 0.52];
const VOICE_PAN: [f32; VOICES] = [0.0, -0.5, 0.5, -0.85, 0.85];
/// Overall level of the voices before the warm stages.
const NORM: f32 = 0.21;
/// FREE: each voice's level (a note in unison shares it), and how fast a
/// voice fades in for its key and out once let go (time constants).
const FREE_GAIN: f32 = 1.2;
const FADE_IN: f32 = 0.3;
const FADE_OUT: f32 = 1.1;
/// The motion's strands: one per thing it moves.
const LANE_PITCH: u32 = 10;
pub const LANE_LEVEL: u32 = 20;
pub const LANE_PAN: u32 = 30;
const LANE_DETUNE: u32 = 40;
const LANE_PARTIAL: u32 = 100;
pub const LANE_COLOR: u32 = 1;
pub const LANE_BRIGHT: u32 = 2;
const LANE_CHORUS: u32 = 3;
const LANE_AIR: u32 = 4;
/// Partials move faster the higher they are: their time scale.
fn partial_speed(k: usize) -> f64 {
    1.0 + 0.6 * k as f64
}
/// Motion faster than this (cells shorter, in seconds) fades out: it
/// would tremble rather than drift.
const SHORTEST: f32 = 0.12;

/// Each octave's share for a motion whose slowest cell lasts `cell`
/// seconds.
fn weights(cell: f32) -> [f32; OCTAVES] {
    std::array::from_fn(|o| (((cell / (1u32 << o) as f32) - SHORTEST) / SHORTEST).clamp(0.0, 1.0))
}

/// Eight oscillators side by side, a lane per partial: phasors turned by
/// a fixed rotation each sample (cheaper than a sine each), the eight
/// lanes running as one vector.
#[derive(Clone, Copy, Default)]
struct Bank {
    /// Each phase's cosine and sine.
    c: [f32; PARTIALS],
    s: [f32; PARTIALS],
    /// The turn per sample.
    rc: [f32; PARTIALS],
    rs: [f32; PARTIALS],
    /// Left and right gains now, their step per sample, and the smoothed
    /// targets they ramp to over the block.
    gl: [f32; PARTIALS],
    gr: [f32; PARTIALS],
    dl: [f32; PARTIALS],
    dr: [f32; PARTIALS],
    tl: [f32; PARTIALS],
    tr: [f32; PARTIALS],
    live: bool,
}

#[allow(clippy::needless_range_loop)] // the lanes of several arrays at once
impl Bank {
    fn new(rng: &mut Rng) -> Self {
        let phase: [f32; PARTIALS] = std::array::from_fn(|_| rng.unit());
        Self { c: phase.map(fastmath::cos_turns), s: phase.map(fastmath::sin_turns), ..Default::default() }
    }

    /// Sets each lane's turn per sample (`inc`, in cycles) and the gains'
    /// ramps over `n` samples; puts the phasors back on the unit circle.
    fn prepare(&mut self, inc: &[f32; PARTIALS], n: usize) {
        let inv_n = 1.0 / n as f32;
        let mut loud = 0.0f32;
        for k in 0..PARTIALS {
            self.rc[k] = fastmath::cos_turns(inc[k]);
            self.rs[k] = fastmath::sin_turns(inc[k]);
            let r = 1.0 / (self.c[k] * self.c[k] + self.s[k] * self.s[k]).sqrt();
            self.c[k] *= r;
            self.s[k] *= r;
            self.dl[k] = (self.tl[k] - self.gl[k]) * inv_n;
            self.dr[k] = (self.tr[k] - self.gr[k]) * inv_n;
            loud = loud.max(self.gl[k]).max(self.gr[k]).max(self.tl[k]).max(self.tr[k]);
        }
        self.live = loud > SILENT;
    }

    /// One sample, added lane by lane into `l` and `r`.
    #[inline]
    fn tick(&mut self, l: &mut [f32; PARTIALS], r: &mut [f32; PARTIALS]) {
        for k in 0..PARTIALS {
            let (c, s) = (self.c[k], self.s[k]);
            self.c[k] = c * self.rc[k] - s * self.rs[k];
            self.s[k] = s * self.rc[k] + c * self.rs[k];
            l[k] += self.s[k] * self.gl[k];
            r[k] += self.s[k] * self.gr[k];
            self.gl[k] += self.dl[k];
            self.gr[k] += self.dr[k];
        }
    }

    /// The block is done: the gains are at their targets.
    fn settle(&mut self) {
        self.gl = self.tl;
        self.gr = self.tr;
    }
}

/// Keys on the MIDI input.
struct Keys {
    held: Held,
    /// The chord last played, kept after the keys let go.
    latched: [u8; Held::CAP],
    latched_len: usize,
}

impl Keys {
    /// Where the notes come from (see [`viz::SOURCE`]) and the keys: held
    /// ones when gated; else the chord of the last key pressed, which stays
    /// as the keys let go one by one.
    fn now(&self, gated: bool) -> (usize, &[u8]) {
        let latched = &self.latched[..self.latched_len];
        match (self.held.is_empty(), gated) {
            (false, true) => (1, self.held.notes()),
            (false, false) => (1, latched),
            _ if !latched.is_empty() => (2, latched),
            _ => (0, &[]),
        }
    }

    fn handle(&mut self, msg: [u8; 3]) {
        match note_event(msg) {
            Some(true) => {
                // A fresh chord once every key was up.
                if self.held.is_empty() {
                    self.latched_len = 0;
                }
                self.held.press(msg[1]);
                let notes = self.held.notes();
                self.latched[..notes.len()].copy_from_slice(notes);
                self.latched_len = notes.len();
            }
            Some(false) => self.held.release(msg[1]),
            // All notes off, all sound off.
            None if msg[0] & 0xF0 == 0xB0 && matches!(msg[1], 120 | 123) => self.held.clear(),
            None => {}
        }
    }
}

/// Which note each voice plays. In the chord shapes every voice is on its
/// chord tone; in FREE each note gets a voice of its own, which keeps its
/// note as it fades out once the note is gone.
struct Voices {
    note: [f32; VOICES],
    wanted: [bool; VOICES],
    /// How far each voice has faded in (0…1).
    env: [f32; VOICES],
    gain: [f32; VOICES],
    /// Start at the note rather than glide there (the voice was silent).
    jump: [bool; VOICES],
}

impl Voices {
    fn new(chord: [f32; VOICES]) -> Self {
        Self { note: chord, wanted: [true; VOICES], env: [1.0; VOICES], gain: VOICE_GAIN, jump: [false; VOICES] }
    }

    /// Every voice on its tone of `chord`.
    fn chord(&mut self, chord: &[f32; VOICES]) {
        self.note = *chord;
        self.wanted = [true; VOICES];
        self.gain = VOICE_GAIN;
    }

    /// FREE: a voice for each of `notes`. A voice already on one keeps it;
    /// the rest let go, and the quietest of them take the new notes.
    fn free(&mut self, notes: &[f32]) {
        let mut served = [false; VOICES];
        let mut keep = [false; VOICES];
        for (v, kept) in keep.iter_mut().enumerate() {
            if let Some(j) = (0..notes.len()).find(|&j| self.wanted[v] && !served[j] && notes[j] == self.note[v]) {
                served[j] = true;
                *kept = true;
            }
        }
        self.wanted = keep;
        for (&note, _) in notes.iter().zip(served).filter(|(_, done)| !done) {
            let free = (0..VOICES).filter(|&v| !self.wanted[v]);
            let Some(v) = free.min_by(|&a, &b| self.env[a].total_cmp(&self.env[b])) else { break };
            self.jump[v] = self.env[v] < 0.05;
            self.note[v] = note;
            self.wanted[v] = true;
        }
        // A note in unison shares its level among its voices.
        for v in (0..VOICES).filter(|&v| self.wanted[v]) {
            let share = (0..VOICES).filter(|&u| self.wanted[u] && self.note[u] == self.note[v]).count();
            self.gain[v] = FREE_GAIN / (share as f32).sqrt();
        }
    }

    /// Wanted voices fade in, the others out, over a block of `secs`.
    fn fade(&mut self, secs: f32) {
        let (up, down) = (1.0 - (-secs / FADE_IN).exp(), 1.0 - (-secs / FADE_OUT).exp());
        for (env, &wanted) in self.env.iter_mut().zip(&self.wanted) {
            if wanted {
                *env += (1.0 - *env) * up;
            } else {
                *env -= *env * down;
                if *env < 1e-3 {
                    *env = 0.0;
                }
            }
        }
    }

    /// The pitches of the voices that sound, lowest and highest two.
    fn span(&self, pitch: &[f32; VOICES]) -> (f32, [f32; 2]) {
        let mut low = f32::MAX;
        let mut top = [f32::MIN; 2];
        for v in (0..VOICES).filter(|&v| self.wanted[v]) {
            low = low.min(pitch[v]);
            if pitch[v] > top[0] {
                top = [pitch[v], top[0]];
            } else if pitch[v] > top[1] {
                top[1] = pitch[v];
            }
        }
        if low == f32::MAX {
            return (pitch[0], [pitch[VOICES - 1], pitch[VOICES - 2]]);
        }
        if top[1] == f32::MIN {
            top[1] = top[0];
        }
        (low, top)
    }
}

pub struct Aether {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    sr: f32,
    rng: Rng,
    keys: Keys,
    unlatch_seen: f32,

    // Motion.
    pitch_lanes: [Lane; VOICES],
    level_lanes: [Lane; VOICES],
    pan_lanes: [Lane; VOICES],
    detune_lanes: [Lane; VOICES],
    partial_lanes: [[Lane; PARTIALS]; VOICES],
    color_lane: Lane,
    bright_lane: Lane,
    chorus_lane: Lane,
    air_lane: Lane,
    /// Each partial's ripple, worked out every other block (half the voices
    /// at a time: their levels glide anyway).
    ripple: [[f32; PARTIALS]; VOICES],
    blocks: usize,
    /// Position when free, in EVOLVE periods.
    free_pos: f64,
    /// Beats while the transport stands still (keys keep the grid going).
    clock: f64,

    // Voices.
    voices: Voices,
    /// Each voice's pitch (MIDI note, drift included), gliding.
    pitch: [f32; VOICES],
    started: bool,
    banks: [Bank; BANKS],
    /// Swell in and out.
    run: f32,
    level: f32,
    /// The tone's two-pole low-pass.
    lp: [[f32; 2]; 2],

    // Glitches and the warm stages.
    steps: StepClock,
    glitch: Glitch,
    warmth: Warmth,
    chorus: Chorus,
    space: Space,
    breath_hz: [f32; 2],
    quiet: Quiet,
}

impl Aether {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        let lanes = |base: u32| std::array::from_fn(|v| Lane::new(base + v as u32));
        let pitch = knob_chord(&params);
        let mut rng = Rng::new(0xAE7_4E21);
        Self {
            midi_cursor: midi.stream_head(),
            midi,
            sr: sample_rate,
            banks: std::array::from_fn(|_| Bank::new(&mut rng)),
            rng,
            keys: Keys { held: Held::new(), latched: [0; Held::CAP], latched_len: 0 },
            unlatch_seen: params.viz[viz::UNLATCH].get(),
            pitch_lanes: lanes(LANE_PITCH),
            level_lanes: lanes(LANE_LEVEL),
            pan_lanes: lanes(LANE_PAN),
            detune_lanes: lanes(LANE_DETUNE),
            partial_lanes: std::array::from_fn(|v| std::array::from_fn(|k| Lane::new(LANE_PARTIAL + (v * 16 + k) as u32))),
            color_lane: Lane::new(LANE_COLOR),
            bright_lane: Lane::new(LANE_BRIGHT),
            chorus_lane: Lane::new(LANE_CHORUS),
            air_lane: Lane::new(LANE_AIR),
            ripple: [[0.0; PARTIALS]; VOICES],
            blocks: 0,
            free_pos: 0.0,
            clock: 0.0,
            voices: Voices::new(pitch),
            pitch,
            started: false,
            run: 0.0,
            level: params.get(LEVEL),
            lp: [[0.0; 2]; 2],
            steps: StepClock::new(),
            glitch: Glitch::new(sample_rate),
            warmth: Warmth::new(sample_rate),
            chorus: Chorus::new(sample_rate),
            space: Space::new(sample_rate),
            breath_hz: [1000.0, 1500.0],
            quiet: Quiet::new((sample_rate * 2.0) as usize),
            params,
        }
    }

    /// Gives the voices their notes; where they come from (see
    /// [`viz::SOURCE`]).
    fn assign(&mut self) -> usize {
        let p = &self.params;
        let (source, keys) = self.keys.now(gated(p));
        if free(p) {
            let (notes, n) = free_notes(keys, p.get(ROOT));
            self.voices.free(&notes[..n]);
        } else if keys.is_empty() {
            self.voices.chord(&knob_chord(p));
        } else {
            self.voices.chord(&voicing(keys, &CHORDS[chord(p)].2));
        }
        source
    }

    /// Plays a glitch that has come due.
    fn fire(&mut self, what: &glitch::Pending, spark: f32, decay: f32, width: f32) {
        let p = &self.params;
        // On a voice that sounds (in FREE some may be fading out).
        let v = (what.voice as usize).min(VOICES - 1);
        let v = (0..VOICES).map(|k| (v + k) % VOICES).find(|&u| self.voices.wanted[u]).unwrap_or(v);
        let h = (what.harmonic as usize).min(HARMONICS.len() - 1);
        let level = what.level * (0.3 + 0.7 * spark);
        let (hz, kind) = if what.skip {
            let root = note_hz(self.voices.span(&self.pitch).0);
            (self.glitch.skip(&mut self.rng, root, h, what.warp, level, decay), glitch::SKIP)
        } else {
            let hz = note_hz(self.pitch[v]) * HARMONICS[h] as f32;
            (self.glitch.ping(&mut self.rng, hz, level * 0.32, decay, width), glitch::PING)
        };
        let count = p.viz[viz::FIRED].get() + 1.0;
        p.viz[viz::FIRED].set(count);
        let at = viz::EVENT + 3 * (count as usize % viz::EVENTS);
        p.viz[at].set(count);
        p.viz[at + 1].set(hz);
        p.viz[at + 2].set(kind as f32);
    }
}

impl Processor for Aether {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let sr = self.sr;
        let block_secs = n as f32 / sr;

        // MIDI: the cable if patched, else the keyboards.
        let keys = &mut self.keys;
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                keys.handle(e.data);
            }
            self.midi_cursor = self.midi.stream_head();
        } else {
            self.midi.read_stream(&mut self.midi_cursor, |msg| keys.handle(msg));
        }
        let unlatch = p.viz[viz::UNLATCH].get();
        if unlatch != self.unlatch_seen {
            self.unlatch_seen = unlatch;
            self.keys.latched_len = 0;
        }

        let source = self.assign();
        self.voices.fade(block_secs);
        let held = !self.keys.held.is_empty();
        let sounding = if gated(&p) { held } else { ctx.playing || held };
        let run_t = if sounding { 1.0 } else { 0.0 };

        if ctx.restart {
            self.steps = StepClock::new();
            self.glitch.clear();
            self.free_pos = 0.0;
        }
        let bps = ctx.beats_per_sample.max(1e-9);
        let beat0 = if ctx.playing { ctx.beat } else { self.clock };
        self.clock = beat0 + bps * n as f64;

        if !sounding && self.run < 1e-4 && self.quiet.resting() {
            // Faded out and the space has gone quiet: nothing to compute.
            p.viz[viz::RUN].set(0.0);
            p.viz[viz::STEP].set(-1.0);
            p.viz[viz::SOURCE].set(source as f32);
            silence(outputs, n);
            return;
        }

        // The motion's position: on the bars when synced, else free.
        let synced = p.toggled();
        let evolve = p.get(EVOLVE);
        let seconds_per_beat = 1.0 / (bps * sr as f64);
        let period = if synced { evolve_bars(evolve) as f64 * 4.0 * seconds_per_beat } else { evolve_seconds(evolve) as f64 };
        let pos = if synced {
            beat0 / (evolve_bars(evolve) as f64 * 4.0)
        } else {
            self.free_pos += block_secs as f64 / period;
            self.free_pos
        };
        let seed = seed(&p);
        let gain = fractal::gain(p.get(ROUGH));
        let slow = weights(period as f32);

        let morph = p.get(MORPH);
        let shimmer = p.get(SHIMMER);
        let width = p.get(WIDTH);
        let color = (p.get(COLOR) + morph * 0.55 * self.color_lane.value(seed, pos, gain, &slow)).clamp(0.0, 1.0);
        let bright = (p.get(TONE) + morph * 0.28 * self.bright_lane.value(seed, pos * 1.5, gain, &slow)).clamp(0.0, 1.0);
        let shape = spectrum(color);
        let tilt_exp = -(1.0 - bright) * 0.7;
        let tilt: [f32; PARTIALS] = std::array::from_fn(|k| ((k + 1) as f32).powf(tilt_exp));
        let detune = p.get(DETUNE);
        let drift_cents = 3.0 + 9.0 * morph;

        // Voices: glide (or step) to the chord, drift, breathe, sway.
        let glides = glides(&p);
        let glide = 1.0 - (-block_secs / 0.18).exp();
        let smooth = 1.0 - (-block_secs / 0.04).exp();
        let partial_weights: [[f32; OCTAVES]; PARTIALS] = std::array::from_fn(|k| weights(period as f32 / partial_speed(k) as f32));
        for (v, home) in VOICE_PAN.into_iter().enumerate() {
            let drift = self.pitch_lanes[v].value(seed, pos * 0.8, gain, &slow) * drift_cents / 100.0;
            let target = self.voices.note[v] + drift;
            if self.started && glides && !self.voices.jump[v] {
                self.pitch[v] += (target - self.pitch[v]) * glide;
            } else {
                self.pitch[v] = target;
                self.voices.jump[v] = false;
            }
            let hz = note_hz(self.pitch[v]);
            let breathe = self.level_lanes[v].value(seed, pos * 1.2, gain, &slow);
            let level = self.voices.gain[v] * self.voices.env[v] * (1.0 + 0.4 * breathe).max(0.0);
            let sway = self.pan_lanes[v].value(seed, pos * 0.9, gain, &slow);
            let center = (home * 0.7 + sway * 0.35) * width;
            let spread = detune * (0.55 + 0.45 * self.detune_lanes[v].value(seed, pos * 1.1, gain, &slow));

            // The partials: the timbre, tilted, each rippling on its own.
            let mut amp = [0.0f32; PARTIALS];
            let mut ratio = [0.0f32; PARTIALS];
            if !self.started || (self.blocks + v).is_multiple_of(2) {
                for (k, (m, lane)) in self.ripple[v].iter_mut().zip(&mut self.partial_lanes[v]).enumerate() {
                    *m = lane.value(seed, pos * partial_speed(k), gain, &partial_weights[k]);
                }
            }
            for k in 0..PARTIALS {
                let m = self.ripple[v][k];
                amp[k] = shape[k] * tilt[k] * (1.0 + shimmer * 0.95 * m).max(0.0);
                ratio[k] = (k + 1) as f32 * (1.0 + shimmer * 0.0012 * m * ((k + 1) as f32).sqrt());
                p.viz[viz::AMP + v * PARTIALS + k].set(amp[k] * level);
            }
            // Same loudness whatever the timbre.
            let energy = amp.iter().map(|a| a * a).sum::<f32>().sqrt().max(1e-6);
            for layer in 0..2 {
                let side = if layer == 0 { -1.0 } else { 1.0 };
                let f = hz * (side * spread * 0.5 / 1200.0).exp2();
                let (pl, pr) = pan_gains(center + side * 0.3 * width);
                let bank = &mut self.banks[v * 2 + layer];
                let inc: [f32; PARTIALS] = std::array::from_fn(|k| f * ratio[k] / sr);
                for k in 0..PARTIALS {
                    // Partials near Nyquist fade, above it they are silent.
                    let alias = ((0.42 - inc[k]) / 0.1).clamp(0.0, 1.0);
                    let a = amp[k] / energy * level * NORM * alias;
                    let (tl, tr) = (a * pl, a * pr);
                    if self.started {
                        bank.tl[k] += (tl - bank.tl[k]) * smooth;
                        bank.tr[k] += (tr - bank.tr[k]) * smooth;
                    } else {
                        bank.tl[k] = tl;
                        bank.tr[k] = tr;
                    }
                }
                bank.prepare(&inc, n);
            }
            p.viz[viz::NOTE + v].set(self.pitch[v]);
            p.viz[viz::LIVE + v].set(self.voices.env[v]);
        }
        self.started = true;
        self.blocks = self.blocks.wrapping_add(1);

        // Additive voices for the whole block, bank by bank (a bank's state
        // stays in registers), lane by lane until the end.
        let mut lanes_l = [[0.0f32; PARTIALS]; BLOCK];
        let mut lanes_r = [[0.0f32; PARTIALS]; BLOCK];
        for bank in self.banks.iter_mut() {
            if bank.live {
                for (l, r) in lanes_l[..n].iter_mut().zip(&mut lanes_r[..n]) {
                    bank.tick(l, r);
                }
            }
            bank.settle();
        }
        let bl: [f32; BLOCK] = std::array::from_fn(|i| lanes_l[i].iter().sum());
        let br: [f32; BLOCK] = std::array::from_fn(|i| lanes_r[i].iter().sum());

        // Glitches on the grid, while sounding.
        let gctx = ProcessCtx { beat: beat0, playing: true, ..*ctx };
        let per_beat = steps_per_beat(division(&p) as f32);
        let mut found = [Step { frame: 0, beat: 0.0, index: 0 }; MAX_STEPS];
        let count = if sounding { self.steps.steps(&gctx, n, per_beat, 0.0, &mut found) } else { 0 };
        let settings = Settings {
            chance: p.get(GLITCH),
            allowed: glitch::allowed(p.get(HARM)),
            repeat: p.get(REPEAT),
            skip: p.get(SKIP),
            voices: VOICES,
        };
        for step in &found[..count] {
            self.glitch.step(&mut self.rng, step, 1.0 / per_beat, &settings);
            p.viz[viz::STEP].set(step.index as f32);
        }
        if !sounding {
            p.viz[viz::STEP].set(-1.0);
        }
        let mut fires = [Fire::default(); QUEUE];
        let due = if sounding { self.glitch.due(&gctx, n, &mut fires) } else { 0 };
        let (spark, decay) = (p.get(SPARK), p.get(DECAY));

        // The warm stages' settings for this block.
        let fc = 220.0 * (bright * 6.5).exp2();
        let lp_a = 1.0 - (-std::f32::consts::TAU * fc / sr).exp();
        // Oversampled only with partials high enough to fold back.
        let (_, highest) = self.voices.span(&self.pitch);
        let top = note_hz(highest[0]) * PARTIALS as f32;
        self.warmth.set_drive(p.get(DRIVE), fc > 5000.0 && top > 7000.0);
        let chorus_mix = p.get(CHORUS);
        let chorus_rate = 0.16 * (1.0 + 0.5 * self.chorus_lane.value(seed, pos * 2.0, gain, &slow)) / sr;
        let air = p.get(AIR);
        // The breath sits on upper harmonics of the top voices, wandering
        // between them with the motion.
        let wander = self.air_lane.value(seed, pos * 0.7, gain, &slow);
        const BREATH: [f32; 4] = [4.0, 5.0, 6.0, 8.0];
        let pick = |x: f32| BREATH[(((x * 0.5 + 0.5) * 3.99) as usize).min(3)];
        let breath_t = [note_hz(highest[0]) * pick(wander), note_hz(highest[1]) * pick(-wander)];
        let follow = if glides { glide } else { smooth };
        for (b, t) in self.breath_hz.iter_mut().zip(breath_t) {
            *b += (t.min(sr * 0.2) - *b) * follow;
        }
        let space = self.space.settings(p.get(SPACE), bright, air, self.breath_hz, self.run);
        self.chorus.begin(chorus_rate, chorus_mix, chorus_mix, n);
        let up = time_coeff(0.35, sr);
        let down = time_coeff(0.5, sr);
        let level_t = p.get(LEVEL);

        let mut peak = 0.0f32;
        let mut tail = 0.0f32;
        let mut next = 0;
        for i in 0..n {
            while next < due && fires[next].frame <= i {
                let what = fires[next].what;
                self.fire(&what, spark, decay, width);
                next += 1;
            }
            let k = if run_t > self.run { up } else { down };
            approach(&mut self.run, run_t, k);
            approach(&mut self.level, level_t, ctx.smooth);
            let mut x = [bl[i], br[i]];
            for (ch, xc) in x.iter_mut().enumerate() {
                let [a, b] = &mut self.lp[ch];
                *a += lp_a * (*xc - *a);
                *b += lp_a * (*a - *b);
                *xc = *b;
            }
            let sp = self.glitch.sparks();
            let x = [(x[0] + sp[0]) * self.run, (x[1] + sp[1]) * self.run];
            let x = self.warmth.process(x);
            let x = self.glitch.skipping(x);
            let x = self.chorus.process(x);
            let (y, t) = self.space.process(x, &space);
            tail = tail.max(t);
            let (yl, yr) = (soft_limit(y[0] * self.level), soft_limit(y[1] * self.level));
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak = peak.max(yl.abs()).max(yr.abs());
        }
        let idle = !sounding && self.run < 1e-4 && !self.glitch.active();
        self.quiet.update(idle, tail.max(peak), n);
        p.meter.fetch_max(peak);

        p.viz[viz::POS].set(pos as f32);
        p.viz[viz::COLOR].set(color);
        p.viz[viz::BRIGHT].set(bright);
        p.viz[viz::RUN].set(self.run);
        p.viz[viz::SOURCE].set(source as f32);
    }
}

