//! Nullpunkt: an instrument for clicks, pulses and glitches, in the manner
//! of Alva Noto. Four micro-voices play step patterns of their own lengths,
//! so the patterns phase against each other on the board's clock: CLICK
//! (filtered impulses), SINE (high sine blips), SUB (sub-bass sine pulses)
//! and DATA (1-bit digital noise). HITS and LEN lay a Euclidean pattern
//! down, which can then be edited step by step; PROB lets a step play only
//! sometimes, JITTER varies each hit, MUTATE flips steps and adds ratchets.
//! A STUTTER repeats grains of the sound, each a subdivision of the step
//! (so always in time with the board), scattered an octave up or down,
//! backwards or from earlier grains. A GLITCH stage then skips (a CD stuck
//! on a slice), reverses, warps (an octave up or down), cuts, sprinkles
//! static, crushes and decimates the lot. Tones to lay under it come from
//! other nodes.

use std::sync::Arc;

use super::steps::{MAX_STEPS, Step, StepClock, roll};
use super::{DelayLine, ModuleDef, ModuleParams, Rng, approach, soft_limit};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{ParamSpec, Unit};

/// The four voices.
pub mod voice {
    pub const CLICK: usize = 0;
    pub const SINE: usize = 1;
    pub const SUB: usize = 2;
    pub const DATA: usize = 3;
    pub const COUNT: usize = 4;
    pub const NAMES: [&str; COUNT] = ["CLICK", "SINE", "SUB", "DATA"];
}

/// A voice's knobs, relative to its first (see [`vp`]).
pub mod knob {
    pub const HITS: usize = 0;
    pub const LEN: usize = 1;
    /// The chance a step that hits plays.
    pub const PROB: usize = 2;
    pub const TONE: usize = 3;
    pub const DECAY: usize = 4;
    pub const LEVEL: usize = 5;
    pub const COUNT: usize = 6;
}

pub const fn vp(v: usize, k: usize) -> usize {
    v * knob::COUNT + k
}

/// The longest pattern.
pub const MAX_LEN: usize = 32;

pub const CRUSH: usize = voice::COUNT * knob::COUNT;
pub const RATE: usize = CRUSH + 1;
pub const SKIP: usize = CRUSH + 2;
pub const CUT: usize = CRUSH + 3;
pub const REVERSE: usize = CRUSH + 4;
pub const WARP: usize = CRUSH + 5;
pub const STATIC: usize = CRUSH + 6;
/// The stutter: chance a step starts one, its longest run in steps, how
/// fine its grains go, how scattered they are.
pub const STUTTER: usize = CRUSH + 7;
pub const STUTTER_LEN: usize = CRUSH + 8;
pub const GRAIN: usize = CRUSH + 9;
pub const SCATTER: usize = CRUSH + 10;
/// How much each hit varies (level, tone, decay).
pub const JITTER: usize = CRUSH + 11;
pub const MUTATE: usize = CRUSH + 12;
pub const WIDTH: usize = CRUSH + 13;
pub const LEVEL: usize = CRUSH + 14;
pub const PARAM_COUNT: usize = LEVEL + 1;

/// Each voice's TONE and DECAY ranges (lo, hi, default).
const TONES: [(f32, f32, f32); voice::COUNT] = [
    (300.0, 16_000.0, 3800.0),
    (800.0, 18_000.0, 8200.0),
    (25.0, 160.0, 47.0),
    (200.0, 24_000.0, 6000.0),
];
const DECAYS: [(f32, f32, f32); voice::COUNT] = [
    (0.0005, 0.05, 0.004),
    (0.002, 0.4, 0.03),
    (0.02, 1.5, 0.32),
    (0.001, 0.4, 0.018),
];
/// Each voice's starting pattern: hits, length, rotation.
const STARTS: [(usize, usize, usize); voice::COUNT] =
    [(7, 16, 0), (3, 13, 2), (4, 16, 0), (5, 12, 3)];

/// HITS, LEN, PROB, TONE, DECAY, LEVEL of voice `$v`.
macro_rules! voice {
    ($s:literal, $tone:literal, $v:expr, $level:expr) => {
        [
            ParamSpec::lin(
                concat!($s, " HITS"),
                0.0,
                MAX_LEN as f32,
                STARTS[$v].0 as f32,
                Unit::Integer,
            ),
            ParamSpec::lin(
                concat!($s, " LEN"),
                1.0,
                MAX_LEN as f32,
                STARTS[$v].1 as f32,
                Unit::Integer,
            ),
            ParamSpec::lin(concat!($s, " PROB"), 0.0, 1.0, 1.0, Unit::Percent),
            ParamSpec::log(
                concat!($s, " ", $tone),
                TONES[$v].0,
                TONES[$v].1,
                TONES[$v].2,
                Unit::Hz,
            ),
            ParamSpec::log(
                concat!($s, " DECAY"),
                DECAYS[$v].0,
                DECAYS[$v].1,
                DECAYS[$v].2,
                Unit::Seconds,
            ),
            ParamSpec::lin(concat!($s, " LEVEL"), 0.0, 1.0, $level, Unit::Percent),
        ]
    };
}

const PARAMS: [ParamSpec; PARAM_COUNT] = {
    const V: [[ParamSpec; knob::COUNT]; voice::COUNT] = [
        voice!("CLICK", "COLOR", 0, 0.7),
        voice!("SINE", "PITCH", 1, 0.3),
        voice!("SUB", "PITCH", 2, 0.8),
        voice!("DATA", "GRAIN", 3, 0.3),
    ];
    let mut out = [ParamSpec::lin("", 0.0, 1.0, 0.0, Unit::Percent); PARAM_COUNT];
    let mut v = 0;
    while v < voice::COUNT {
        let mut k = 0;
        while k < knob::COUNT {
            out[vp(v, k)] = V[v][k];
            k += 1;
        }
        v += 1;
    }
    out[CRUSH] = ParamSpec::lin("CRUSH", 0.0, 1.0, 0.0, Unit::Percent);
    out[RATE] = ParamSpec::lin("RATE", 0.0, 1.0, 0.0, Unit::Percent);
    out[SKIP] = ParamSpec::lin("SKIP", 0.0, 1.0, 0.1, Unit::Percent);
    out[CUT] = ParamSpec::lin("CUT", 0.0, 1.0, 0.05, Unit::Percent);
    out[REVERSE] = ParamSpec::lin("REVERSE", 0.0, 1.0, 0.0, Unit::Percent);
    out[WARP] = ParamSpec::lin("WARP", 0.0, 1.0, 0.0, Unit::Percent);
    out[STATIC] = ParamSpec::lin("STATIC", 0.0, 1.0, 0.0, Unit::Percent);
    out[STUTTER] = ParamSpec::lin("STUTTER", 0.0, 1.0, 0.1, Unit::Percent);
    out[STUTTER_LEN] = ParamSpec::lin("STUTTER LENGTH", 1.0, 8.0, 2.0, Unit::Integer);
    out[GRAIN] = ParamSpec::lin("GRAIN", 0.0, 1.0, 0.45, Unit::Percent);
    out[SCATTER] = ParamSpec::lin("SCATTER", 0.0, 1.0, 0.3, Unit::Percent);
    out[JITTER] = ParamSpec::lin("JITTER", 0.0, 1.0, 0.15, Unit::Percent);
    out[MUTATE] = ParamSpec::lin("MUTATE", 0.0, 1.0, 0.15, Unit::Percent);
    out[WIDTH] = ParamSpec::lin("WIDTH", 0.0, 1.0, 0.6, Unit::Percent);
    out[LEVEL] = ParamSpec::lin("LEVEL", 0.0, 1.0, 0.8, Unit::Percent);
    out
};

/// Settings beyond the knobs.
pub mod data {
    /// Each voice's pattern rotation, in steps.
    pub const ROT: usize = 0;
    /// The grid: one of [`super::DIVS`].
    pub const DIV: usize = 4;
    /// Each voice muted.
    pub const MUTE: usize = 5;
    /// Each voice's steps: the low and high 16 as numbers.
    pub const STEPS: usize = 9;
    pub const LEN: usize = STEPS + 2 * super::voice::COUNT;
}

const DATA: [f32; data::LEN] = {
    let mut d = [0.0; data::LEN];
    let mut v = 0;
    while v < voice::COUNT {
        let (hits, len, rot) = STARTS[v];
        let bits = euclid_steps(hits, len, rot);
        d[data::ROT + v] = rot as f32;
        d[data::STEPS + 2 * v] = (bits & 0xFFFF) as f32;
        d[data::STEPS + 2 * v + 1] = (bits >> 16) as f32;
        v += 1;
    }
    d[data::DIV] = SIXTEENTHS as f32;
    d
};
/// The grid's divisions: name and steps per beat.
pub const DIVS: [(&str, f64); 8] = [
    ("1/4", 1.0),
    ("1/8", 2.0),
    ("1/8T", 3.0),
    ("1/16", 4.0),
    ("1/16T", 6.0),
    ("1/32", 8.0),
    ("1/32T", 12.0),
    ("1/64", 16.0),
];
const SIXTEENTHS: usize = 3;
/// A stutter's grains: subdivisions of a step, coarse to fine (triplets
/// among them). GRAIN sets how far down the list a stutter may go.
pub const SUBDIVISIONS: [usize; 9] = [2, 3, 4, 6, 8, 12, 16, 24, 32];

/// What the processor shows the panel.
pub mod viz {
    use super::voice;
    /// The last step (−1: stopped).
    pub const STEP: usize = 0;
    /// Hits per voice so far (the lanes flash on a change).
    pub const HITS: usize = 1;
    /// The glitch on the step now (see [`super::glitch`]), and static
    /// crackling (1 while on).
    pub const GLITCH: usize = HITS + voice::COUNT;
    pub const STATIC: usize = GLITCH + 1;
    /// The output's recent peaks, a ring of [`STRIP_LEN`], and where it
    /// was last written.
    pub const STRIP_POS: usize = STATIC + 1;
    pub const STRIP: usize = STRIP_POS + 1;
    pub const STRIP_LEN: usize = 128;
    pub const END: usize = STRIP + STRIP_LEN;
}

/// What a step's glitch can be (as [`viz::GLITCH`] shows it).
pub mod glitch {
    pub const NONE: usize = 0;
    pub const SKIP: usize = 1;
    pub const REVERSE: usize = 2;
    pub const WARP: usize = 3;
    pub const CUT: usize = 4;
    pub const STUTTER: usize = 5;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Nullpunkt",
    prefix: "NULLPUNKT",
    hint: "clicks · pulses · glitch",
    inputs: &[],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::END,
};

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// Whether step `pos` of a Euclidean pattern of `hits` over `len` is a hit
/// (hits spread as evenly as they go, the first on step 0).
pub const fn euclid(hits: usize, len: usize, pos: usize) -> bool {
    let len = if len == 0 { 1 } else { len };
    let hits = if hits > len { len } else { hits };
    (pos % len) * hits % len < hits
}

/// A Euclidean pattern as step bits, turned `rot` steps later.
pub const fn euclid_steps(hits: usize, len: usize, rot: usize) -> u32 {
    let len = if len == 0 {
        1
    } else if len > MAX_LEN {
        MAX_LEN
    } else {
        len
    };
    let mut bits = 0u32;
    let mut s = 0;
    while s < len {
        if euclid(hits, len, (s + len - rot % len) % len) {
            bits |= 1 << s;
        }
        s += 1;
    }
    bits
}

/// Voice `v`'s generator: hits, length, rotation.
pub fn pattern(p: &ModuleParams, v: usize) -> (usize, usize, usize) {
    let len = (p.get(vp(v, knob::LEN)).round().max(1.0) as usize).min(MAX_LEN);
    let hits = (p.get(vp(v, knob::HITS)).round().max(0.0) as usize).min(len);
    let rot = p.data[data::ROT + v].get().round().rem_euclid(len as f32) as usize;
    (hits, len, rot)
}

/// Voice `v`'s steps, one bit each (only the first LEN play).
pub fn steps(p: &ModuleParams, v: usize) -> u32 {
    let at = data::STEPS + 2 * v;
    let lo = p.data[at].get().max(0.0) as u32 & 0xFFFF;
    let hi = p.data[at + 1].get().max(0.0) as u32 & 0xFFFF;
    lo | hi << 16
}

pub fn set_steps(p: &ModuleParams, v: usize, bits: u32) {
    let at = data::STEPS + 2 * v;
    p.data[at].set((bits & 0xFFFF) as f32);
    p.data[at + 1].set((bits >> 16) as f32);
}

pub fn set_step(p: &ModuleParams, v: usize, step: usize, on: bool) {
    let bit = 1u32 << step.min(MAX_LEN - 1);
    let bits = steps(p, v);
    set_steps(p, v, if on { bits | bit } else { bits & !bit });
}

/// Lays voice `v`'s Euclidean pattern down again (HITS, LEN, rotation),
/// over any edits.
pub fn regenerate(p: &ModuleParams, v: usize) {
    let (hits, len, rot) = pattern(p, v);
    set_steps(p, v, euclid_steps(hits, len, rot));
}

/// Turns voice `v`'s steps `by` steps later (earlier if negative), within
/// its length.
pub fn rotate(p: &ModuleParams, v: usize, by: i32) {
    let (_, len, rot) = pattern(p, v);
    let bits = steps(p, v);
    let mut out = bits & !((1u64 << len) - 1) as u32;
    for s in 0..len {
        if bits >> s & 1 == 1 {
            out |= 1 << (s as i32 + by).rem_euclid(len as i32);
        }
    }
    set_steps(p, v, out);
    p.data[data::ROT + v].set((rot as i32 + by).rem_euclid(len as i32) as f32);
}

pub fn muted(p: &ModuleParams, v: usize) -> bool {
    p.data[data::MUTE + v].get() > 0.5
}

/// Where voice `v` is in its pattern at step `index`.
pub fn position(p: &ModuleParams, v: usize, index: i64) -> usize {
    let (_, len, _) = pattern(p, v);
    index.rem_euclid(len as i64) as usize
}

pub fn division(p: &ModuleParams) -> usize {
    (p.data[data::DIV].get().round().max(0.0) as usize).min(DIVS.len() - 1)
}

/// Rolls the dice on every voice that isn't muted: new lengths, densities
/// and rotations, a few steps flipped and (with `sounds`) new tones and
/// decays. Levels and the glitch stage stay as they are.
pub fn randomize(p: &ModuleParams, seed: u32, sounds: bool) {
    let mut rng = Rng::new(seed);
    // How busy each voice tends to be (share of steps), and the lengths
    // that phase well.
    const DENSITY: [(f32, f32); voice::COUNT] =
        [(0.25, 0.7), (0.08, 0.35), (0.1, 0.3), (0.15, 0.55)];
    const LENGTHS: [usize; 9] = [8, 11, 12, 13, 15, 16, 16, 24, 32];
    for v in 0..voice::COUNT {
        if muted(p, v) {
            continue;
        }
        let len = LENGTHS[(rng.unit() * LENGTHS.len() as f32) as usize % LENGTHS.len()];
        let (lo, hi) = DENSITY[v];
        let hits = ((lo + (hi - lo) * rng.unit()) * len as f32)
            .round()
            .max(1.0) as usize;
        let rot = (rng.unit() * len as f32) as usize % len;
        p.set(vp(v, knob::LEN), len as f32);
        p.set(vp(v, knob::HITS), hits as f32);
        p.data[data::ROT + v].set(rot as f32);
        let mut bits = euclid_steps(hits, len, rot);
        for s in 0..len {
            if rng.unit() < 0.08 {
                bits ^= 1 << s;
            }
        }
        set_steps(p, v, bits);
        if sounds {
            // Within the lower, more musical part of each range.
            let (tlo, thi, _) = TONES[v];
            let (dlo, dhi, _) = DECAYS[v];
            p.set(
                vp(v, knob::TONE),
                tlo * (thi / tlo).powf(0.15 + 0.7 * rng.unit()),
            );
            p.set(
                vp(v, knob::DECAY),
                dlo * (dhi / dlo).powf(0.1 + 0.5 * rng.unit()),
            );
        }
    }
}

/// A knob's value as the panel shows it.
pub fn format(p: &ModuleParams, i: usize) -> String {
    let v = p.get(i);
    let spec = &PARAMS[i];
    match spec.unit {
        Unit::Integer => format!("{v:.0}"),
        Unit::Hz if v >= 1000.0 => format!("{:.1}k", v / 1000.0),
        Unit::Hz => format!("{v:.0} Hz"),
        Unit::Seconds if v < 0.1 => format!("{:.1} ms", v * 1000.0),
        Unit::Seconds => format!("{:.2} s", v),
        _ if i == CRUSH && v > 0.0 => format!("{:.0} bit", crush_bits(v)),
        _ if i == RATE && v > 0.0 => format!("÷{:.0}", hold_samples(v)),
        _ if (i == CRUSH || i == RATE) => "off".to_owned(),
        _ if i == GRAIN => format!("≤1/{}", SUBDIVISIONS[grain_depth(v)]),
        _ => format!("{:.0}%", v * 100.0),
    }
}

/// The finest subdivision GRAIN allows (an index into [`SUBDIVISIONS`]).
fn grain_depth(v: f32) -> usize {
    ((v * (SUBDIVISIONS.len() - 1) as f32).round() as usize).min(SUBDIVISIONS.len() - 1)
}

fn crush_bits(v: f32) -> f32 {
    16.0 - 14.0 * v
}

fn hold_samples(v: f32) -> f32 {
    1.0 + 47.0 * v * v
}

/// One micro-voice.
#[derive(Clone, Copy, Default)]
struct Voice {
    /// Decaying envelope, its fall per sample, and an attack still to go
    /// (SUB fades in over a millisecond).
    env: f32,
    fall: f32,
    rise: f32,
    /// Sine phase and step (SINE, SUB); DATA's held value and clock.
    phase: f32,
    inc: f32,
    held: f32,
    /// CLICK's resonator (a band-pass SVF): state and coefficients.
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    /// The impulse still to come into the resonator (a doublet).
    kick: [f32; 2],
    pan: (f32, f32),
    /// This hit's level (JITTER varies it).
    gain: f32,
    /// Ratchets: repeats left, samples between them, samples to the next.
    repeats: u8,
    every: usize,
    next: usize,
}

impl Voice {
    fn trigger(&mut self, kind: usize, tone: f32, decay: f32, pan: (f32, f32), sr: f32) {
        self.fall = (-6.9 / (decay.max(0.0003) * sr)).exp();
        self.pan = pan;
        match kind {
            voice::CLICK => {
                let f = tone.min(sr * 0.45);
                let g = fastmath::tan_turns(f / (2.0 * sr));
                let q = (std::f32::consts::PI * f * decay / 3.0).clamp(0.6, 40.0);
                self.k = 1.0 / q;
                self.a1 = 1.0 / (1.0 + g * (g + self.k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.kick = [1.0, -1.0];
                self.env = 1.0;
            }
            voice::SUB => {
                self.inc = tone / sr;
                self.rise = 1.0 / (0.001 * sr);
                self.env = self.env.max(0.0);
            }
            voice::SINE | voice::DATA => {
                self.inc = tone / sr;
                self.env = 1.0;
                self.rise = 0.0;
            }
            _ => {}
        }
    }

    #[inline]
    fn tick(&mut self, kind: usize, rng: &mut Rng) -> f32 {
        match kind {
            voice::CLICK => {
                let x = self.kick[0];
                self.kick = [self.kick[1], 0.0];
                let v3 = x - self.ic2;
                let v1 = self.a1 * self.ic1 + self.a2 * v3;
                let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
                self.ic1 = 2.0 * v1 - self.ic1;
                self.ic2 = 2.0 * v2 - self.ic2;
                self.env *= self.fall;
                v1 * 3.0 + x * 0.35
            }
            voice::DATA => {
                // 1-bit noise, held at the GRAIN rate.
                self.phase += self.inc;
                if self.phase >= 1.0 {
                    self.phase -= self.phase.floor();
                    self.held = if rng.unit() < 0.5 { -1.0 } else { 1.0 };
                }
                self.env *= self.fall;
                self.held * self.env * 0.8
            }
            _ => {
                if self.rise > 0.0 {
                    self.env += self.rise;
                    if self.env >= 1.0 {
                        self.env = 1.0;
                        self.rise = 0.0;
                    }
                } else {
                    self.env *= self.fall;
                }
                self.phase = (self.phase + self.inc).fract();
                fastmath::sin_turns(self.phase) * self.env
            }
        }
    }

    fn active(&self, kind: usize) -> bool {
        self.rise > 0.0
            || self.env > 1e-4
            || (kind == voice::CLICK && (self.ic1.abs() > 1e-5 || self.ic2.abs() > 1e-5))
    }
}

/// A sound event this block.
#[derive(Clone, Copy)]
struct Event {
    frame: usize,
    /// A voice to strike (with ratchets), or (`usize::MAX`) a step for the
    /// glitch stage.
    voice: usize,
    ratchet: u8,
}

/// The glitch on the step playing.
#[derive(Clone, Copy)]
enum Glitch {
    None,
    /// A slice of what just played, looping: its length, samples in.
    Skip {
        slice: usize,
        t: usize,
    },
    /// The last step, backwards.
    Reverse {
        t: usize,
    },
    /// A slice looping an octave up (speed 2) or down (½).
    Warp {
        slice: usize,
        speed: f32,
        t: usize,
    },
    /// Grains of the sound since the stutter began, `grain` long (a
    /// subdivision of the step), over `steps` steps; `e` samples in, on
    /// grain `k`, which plays as `shape`.
    Stutter {
        grain: f32,
        steps: usize,
        e: usize,
        k: usize,
        shape: Grain,
    },
}

/// How a stutter's grain plays its source (the stutter's first grain, or
/// one `back` grains before it).
#[derive(Clone, Copy)]
enum Grain {
    Plain,
    Up,
    Down,
    Reverse,
    Earlier(usize),
}

/// Grains fade in and out over this many samples.
const GRAIN_FADE: f32 = 32.0;

const MAX_EVENTS: usize = MAX_STEPS * (voice::COUNT + 1);
/// The glitch buffer: long enough for two slow steps.
const RECENT_SECONDS: f32 = 6.0;
/// Cuts open and close this fast (samples).
const CUT_RAMP: f32 = 32.0;

pub struct Nullpunkt {
    params: Arc<ModuleParams>,
    sr: f32,
    rng: Rng,
    clock: StepClock,
    voices: [Voice; voice::COUNT],
    hits: [u32; voice::COUNT],
    /// The glitch stage: the recent mix, this step's glitch and samples
    /// left of it, a cut (samples left) and its gain, static (samples to
    /// wait, samples left, its held value and hold), the decimator.
    recent: [DelayLine; 2],
    glitch: Glitch,
    glitch_left: usize,
    cut_left: usize,
    cut_gain: f32,
    static_wait: usize,
    static_left: usize,
    static_value: f32,
    static_hold: usize,
    hold: [f32; 2],
    hold_count: f32,
    level: f32,
    /// The output strip for the panel: peak so far and blocks in it.
    strip_peak: f32,
    strip_blocks: usize,
    strip_pos: usize,
}

impl Nullpunkt {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        let len = (RECENT_SECONDS * sample_rate) as usize;
        params.viz[viz::STEP].set(-1.0);
        Self {
            level: params.get(LEVEL),
            params,
            sr: sample_rate,
            rng: Rng::new(0x0a1f_a707),
            clock: StepClock::new(),
            voices: [Voice::default(); voice::COUNT],
            hits: [0; voice::COUNT],
            recent: [DelayLine::new(len), DelayLine::new(len)],
            glitch: Glitch::None,
            glitch_left: 0,
            cut_left: 0,
            cut_gain: 1.0,
            static_wait: 0,
            static_left: 0,
            static_value: 0.0,
            static_hold: 0,
            hold: [0.0; 2],
            hold_count: 0.0,
            strip_peak: 0.0,
            strip_blocks: 0,
            strip_pos: 0,
        }
    }

    /// A random place in the stereo field, as wide as WIDTH.
    fn place(&mut self, width: f32) -> (f32, f32) {
        let pan = self.rng.bipolar() * width;
        let a = (pan + 1.0) * 0.125;
        (fastmath::cos_turns(a), fastmath::sin_turns(a))
    }

    /// At a step: which glitch (if any) takes it, and whether static
    /// crackles somewhere in it.
    fn roll_glitch(&mut self, p: &ModuleParams, step_len: usize) {
        let rng = &mut self.rng;
        // A stutter runs on over its steps (now and then, with SCATTER,
        // starting afresh on this one).
        if let Glitch::Stutter { steps, .. } = self.glitch
            && steps > 1
        {
            self.glitch_left = step_len + step_len / 4;
            if roll(rng, p.get(SCATTER) * 0.5) {
                self.glitch = stutter(p, rng, step_len, steps - 1);
            } else if let Glitch::Stutter { steps, .. } = &mut self.glitch {
                *steps -= 1;
            }
            return;
        }
        self.glitch = Glitch::None;
        self.cut_left = 0;
        self.glitch_left = step_len;
        if roll(rng, p.get(STUTTER)) {
            // At most as many steps as the buffer holds (with grains to
            // reach back to).
            let room = ((self.recent[0].len() / step_len.max(1)).saturating_sub(3)).max(1);
            let most = (p.get(STUTTER_LEN).round().max(1.0) as usize).min(room);
            let steps = 1 + (rng.unit() * most as f32) as usize % most;
            self.glitch = stutter(p, rng, step_len, steps);
            self.glitch_left = step_len + step_len / 4;
        } else if roll(rng, p.get(SKIP) * 0.5) {
            let slice = (step_len >> (1 + (rng.unit() * 4.0) as usize)).max(64);
            self.glitch = Glitch::Skip { slice, t: 0 };
        } else if roll(rng, p.get(REVERSE) * 0.5) {
            self.glitch = Glitch::Reverse { t: 0 };
        } else if roll(rng, p.get(WARP) * 0.5) {
            let slice = (step_len >> (1 + (rng.unit() * 2.0) as usize)).max(64);
            let speed = if rng.unit() < 0.5 { 2.0 } else { 0.5 };
            self.glitch = Glitch::Warp { slice, speed, t: 0 };
        } else if roll(rng, p.get(CUT) * 0.5) {
            self.cut_left = step_len;
        }
        if roll(rng, p.get(STATIC) * 0.5) {
            self.static_wait = (rng.unit() * step_len as f32 * 0.7) as usize;
            self.static_left = ((0.002 + 0.02 * rng.unit()) * self.sr) as usize;
        }
    }
}

/// A stutter over `steps` steps, its grains a subdivision of the step as
/// fine as GRAIN allows.
fn stutter(p: &ModuleParams, rng: &mut Rng, step_len: usize, steps: usize) -> Glitch {
    let depth = grain_depth(p.get(GRAIN));
    let sub = SUBDIVISIONS[(rng.unit() * (depth + 1) as f32) as usize % (depth + 1)];
    let grain = (step_len as f32 / sub as f32).max(GRAIN_FADE * 2.0);
    Glitch::Stutter {
        grain,
        steps,
        e: 0,
        k: 0,
        shape: Grain::Plain,
    }
}

impl Processor for Nullpunkt {
    #[allow(clippy::needless_range_loop)] // both channels, voices by index
    fn process(&mut self, ctx: &ProcessCtx, _inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let sr = self.sr;

        // The grid: which voices strike on which steps, and the glitches.
        let per_beat = DIVS[division(&p)].1;
        let mut steps_now = [Step {
            frame: 0,
            beat: 0.0,
            index: 0,
        }; MAX_STEPS];
        let found = if ctx.playing {
            self.clock.steps(ctx, n, per_beat, 0.0, &mut steps_now)
        } else {
            0
        };
        if ctx.restart || !ctx.playing {
            self.clock = StepClock::new();
        }
        let step_len = if ctx.beats_per_sample > 0.0 {
            (1.0 / (per_beat * ctx.beats_per_sample)) as usize
        } else {
            (sr * 0.125) as usize
        };
        let mutate = p.get(MUTATE);
        let mut events = [Event {
            frame: 0,
            voice: 0,
            ratchet: 0,
        }; MAX_EVENTS];
        let mut count = 0;
        for step in &steps_now[..found] {
            p.viz[viz::STEP].set((step.index.rem_euclid(1 << 20)) as f32);
            for v in 0..voice::COUNT {
                if muted(&p, v) {
                    continue;
                }
                let bits = steps(&p, v);
                let mut hit = bits >> position(&p, v, step.index) & 1 == 1;
                // MUTATE: now and then a step flips.
                if bits != 0 && roll(&mut self.rng, mutate * 0.12) {
                    hit = !hit;
                }
                if hit
                    && p.get(vp(v, knob::LEVEL)) > 0.0
                    && roll(&mut self.rng, p.get(vp(v, knob::PROB)))
                {
                    let ratchet = if roll(&mut self.rng, mutate * 0.25) {
                        1 + (self.rng.unit() * 3.0) as u8
                    } else {
                        0
                    };
                    events[count] = Event {
                        frame: step.frame,
                        voice: v,
                        ratchet,
                    };
                    count += 1;
                }
            }
            events[count] = Event {
                frame: step.frame,
                voice: usize::MAX,
                ratchet: 0,
            };
            count += 1;
        }
        if !ctx.playing {
            p.viz[viz::STEP].set(-1.0);
        }

        // Per-voice sound settings, for this block.
        let tones: [f32; voice::COUNT] = std::array::from_fn(|v| p.get(vp(v, knob::TONE)));
        let decays: [f32; voice::COUNT] = std::array::from_fn(|v| p.get(vp(v, knob::DECAY)));
        let levels: [f32; voice::COUNT] = std::array::from_fn(|v| p.get(vp(v, knob::LEVEL)));
        let width = p.get(WIDTH);
        let jitter = p.get(JITTER);
        let scatter = p.get(SCATTER);
        let crush = p.get(CRUSH);
        let quant = fastmath::exp2(crush_bits(crush) - 1.0);
        let hold_n = hold_samples(p.get(RATE));
        let level_t = p.get(LEVEL);

        let mut next = 0;
        let mut peak = 0.0f32;
        for i in 0..n {
            while next < count && events[next].frame <= i {
                let e = events[next];
                next += 1;
                if e.voice == usize::MAX {
                    self.roll_glitch(&p, step_len);
                    continue;
                }
                let pan = self.place(width);
                // JITTER: each hit a little off in level, tone and decay.
                let tone = tones[e.voice] * fastmath::exp2((self.rng.unit() - 0.5) * jitter);
                let decay =
                    decays[e.voice] * fastmath::exp2((self.rng.unit() - 0.5) * jitter * 1.5);
                let gain = 1.0 - jitter * 0.6 * self.rng.unit();
                let v = &mut self.voices[e.voice];
                v.trigger(e.voice, tone, decay, pan, sr);
                v.gain = gain;
                v.repeats = e.ratchet;
                v.every = (step_len / (e.ratchet as usize + 1)).max(1);
                v.next = v.every;
                self.hits[e.voice] = self.hits[e.voice].wrapping_add(1);
            }

            // The voices.
            let (mut l, mut r) = (0.0, 0.0);
            for v in 0..voice::COUNT {
                let vo = &mut self.voices[v];
                if vo.repeats > 0 {
                    vo.next -= 1;
                    if vo.next == 0 {
                        vo.repeats -= 1;
                        vo.next = vo.every;
                        let pan = vo.pan;
                        vo.trigger(v, tones[v], decays[v], pan, sr);
                    }
                }
                if !vo.active(v) {
                    continue;
                }
                let y = vo.tick(v, &mut self.rng) * levels[v] * vo.gain;
                l += y * vo.pan.0;
                r += y * vo.pan.1;
            }

            // Glitch: skip, reverse, warp, cut, static, decimate, crush.
            self.recent[0].push(l);
            self.recent[1].push(r);
            let (mut yl, mut yr) = (l, r);
            let mut fade = 1.0;
            let back = match &mut self.glitch {
                Glitch::None => None,
                Glitch::Stutter {
                    grain, e, k, shape, ..
                } => {
                    let g = *grain;
                    let kk = (*e as f32 / g) as usize;
                    if kk != *k {
                        // A new grain: plain, or with SCATTER pitched,
                        // reversed or taken from before the stutter.
                        *k = kk;
                        *shape = if roll(&mut self.rng, scatter) {
                            match (self.rng.unit() * 4.0) as usize {
                                0 => Grain::Up,
                                1 => Grain::Down,
                                2 => Grain::Reverse,
                                _ => Grain::Earlier(1 + (self.rng.unit() * 3.0) as usize),
                            }
                        } else {
                            Grain::Plain
                        };
                    }
                    let u = *e as f32 - kk as f32 * g;
                    let (from, back) = match *shape {
                        Grain::Plain => (u, 0.0),
                        Grain::Up => ((2.0 * u) % g, 0.0),
                        Grain::Down => (0.5 * u, 0.0),
                        Grain::Reverse => (g - 1.0 - u, 0.0),
                        Grain::Earlier(j) => (u, j as f32 * g),
                    };
                    // The first grain plays live (it is being recorded);
                    // the others fade in and out.
                    if kk > 0 {
                        fade = (u / GRAIN_FADE).min((g - u) / GRAIN_FADE).clamp(0.0, 1.0);
                    }
                    *e += 1;
                    Some(*e as f32 - 1.0 - from + back)
                }
                Glitch::Skip { slice, t } => {
                    let d = *slice + *t - *t % *slice;
                    *t += 1;
                    Some(d as f32)
                }
                Glitch::Reverse { t } => {
                    // The step before, from its end backwards.
                    let d = 2 * *t + 1;
                    *t += 1;
                    Some(d as f32)
                }
                Glitch::Warp { slice, speed, t } => {
                    let at = (*t as f32 * *speed) % *slice as f32;
                    let d = *slice as f32 + *t as f32 - at;
                    *t += 1;
                    Some(d)
                }
            };
            if let Some(d) = back {
                yl = self.recent[0].tap(d) * fade;
                yr = self.recent[1].tap(d) * fade;
                self.glitch_left = self.glitch_left.saturating_sub(1);
                if self.glitch_left == 0 {
                    self.glitch = Glitch::None;
                }
            }
            let cut_t = if self.cut_left > 0 {
                self.cut_left -= 1;
                0.0
            } else {
                1.0
            };
            self.cut_gain += (cut_t - self.cut_gain).clamp(-1.0 / CUT_RAMP, 1.0 / CUT_RAMP);
            yl *= self.cut_gain;
            yr *= self.cut_gain;
            if self.static_wait > 0 {
                self.static_wait -= 1;
            } else if self.static_left > 0 {
                // Data errors: random values, held a few samples each.
                self.static_left -= 1;
                if self.static_hold == 0 {
                    self.static_hold = 1 + (self.rng.unit() * 8.0) as usize;
                    self.static_value = self.rng.bipolar() * (0.15 + 0.5 * self.rng.unit());
                }
                self.static_hold -= 1;
                yl = self.static_value;
                yr = if self.rng.unit() < 0.7 {
                    self.static_value
                } else {
                    -self.static_value
                };
            }
            if hold_n > 1.0 {
                self.hold_count += 1.0;
                if self.hold_count >= hold_n {
                    self.hold_count -= hold_n;
                    self.hold = [yl, yr];
                }
                [yl, yr] = self.hold;
            }
            if crush > 0.0 {
                yl = (yl * quant).round() / quant;
                yr = (yr * quant).round() / quant;
            }
            approach(&mut self.level, level_t, ctx.smooth);
            let (ol, or) = (soft_limit(yl * self.level), soft_limit(yr * self.level));
            outputs[0][0][i] = ol;
            outputs[0][1][i] = or;
            peak = peak.max(ol.abs()).max(or.abs());
        }

        // The panel's readings.
        for (k, h) in self.hits.iter().enumerate() {
            p.viz[viz::HITS + k].set(*h as f32);
        }
        let shown = match self.glitch {
            Glitch::None if self.cut_left > 0 => glitch::CUT,
            Glitch::None => glitch::NONE,
            Glitch::Skip { .. } => glitch::SKIP,
            Glitch::Reverse { .. } => glitch::REVERSE,
            Glitch::Warp { .. } => glitch::WARP,
            Glitch::Stutter { .. } => glitch::STUTTER,
        };
        p.viz[viz::GLITCH].set(shown as f32);
        p.viz[viz::STATIC].set(if self.static_wait == 0 && self.static_left > 0 {
            1.0
        } else {
            0.0
        });
        // The strip: one bar per four blocks.
        self.strip_peak = self.strip_peak.max(peak);
        self.strip_blocks += 1;
        if self.strip_blocks >= 4 {
            self.strip_pos = (self.strip_pos + 1) % viz::STRIP_LEN;
            p.viz[viz::STRIP + self.strip_pos].set(self.strip_peak);
            p.viz[viz::STRIP_POS].set(self.strip_pos as f32);
            self.strip_peak = 0.0;
            self.strip_blocks = 0;
        }
        p.meter.fetch_max(peak);
    }
}

/// A built-in scene: its name, and what it changes from the defaults.
pub type Scene = (&'static str, fn(&ModuleParams));

pub const FACTORY: [Scene; 5] = [
    ("Default", |_| {}),
    ("Raster", |p| {
        p.data[data::DIV].set(5.0);
        lay(p, voice::CLICK, 11, 32, 0, 6000.0, 0.002);
        p.data[data::MUTE + voice::SINE].set(1.0);
        lay(p, voice::SUB, 2, 16, 0, 44.0, 0.5);
        lay(p, voice::DATA, 7, 24, 5, 9000.0, 0.008);
        p.set(SKIP, 0.15);
        p.set(MUTATE, 0.1);
    }),
    ("Pulse", |p| {
        lay(p, voice::CLICK, 4, 16, 2, 2000.0, 0.008);
        lay(p, voice::SINE, 5, 16, 1, 12_000.0, 0.05);
        lay(p, voice::SUB, 4, 16, 0, 42.0, 0.6);
        p.data[data::MUTE + voice::DATA].set(1.0);
        p.set(SKIP, 0.0);
        p.set(CUT, 0.08);
        p.set(MUTATE, 0.0);
    }),
    ("Static", |p| {
        lay(p, voice::CLICK, 5, 12, 0, 4500.0, 0.003);
        lay(p, voice::SUB, 3, 16, 0, 38.0, 0.4);
        lay(p, voice::DATA, 9, 16, 1, 12_000.0, 0.06);
        p.set(vp(voice::DATA, knob::LEVEL), 0.45);
        p.set(STATIC, 0.4);
        p.set(CRUSH, 0.35);
        p.set(RATE, 0.2);
        p.set(SKIP, 0.2);
    }),
    ("Broken", |p| {
        p.set(SKIP, 0.5);
        p.set(REVERSE, 0.35);
        p.set(WARP, 0.35);
        p.set(CUT, 0.15);
        p.set(MUTATE, 0.45);
    }),
];

/// Sets voice `v`'s pattern (laid down at once) and sound.
fn lay(p: &ModuleParams, v: usize, hits: usize, len: usize, rot: usize, tone: f32, decay: f32) {
    p.set(vp(v, knob::HITS), hits as f32);
    p.set(vp(v, knob::LEN), len as f32);
    p.data[data::ROT + v].set(rot as f32);
    p.set(vp(v, knob::TONE), tone);
    p.set(vp(v, knob::DECAY), decay);
    regenerate(p, v);
}

/// Back to the defaults, then built-in scene `k`.
pub fn load_factory(p: &ModuleParams, k: usize) {
    for (i, spec) in PARAMS.iter().enumerate() {
        p.set(i, spec.default);
    }
    for (d, &v) in p.data.iter().zip(&DATA) {
        d.set(v);
    }
    (FACTORY[k.min(FACTORY.len() - 1)].1)(p);
}
