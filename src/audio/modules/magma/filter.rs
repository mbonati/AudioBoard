//! One Magma filter: a non-linear state-variable design. Each style pushes
//! the input through its own drive curve and saturates the resonant loop
//! with its own shape, so the peak warms, growls and (in most styles)
//! self-oscillates at the top. Slopes stack Butterworth stages, with the
//! resonance on the last one.

use crate::audio::fastmath;

pub const LOW_PASS: usize = 0;
pub const HIGH_PASS: usize = 1;
pub const BAND_PASS: usize = 2;
pub const BELL: usize = 3;
pub const LOW_SHELF: usize = 4;
pub const HIGH_SHELF: usize = 5;
pub const NOTCH: usize = 6;
pub const ALL_PASS: usize = 7;
/// (Button, long name).
pub const SHAPES: [(&str, &str); 8] = [
    ("LP", "Low pass"),
    ("HP", "High pass"),
    ("BP", "Band pass"),
    ("BELL", "Bell"),
    ("LS", "Low shelf"),
    ("HS", "High shelf"),
    ("NOTCH", "Notch"),
    ("AP", "All pass"),
];

/// Slopes in dB/octave (6 isn't offered by band pass and notch: they use 12).
pub const SLOPES: [(&str, &str); 4] =
    [("6", "6 dB/octave"), ("12", "12 dB/octave"), ("24", "24 dB/octave"), ("48", "48 dB/octave")];

pub const STYLES: [(&str, &str); 11] = [
    ("Classic", "The original: warm drive, singing self-oscillation"),
    ("Smooth", "Like the cream in your coffee"),
    ("Raw", "Lots of overdrive, a character of its own"),
    ("Hard", "Moderate distortion with a clean whistle"),
    ("Hollow", "Juicy distortion, low-end self-oscillation"),
    ("Extreme", "Folding distortion for wild ideas"),
    ("Gentle", "Smooth and clean, never oscillates"),
    ("Tube", "Warm, asymmetric overdrive"),
    ("Metal", "Rough, sharp and distorted"),
    ("Easy Going", "A softer Tube"),
    ("Clean", "Linear: no drive, no clipping"),
];

/// Bell gain at the peak's ends, in dB.
pub const BELL_DB: f32 = 24.0;

#[derive(Clone, Copy, PartialEq)]
enum Sat {
    Linear,
    Tanh,
    Soft,
    Hard,
    Tube,
    Fold,
}

#[derive(Clone, Copy)]
struct Style {
    /// The drive curve, and how hard the input is pushed into it.
    pre: Sat,
    drive: f32,
    /// The curve limiting the resonant loop.
    feedback: Sat,
    /// How far past zero damping the top of PEAK goes (0: no oscillation).
    osc: f32,
    /// Also saturate the low-pass state (Hollow's low-end growl).
    low: bool,
}

const fn style(pre: Sat, drive: f32, feedback: Sat, osc: f32, low: bool) -> Style {
    Style { pre, drive, feedback, osc, low }
}

const STYLE_TABLE: [Style; 11] = [
    style(Sat::Tanh, 1.0, Sat::Tanh, 0.06, false),
    style(Sat::Soft, 0.6, Sat::Soft, 0.03, false),
    style(Sat::Tube, 4.0, Sat::Hard, 0.08, false),
    style(Sat::Linear, 1.0, Sat::Hard, 0.06, false),
    style(Sat::Tanh, 1.5, Sat::Tanh, 0.08, true),
    style(Sat::Fold, 2.5, Sat::Fold, 0.12, false),
    style(Sat::Soft, 0.4, Sat::Soft, 0.0, false),
    style(Sat::Tube, 1.5, Sat::Tanh, 0.05, false),
    style(Sat::Hard, 2.0, Sat::Fold, 0.08, false),
    style(Sat::Tube, 0.8, Sat::Soft, 0.03, false),
    style(Sat::Linear, 1.0, Sat::Linear, 0.0, false),
];

/// tanh(0.3): the tube curve's offset, removed so silence stays silent.
const TUBE_BIAS: f32 = 0.291_312_6;

#[inline(always)]
fn sat(kind: Sat, x: f32) -> f32 {
    match kind {
        Sat::Linear => x,
        Sat::Tanh => fastmath::tanh(x),
        Sat::Soft => x / (1.0 + x.abs()),
        Sat::Hard => x.clamp(-1.0, 1.0),
        Sat::Tube => fastmath::tanh(x + 0.3) - TUBE_BIAS,
        // A triangle fold: linear in ±1, folding back beyond.
        Sat::Fold => {
            let t = (x + 1.0) * 0.25;
            1.0 - ((t - t.floor()) * 4.0 - 2.0).abs()
        }
    }
}

/// Butterworth dampings (1/Q) of the stages for 12, 24 and 48 dB/octave.
const K12: [f32; 1] = [std::f32::consts::SQRT_2];
const K24: [f32; 2] = [1.847_759, 0.765_366_9];
const K48: [f32; 4] = [1.961_571, 1.662_939_2, 1.111_140_5, 0.390_180_6];

/// The stage dampings for a pass filter's slope (6 has none: one pole).
fn butterworth(slope: usize) -> &'static [f32] {
    match slope {
        0 => &[],
        1 => &K12,
        2 => &K24,
        _ => &K48,
    }
}

/// Damping of a resonant stage: `base` (its Butterworth value) at the
/// middle of PEAK, less towards the top and, for styles that can, past
/// zero so the filter self-oscillates.
fn resonant(base: f32, peak: f32, osc: f32) -> f32 {
    let r = ((peak + 1.0) * 0.5).clamp(0.0, 1.0);
    let k = base * std::f32::consts::SQRT_2 * (1.0 - r).sqrt() - osc * r.powi(8);
    k.max(if osc > 0.0 { -osc } else { 0.02 })
}

/// Bell and shelf width per slope (steeper is narrower), widened or
/// narrowed further by |PEAK|.
fn bell_q(slope: usize, peak: f32) -> f32 {
    [0.35, 0.7, 1.4, 2.8][slope.min(3)] * (1.0 + peak.abs() * 1.5)
}

fn shelf_q(slope: usize, peak: f32) -> f32 {
    [0.5, std::f32::consts::FRAC_1_SQRT_2, 1.0, 1.6][slope.min(3)] * (1.0 + peak.abs() * 0.5)
}

/// Bell/shelf amplitude A for a peak (gain = A² = ±24 dB).
fn amplitude(peak: f32) -> f32 {
    fastmath::exp2(peak * BELL_DB * 0.083_048_2)
}

#[derive(Clone, Copy, Default)]
struct Stage {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Default)]
struct Coeffs {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Coeffs {
    fn svf(g: f32, k: f32) -> Self {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self { a1, a2, a3: g * a2, k }
    }
}

impl Stage {
    /// One sample: (band, low), with the loop saturated by `style`.
    #[inline(always)]
    fn tick(&mut self, c: &Coeffs, v0: f32, style: &Style) -> (f32, f32) {
        let v3 = v0 - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        if style.feedback != Sat::Linear {
            self.ic1 = sat(style.feedback, self.ic1);
            if style.low {
                self.ic2 = sat(style.feedback, self.ic2 * 0.7) / 0.7;
            }
        }
        (v1, v2)
    }
}

/// What one filter does this chunk, already worked out.
#[derive(Clone, Copy)]
pub struct Setting {
    pub shape: usize,
    pub slope: usize,
    pub style: usize,
    /// Per channel (frequency pan moves them apart), in Hz.
    pub freq: [f32; 2],
    pub peak: f32,
}

pub struct Filter {
    stages: [[Stage; 4]; 2],
    coeffs: [[Coeffs; 4]; 2],
    count: usize,
    /// One pole (6 dB/octave and the first-order all-pass), per channel.
    pole: [f32; 2],
    pole_g: [f32; 2],
    shape: usize,
    style: Style,
    /// Output mix for bell and shelves: y = m0·x + m1·band + m2·low.
    m: [f32; 3],
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            stages: [[Stage::default(); 4]; 2],
            coeffs: [[Coeffs::default(); 4]; 2],
            count: 0,
            pole: [0.0; 2],
            pole_g: [0.0; 2],
            shape: LOW_PASS,
            style: STYLE_TABLE[0],
            m: [1.0, 0.0, 0.0],
        }
    }
}

impl Filter {
    /// Silences the filter (its loop may be ringing on its own).
    pub fn reset(&mut self) {
        self.stages = [[Stage::default(); 4]; 2];
        self.pole = [0.0; 2];
    }

    /// New coefficients for `s`, at `sr` (the processing rate).
    pub fn update(&mut self, s: &Setting, sr: f32) {
        self.shape = s.shape;
        let style = STYLE_TABLE[s.style.min(STYLE_TABLE.len() - 1)];
        self.style = style;
        let slope = s.slope.min(3);
        for ch in 0..2 {
            let g = fastmath::tan_turns(s.freq[ch].clamp(10.0, sr * 0.45) / (2.0 * sr));
            self.pole_g[ch] = g / (1.0 + g);
            let c = &mut self.coeffs[ch];
            self.count = match s.shape {
                LOW_PASS | HIGH_PASS => {
                    let ks = butterworth(slope);
                    for (i, &kb) in ks.iter().enumerate() {
                        let k = if i + 1 == ks.len() { resonant(kb, s.peak, style.osc) } else { kb };
                        c[i] = Coeffs::svf(g, k);
                    }
                    ks.len()
                }
                BAND_PASS | NOTCH | ALL_PASS => {
                    let stages = match slope {
                        0 if s.shape == ALL_PASS => 0,
                        0 | 1 => 1,
                        2 => 2,
                        _ => 4,
                    };
                    let osc = if s.shape == BAND_PASS { style.osc } else { 0.0 };
                    let k = resonant(std::f32::consts::SQRT_2, s.peak, osc);
                    let k = if s.shape == BAND_PASS { k } else { k.max(0.05) };
                    for ci in c.iter_mut().take(stages) {
                        *ci = Coeffs::svf(g, k);
                    }
                    stages
                }
                BELL => {
                    let a = amplitude(s.peak);
                    let k = 1.0 / (bell_q(slope, s.peak) * a);
                    c[0] = Coeffs::svf(g, k);
                    self.m = [1.0, k * (a * a - 1.0), 0.0];
                    1
                }
                LOW_SHELF => {
                    let a = amplitude(s.peak);
                    let k = 1.0 / shelf_q(slope, s.peak);
                    c[0] = Coeffs::svf(g / a.sqrt(), k);
                    self.m = [1.0, k * (a - 1.0), a * a - 1.0];
                    1
                }
                _ => {
                    let a = amplitude(s.peak);
                    let k = 1.0 / shelf_q(slope, s.peak);
                    c[0] = Coeffs::svf(g * a.sqrt(), k);
                    self.m = [a * a, k * (1.0 - a) * a, 1.0 - a * a];
                    1
                }
            };
        }
    }

    /// One sample of channel `ch`; `drive` is the input gain into the
    /// style's drive curve (the level is compensated after it).
    #[inline]
    pub fn process(&mut self, ch: usize, x: f32, drive: f32) -> f32 {
        let style = self.style;
        let x = if style.pre == Sat::Linear {
            x
        } else {
            let d = drive * style.drive;
            sat(style.pre, x * d) / d
        };
        let c = &self.coeffs[ch];
        let st = &mut self.stages[ch];
        let mut y = x;
        match self.shape {
            LOW_PASS | HIGH_PASS if self.count == 0 => {
                let v = (y - self.pole[ch]) * self.pole_g[ch];
                let low = v + self.pole[ch];
                self.pole[ch] = low + v;
                y = if self.shape == LOW_PASS { low } else { y - low };
            }
            LOW_PASS => {
                for i in 0..self.count {
                    y = st[i].tick(&c[i], y, &style).1;
                }
            }
            HIGH_PASS => {
                for i in 0..self.count {
                    let (v1, v2) = st[i].tick(&c[i], y, &style);
                    y = y - c[i].k * v1 - v2;
                }
            }
            BAND_PASS => {
                for i in 0..self.count {
                    y = c[i].k * st[i].tick(&c[i], y, &style).0;
                }
            }
            NOTCH => {
                for i in 0..self.count {
                    y -= c[i].k * st[i].tick(&c[i], y, &style).0;
                }
            }
            ALL_PASS if self.count == 0 => {
                let v = (y - self.pole[ch]) * self.pole_g[ch];
                let low = v + self.pole[ch];
                self.pole[ch] = low + v;
                y = 2.0 * low - y;
            }
            ALL_PASS => {
                for i in 0..self.count {
                    y -= 2.0 * c[i].k * st[i].tick(&c[i], y, &style).0;
                }
            }
            _ => {
                let (v1, v2) = st[0].tick(&c[0], y, &style);
                y = self.m[0] * x + self.m[1] * v1 + self.m[2] * v2;
            }
        }
        if !y.is_finite() || y.abs() > 1e4 {
            self.reset();
            return 0.0;
        }
        y
    }
}

// ---------------------------------------------------------------------------
// Responses, for the display
// ---------------------------------------------------------------------------

/// A complex number (re, im).
pub type Complex = (f32, f32);

pub fn mul(a: Complex, b: Complex) -> Complex {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

fn div(a: Complex, b: Complex) -> Complex {
    let d = (b.0 * b.0 + b.1 * b.1).max(1e-30);
    ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
}

/// (s² + b·s + c) at s = jw.
fn quad(w: f32, a: f32, b: f32, c: f32) -> Complex {
    (c - a * w * w, b * w)
}

/// A filter's (linear) response at `f`: its shape and slope at `fc` with
/// `peak`, warped as the digital filter is.
pub fn response(s: &Setting, fc: f32, f: f32, sr: f32) -> Complex {
    let nyq = sr * 0.4999;
    let w = fastmath::tan_turns(f.min(nyq) / (2.0 * sr)) / fastmath::tan_turns(fc.clamp(10.0, nyq) / (2.0 * sr));
    let style = STYLE_TABLE[s.style.min(STYLE_TABLE.len() - 1)];
    let slope = s.slope.min(3);
    let one = (1.0, 0.0);
    let jw = (0.0, w);
    let pole = |num: Complex| div(num, (1.0, w));
    let svf = |k: f32, num: Complex| div(num, quad(w, 1.0, k, 1.0));
    match s.shape {
        LOW_PASS | HIGH_PASS => {
            let ks = butterworth(slope);
            if ks.is_empty() {
                return pole(if s.shape == LOW_PASS { one } else { jw });
            }
            let mut h = one;
            for (i, &kb) in ks.iter().enumerate() {
                let k = if i + 1 == ks.len() { resonant(kb, s.peak, style.osc).max(0.02) } else { kb };
                let num = if s.shape == LOW_PASS { one } else { (-w * w, 0.0) };
                h = mul(h, svf(k, num));
            }
            h
        }
        BAND_PASS | NOTCH | ALL_PASS => {
            if s.shape == ALL_PASS && slope == 0 {
                return div((1.0, -w), (1.0, w));
            }
            let stages = match slope {
                0 | 1 => 1,
                2 => 2,
                _ => 4,
            };
            let osc = if s.shape == BAND_PASS { style.osc } else { 0.0 };
            let k = resonant(std::f32::consts::SQRT_2, s.peak, osc).max(if s.shape == BAND_PASS { 0.02 } else { 0.05 });
            let num = match s.shape {
                BAND_PASS => (0.0, k * w),
                NOTCH => quad(w, 1.0, 0.0, 1.0),
                _ => quad(w, 1.0, -k, 1.0),
            };
            let one_stage = svf(k, num);
            (0..stages).fold(one, |h, _| mul(h, one_stage))
        }
        BELL => {
            let a = amplitude(s.peak);
            let q = bell_q(slope, s.peak);
            div(quad(w, 1.0, a / q, 1.0), quad(w, 1.0, 1.0 / (a * q), 1.0))
        }
        kind => {
            let a = amplitude(s.peak);
            let sa = a.sqrt() / shelf_q(slope, s.peak);
            let (num, den) = if kind == LOW_SHELF {
                (quad(w, 1.0, sa, a), quad(w, a, sa, 1.0))
            } else {
                (quad(w, a, sa, 1.0), quad(w, 1.0, sa, a))
            };
            let h = div(num, den);
            (h.0 * a, h.1 * a)
        }
    }
}

/// Magnitude in dB.
pub fn db(h: Complex) -> f32 {
    10.0 * (h.0 * h.0 + h.1 * h.1).max(1e-20).log10()
}
