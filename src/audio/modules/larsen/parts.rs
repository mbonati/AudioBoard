//! The parts of a feedback loop: the string, the drive, the filters, the
//! reverb, and the complex arithmetic that tunes them to a note.

use std::f32::consts::{PI, TAU};

use crate::audio::fastmath;
use crate::audio::modules::DelayLine;

/// Bounded saturation: about linear near 0, exactly ±1 from ±3.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x <= -3.0 {
        -1.0
    } else if x >= 3.0 {
        1.0
    } else {
        x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
    }
}

/// The loop's drive: a steep, fixed saturation (+14 dB into two soft
/// clips). It is what bounds the loop: whatever the feedback, the drone
/// saturates here rather than running away.
#[inline]
pub fn drive(x: f32) -> f32 {
    soft_clip(soft_clip(x * 5.2))
}

/// A complex number, for filter responses.
#[derive(Clone, Copy, Debug)]
pub struct C {
    pub re: f32,
    pub im: f32,
}

impl C {
    /// e^(−jw): one sample's delay at `w` radians a sample.
    pub fn delay(w: f32) -> C {
        C { re: w.cos(), im: -w.sin() }
    }

    pub fn mul(self, o: C) -> C {
        C { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }

    pub fn div(self, o: C) -> C {
        let d = (o.re * o.re + o.im * o.im).max(1e-30);
        C { re: (self.re * o.re + self.im * o.im) / d, im: (self.im * o.re - self.re * o.im) / d }
    }

    pub fn arg(self) -> f32 {
        self.im.atan2(self.re)
    }

    pub fn abs(self) -> f32 {
        self.re.hypot(self.im)
    }
}

/// A one-pole low-pass: `y += (x − y)(1 − c)`.
fn one_pole(hz: f32, sr: f32) -> f32 {
    let b = 2.0 - (TAU * hz / sr).cos();
    b - (b * b - 1.0).sqrt()
}

/// The string's DC blocker, at about 76 Hz.
const DC_HZ: f32 = 76.0;
/// The string's own damping, a low-pass in its loop.
const DAMP_HZ: f32 = 8000.0;
/// What the string keeps of each pass.
const STRING_GAIN: f32 = 0.8;

/// A plucked string (Karplus-Strong): a delay line one period long, fed
/// back through a DC blocker and a low-pass. It rings on its period and
/// every harmonic, and keeps whatever excites it ringing a little while.
pub struct Strand {
    line: Box<[f32]>,
    mask: usize,
    pos: usize,
    /// The period, in samples.
    period: f32,
    dc_r: f32,
    dc_x: f32,
    dc_y: f32,
    lp_c: f32,
    lp: f32,
}

impl Strand {
    /// Long enough for `lowest` Hz.
    pub fn new(sr: f32, lowest: f32) -> Self {
        let len = ((sr / lowest) as usize + 8).next_power_of_two();
        Self {
            line: vec![0.0; len].into_boxed_slice(),
            mask: len - 1,
            pos: 0,
            period: 100.0,
            dc_r: 1.0 - TAU * DC_HZ / sr,
            dc_x: 0.0,
            dc_y: 0.0,
            lp_c: one_pole(DAMP_HZ, sr),
            lp: 0.0,
        }
    }

    pub fn clear(&mut self) {
        self.line.fill(0.0);
        (self.dc_x, self.dc_y, self.lp) = (0.0, 0.0, 0.0);
    }

    /// The period in samples (at least 4: a quarter of the sample rate).
    pub fn set_period(&mut self, samples: f32) {
        self.period = samples.clamp(4.0, (self.line.len() - 4) as f32);
    }

    /// The filters in the string's loop, at `w` radians a sample.
    pub fn response(sr: f32, w: f32) -> C {
        let z1 = C::delay(w);
        let r = 1.0 - TAU * DC_HZ / sr;
        let c = one_pole(DAMP_HZ, sr);
        let dc = C { re: 1.0 - z1.re, im: -z1.im }.div(C { re: 1.0 - r * z1.re, im: -r * z1.im });
        let lp = C { re: 1.0 - c, im: 0.0 }.div(C { re: 1.0 - c * z1.re, im: -c * z1.im });
        dc.mul(lp)
    }

    #[inline]
    fn age(&self, k: usize) -> f32 {
        // SAFETY: the length is a power of two and `mask` is length − 1.
        unsafe { *self.line.get_unchecked(self.pos.wrapping_sub(k) & self.mask) }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        // One period back, between samples by a cubic (Hermite) curve.
        let k = self.period - 1.0;
        let i = k as u32 as usize;
        let f = k - i as f32;
        let (ym1, y0, y1, y2) = (self.age(i.wrapping_sub(1)), self.age(i), self.age(i + 1), self.age(i + 2));
        let c = (y1 - ym1) * 0.5;
        let v = y0 - y1;
        let w = c + v;
        let a = w + v + (y2 - y0) * 0.5;
        let back = ((a * f - (w + a)) * f + c) * f + y0;

        let s = (back + x).clamp(-20.0, 20.0);
        let y = s - self.dc_x + self.dc_r * self.dc_y;
        (self.dc_x, self.dc_y) = (s, y);
        let s = y * STRING_GAIN;
        self.lp = s + (self.lp - s) * self.lp_c;
        self.pos = (self.pos + 1) & self.mask;
        // SAFETY: `pos` is masked as in `age`.
        unsafe { *self.line.get_unchecked_mut(self.pos) = self.lp };
        self.lp
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Low,
    High,
    Band,
}

/// A second-order filter (bilinear, transposed direct form II) for two
/// channels.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    s1: [f32; 2],
    s2: [f32; 2],
}

impl Biquad {
    pub fn set(&mut self, kind: Kind, hz: f32, q: f32, sr: f32) {
        let k = (PI * hz.clamp(1.0, sr * 0.49) / sr).tan();
        let k2 = k * k;
        let norm = 1.0 / (1.0 + k / q + k2);
        self.b = match kind {
            Kind::Low => [k2 * norm, 2.0 * k2 * norm, k2 * norm],
            Kind::High => [norm, -2.0 * norm, norm],
            Kind::Band => [k / q * norm, 0.0, -k / q * norm],
        };
        self.a = [2.0 * (k2 - 1.0) * norm, (1.0 - k / q + k2) * norm];
    }

    pub fn clear(&mut self) {
        (self.s1, self.s2) = ([0.0; 2], [0.0; 2]);
    }

    /// The response at `w` radians a sample.
    pub fn response(&self, w: f32) -> C {
        let z1 = C::delay(w);
        let z2 = z1.mul(z1);
        let num = C { re: self.b[0] + self.b[1] * z1.re + self.b[2] * z2.re, im: self.b[1] * z1.im + self.b[2] * z2.im };
        let den = C { re: 1.0 + self.a[0] * z1.re + self.a[1] * z2.re, im: self.a[0] * z1.im + self.a[1] * z2.im };
        num.div(den)
    }

    #[inline]
    pub fn process(&mut self, x: f32, ch: usize) -> f32 {
        let y = self.b[0] * x + self.s1[ch];
        self.s1[ch] = self.s2[ch] + self.b[1] * x - self.a[0] * y;
        self.s2[ch] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Two band-passes in a row (fourth order), for two channels: keeps a loop,
/// or what it hears from the others, on one note.
#[derive(Clone, Copy, Default)]
pub struct Band4 {
    stages: [Biquad; 2],
}

impl Band4 {
    pub const Q: f32 = 1.5;

    pub fn set(&mut self, hz: f32, sr: f32) {
        for s in &mut self.stages {
            s.set(Kind::Band, hz, Self::Q, sr);
        }
    }

    pub fn clear(&mut self) {
        for s in &mut self.stages {
            s.clear();
        }
    }

    #[inline]
    pub fn process(&mut self, x: f32, ch: usize) -> f32 {
        let y = self.stages[0].process(x, ch);
        self.stages[1].process(y, ch)
    }
}

/// The reverb inside the loop: eight delay lines, each read through a
/// slowly swaying tap and a damping low-pass, mixed back into all of them
/// (a lossless Householder junction), so that DECAY alone sets how long it
/// rings, up to forever.
pub struct Verb {
    lines: [DelayLine; 8],
    /// Each line's length and how far its tap sways, in samples.
    base: [f32; 8],
    depth: f32,
    /// The taps now and at the end of the block.
    from: [f32; 8],
    to: [f32; 8],
    phase: [f32; 8],
    rate: [f32; 8],
    damp: [f32; 8],
    damp_c: f32,
    feedback: f32,
    step: f32,
}

/// The lines' lengths (seconds): spread, with no common factors to speak
/// of.
const VERB_TIMES: [f32; 8] = [0.0473, 0.0539, 0.0611, 0.0682, 0.0457, 0.0517, 0.0751, 0.0829];
/// Their sway (turns a second).
const VERB_RATES: [f32; 8] = [0.31, 0.23, 0.37, 0.19, 0.29, 0.41, 0.17, 0.27];

impl Verb {
    pub fn new(sr: f32) -> Self {
        let depth = 0.0012 * sr;
        let base = VERB_TIMES.map(|t| t * sr);
        Self {
            lines: std::array::from_fn(|i| DelayLine::new((base[i] + depth) as usize + 8)),
            base,
            depth,
            from: base,
            to: base,
            phase: std::array::from_fn(|i| i as f32 * 0.137),
            rate: VERB_RATES.map(|r| r / sr),
            damp: [0.0; 8],
            damp_c: one_pole(12_000.0, sr),
            feedback: 0.85,
            step: 0.0,
        }
    }

    pub fn clear(&mut self) {
        for line in &mut self.lines {
            for _ in 0..line.len() {
                line.push(0.0);
            }
        }
        self.damp = [0.0; 8];
    }

    /// For the next block of `n` samples, with DECAY `feedback`.
    pub fn begin(&mut self, feedback: f32, n: usize) {
        self.feedback = feedback;
        self.from = self.to;
        for i in 0..8 {
            self.phase[i] = (self.phase[i] + self.rate[i] * n as f32).fract();
            self.to[i] = self.base[i] + self.depth * fastmath::sin_turns(self.phase[i]);
        }
        self.step = 1.0 / n.max(1) as f32;
    }

    /// Sample `i` of the block.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32, i: usize) -> (f32, f32) {
        let t = i as f32 * self.step;
        let mut y = [0.0f32; 8];
        for (k, y) in y.iter_mut().enumerate() {
            let d = self.from[k] + (self.to[k] - self.from[k]) * t;
            let s = self.lines[k].tap(d - 1.0) * self.feedback;
            self.damp[k] = s + (self.damp[k] - s) * self.damp_c;
            *y = self.damp[k];
        }
        let junction = 0.25 * y.iter().sum::<f32>();
        for (k, line) in self.lines.iter_mut().enumerate() {
            let input = if k % 2 == 0 { l } else { r };
            line.push(junction - y[k] + input);
        }
        let out_l = 0.35 * (y[0] + y[2] + y[4] + y[6]);
        let out_r = 0.35 * (y[1] + y[3] + y[5] + y[7]);
        (out_l, out_r)
    }
}
