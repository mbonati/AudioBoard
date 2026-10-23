//! Equalizer: a dynamic EQ after Waves F6 Floating-Band Dynamic EQ. Six
//! floating bands (low shelf, bell or high shelf, 16 Hz–21 kHz, Q 0.4–60,
//! ±18 dB) between a high-pass and a low-pass filter (6–24 dB/octave), all
//! in series and with zero latency.
//!
//! Each band also has dynamics: when the level its side chain sees rises
//! above THRESHOLD, its gain moves by up to RANGE (negative compresses,
//! positive expands), with ATTACK and RELEASE. The side chain is the
//! node's own input (INT) or the SC input (EXT), filtered around the band
//! (SPLIT) or full range (WIDE). Every band and filter works on stereo, or
//! on the mid or side signal only. SOLO plays a band's side chain (or what
//! a filter lets through). Global release: each band's own, longer as it
//! works harder, or ARC, shorter on transients. MIX blends in the dry
//! signal (parallel processing), OUT sets the level.
//!
//! The processor also feeds the panel's real-time analyzer: the input, the
//! output or the side chain, as L, R, L+R or sides.

use std::sync::Arc;

use super::{ModuleDef, ModuleParams, Quiet, approach, silence, soft_limit};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{ParamSpec, Unit};

pub const BANDS: usize = 6;

/// A band's parameters, relative to its first one (see [`bp`]).
pub mod band {
    pub const FREQ: usize = 0;
    pub const Q: usize = 1;
    pub const GAIN: usize = 2;
    pub const RANGE: usize = 3;
    pub const THRESHOLD: usize = 4;
    pub const ATTACK: usize = 5;
    pub const RELEASE: usize = 6;
    pub const COUNT: usize = 7;
}

/// Index of band `b`'s parameter `k`.
pub const fn bp(b: usize, k: usize) -> usize {
    b * band::COUNT + k
}

pub const HP_FREQ: usize = BANDS * band::COUNT;
pub const LP_FREQ: usize = HP_FREQ + 1;
pub const MIX: usize = LP_FREQ + 1;
pub const OUT: usize = MIX + 1;
pub const PARAM_COUNT: usize = OUT + 1;

pub const FREQ_RANGE: (f32, f32) = (16.0, 21_000.0);

macro_rules! band_params {
    ($n:literal, $freq:expr, $q:expr) => {
        [
            ParamSpec::log(concat!("B", $n, " FREQ"), FREQ_RANGE.0, FREQ_RANGE.1, $freq, Unit::Hz),
            ParamSpec::log(concat!("B", $n, " Q"), 0.4, 60.0, $q, Unit::Q),
            ParamSpec::lin(concat!("B", $n, " GAIN"), -18.0, 18.0, 0.0, Unit::Db).bipolar(),
            ParamSpec::lin(concat!("B", $n, " RANGE"), -18.0, 18.0, 0.0, Unit::Db).bipolar(),
            ParamSpec::lin(concat!("B", $n, " THRESHOLD"), -60.0, 0.0, -20.0, Unit::Db),
            ParamSpec::log(concat!("B", $n, " ATTACK"), 0.0005, 0.5, 0.015, Unit::Seconds),
            ParamSpec::log(concat!("B", $n, " RELEASE"), 0.005, 5.0, 0.16, Unit::Seconds),
        ]
    };
}

const PARAMS: [ParamSpec; PARAM_COUNT] = {
    const BAND_PARAMS: [[ParamSpec; band::COUNT]; BANDS] = [
        band_params!("1", 60.0, 0.71),
        band_params!("2", 200.0, 1.0),
        band_params!("3", 600.0, 1.0),
        band_params!("4", 2_000.0, 1.0),
        band_params!("5", 5_000.0, 1.0),
        band_params!("6", 12_000.0, 0.71),
    ];
    let mut out = [ParamSpec::lin("MIX", 0.0, 1.0, 1.0, Unit::Percent); PARAM_COUNT];
    let mut b = 0;
    while b < BANDS {
        let mut k = 0;
        while k < band::COUNT {
            out[bp(b, k)] = BAND_PARAMS[b][k];
            k += 1;
        }
        b += 1;
    }
    out[HP_FREQ] = ParamSpec::log("HP FREQ", FREQ_RANGE.0, FREQ_RANGE.1, 30.0, Unit::Hz);
    out[LP_FREQ] = ParamSpec::log("LP FREQ", FREQ_RANGE.0, FREQ_RANGE.1, 18_000.0, Unit::Hz);
    out[OUT] = ParamSpec::lin("OUT", -18.0, 18.0, 0.0, Unit::Db).bipolar();
    out
};

/// Band types.
pub const LOW_SHELF: usize = 0;
pub const BELL: usize = 1;
pub const HIGH_SHELF: usize = 2;
/// What a band or filter works on.
pub const STEREO: usize = 0;
pub const MID: usize = 1;
pub const SIDE: usize = 2;
/// HP/LP slopes, in dB/octave.
pub const SLOPES: [usize; 4] = [6, 12, 18, 24];
/// The HP and LP filters in [`data::SOLO`] and [`data::SELECTED`].
pub const HP: usize = BANDS;
pub const LP: usize = BANDS + 1;

/// Analyzer settings.
pub const RTA_POINTS: [&str; 4] = ["PRE", "POST", "SC", "OFF"];
pub const RTA_SOURCES: [&str; 4] = ["L", "R", "L+R", "SIDES"];
pub const BANDINGS: [(&str, f32); 5] = [("1/24", 24.0), ("1/12", 12.0), ("1/6", 6.0), ("1/3", 3.0), ("1 OCT", 1.0)];
/// Averaging: (label, seconds).
pub const AVERAGING: [(&str, f32); 4] = [("FAST", 0.05), ("MEDIUM", 0.2), ("SLOW", 0.6), ("SLOWEST", 2.0)];

/// What `ModuleParams::data` holds besides the knobs.
pub mod data {
    use super::BANDS;
    /// Per band.
    pub const BAND_ON: usize = 0;
    pub const BAND_MODE: usize = BAND_ON + BANDS;
    pub const BAND_TYPE: usize = BAND_MODE + BANDS;
    /// Side chain from the SC input (else the node's input).
    pub const SC_EXT: usize = BAND_TYPE + BANDS;
    /// Full-range side chain (else filtered around the band).
    pub const SC_WIDE: usize = SC_EXT + BANDS;
    pub const HP_ON: usize = SC_WIDE + BANDS;
    pub const HP_MODE: usize = HP_ON + 1;
    /// Index into [`super::SLOPES`].
    pub const HP_SLOPE: usize = HP_MODE + 1;
    pub const LP_ON: usize = HP_SLOPE + 1;
    pub const LP_MODE: usize = LP_ON + 1;
    pub const LP_SLOPE: usize = LP_MODE + 1;
    /// What is soloed: −1 nothing, 0…5 a band's side chain, 6 HP, 7 LP.
    pub const SOLO: usize = LP_SLOPE + 1;
    /// Global release: automatic (ARC) or each band's own.
    pub const ARC: usize = SOLO + 1;
    /// The panel's selected band (6 HP, 7 LP).
    pub const SELECTED: usize = ARC + 1;
    /// Analyzer: on, where it listens ([`super::RTA_POINTS`]), which
    /// channel ([`super::RTA_SOURCES`]), bars or filled, RMS or peak,
    /// white or pink slope, banding, averaging, average and max lines.
    pub const RTA_ON: usize = SELECTED + 1;
    pub const RTA_POINT: usize = RTA_ON + 1;
    pub const RTA_SOURCE: usize = RTA_POINT + 1;
    pub const RTA_BARS: usize = RTA_SOURCE + 1;
    pub const RTA_RMS: usize = RTA_BARS + 1;
    pub const RTA_WHITE: usize = RTA_RMS + 1;
    pub const RTA_BANDING: usize = RTA_WHITE + 1;
    pub const RTA_AVERAGING: usize = RTA_BANDING + 1;
    pub const RTA_MA: usize = RTA_AVERAGING + 1;
    pub const RTA_MAX: usize = RTA_MA + 1;
    pub const LEN: usize = RTA_MAX + 1;
}

const DATA: [f32; data::LEN] = {
    let mut d = [0.0; data::LEN];
    let mut b = 0;
    while b < BANDS {
        d[data::BAND_ON + b] = 1.0;
        d[data::BAND_TYPE + b] = BELL as f32;
        b += 1;
    }
    d[data::BAND_TYPE] = LOW_SHELF as f32;
    d[data::BAND_TYPE + BANDS - 1] = HIGH_SHELF as f32;
    d[data::HP_SLOPE] = 1.0;
    d[data::LP_SLOPE] = 1.0;
    d[data::SOLO] = -1.0;
    d[data::SELECTED] = 1.0;
    d[data::RTA_ON] = 1.0;
    d[data::RTA_POINT] = 1.0;
    d[data::RTA_SOURCE] = 2.0;
    d[data::RTA_BANDING] = 2.0;
    d[data::RTA_AVERAGING] = 2.0;
    d[data::RTA_MA] = 1.0;
    d
};

/// Samples the analyzer looks at.
pub const RTA_LEN: usize = 8192;

/// Layout of `ModuleParams::viz`.
pub mod viz {
    use super::{BANDS, RTA_LEN};
    /// Each band's live gain change from its dynamics, in dB.
    pub const DYN: usize = 0;
    /// Each band's side-chain level, in dB.
    pub const SC_LEVEL: usize = DYN + BANDS;
    /// Output peaks (taken by the panel).
    pub const OUT_L: usize = SC_LEVEL + BANDS;
    pub const OUT_R: usize = OUT_L + 1;
    /// The sample rate, for the panel's curves.
    pub const SR: usize = OUT_R + 1;
    /// The analyzer's ring of samples, and where the next one goes.
    pub const RTA_WRITE: usize = SR + 1;
    pub const RTA: usize = RTA_WRITE + 1;
    pub const LEN: usize = RTA + RTA_LEN;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Equalizer",
    prefix: "EQ",
    hint: "6-band dynamic EQ · M/S · side chain",
    inputs: &["IN", "SC"],
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
    viz: viz::LEN,
};

pub fn on(p: &ModuleParams, i: usize) -> bool {
    p.data[i].get() > 0.5
}

/// A setting with `count` choices.
pub fn choice(p: &ModuleParams, i: usize, count: usize) -> usize {
    (p.data[i].get().round().max(0.0) as usize).min(count - 1)
}

/// The soloed band or filter, if any.
pub fn solo(p: &ModuleParams) -> Option<usize> {
    let s = p.data[data::SOLO].get().round();
    (0.0..=LP as f32).contains(&s).then_some(s as usize)
}

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

// ---------------------------------------------------------------------------
// Responses, for the panel's curves
// ---------------------------------------------------------------------------

/// dB to the shelf/bell amplitude A (gain = A²).
#[inline]
pub(crate) fn amp(db: f32) -> f32 {
    fastmath::exp2(db * 0.083_048_2)
}

/// Frequency as the bilinear transform warps it, relative to `f0`.
fn warped(f: f32, f0: f32, sr: f32) -> f32 {
    let nyq = sr * 0.4999;
    fastmath::tan_turns(f.min(nyq) / (2.0 * sr)) / fastmath::tan_turns(f0.clamp(1.0, nyq) / (2.0 * sr))
}

/// A band's gain in dB at `f`, set to `db` (its gain plus its dynamics).
pub fn band_db(kind: usize, f0: f32, q: f32, db: f32, f: f32, sr: f32) -> f32 {
    let w = warped(f, f0, sr);
    let a = amp(db);
    let q = shelf_q(kind, q);
    let w2 = w * w;
    let mag2 = |nr: f32, ni: f32, dr: f32, di: f32| (nr * nr + ni * ni) / (dr * dr + di * di).max(1e-20);
    let m2 = match kind {
        LOW_SHELF => {
            let sa = a.sqrt() / q * w;
            a * a * mag2(a - w2, sa, 1.0 - a * w2, sa)
        }
        HIGH_SHELF => {
            let sa = a.sqrt() / q * w;
            a * a * mag2(1.0 - a * w2, sa, a - w2, sa)
        }
        _ => mag2(1.0 - w2, w * a / q, 1.0 - w2, w / (a * q)),
    };
    10.0 * m2.max(1e-20).log10()
}

/// The HP or LP filter's gain in dB at `f`: Butterworth of `slope` dB/oct.
pub fn pass_db(high_pass: bool, fc: f32, slope: usize, f: f32, sr: f32) -> f32 {
    let w = warped(f, fc, sr).max(1e-9);
    let order = (slope / 6) as i32;
    let x = if high_pass { 1.0 / w } else { w };
    -10.0 * (1.0 + x.powi(2 * order)).log10()
}

/// Very high Q makes shelves ring: they stop at 6.
fn shelf_q(kind: usize, q: f32) -> f32 {
    if kind == BELL { q } else { q.min(6.0) }
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// State of a state-variable filter (Simper's trapezoidal SVF).
#[derive(Clone, Copy, Default)]
pub(crate) struct Svf {
    ic1: f32,
    ic2: f32,
}

/// SVF coefficients and output mix: y = m0·x + m1·band + m2·low.
#[derive(Clone, Copy, Default)]
pub(crate) struct Coeffs {
    a1: f32,
    a2: f32,
    a3: f32,
    m0: f32,
    m1: f32,
    m2: f32,
}

impl Coeffs {
    pub(crate) fn svf(g: f32, k: f32, m: (f32, f32, f32)) -> Self {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self { a1, a2, a3: g * a2, m0: m.0, m1: m.1, m2: m.2 }
    }

    /// A band of type `kind`: `g` is tan(π·f/sr), `a` the amplitude.
    pub(crate) fn band(kind: usize, g: f32, q: f32, a: f32) -> Self {
        let q = shelf_q(kind, q);
        match kind {
            LOW_SHELF => {
                let k = 1.0 / q;
                Self::svf(g / a.sqrt(), k, (1.0, k * (a - 1.0), a * a - 1.0))
            }
            HIGH_SHELF => {
                let k = 1.0 / q;
                Self::svf(g * a.sqrt(), k, (a * a, k * (1.0 - a) * a, 1.0 - a * a))
            }
            _ => {
                let k = 1.0 / (q * a);
                Self::svf(g, k, (1.0, k * (a * a - 1.0), 0.0))
            }
        }
    }
}

impl Svf {
    /// Advances by one sample: (band, low).
    #[inline]
    fn tick(&mut self, c: &Coeffs, v0: f32) -> (f32, f32) {
        let v3 = v0 - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v1, v2)
    }

    #[inline]
    pub(crate) fn apply(&mut self, c: &Coeffs, x: f32) -> f32 {
        let (v1, v2) = self.tick(c, x);
        c.m0 * x + c.m1 * v1 + c.m2 * v2
    }
}

/// Into mid/side and back.
#[inline]
fn to_ms(l: f32, r: f32) -> (f32, f32) {
    ((l + r) * 0.5, (l - r) * 0.5)
}

/// Runs `f` on what `mode` covers: both channels, the mid or the side.
#[inline]
fn on_mode(mode: usize, x: (f32, f32), mut f: impl FnMut(usize, f32) -> f32) -> (f32, f32) {
    match mode {
        MID | SIDE => {
            let (m, s) = to_ms(x.0, x.1);
            let (m, s) = if mode == MID { (f(0, m), s) } else { (m, f(0, s)) };
            (m + s, m - s)
        }
        _ => (f(0, x.0), f(1, x.1)),
    }
}

/// The HP or LP filter: Butterworth, 6 to 24 dB/octave, from a one-pole and
/// up to two SVFs.
struct Pass {
    high: bool,
    freq: f32,
    g: f32,
    tuned: f32,
    slope: usize,
    /// The stages' coefficients, for `g` and `slope` as they were made.
    stages: [Coeffs; 2],
    made: (f32, usize),
    pole: [f32; 2],
    svf: [[Svf; 2]; 2],
}

/// Q of the SVF stages for 12, 18 and 24 dB/octave.
const STAGES: [&[f32]; 4] = [&[], &[std::f32::consts::FRAC_1_SQRT_2], &[1.0], &[0.541_196_1, 1.306_563]];

impl Pass {
    fn new(high: bool, freq: f32) -> Self {
        Self {
            high,
            freq,
            g: 0.0,
            tuned: -1.0,
            slope: 1,
            stages: [Coeffs::default(); 2],
            made: (-1.0, 0),
            pole: [0.0; 2],
            svf: [[Svf::default(); 2]; 2],
        }
    }

    /// Remakes the stages' coefficients if the frequency or slope moved.
    fn prepare(&mut self) {
        if self.made != (self.g, self.slope) {
            self.made = (self.g, self.slope);
            for (c, &q) in self.stages.iter_mut().zip(STAGES[self.slope]) {
                *c = Coeffs::svf(self.g, 1.0 / q, (0.0, 0.0, 0.0));
            }
        }
    }

    #[inline]
    fn process(&mut self, ch: usize, x: f32) -> f32 {
        let g = self.g;
        let mut y = x;
        // One pole for the odd slopes.
        if self.slope.is_multiple_of(2) {
            let v = (y - self.pole[ch]) * (g / (1.0 + g));
            let low = v + self.pole[ch];
            self.pole[ch] = low + v;
            y = if self.high { y - low } else { low };
        }
        for (k, &q) in STAGES[self.slope].iter().enumerate() {
            let (v1, v2) = self.svf[k][ch].tick(&self.stages[k], y);
            y = if self.high { y - v1 / q - v2 } else { v2 };
        }
        y
    }
}

struct Band {
    freq: f32,
    q: f32,
    gain: f32,
    /// tan(π·f/sr), and the frequency it is for.
    g: f32,
    tuned: f32,
    svf: [Svf; 2],
    /// Side-chain filter (SPLIT), per channel.
    det_state: [Svf; 2],
    /// Peak follower of the side chain (linear), its level in dB, a slow
    /// average of that (for ARC), and the gain change it causes.
    follower: f32,
    level: f32,
    average: f32,
    dynamic: f32,
    /// The EQ and side-chain filters' coefficients, remade every
    /// [`CONTROL`] samples; `flat` while the band's gain is 0 dB.
    eq: Coeffs,
    det: Coeffs,
    det_k: f32,
    flat: bool,
    /// The band was doing something last block (else its states rest).
    active: bool,
}

/// Samples between updates of the bands' dynamics and coefficients.
const CONTROL: usize = 4;

impl Band {
    fn new(p: &ModuleParams, b: usize) -> Self {
        Self {
            freq: p.get(bp(b, band::FREQ)),
            q: p.get(bp(b, band::Q)),
            gain: p.get(bp(b, band::GAIN)),
            g: 0.0,
            tuned: -1.0,
            svf: [Svf::default(); 2],
            det_state: [Svf::default(); 2],
            follower: 0.0,
            level: -120.0,
            average: -120.0,
            dynamic: 0.0,
            eq: Coeffs::default(),
            det: Coeffs::default(),
            det_k: 1.0,
            flat: true,
            active: false,
        }
    }
}

/// The dynamics' gain change in dB for a side-chain level `over` dB above
/// the threshold: as much as the overshoot (a soft 6 dB knee), up to RANGE.
#[inline]
pub fn dynamic_gain(over: f32, range: f32) -> f32 {
    const KNEE: f32 = 6.0;
    let x = if over <= -KNEE * 0.5 {
        0.0
    } else if over >= KNEE * 0.5 {
        over
    } else {
        let t = over + KNEE * 0.5;
        t * t / (2.0 * KNEE)
    };
    x.min(range.abs()) * range.signum()
}

pub struct Equalizer {
    params: Arc<ModuleParams>,
    sr: f32,
    bands: [Band; BANDS],
    hp: Pass,
    lp: Pass,
    mix: f32,
    out: f32,
    quiet: Quiet,
    rta_write: usize,
}

impl Equalizer {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        params.viz[viz::SR].set(sample_rate);
        Self {
            bands: std::array::from_fn(|b| Band::new(&params, b)),
            hp: Pass::new(true, params.get(HP_FREQ)),
            lp: Pass::new(false, params.get(LP_FREQ)),
            mix: params.get(MIX),
            out: params.get(OUT),
            quiet: Quiet::new((sample_rate * 0.5) as usize),
            rta_write: 0,
            params,
            sr: sample_rate,
        }
    }
}

/// tan(π·f/sr), recomputed only when `f` moves.
#[inline]
fn tune(g: &mut f32, tuned: &mut f32, f: f32, sr: f32) {
    if f != *tuned {
        *tuned = f;
        *g = fastmath::tan_turns(f.clamp(10.0, sr * 0.45) / (2.0 * sr));
    }
}

/// Seconds to a one-pole coefficient.
#[inline]
fn coeff(seconds: f32, sr: f32) -> f32 {
    1.0 - (-1.0 / (seconds * sr).max(1.0)).exp()
}

impl Processor for Equalizer {
    #[allow(clippy::needless_range_loop)] // both channels, sample by sample
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let sr = self.sr;
        let input = inputs.get(0);
        let side = inputs.get(1);
        let silent_in = Quiet::silent(input, n);
        let rta = on(&p, data::RTA_ON) && choice(&p, data::RTA_POINT, 4) != 3;
        if silent_in && self.quiet.resting() && !(rta && side.is_some()) {
            // Gone quiet: nothing to compute until input returns.
            silence(outputs, n);
            for b in 0..BANDS {
                p.viz[viz::DYN + b].set(0.0);
                p.viz[viz::SC_LEVEL + b].set(-120.0);
            }
            return;
        }

        // This block's settings.
        struct Set {
            on: bool,
            kind: usize,
            mode: usize,
            ext: bool,
            wide: bool,
            freq: f32,
            q: f32,
            gain: f32,
            range: f32,
            threshold: f32,
            attack: f32,
            release: f32,
        }
        let arc = on(&p, data::ARC);
        let sets: [Set; BANDS] = std::array::from_fn(|b| Set {
            on: on(&p, data::BAND_ON + b),
            kind: choice(&p, data::BAND_TYPE + b, 3),
            mode: choice(&p, data::BAND_MODE + b, 3),
            ext: on(&p, data::SC_EXT + b),
            wide: on(&p, data::SC_WIDE + b),
            freq: p.get(bp(b, band::FREQ)),
            q: p.get(bp(b, band::Q)),
            gain: p.get(bp(b, band::GAIN)),
            range: p.get(bp(b, band::RANGE)),
            threshold: p.get(bp(b, band::THRESHOLD)),
            attack: coeff(p.get(bp(b, band::ATTACK)), sr),
            release: {
                // Longer as the band works harder; ARC shortens it on
                // transients (never beyond the band's own).
                let band = &self.bands[b];
                let mut seconds = p.get(bp(b, band::RELEASE)) * (1.0 + band.dynamic.abs() / 12.0);
                if arc {
                    let crest = (band.level - band.average).max(0.0);
                    seconds *= 0.15 + 0.85 * (1.0 - crest / 12.0).clamp(0.0, 1.0);
                }
                coeff(seconds, sr)
            },
        });
        let hp_on = on(&p, data::HP_ON);
        let lp_on = on(&p, data::LP_ON);
        let (hp_mode, lp_mode) = (choice(&p, data::HP_MODE, 3), choice(&p, data::LP_MODE, 3));
        self.hp.slope = choice(&p, data::HP_SLOPE, 4);
        self.lp.slope = choice(&p, data::LP_SLOPE, 4);
        let (hp_t, lp_t) = (p.get(HP_FREQ), p.get(LP_FREQ));
        let (mix_t, out_t) = (p.get(MIX), p.get(OUT));
        let solo = solo(&p);
        let average_k = coeff(0.4, sr / CONTROL as f32);
        let control_smooth = 1.0 - (1.0 - ctx.smooth).powi(CONTROL as i32);
        let rta_point = choice(&p, data::RTA_POINT, 4);
        let rta_source = choice(&p, data::RTA_SOURCE, 4);

        let mut peak = [0.0f32; 2];
        let mut inner = 0.0f32;
        for i in 0..n {
            let (dry_l, dry_r) = input.map_or((0.0, 0.0), |b| (b[0][i], b[1][i]));
            let (sc_l, sc_r) = side.map_or((0.0, 0.0), |b| (b[0][i], b[1][i]));
            let mut x = (dry_l, dry_r);
            let mut heard = None;

            if hp_on || solo == Some(HP) {
                approach(&mut self.hp.freq, hp_t, ctx.smooth);
                let hp = &mut self.hp;
                tune(&mut hp.g, &mut hp.tuned, hp.freq, sr);
                hp.prepare();
                x = on_mode(hp_mode, x, |ch, v| hp.process(ch, v));
                if solo == Some(HP) {
                    heard = Some(x);
                }
            }

            for (b, (band, s)) in self.bands.iter_mut().zip(&sets).enumerate() {
                let soloed = solo == Some(b);
                let working = (s.on && (s.gain != 0.0 || s.range != 0.0)) || band.gain.abs() > 1e-3 || band.dynamic.abs() > 1e-3;
                if !working && !soloed {
                    if band.active {
                        // Rests: start clean when it comes back.
                        *band = Band::new(&p, b);
                    }
                    continue;
                }
                let update = i % CONTROL == 0 || !band.active;
                band.active = true;
                approach(&mut band.freq, s.freq, ctx.smooth);
                approach(&mut band.q, s.q, ctx.smooth);
                approach(&mut band.gain, if s.on { s.gain } else { 0.0 }, ctx.smooth);
                if update {
                    tune(&mut band.g, &mut band.tuned, band.freq, sr);
                    band.det_k = 1.0 / shelf_q(s.kind, band.q);
                    band.det = Coeffs::svf(band.g, band.det_k, (0.0, 0.0, 0.0));
                }

                // Side chain: the band's own input or the SC input, around
                // the band (SPLIT) or full range (WIDE), on what it covers.
                let (l, r) = if s.ext { (sc_l, sc_r) } else { (dry_l, dry_r) };
                let (det, k, states) = (&band.det, band.det_k, &mut band.det_state);
                let mut filtered = |ch: usize, v: f32| {
                    if s.wide {
                        return v;
                    }
                    let (bp, lp) = states[ch].tick(det, v);
                    match s.kind {
                        LOW_SHELF => lp,
                        HIGH_SHELF => v - k * bp - lp,
                        _ => k * bp,
                    }
                };
                let (dl, dr) = match s.mode {
                    MID => {
                        let m = filtered(0, (l + r) * 0.5);
                        (m, m)
                    }
                    SIDE => {
                        let v = filtered(0, (l - r) * 0.5);
                        (v, v)
                    }
                    _ => (filtered(0, l), filtered(1, r)),
                };
                if soloed {
                    heard = Some((dl, dr));
                }
                let a = dl.abs().max(dr.abs());
                let k = if a > band.follower { s.attack } else { s.release };
                band.follower += (a - band.follower) * k;

                if update {
                    band.level = 6.020_6 * fastmath::log2(band.follower + 1e-9);
                    band.average += (band.level - band.average) * average_k;
                    if s.on && s.range != 0.0 {
                        band.dynamic = dynamic_gain(band.level - s.threshold, s.range);
                    } else {
                        approach(&mut band.dynamic, 0.0, control_smooth);
                    }
                    let db = band.gain + band.dynamic;
                    band.flat = db.abs() < 1e-4;
                    if !band.flat {
                        band.eq = Coeffs::band(s.kind, band.g, band.q, amp(db));
                    }
                }
                if band.flat {
                    continue;
                }
                let (c, svf) = (&band.eq, &mut band.svf);
                x = on_mode(s.mode, x, |ch, v| svf[ch].apply(c, v));
            }

            if lp_on || solo == Some(LP) {
                approach(&mut self.lp.freq, lp_t, ctx.smooth);
                let lp = &mut self.lp;
                tune(&mut lp.g, &mut lp.tuned, lp.freq, sr);
                lp.prepare();
                x = on_mode(lp_mode, x, |ch, v| lp.process(ch, v));
                if solo == Some(LP) {
                    heard = Some(x);
                }
            }

            approach(&mut self.mix, mix_t, ctx.smooth);
            approach(&mut self.out, out_t, ctx.smooth);
            let gain = amp(self.out * 2.0);
            let (wl, wr) = heard.unwrap_or((dry_l + (x.0 - dry_l) * self.mix, dry_r + (x.1 - dry_r) * self.mix));
            let yl = soft_limit(wl * gain);
            let yr = soft_limit(wr * gain);
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak[0] = peak[0].max(yl.abs());
            peak[1] = peak[1].max(yr.abs());
            inner = inner.max(yl.abs()).max(yr.abs());

            if rta {
                let (l, r) = match rta_point {
                    0 => (dry_l, dry_r),
                    1 => (yl, yr),
                    _ => (sc_l, sc_r),
                };
                let v = match rta_source {
                    0 => l,
                    1 => r,
                    2 => (l + r) * 0.5,
                    _ => (l - r) * 0.5,
                };
                p.viz[viz::RTA + self.rta_write].set(v);
                self.rta_write = (self.rta_write + 1) % RTA_LEN;
            }
        }
        if rta {
            p.viz[viz::RTA_WRITE].set(self.rta_write as f32);
        }
        for (b, band) in self.bands.iter().enumerate() {
            p.viz[viz::DYN + b].set(band.dynamic);
            p.viz[viz::SC_LEVEL + b].set(if band.active { band.level } else { -120.0 });
        }
        p.viz[viz::OUT_L].fetch_max(peak[0]);
        p.viz[viz::OUT_R].fetch_max(peak[1]);
        self.quiet.update(silent_in, inner, n);
        p.meter.fetch_max(peak[0].max(peak[1]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_match_their_settings() {
        let sr = 48_000.0;
        // A bell peaks at its gain, at its frequency.
        assert!((band_db(BELL, 1000.0, 1.0, 9.0, 1000.0, sr) - 9.0).abs() < 0.01);
        assert!(band_db(BELL, 1000.0, 1.0, 9.0, 50.0, sr).abs() < 0.2);
        // Shelves reach their gain far on their side, and 0 on the other.
        assert!((band_db(LOW_SHELF, 200.0, 0.71, -6.0, 20.0, sr) + 6.0).abs() < 0.1);
        assert!(band_db(LOW_SHELF, 200.0, 0.71, -6.0, 10_000.0, sr).abs() < 0.1);
        assert!((band_db(HIGH_SHELF, 5000.0, 0.71, 4.0, 20_000.0, sr) - 4.0).abs() < 0.2);
        // Butterworth: −3 dB at the corner, the slope beyond it.
        assert!((pass_db(true, 100.0, 12, 100.0, sr) + 3.01).abs() < 0.05);
        assert!((pass_db(true, 100.0, 24, 50.0, sr) + 24.0).abs() < 0.5);
        assert!((pass_db(false, 1000.0, 6, 4000.0, sr) + 12.0).abs() < 0.6);
    }

    #[test]
    fn dynamics_follow_range_and_threshold() {
        assert_eq!(dynamic_gain(-10.0, -6.0), 0.0);
        assert!((dynamic_gain(4.0, -6.0) + 4.0).abs() < 1e-6);
        assert_eq!(dynamic_gain(20.0, -6.0), -6.0);
        assert_eq!(dynamic_gain(20.0, 3.0), 3.0);
        assert!(dynamic_gain(0.0, -6.0) < 0.0 && dynamic_gain(0.0, -6.0) > -1.0, "soft knee");
    }
}
