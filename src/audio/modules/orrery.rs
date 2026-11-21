//! Orrery: a Euclidean rhythm generator drawn as concentric rings, like the
//! orbits of an orrery. Each of the eight rings is a rhythm of its own: a
//! number of STEPS round it, PULSES spread over them as evenly as they go
//! (Bjorklund's Euclidean rhythms: 3 over 8 is the tresillo, 5 over 8 the
//! cinquillo), and a note on a MIDI channel of its own. Every step can be
//! switched on, off or accented by hand afterwards.
//!
//! Rings turn: ROTATE moves a ring's rhythm round by steps (or by any part
//! of a step), so the rings fall out of phase with each other, and TWIST
//! turns each ring a little further than the one outside it.
//!
//! Two ways to run the rings:
//! - **GRID**: every ring steps on the board's grid (RATE): a ring of 5
//!   steps comes round sooner than one of 16, so they drift (polymeter);
//! - **CYCLE**: every ring goes round once in CYCLE beats, its steps
//!   dividing the cycle evenly: 3 against 4 against 5 (polyrhythm).
//!
//! Patterns: eight buttons each hold a pattern (everything but the rings'
//! notes and channels) to recall in one click, or by Program Change 1–8 on
//! the MIDI input. A recalled pattern comes in now, on the next beat or on
//! the next bar (LAUNCH), switched here, on the audio thread, so it lands on
//! time. The panel keeps a library of named patterns besides.

use std::sync::Arc;

use super::automa::hash01;
use super::steps::{
    MAX_STEPS as MAX_BLOCK_STEPS, Step, StepClock, block_end, roll, steps_per_beat,
};
use super::{ModuleDef, ModuleParams, Rng};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{DIVISIONS, ParamSpec, Unit};

/// Rings, from the outside in.
pub const RINGS: usize = 8;
/// Most steps a ring can have.
pub const MAX_STEPS: usize = 32;

pub const RATE: usize = 0;
pub const CYCLE: usize = 1;
pub const SWING: usize = 2;
pub const ACCENT: usize = 3;
pub const TWIST: usize = 4;

const PARAMS: [ParamSpec; 5] = [
    ParamSpec::lin("RATE", 0.0, 5.0, 3.0, Unit::Division),
    ParamSpec::lin("CYCLE", 1.0, 32.0, 4.0, Unit::Integer),
    ParamSpec::lin("SWING", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("ACCENT", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("TWIST", -2.0, 2.0, 0.0, Unit::Plain).bipolar(),
];

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// The modes: steps on the board's grid, or each ring once round a cycle.
pub const GRID: usize = 0;
pub const CYCLE_MODE: usize = 1;

/// A step: off, on, or on and accented.
pub const OFF: u8 = 0;
pub const ON: u8 = 1;
pub const ACCENTED: u8 = 2;

/// Pattern buttons.
pub const SLOTS: usize = 8;
/// The slot after the buttons': where a pattern from the library waits to
/// come in.
pub const CUE: usize = SLOTS;

/// When a recalled pattern comes in.
pub const NOW: usize = 0;
pub const BEAT: usize = 1;
pub const BAR: usize = 2;

/// What `ModuleParams::data` holds.
pub mod data {
    /// The ring selected on the panel.
    pub const SELECTED: usize = 0;
    /// The rings, [`super::ring::LEN`] values each.
    pub const RINGS: usize = 1;
    /// When a recalled pattern comes in: [`super::NOW`], [`super::BEAT`] or
    /// [`super::BAR`].
    pub const LAUNCH: usize = RINGS + super::RINGS * super::ring::LEN;
    /// The slot waiting to come in, plus one (0: none).
    pub const PENDING: usize = LAUNCH + 1;
    /// The button recalled last, plus one (0: none).
    pub const ACTIVE: usize = PENDING + 1;
    /// The buttons' patterns, then the cue: [`super::slot::LEN`] values each.
    pub const SLOTS: usize = ACTIVE + 1;
    pub const LEN: usize = SLOTS + (super::SLOTS + 1) * super::slot::LEN;
}

/// A stored pattern's values: everything but the rings' notes and channels.
pub mod slot {
    /// 1 if the slot holds a pattern.
    pub const USED: usize = 0;
    pub const MODE: usize = 1;
    pub const RATE: usize = 2;
    pub const CYCLE: usize = 3;
    pub const SWING: usize = 4;
    pub const ACCENT: usize = 5;
    pub const TWIST: usize = 6;
    /// Its name, three characters a value.
    pub const NAME: usize = 7;
    pub const NAME_VALUES: usize = 4;
    pub const NAME_CHARS: usize = NAME_VALUES * 3;
    /// The rings, [`RING_LEN`] values each.
    pub const RINGS: usize = NAME + NAME_VALUES;
    pub const ON: usize = 0;
    pub const STEPS: usize = 1;
    pub const PULSES: usize = 2;
    pub const ROTATE: usize = 3;
    pub const VELOCITY: usize = 4;
    pub const GATE: usize = 5;
    pub const CHANCE: usize = 6;
    /// The hits and the accents as bits, sixteen steps a value.
    pub const HITS: usize = 7;
    pub const ACCENTS: usize = 9;
    pub const RING_LEN: usize = 11;
    pub const LEN: usize = RINGS + super::RINGS * RING_LEN;
}

/// A stored pattern.
pub type Pattern = [f32; slot::LEN];

/// A ring's values in `data`.
pub mod ring {
    /// 1 while the ring plays (0: muted).
    pub const ON: usize = 0;
    pub const STEPS: usize = 1;
    pub const PULSES: usize = 2;
    /// How far the ring is turned, in steps (any part of a step).
    pub const ROTATE: usize = 3;
    pub const NOTE: usize = 4;
    /// MIDI channel, 1–16.
    pub const CHANNEL: usize = 5;
    pub const VELOCITY: usize = 6;
    /// Note length, as a part of the step.
    pub const GATE: usize = 7;
    /// The chance a hit plays.
    pub const CHANCE: usize = 8;
    /// Each step: [`super::OFF`], [`super::ON`] or [`super::ACCENTED`].
    pub const PATTERN: usize = 9;
    pub const LEN: usize = PATTERN + super::MAX_STEPS;
}

/// What the processor publishes for the panel.
pub mod viz {
    use super::RINGS;

    /// 1 while the rings play.
    pub const LIVE: usize = 0;
    /// Per ring: where its planet is, in steps from the top of the ring
    /// (0…steps): a step's hit plays as the planet passes it.
    pub const PHASE: usize = 1;
    /// Per ring: the pattern step reached last, or −1.
    pub const STEP: usize = PHASE + RINGS;
    /// Per ring: notes played so far (the panel flashes on a change).
    pub const HITS: usize = STEP + RINGS;
    /// Per ring: the step the last note played on.
    pub const HIT_STEP: usize = HITS + RINGS;
    /// Blocks processed (wrapping): the panel sees the processor is running.
    pub const TICK: usize = HIT_STEP + RINGS;
    pub const LEN: usize = TICK + 1;
}

/// The Euclidean rhythm of `pulses` over `steps`: whether step `i` plays.
/// The pulses fall as evenly as the steps allow, the first on step 0.
pub const fn euclid(pulses: usize, steps: usize, i: usize) -> bool {
    let n = if steps == 0 { 1 } else { steps };
    let k = if pulses > n { n } else { pulses };
    k > 0 && (i % n) * k % n < k
}

/// The rings' notes to start with: a General MIDI kit on channel 10
/// (bass drum, snare, closed and open hi-hat, side stick, low tom, hand
/// clap, cowbell). Patterns leave them as they are.
const NOTES: [u8; RINGS] = [36, 38, 42, 46, 37, 45, 39, 56];

/// A ring of a built-in pattern: on, steps, pulses, turn, velocity, gate,
/// chance, and accents on the hits of every so many steps (4: the beats of
/// sixteenths; 0: none).
#[derive(Clone, Copy)]
struct R(bool, usize, usize, usize, u8, f32, f32, usize);

/// A built-in pattern: its name, mode, RATE, CYCLE, SWING, TWIST, rings.
struct Factory(&'static str, usize, f32, f32, f32, f32, [R; RINGS]);

const OFF_RING: R = R(false, 16, 0, 0, 90, 0.4, 1.0, 0);

const FACTORY: [Factory; SLOTS] = [
    // The groove it starts with; the inner rings muted, ready to bring in.
    Factory(
        "Groove",
        GRID,
        3.0,
        4.0,
        0.0,
        0.0,
        [
            R(true, 16, 5, 0, 112, 0.5, 1.0, 0),
            R(true, 16, 2, 4, 104, 0.5, 1.0, 0),
            R(true, 16, 11, 0, 76, 0.25, 1.0, 4),
            R(true, 16, 2, 2, 84, 0.4, 1.0, 0),
            R(false, 12, 5, 0, 84, 0.3, 1.0, 0),
            R(false, 7, 3, 0, 92, 0.5, 1.0, 0),
            R(false, 9, 4, 1, 92, 0.4, 1.0, 0),
            R(false, 5, 2, 0, 80, 0.3, 1.0, 0),
        ],
    ),
    // Kick on every beat, hats on the offbeats, clap on 2 and 4.
    Factory(
        "Four floor",
        GRID,
        3.0,
        4.0,
        0.0,
        0.0,
        [
            R(true, 16, 4, 0, 116, 0.5, 1.0, 0),
            OFF_RING,
            R(true, 16, 4, 2, 84, 0.25, 1.0, 0),
            R(true, 16, 16, 0, 52, 0.15, 0.6, 0),
            OFF_RING,
            OFF_RING,
            R(true, 16, 2, 4, 100, 0.4, 1.0, 0),
            OFF_RING,
        ],
    ),
    // 3 over 8 in the kick, the cinquillo on the side stick.
    Factory(
        "Tresillo",
        GRID,
        3.0,
        4.0,
        0.0,
        0.0,
        [
            R(true, 8, 3, 0, 112, 0.5, 1.0, 0),
            R(true, 16, 2, 4, 100, 0.5, 1.0, 0),
            R(true, 16, 8, 0, 72, 0.25, 1.0, 4),
            OFF_RING,
            R(true, 8, 5, 1, 84, 0.3, 1.0, 0),
            OFF_RING,
            OFF_RING,
            R(true, 16, 3, 2, 70, 0.3, 1.0, 0),
        ],
    ),
    // The bossa clave on the side stick over sixteenths.
    Factory(
        "Bossa",
        GRID,
        3.0,
        4.0,
        0.08,
        0.0,
        [
            R(true, 16, 4, 0, 100, 0.6, 1.0, 0),
            OFF_RING,
            R(true, 16, 16, 0, 64, 0.2, 1.0, 4),
            OFF_RING,
            R(true, 16, 5, 3, 92, 0.3, 1.0, 0),
            R(true, 16, 2, 14, 80, 0.5, 0.5, 0),
            OFF_RING,
            OFF_RING,
        ],
    ),
    // Triplet eighths: the hi-hat's 8 over 12 is the shuffle.
    Factory(
        "Shuffle",
        GRID,
        2.0,
        4.0,
        0.0,
        0.0,
        [
            R(true, 12, 4, 0, 110, 0.5, 1.0, 0),
            R(true, 12, 2, 3, 100, 0.5, 1.0, 0),
            R(true, 12, 8, 0, 74, 0.3, 1.0, 3),
            OFF_RING,
            OFF_RING,
            R(true, 24, 2, 21, 86, 0.4, 0.7, 0),
            OFF_RING,
            OFF_RING,
        ],
    ),
    // A broken beat with ghost notes and a touch of swing.
    Factory(
        "Breaks",
        GRID,
        3.0,
        4.0,
        0.15,
        0.0,
        [
            R(true, 16, 3, 0, 116, 0.5, 1.0, 0),
            R(true, 16, 2, 4, 108, 0.5, 1.0, 0),
            R(true, 16, 16, 0, 70, 0.2, 0.85, 2),
            R(true, 16, 1, 14, 80, 0.4, 1.0, 0),
            R(true, 16, 3, 5, 60, 0.2, 0.6, 0),
            OFF_RING,
            OFF_RING,
            OFF_RING,
        ],
    ),
    // CYCLE: 4 against 3 against 5, 6 and 7 in one bar.
    Factory(
        "Poly 3:4:5",
        CYCLE_MODE,
        3.0,
        4.0,
        0.0,
        0.0,
        [
            R(true, 4, 4, 0, 112, 0.5, 1.0, 0),
            OFF_RING,
            R(true, 5, 5, 0, 72, 0.3, 1.0, 0),
            OFF_RING,
            R(true, 3, 3, 0, 90, 0.3, 1.0, 0),
            R(true, 7, 3, 0, 88, 0.5, 1.0, 0),
            OFF_RING,
            R(true, 6, 2, 1, 76, 0.3, 1.0, 0),
        ],
    ),
    // Rings of different lengths drifting apart, twisted.
    Factory(
        "Drift",
        GRID,
        3.0,
        4.0,
        0.0,
        0.25,
        [
            R(true, 16, 4, 0, 108, 0.5, 1.0, 0),
            R(true, 12, 3, 2, 96, 0.4, 1.0, 0),
            R(true, 9, 5, 0, 70, 0.25, 0.9, 0),
            R(true, 7, 2, 0, 80, 0.4, 1.0, 0),
            R(true, 10, 3, 0, 80, 0.3, 1.0, 0),
            R(true, 11, 2, 0, 90, 0.5, 1.0, 0),
            R(true, 13, 2, 4, 90, 0.4, 0.8, 0),
            R(true, 5, 1, 0, 70, 0.3, 1.0, 0),
        ],
    ),
];

/// A ring's hits (bit i: step i plays) and accents, of a built-in ring.
const fn factory_bits(ring: R) -> (u32, u32) {
    let R(_, steps, pulses, _, _, _, _, every) = ring;
    let (mut hits, mut accents, mut i) = (0u32, 0u32, 0);
    while i < steps && i < MAX_STEPS {
        if euclid(pulses, steps, i) {
            hits |= 1 << i;
            if every > 0 && i % every == 0 {
                accents |= 1 << i;
            }
        }
        i += 1;
    }
    (hits, accents)
}

/// `name`, three characters a value (printable ASCII; others read as ?).
const fn pack_name(name: &str) -> [f32; slot::NAME_VALUES] {
    let bytes = name.as_bytes();
    let mut out = [0.0; slot::NAME_VALUES];
    let mut v = 0;
    while v < slot::NAME_VALUES {
        let mut word = 0u32;
        let mut c = 0;
        while c < 3 {
            let i = v * 3 + c;
            let b = if i < bytes.len() {
                if bytes[i] >= 0x20 && bytes[i] < 0x7F {
                    bytes[i]
                } else {
                    b'?'
                }
            } else {
                0
            };
            word |= (b as u32) << (8 * c);
            c += 1;
        }
        out[v] = word as f32;
        v += 1;
    }
    out
}

/// A built-in pattern's values.
const fn factory_pattern(k: usize) -> Pattern {
    let Factory(name, mode, rate, cycle, swing, twist, rings) = &FACTORY[k];
    let mut s = [0.0; slot::LEN];
    s[slot::USED] = 1.0;
    s[slot::MODE] = *mode as f32;
    s[slot::RATE] = *rate;
    s[slot::CYCLE] = *cycle;
    s[slot::SWING] = *swing;
    s[slot::ACCENT] = 0.5;
    s[slot::TWIST] = *twist;
    let packed = pack_name(name);
    let mut v = 0;
    while v < slot::NAME_VALUES {
        s[slot::NAME + v] = packed[v];
        v += 1;
    }
    let mut r = 0;
    while r < RINGS {
        let ring = rings[r];
        let R(on, steps, pulses, rotate, velocity, gate, chance, _) = ring;
        let at = slot::RINGS + r * slot::RING_LEN;
        s[at + slot::ON] = if on { 1.0 } else { 0.0 };
        s[at + slot::STEPS] = steps as f32;
        s[at + slot::PULSES] = pulses as f32;
        s[at + slot::ROTATE] = rotate as f32;
        s[at + slot::VELOCITY] = velocity as f32;
        s[at + slot::GATE] = gate;
        s[at + slot::CHANCE] = chance;
        let (hits, accents) = factory_bits(ring);
        s[at + slot::HITS] = (hits & 0xFFFF) as f32;
        s[at + slot::HITS + 1] = (hits >> 16) as f32;
        s[at + slot::ACCENTS] = (accents & 0xFFFF) as f32;
        s[at + slot::ACCENTS + 1] = (accents >> 16) as f32;
        r += 1;
    }
    s
}

/// The built-in patterns' names.
pub const FACTORY_NAMES: [&str; SLOTS] = {
    let mut names = [""; SLOTS];
    let mut k = 0;
    while k < SLOTS {
        names[k] = FACTORY[k].0;
        k += 1;
    }
    names
};

/// Built-in pattern `k`.
pub fn factory(k: usize) -> Pattern {
    factory_pattern(k.min(SLOTS - 1))
}

const fn defaults() -> [f32; data::LEN] {
    let mut d = [0.0; data::LEN];
    // The rings: the first built-in pattern, on the kit's notes.
    let first = factory_pattern(0);
    let mut r = 0;
    while r < RINGS {
        let at = data::RINGS + r * ring::LEN;
        let from = slot::RINGS + r * slot::RING_LEN;
        d[at + ring::ON] = first[from + slot::ON];
        d[at + ring::STEPS] = first[from + slot::STEPS];
        d[at + ring::PULSES] = first[from + slot::PULSES];
        d[at + ring::ROTATE] = first[from + slot::ROTATE];
        d[at + ring::NOTE] = NOTES[r] as f32;
        d[at + ring::CHANNEL] = 10.0;
        d[at + ring::VELOCITY] = first[from + slot::VELOCITY];
        d[at + ring::GATE] = first[from + slot::GATE];
        d[at + ring::CHANCE] = first[from + slot::CHANCE];
        let (hits, accents) = factory_bits(FACTORY[0].6[r]);
        let mut i = 0;
        while i < MAX_STEPS {
            if hits >> i & 1 == 1 {
                d[at + ring::PATTERN + i] = if accents >> i & 1 == 1 {
                    ACCENTED as f32
                } else {
                    ON as f32
                };
            }
            i += 1;
        }
        r += 1;
    }
    // The buttons: the built-in patterns, the first one recalled.
    d[data::LAUNCH] = BAR as f32;
    d[data::ACTIVE] = 1.0;
    let mut k = 0;
    while k < SLOTS {
        let pattern = factory_pattern(k);
        let mut i = 0;
        while i < slot::LEN {
            d[data::SLOTS + k * slot::LEN + i] = pattern[i];
            i += 1;
        }
        k += 1;
    }
    d
}

const DATA: [f32; data::LEN] = defaults();

pub static DEF: ModuleDef = ModuleDef {
    name: "Orrery",
    prefix: "ORRERY",
    hint: "Euclidean rings → MIDI",
    inputs: &[],
    outputs: &[],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["GRID", "CYCLE"],
    toggle: Some("PLAY"),
    toggle_default: true,
    map: false,
    midi_in: true,
    midi_out: true,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

// ---------------------------------------------------------------------------
// The rings' settings
// ---------------------------------------------------------------------------

/// Where ring `r`'s value `i` is in `data`.
pub fn at(r: usize, i: usize) -> usize {
    data::RINGS + r.min(RINGS - 1) * ring::LEN + i
}

pub fn get(p: &ModuleParams, r: usize, i: usize) -> f32 {
    p.data[at(r, i)].get()
}

pub fn set(p: &ModuleParams, r: usize, i: usize, value: f32) {
    p.data[at(r, i)].set(value);
}

/// A ring's value as a whole number within `lo..=hi`.
pub fn int(p: &ModuleParams, r: usize, i: usize, lo: i32, hi: i32) -> i32 {
    (get(p, r, i).round() as i32).clamp(lo, hi)
}

pub fn selected(p: &ModuleParams) -> usize {
    (p.data[data::SELECTED].get().round().max(0.0) as usize).min(RINGS - 1)
}

pub fn select(p: &ModuleParams, r: usize) {
    p.data[data::SELECTED].set(r.min(RINGS - 1) as f32);
}

pub fn is_on(p: &ModuleParams, r: usize) -> bool {
    get(p, r, ring::ON) > 0.5
}

pub fn steps(p: &ModuleParams, r: usize) -> usize {
    int(p, r, ring::STEPS, 1, MAX_STEPS as i32) as usize
}

pub fn pulses(p: &ModuleParams, r: usize) -> usize {
    int(p, r, ring::PULSES, 0, steps(p, r) as i32) as usize
}

pub fn note(p: &ModuleParams, r: usize) -> u8 {
    int(p, r, ring::NOTE, 0, 127) as u8
}

/// The ring's channel, 1–16.
pub fn channel(p: &ModuleParams, r: usize) -> u8 {
    int(p, r, ring::CHANNEL, 1, 16) as u8
}

pub fn velocity(p: &ModuleParams, r: usize) -> u8 {
    int(p, r, ring::VELOCITY, 1, 127) as u8
}

pub fn gate(p: &ModuleParams, r: usize) -> f32 {
    get(p, r, ring::GATE).clamp(0.05, 1.0)
}

pub fn chance(p: &ModuleParams, r: usize) -> f32 {
    get(p, r, ring::CHANCE).clamp(0.0, 1.0)
}

/// How far the ring itself is turned, in steps (0…steps).
pub fn rotation(p: &ModuleParams, r: usize) -> f32 {
    get(p, r, ring::ROTATE).rem_euclid(steps(p, r) as f32)
}

/// Turns the ring to `steps_turned` (taken round its steps).
pub fn set_rotation(p: &ModuleParams, r: usize, steps_turned: f32) {
    let n = steps(p, r) as f32;
    let mut value = steps_turned.rem_euclid(n);
    // Whole steps stay whole (no 2.9999 from the wrap).
    if (value - value.round()).abs() < 1e-4 {
        value = value.round() % n;
    }
    set(p, r, ring::ROTATE, value);
}

/// How far the ring plays turned: its own rotation and TWIST's share (each
/// ring TWIST steps further than the one outside it), in steps (0…steps).
pub fn turn(p: &ModuleParams, r: usize) -> f32 {
    (get(p, r, ring::ROTATE) + r as f32 * p.get(TWIST)).rem_euclid(steps(p, r) as f32)
}

/// Step `i` of the ring's pattern.
pub fn step(p: &ModuleParams, r: usize, i: usize) -> u8 {
    if i >= MAX_STEPS {
        return OFF;
    }
    (get(p, r, ring::PATTERN + i).round().clamp(0.0, 2.0)) as u8
}

pub fn set_step(p: &ModuleParams, r: usize, i: usize, value: u8) {
    if i < MAX_STEPS {
        set(p, r, ring::PATTERN + i, value.min(ACCENTED) as f32);
    }
}

/// Steps of the ring that play.
pub fn hits(p: &ModuleParams, r: usize) -> usize {
    (0..steps(p, r)).filter(|&i| step(p, r, i) != OFF).count()
}

/// Lays the ring's Euclidean rhythm down again (over hand edits); a step
/// still on keeps its accent.
pub fn lay(p: &ModuleParams, r: usize) {
    let (n, k) = (steps(p, r), pulses(p, r));
    for i in 0..MAX_STEPS {
        let was = step(p, r, i);
        let now = if i < n && euclid(k, n, i) {
            if was == ACCENTED { ACCENTED } else { ON }
        } else {
            OFF
        };
        set_step(p, r, i, now);
    }
}

/// New STEPS: the pulses fit within them and the rhythm is laid again.
pub fn set_steps(p: &ModuleParams, r: usize, n: usize) {
    let n = n.clamp(1, MAX_STEPS);
    let turned = rotation(p, r);
    set(p, r, ring::STEPS, n as f32);
    set(p, r, ring::PULSES, pulses(p, r).min(n) as f32);
    set_rotation(p, r, turned.min(n as f32 - 1.0));
    lay(p, r);
}

/// New PULSES: the rhythm is laid again.
pub fn set_pulses(p: &ModuleParams, r: usize, k: usize) {
    set(p, r, ring::PULSES, k.min(steps(p, r)) as f32);
    lay(p, r);
}

/// Every step the other way round: hits become rests, rests hits.
pub fn invert(p: &ModuleParams, r: usize) {
    for i in 0..steps(p, r) {
        set_step(p, r, i, if step(p, r, i) == OFF { ON } else { OFF });
    }
}

pub fn clear(p: &ModuleParams, r: usize) {
    for i in 0..MAX_STEPS {
        set_step(p, r, i, OFF);
    }
}

/// New rhythms from `seed`: the pulses and the turn of each ring that
/// plays (or of ring `only`), within what suits its length. Steps, notes and
/// channels stay.
pub fn dice(p: &ModuleParams, seed: u32, only: Option<usize>) {
    for r in 0..RINGS {
        if only.is_some_and(|o| o != r) || (only.is_none() && !is_on(p, r)) {
            continue;
        }
        let n = steps(p, r);
        let roll = |salt: u32| hash01(seed, r as u64, salt);
        // Sparse to busy, most often in the middle.
        let density = 0.12 + 0.5 * (roll(1) + roll(2)) * 0.5 + 0.25 * roll(3).powi(3);
        let k = ((n as f32 * density).round() as usize).clamp(1, n);
        set(p, r, ring::PULSES, k as f32);
        set(
            p,
            r,
            ring::ROTATE,
            (roll(4) * n as f32).floor().min(n as f32 - 1.0),
        );
        lay(p, r);
    }
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

/// Where slot `k`'s value `i` is in `data` (`k` up to [`CUE`]).
fn slot_at(k: usize, i: usize) -> usize {
    data::SLOTS + k.min(CUE) * slot::LEN + i
}

/// The rings and the clock now, as a pattern (unnamed).
pub fn capture(p: &ModuleParams) -> Pattern {
    let mut s = [0.0; slot::LEN];
    s[slot::USED] = 1.0;
    s[slot::MODE] = p.mode().min(CYCLE_MODE) as f32;
    s[slot::RATE] = p.get(RATE);
    s[slot::CYCLE] = p.get(CYCLE);
    s[slot::SWING] = p.get(SWING);
    s[slot::ACCENT] = p.get(ACCENT);
    s[slot::TWIST] = p.get(TWIST);
    for r in 0..RINGS {
        let at = slot::RINGS + r * slot::RING_LEN;
        for (to, from) in [
            (slot::ON, ring::ON),
            (slot::VELOCITY, ring::VELOCITY),
            (slot::GATE, ring::GATE),
            (slot::CHANCE, ring::CHANCE),
        ] {
            s[at + to] = get(p, r, from);
        }
        s[at + slot::STEPS] = steps(p, r) as f32;
        s[at + slot::PULSES] = pulses(p, r) as f32;
        s[at + slot::ROTATE] = rotation(p, r);
        let (mut hits, mut accents) = (0u32, 0u32);
        for i in 0..MAX_STEPS {
            match step(p, r, i) {
                OFF => {}
                ON => hits |= 1 << i,
                _ => {
                    hits |= 1 << i;
                    accents |= 1 << i;
                }
            }
        }
        s[at + slot::HITS] = (hits & 0xFFFF) as f32;
        s[at + slot::HITS + 1] = (hits >> 16) as f32;
        s[at + slot::ACCENTS] = (accents & 0xFFFF) as f32;
        s[at + slot::ACCENTS + 1] = (accents >> 16) as f32;
    }
    s
}

/// Step `i` of ring `r` in a pattern.
pub fn pattern_step(s: &Pattern, r: usize, i: usize) -> u8 {
    let at = slot::RINGS + r.min(RINGS - 1) * slot::RING_LEN;
    let bits = |k: usize| {
        (s[at + k].max(0.0) as u32 & 0xFFFF) | (s[at + k + 1].max(0.0) as u32 & 0xFFFF) << 16
    };
    if i >= MAX_STEPS || bits(slot::HITS) >> i & 1 == 0 {
        OFF
    } else if bits(slot::ACCENTS) >> i & 1 == 1 {
        ACCENTED
    } else {
        ON
    }
}

/// Ring `r`'s value `i` in a pattern.
pub fn pattern_ring(s: &Pattern, r: usize, i: usize) -> f32 {
    s[slot::RINGS + r.min(RINGS - 1) * slot::RING_LEN + i]
}

/// Sets the rings and the clock to a pattern (the notes and channels stay).
pub fn apply(p: &ModuleParams, s: &Pattern) {
    p.set_mode(if s[slot::MODE] > 0.5 {
        CYCLE_MODE
    } else {
        GRID
    });
    for (i, k) in [
        (RATE, slot::RATE),
        (CYCLE, slot::CYCLE),
        (SWING, slot::SWING),
        (ACCENT, slot::ACCENT),
        (TWIST, slot::TWIST),
    ] {
        let spec = spec(i);
        p.set(i, s[k].clamp(spec.min, spec.max));
    }
    for r in 0..RINGS {
        let n = (pattern_ring(s, r, slot::STEPS).round() as usize).clamp(1, MAX_STEPS);
        set(
            p,
            r,
            ring::ON,
            if pattern_ring(s, r, slot::ON) > 0.5 {
                1.0
            } else {
                0.0
            },
        );
        set(p, r, ring::STEPS, n as f32);
        set(
            p,
            r,
            ring::PULSES,
            pattern_ring(s, r, slot::PULSES)
                .round()
                .clamp(0.0, n as f32),
        );
        set(
            p,
            r,
            ring::ROTATE,
            pattern_ring(s, r, slot::ROTATE).rem_euclid(n as f32),
        );
        set(
            p,
            r,
            ring::VELOCITY,
            pattern_ring(s, r, slot::VELOCITY).round().clamp(1.0, 127.0),
        );
        set(
            p,
            r,
            ring::GATE,
            pattern_ring(s, r, slot::GATE).clamp(0.05, 1.0),
        );
        set(
            p,
            r,
            ring::CHANCE,
            pattern_ring(s, r, slot::CHANCE).clamp(0.0, 1.0),
        );
        for i in 0..MAX_STEPS {
            set_step(p, r, i, pattern_step(s, r, i));
        }
    }
}

/// Slot `k`'s pattern (`k` up to [`CUE`]).
pub fn slot(p: &ModuleParams, k: usize) -> Pattern {
    std::array::from_fn(|i| p.data[slot_at(k, i)].get())
}

pub fn set_slot(p: &ModuleParams, k: usize, s: &Pattern) {
    for (i, &v) in s.iter().enumerate() {
        p.data[slot_at(k, i)].set(v);
    }
}

pub fn slot_used(p: &ModuleParams, k: usize) -> bool {
    p.data[slot_at(k, slot::USED)].get() > 0.5
}

/// Stores the rings in button `k`, keeping its name; it becomes the one
/// recalled.
pub fn store(p: &ModuleParams, k: usize) {
    let name = slot_name(p, k);
    let mut s = capture(p);
    set_name(&mut s, &name);
    set_slot(p, k, &s);
    p.data[data::ACTIVE].set((k.min(SLOTS - 1) + 1) as f32);
}

pub fn clear_slot(p: &ModuleParams, k: usize) {
    set_slot(p, k, &[0.0; slot::LEN]);
    if active(p) == Some(k) {
        p.data[data::ACTIVE].set(0.0);
    }
}

/// Recalls slot `k` (a button, or [`CUE`]): it comes in as LAUNCH says.
pub fn recall(p: &ModuleParams, k: usize) {
    if k == CUE || slot_used(p, k) {
        p.data[data::PENDING].set((k.min(CUE) + 1) as f32);
    }
}

/// Puts a pattern (from the library) in the cue and recalls it.
pub fn cue(p: &ModuleParams, s: &Pattern) {
    set_slot(p, CUE, s);
    recall(p, CUE);
}

/// The slot waiting to come in.
pub fn pending(p: &ModuleParams) -> Option<usize> {
    let v = p.data[data::PENDING].get().round();
    (v >= 1.0).then(|| (v as usize - 1).min(CUE))
}

/// The button recalled last.
pub fn active(p: &ModuleParams) -> Option<usize> {
    let v = p.data[data::ACTIVE].get().round();
    (v >= 1.0).then(|| (v as usize - 1).min(SLOTS - 1))
}

/// Brings slot `k` in now.
pub fn take(p: &ModuleParams, k: usize) {
    let s = slot(p, k);
    if s[slot::USED] > 0.5 {
        apply(p, &s);
        p.data[data::ACTIVE].set(if k < SLOTS { (k + 1) as f32 } else { 0.0 });
    }
    if pending(p) == Some(k) {
        p.data[data::PENDING].set(0.0);
    }
}

/// The rings were changed since the button recalled last was.
pub fn edited(p: &ModuleParams) -> bool {
    let Some(k) = active(p) else { return false };
    let (now, kept) = (capture(p), slot(p, k));
    (0..slot::LEN)
        .filter(|i| !(slot::NAME..slot::NAME + slot::NAME_VALUES).contains(i))
        .any(|i| (now[i] - kept[i]).abs() > 1e-4)
}

pub fn launch(p: &ModuleParams) -> usize {
    (p.data[data::LAUNCH].get().round().max(0.0) as usize).min(BAR)
}

/// A pattern's name ("" if it has none).
pub fn name_of(s: &Pattern) -> String {
    let mut out = String::new();
    for v in 0..slot::NAME_VALUES {
        let word = s[slot::NAME + v].max(0.0) as u32;
        for c in 0..3 {
            match (word >> (8 * c)) as u8 {
                0 => return out,
                b => out.push(b as char),
            }
        }
    }
    out
}

/// Names a pattern (cut to [`slot::NAME_CHARS`] characters).
pub fn set_name(s: &mut Pattern, name: &str) {
    let name: String = name
        .chars()
        .take(slot::NAME_CHARS)
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c
            } else {
                '?'
            }
        })
        .collect();
    s[slot::NAME..slot::NAME + slot::NAME_VALUES].copy_from_slice(&pack_name(&name));
}

pub fn slot_name(p: &ModuleParams, k: usize) -> String {
    name_of(&slot(p, k))
}

pub fn rename_slot(p: &ModuleParams, k: usize, name: &str) {
    let mut s = slot(p, k);
    set_name(&mut s, name);
    set_slot(p, k, &s);
}

/// When a recall waiting comes in: the first beat at or after `beat` on
/// the LAUNCH grid (a beat, or a bar: in CYCLE, a turn).
pub fn launch_beat(p: &ModuleParams, beat: f64) -> f64 {
    let unit = match launch(p) {
        NOW => return beat,
        BEAT => 1.0,
        _ if p.mode() == CYCLE_MODE => cycle_beats(p),
        _ => 4.0,
    };
    (beat / unit - 1e-9).ceil() * unit
}

/// "E(5,16)": the ring's Euclidean name.
pub fn euclid_name(p: &ModuleParams, r: usize) -> String {
    format!("E({},{})", pulses(p, r), steps(p, r))
}

/// Beats per step of ring `r`: the grid's in GRID, the cycle shared out
/// among its steps in CYCLE.
pub fn step_beats(p: &ModuleParams, r: usize) -> f64 {
    if p.mode() == CYCLE_MODE {
        cycle_beats(p) / steps(p, r) as f64
    } else {
        1.0 / steps_per_beat(p.get(RATE))
    }
}

pub fn cycle_beats(p: &ModuleParams) -> f64 {
    p.get(CYCLE).round().clamp(1.0, 32.0) as f64
}

/// "1/16" (GRID) or "1 bar" (CYCLE).
pub fn clock_label(p: &ModuleParams) -> String {
    if p.mode() == CYCLE_MODE {
        beats_label(cycle_beats(p) as u32)
    } else {
        DIVISIONS[(p.get(RATE).round().max(0.0) as usize).min(DIVISIONS.len() - 1)]
            .0
            .to_owned()
    }
}

/// "3 beats", "1 bar", "2 bars", "6 beats".
pub fn beats_label(beats: u32) -> String {
    match beats {
        1 => "1 beat".to_owned(),
        b if b % 4 == 0 => format!("{} bar{}", b / 4, if b == 4 { "" } else { "s" }),
        b => format!("{b} beats"),
    }
}

/// The General MIDI drum on `note` (channel 10), if there is one.
pub fn drum_name(note: u8) -> Option<&'static str> {
    const DRUMS: [&str; 47] = [
        "Acoustic Bass Drum",
        "Bass Drum",
        "Side Stick",
        "Acoustic Snare",
        "Hand Clap",
        "Electric Snare",
        "Low Floor Tom",
        "Closed Hi-Hat",
        "High Floor Tom",
        "Pedal Hi-Hat",
        "Low Tom",
        "Open Hi-Hat",
        "Low-Mid Tom",
        "Hi-Mid Tom",
        "Crash Cymbal 1",
        "High Tom",
        "Ride Cymbal 1",
        "Chinese Cymbal",
        "Ride Bell",
        "Tambourine",
        "Splash Cymbal",
        "Cowbell",
        "Crash Cymbal 2",
        "Vibraslap",
        "Ride Cymbal 2",
        "Hi Bongo",
        "Low Bongo",
        "Mute Hi Conga",
        "Open Hi Conga",
        "Low Conga",
        "High Timbale",
        "Low Timbale",
        "High Agogo",
        "Low Agogo",
        "Cabasa",
        "Maracas",
        "Short Whistle",
        "Long Whistle",
        "Short Guiro",
        "Long Guiro",
        "Claves",
        "Hi Wood Block",
        "Low Wood Block",
        "Mute Cuica",
        "Open Cuica",
        "Mute Triangle",
        "Open Triangle",
    ];
    (35..=81)
        .contains(&note)
        .then(|| DRUMS[(note - 35) as usize])
}

// ---------------------------------------------------------------------------
// The processor
// ---------------------------------------------------------------------------

/// A note a ring is sounding: its channel (1–16), note and the beat it ends.
#[derive(Clone, Copy)]
struct Held {
    channel: u8,
    note: u8,
    end: f64,
}

/// An event of this block, sorted before it goes out: by frame, note-offs
/// before note-ons, then in the order made.
#[derive(Clone, Copy, Default)]
struct Event {
    frame: usize,
    on: bool,
    order: u16,
    data: [u8; 3],
}

const EVENT_CAP: usize = 64;

pub struct Orrery {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    cursor: u32,
    tick: u32,
    clocks: [StepClock; RINGS],
    held: [Option<Held>; RINGS],
    rng: Rng,
    hits: [u32; RINGS],
    out: MidiBuf,
    events: [Event; EVENT_CAP],
    count: usize,
}

impl Orrery {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>) -> Self {
        Self {
            params,
            cursor: midi.stream_head(),
            midi,
            tick: 0,
            clocks: std::array::from_fn(|_| StepClock::new()),
            held: [None; RINGS],
            rng: Rng::new(0x0E7E_7E11),
            hits: [0; RINGS],
            out: MidiBuf::default(),
            events: [Event::default(); EVENT_CAP],
            count: 0,
        }
    }

    fn push(&mut self, frame: usize, data: [u8; 3]) {
        if self.count < EVENT_CAP {
            let on = data[0] & 0xF0 == 0x90;
            self.events[self.count] = Event {
                frame,
                on,
                order: self.count as u16,
                data,
            };
            self.count += 1;
        }
    }

    /// Ends ring `r`'s note at `frame`.
    fn release(&mut self, r: usize, frame: usize) {
        if let Some(h) = self.held[r].take() {
            self.push(frame, [0x80 | (h.channel - 1), h.note, 0]);
        }
    }

    /// Ends ring `r`'s note if it ends before `beat`, where it ends.
    fn release_before(&mut self, ctx: &ProcessCtx, n: usize, r: usize, beat: f64) {
        if let Some(h) = self.held[r]
            && h.end < beat
        {
            self.release(r, super::steps::frame_of(ctx, n, h.end));
        }
    }

    fn release_all(&mut self) {
        for r in 0..RINGS {
            self.release(r, 0);
        }
    }

    /// The block's events, in order, onto the MIDI output.
    fn flush(&mut self) {
        let events = &mut self.events[..self.count];
        events.sort_unstable_by_key(|e| (e.frame, e.on, e.order));
        for e in events.iter() {
            self.out.push(e.frame, e.data);
        }
        self.count = 0;
    }

    /// Program Change 1–8 (any channel) recalls the buttons.
    fn command(p: &ModuleParams, [status, program, _]: [u8; 3]) {
        if status & 0xF0 == 0xC0 && (program as usize) < SLOTS {
            recall(p, program as usize);
        }
    }

    /// Program changes from the MIDI input: its cable, or else the keyboard.
    fn listen(&mut self, p: &ModuleParams, inputs: &Inputs) {
        if inputs.midi_connected {
            self.cursor = self.midi.stream_head();
            for e in inputs.midi.events() {
                Self::command(p, e.data);
            }
        } else {
            let mut cursor = self.cursor;
            self.midi
                .read_stream(&mut cursor, |msg| Self::command(p, msg));
            self.cursor = cursor;
        }
    }

    /// Publishes where each ring's planet is at `beat`.
    fn publish_phases(&self, p: &ModuleParams, beat: f64) {
        for r in 0..RINGS {
            let n = steps(p, r) as f64;
            let phase = (beat / step_beats(p, r)).rem_euclid(n);
            p.viz[viz::PHASE + r].set(phase as f32);
        }
    }
}

impl Processor for Orrery {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, _outputs: &mut [Buf], n: usize) {
        self.out.clear();
        self.count = 0;
        let p = self.params.clone();
        self.tick = (self.tick + 1) % (1 << 20);
        p.viz[viz::TICK].set(self.tick as f32);
        self.listen(&p, inputs);
        // A recalled pattern comes in when LAUNCH says (at once while
        // stopped): its steps from then on play from it.
        if let Some(k) = pending(&p) {
            let live = p.toggled() && ctx.playing;
            if !live || launch_beat(&p, ctx.beat) < block_end(ctx, n) {
                take(&p, k);
            }
        }
        if ctx.restart {
            // Rewound: every ring from its first step, the notes ended.
            self.clocks = std::array::from_fn(|_| StepClock::new());
            self.release_all();
        }
        // Off, or the transport stopped: end the notes.
        if !p.toggled() || !ctx.playing {
            self.release_all();
            self.flush();
            p.viz[viz::LIVE].set(0.0);
            for r in 0..RINGS {
                p.viz[viz::STEP + r].set(-1.0);
            }
            self.publish_phases(&p, ctx.beat);
            return;
        }
        p.viz[viz::LIVE].set(1.0);
        let cycle = p.mode() == CYCLE_MODE;
        let swing = if cycle { 0.0 } else { p.get(SWING) };
        let accent = p.get(ACCENT).clamp(0.0, 1.0);
        // The shortest note: a few milliseconds, so its end never shares
        // a frame with its start.
        let shortest = 0.004 * ctx.beats_per_sample * ctx.sample_rate as f64;
        let end = block_end(ctx, n);
        for r in 0..RINGS {
            let count = steps(&p, r);
            let len = step_beats(&p, r);
            let turned = turn(&p, r) as f64;
            let (whole, part) = (turned.floor(), turned - turned.floor());
            // A part of a step turned delays every hit by as much: the steps
            // are found on a clock that runs that much behind.
            let shifted = ProcessCtx {
                beat: ctx.beat - part * len,
                ..*ctx
            };
            let mut found = [Step {
                frame: 0,
                beat: 0.0,
                index: 0,
            }; MAX_BLOCK_STEPS];
            let k = self.clocks[r].steps(&shifted, n, 1.0 / len, swing, &mut found);
            for s in &found[..k] {
                let beat = s.beat + part * len;
                self.release_before(ctx, n, r, beat);
                let i = (s.index - whole as i64).rem_euclid(count as i64) as usize;
                p.viz[viz::STEP + r].set(i as f32);
                let value = step(&p, r, i);
                if !is_on(&p, r) || value == OFF || !roll(&mut self.rng, chance(&p, r)) {
                    continue;
                }
                let base = velocity(&p, r) as f32;
                let vel = if value == ACCENTED {
                    base + (127.0 - base) * accent
                } else {
                    base
                };
                let (channel, note) = (channel(&p, r), note(&p, r));
                self.release(r, s.frame);
                self.push(
                    s.frame,
                    [
                        0x90 | (channel - 1),
                        note,
                        vel.round().clamp(1.0, 127.0) as u8,
                    ],
                );
                self.held[r] = Some(Held {
                    channel,
                    note,
                    end: beat + (gate(&p, r) as f64 * len).max(shortest),
                });
                self.hits[r] = self.hits[r].wrapping_add(1);
                p.viz[viz::HITS + r].set((self.hits[r] % (1 << 20)) as f32);
                p.viz[viz::HIT_STEP + r].set(i as f32);
            }
            self.release_before(ctx, n, r, end);
        }
        self.flush();
        self.publish_phases(&p, end);
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}
