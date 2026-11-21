//! Sibyl's effects chain: five stomp-boxes in any order, each switched in
//! or out.
//!
//! - **Tape**: asymmetric saturation at twice the rate, the head's
//!   bump in the bass, the treble's roll-off, hiss (HISS MUTE takes it away)
//!   and, WORN, wow and flutter. Notes played together interact, as in an
//!   amplifier (unlike the distortion of each voice).
//! - **Echo**, a bucket-brigade delay: CHORUS (short, richly modulated:
//!   chorus to flanger), LONG (10 ms to 2 s), or synced to 1/16 or 1/4 times
//!   TIME (1 to 8); ECHO, PPONG (left and right in turn), SWING (the right
//!   at two thirds: triplets) or GROOVE (three quarters: dotted); its
//!   modulation off to MAX; CLEAN, BRIGHT or DARK repeats that saturate as
//!   they come round. Changing TIME glides the pitch, as on tape.
//! - **Res·EQ**: an EQ (low shelf, a mid bell with its Q, high shelf; ±18 dB)
//!   or three resonant band passes at the same frequencies.
//! - **Plate**: a lush plate (a figure-eight tank of all-passes and delays,
//!   modulated) with pre-delay, DECAY from short to near endless, TONE from
//!   dark to bright.
//! - **Conditioner**: GAIN (above the middle it saturates), TRANSIENT
//!   (softer or punchier attacks) and stereo WIDTH.

use super::mods::{Mods, dest};
use super::voice::sat;
use super::{
    COND_GAIN, COND_ON, COND_PUNCH, COND_WIDTH, ECHO_FLAVOR, ECHO_MIX, ECHO_MOD, ECHO_MODE,
    ECHO_ON, ECHO_RANGE, ECHO_REGEN, ECHO_TIME, PARAM_COUNT, PLATE_DECAY, PLATE_MIX, PLATE_ON,
    PLATE_PRE, PLATE_TONE, RESEQ_HIGH, RESEQ_HIGH_F, RESEQ_LOW, RESEQ_LOW_F, RESEQ_MID,
    RESEQ_MID_F, RESEQ_MODE, RESEQ_ON, RESEQ_Q, TAPE_FLAVOR, TAPE_GAIN, TAPE_HUSH, TAPE_ON,
};
use crate::audio::engine::{BLOCK, ProcessCtx};
use crate::audio::fastmath;
use crate::audio::modules::oversample::Oversampler;
use audioboard_node::dsp::{DelayLine, Rng, mix_gains};

/// The effects, by index (the chain's order holds these).
pub const EFFECTS: [&str; 5] = ["TAPE", "ECHO", "RES·EQ", "PLATE", "CONDITIONER"];
pub const TAPE: usize = 0;
pub const ECHO: usize = 1;
pub const RESEQ: usize = 2;
pub const PLATE: usize = 3;
pub const COND: usize = 4;

/// The switch of each effect.
pub const SWITCHES: [usize; 5] = [TAPE_ON, ECHO_ON, RESEQ_ON, PLATE_ON, COND_ON];

fn db(x: f32) -> f32 {
    fastmath::exp2(x * (std::f32::consts::LOG2_10 / 20.0))
}

// ---------------------------------------------------------------------------
// Biquad (transposed direct form II), RBJ cookbook
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    s: [[f32; 2]; 2],
}

#[derive(Clone, Copy, PartialEq)]
pub enum Shape {
    LowShelf,
    HighShelf,
    Peak,
    BandPass,
    LowPass,
}

impl Biquad {
    pub fn set(&mut self, shape: Shape, hz: f32, q: f32, gain_db: f32, sr: f32) {
        let w = std::f32::consts::TAU * (hz / sr).clamp(1e-5, 0.49);
        let (sn, cs) = w.sin_cos();
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = sn / (2.0 * q.max(0.05));
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::Peak => (
                1.0 + alpha * a,
                -2.0 * cs,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cs,
                1.0 - alpha / a,
            ),
            Shape::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            Shape::LowPass => (
                (1.0 - cs) / 2.0,
                1.0 - cs,
                (1.0 - cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            Shape::LowShelf | Shape::HighShelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                if shape == Shape::LowShelf {
                    (
                        a * ((a + 1.0) - (a - 1.0) * cs + sq),
                        2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                        a * ((a + 1.0) - (a - 1.0) * cs - sq),
                        (a + 1.0) + (a - 1.0) * cs + sq,
                        -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                        (a + 1.0) + (a - 1.0) * cs - sq,
                    )
                } else {
                    (
                        a * ((a + 1.0) + (a - 1.0) * cs + sq),
                        -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                        a * ((a + 1.0) + (a - 1.0) * cs - sq),
                        (a + 1.0) - (a - 1.0) * cs + sq,
                        2.0 * ((a - 1.0) - (a + 1.0) * cs),
                        (a + 1.0) - (a - 1.0) * cs - sq,
                    )
                }
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline(always)]
    pub fn run(&mut self, ch: usize, x: f32) -> f32 {
        let s = &mut self.s[ch];
        let y = self.b0 * x + s[0];
        s[0] = self.b1 * x - self.a1 * y + s[1];
        s[1] = self.b2 * x - self.a2 * y;
        y
    }

    fn tidy(&mut self) {
        for s in self.s.iter_mut().flatten() {
            if s.abs() < 1e-20 || !s.is_finite() {
                *s = 0.0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tape
// ---------------------------------------------------------------------------

/// A flavour of tape: drive, asymmetry, head bump (Hz, dB), treble roll-off
/// (Hz), wow & flutter, hiss (dB).
struct TapeKind {
    drive: f32,
    bias: f32,
    bump: (f32, f32),
    top: f32,
    wow: f32,
    hiss: f32,
}

const TAPES: [TapeKind; 4] = [
    TapeKind {
        drive: 1.0,
        bias: 0.08,
        bump: (85.0, 1.5),
        top: 16_000.0,
        wow: 0.0,
        hiss: -80.0,
    },
    TapeKind {
        drive: 1.7,
        bias: 0.15,
        bump: (70.0, 2.5),
        top: 12_000.0,
        wow: 0.0,
        hiss: -76.0,
    },
    TapeKind {
        drive: 3.2,
        bias: 0.22,
        bump: (75.0, 2.0),
        top: 10_000.0,
        wow: 0.0,
        hiss: -74.0,
    },
    TapeKind {
        drive: 1.9,
        bias: 0.18,
        bump: (60.0, 3.0),
        top: 7_000.0,
        wow: 1.0,
        hiss: -64.0,
    },
];

struct Tape {
    os: Oversampler,
    bump: Biquad,
    top: Biquad,
    wow: [DelayLine; 2],
    wow_phase: f32,
    flutter_phase: f32,
    rng: Rng,
    hiss_lp: [f32; 2],
    set: (usize, f32),
}

impl Tape {
    fn new(sr: f32) -> Self {
        Self {
            os: Oversampler::new(),
            bump: Biquad::default(),
            top: Biquad::default(),
            wow: [
                DelayLine::new((sr * 0.02) as usize),
                DelayLine::new((sr * 0.02) as usize),
            ],
            wow_phase: 0.0,
            flutter_phase: 0.0,
            rng: Rng::new(0x7A9E_0001),
            hiss_lp: [0.0; 2],
            set: (usize::MAX, 0.0),
        }
    }

    fn process(&mut self, s: &[f32; PARAM_COUNT], m: &Mods, sr: f32, l: &mut [f32], r: &mut [f32]) {
        let flavor = (s[TAPE_FLAVOR].round().max(0.0) as usize).min(3);
        let t = &TAPES[flavor];
        if self.set.0 != flavor || self.set.1 != sr {
            self.bump.set(Shape::Peak, t.bump.0, 0.7, t.bump.1, sr);
            self.top.set(Shape::LowPass, t.top, 0.6, 0.0, sr);
            self.set = (flavor, sr);
        }
        let gain = db((s[TAPE_GAIN] + m[dest::TAPE_GAIN] * 36.0).clamp(-12.0, 24.0)) * t.drive;
        let makeup = 1.0 / (1.0 + 0.55 * (gain - 1.0).max(0.0)).sqrt();
        let offset = sat(t.bias);
        let hiss = if s[TAPE_HUSH] > 0.5 { 0.0 } else { db(t.hiss) };
        for i in 0..l.len() {
            let x = [l[i] * gain, r[i] * gain];
            let up = self.os.up(x);
            let mut y2 = [[0.0f32; 2]; 2];
            for (k, frame) in up.iter().enumerate() {
                for ch in 0..2 {
                    // Asymmetric: even harmonics, the warmth of tape.
                    y2[k][ch] = sat(frame[ch] * 0.8 + t.bias) - offset;
                }
            }
            let y = self.os.down(y2);
            for (ch, out) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let mut v = self.top.run(ch, self.bump.run(ch, y[ch] * makeup * 1.25));
                if t.wow > 0.0 {
                    let line = &mut self.wow[ch];
                    line.push(v);
                    let wob = 0.0025
                        + 0.0008 * fastmath::sin_turns(self.wow_phase + ch as f32 * 0.1)
                        + 0.00012 * fastmath::sin_turns(self.flutter_phase);
                    v = line.tap(wob * sr);
                }
                if hiss > 0.0 {
                    let w = self.rng.bipolar();
                    self.hiss_lp[ch] += (w - self.hiss_lp[ch]) * 0.45;
                    v += self.hiss_lp[ch] * hiss * 1.7;
                }
                *out = v;
            }
            self.wow_phase = (self.wow_phase + 0.55 / sr).fract();
            self.flutter_phase = (self.flutter_phase + 6.3 / sr).fract();
        }
        self.bump.tidy();
        self.top.tidy();
    }
}

// ---------------------------------------------------------------------------
// Echo
// ---------------------------------------------------------------------------

struct Echo {
    lines: [DelayLine; 2],
    /// The delay times as heard (they glide), samples.
    time: [f32; 2],
    fb: [f32; 2],
    lp: [f32; 2],
    hp: [f32; 2],
    phase: f32,
    max: f32,
}

impl Echo {
    fn new(sr: f32) -> Self {
        let len = (sr * 2.7) as usize;
        Self {
            lines: [DelayLine::new(len), DelayLine::new(len)],
            time: [0.0; 2],
            fb: [0.0; 2],
            lp: [0.0; 2],
            hp: [0.0; 2],
            phase: 0.0,
            max: len as f32 - 4.0,
        }
    }

    fn clear(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
        self.fb = [0.0; 2];
        self.lp = [0.0; 2];
        self.hp = [0.0; 2];
    }

    #[allow(clippy::needless_range_loop)] // both channels of several arrays at once
    fn process(
        &mut self,
        s: &[f32; PARAM_COUNT],
        m: &Mods,
        sr: f32,
        tempo: f64,
        l: &mut [f32],
        r: &mut [f32],
    ) {
        let range = (s[ECHO_RANGE].round().max(0.0) as usize).min(3);
        let time = (s[ECHO_TIME] + m[dest::ECHO_TIME]).clamp(0.0, 1.0);
        let seconds = match range {
            0 => 0.0006 + time * 0.03,
            1 => 0.01 * 200f32.powf(time),
            2 => (1.0 + 7.0 * time) * 0.25 * (60.0 / tempo.max(1.0)) as f32,
            _ => (1.0 + 7.0 * time) * (60.0 / tempo.max(1.0)) as f32,
        };
        let mode = (s[ECHO_MODE].round().max(0.0) as usize).min(3);
        let right = match mode {
            2 => 2.0 / 3.0,
            3 => 0.75,
            _ => 1.0,
        };
        let target = [
            (seconds * sr).clamp(2.0, self.max),
            (seconds * right * sr).clamp(2.0, self.max),
        ];
        if self.time[0] == 0.0 {
            self.time = target;
        }
        let modulation = (s[ECHO_MOD].round().max(0.0) as usize).min(3);
        // Depth (samples) and rate (Hz): wide and quick for the chorus.
        let (depth, rate) = match (range, modulation) {
            (_, 0) => (0.0, 0.0),
            (0, k) => (
                [0.0, 0.0006, 0.0016, 0.0036][k] * sr,
                [0.0, 0.35, 0.65, 1.1][k],
            ),
            (_, k) => (
                [0.0, 0.0006, 0.0018, 0.005][k] * sr,
                [0.0, 0.25, 0.4, 0.6][k],
            ),
        };
        let flavor = (s[ECHO_FLAVOR].round().max(0.0) as usize).min(2);
        let lp_hz = [8500.0, 14_000.0, 3200.0][flavor];
        let lp_k = 1.0 - (-std::f32::consts::TAU * lp_hz / sr).exp();
        let hp_k = 1.0 - (-std::f32::consts::TAU * 90.0 / sr).exp();
        let regen = (s[ECHO_REGEN] + m[dest::ECHO_REGEN]).clamp(0.0, 1.0) * 1.08;
        let mix = (s[ECHO_MIX] + m[dest::ECHO_MIX]).clamp(0.0, 1.0);
        let (dry, wet) = mix_gains(mix);
        let glide = 1.0 - (-1.0 / (0.12 * sr)).exp();
        let pingpong = mode == 1;
        for i in 0..l.len() {
            for ch in 0..2 {
                self.time[ch] += (target[ch] - self.time[ch]) * glide;
            }
            let lfo = if depth > 0.0 {
                [
                    fastmath::sin_turns(self.phase),
                    fastmath::cos_turns(self.phase),
                ]
            } else {
                [0.0; 2]
            };
            let x = [l[i], r[i]];
            let mut out = [0.0; 2];
            for ch in 0..2 {
                let d = (self.time[ch] + depth * (1.0 + lfo[ch])).clamp(1.0, self.max);
                out[ch] = self.lines[ch].tap(d);
            }
            for ch in 0..2 {
                // The bucket brigade's tone: darker and saturating as it repeats.
                let y = out[ch];
                self.lp[ch] += (y - self.lp[ch]) * lp_k;
                self.hp[ch] += (self.lp[ch] - self.hp[ch]) * hp_k;
                self.fb[ch] = sat((self.lp[ch] - self.hp[ch]) * regen);
            }
            let input = if pingpong {
                // In on the left, the repeats cross over.
                [0.5 * (x[0] + x[1]) + self.fb[1], self.fb[0]]
            } else {
                [x[0] + self.fb[0], x[1] + self.fb[1]]
            };
            self.lines[0].push(input[0]);
            self.lines[1].push(input[1]);
            l[i] = x[0] * dry + out[0] * wet;
            r[i] = x[1] * dry + out[1] * wet;
            self.phase = (self.phase + rate / sr).fract();
        }
        for v in self
            .fb
            .iter_mut()
            .chain(self.lp.iter_mut())
            .chain(self.hp.iter_mut())
        {
            if v.abs() < 1e-20 {
                *v = 0.0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Res·EQ
// ---------------------------------------------------------------------------

struct ResEq {
    bands: [Biquad; 3],
    key: [f32; 8],
}

impl ResEq {
    fn new() -> Self {
        Self {
            bands: [Biquad::default(); 3],
            key: [f32::NAN; 8],
        }
    }

    fn process(&mut self, s: &[f32; PARAM_COUNT], sr: f32, l: &mut [f32], r: &mut [f32]) {
        let res = s[RESEQ_MODE] > 0.5;
        let key = [
            s[RESEQ_MODE],
            s[RESEQ_LOW_F],
            s[RESEQ_LOW],
            s[RESEQ_MID_F],
            s[RESEQ_MID],
            s[RESEQ_HIGH_F],
            s[RESEQ_HIGH],
            s[RESEQ_Q],
        ];
        if key != self.key {
            let q01 = s[RESEQ_Q];
            if res {
                let q = 1.0 * 40f32.powf(q01);
                self.bands[0].set(Shape::BandPass, s[RESEQ_LOW_F], q, 0.0, sr);
                self.bands[1].set(Shape::BandPass, s[RESEQ_MID_F], q, 0.0, sr);
                self.bands[2].set(Shape::BandPass, s[RESEQ_HIGH_F], q, 0.0, sr);
            } else {
                let gain = |v: f32| (v - 0.5) * 36.0;
                self.bands[0].set(
                    Shape::LowShelf,
                    s[RESEQ_LOW_F],
                    0.707,
                    gain(s[RESEQ_LOW]),
                    sr,
                );
                self.bands[1].set(
                    Shape::Peak,
                    s[RESEQ_MID_F],
                    0.3 * 27f32.powf(q01),
                    gain(s[RESEQ_MID]),
                    sr,
                );
                self.bands[2].set(
                    Shape::HighShelf,
                    s[RESEQ_HIGH_F],
                    0.707,
                    gain(s[RESEQ_HIGH]),
                    sr,
                );
            }
            self.key = key;
        }
        if res {
            let vol = [s[RESEQ_LOW], s[RESEQ_MID], s[RESEQ_HIGH]];
            let boost = 1.0 + s[RESEQ_Q] * 2.0;
            for i in 0..l.len() {
                for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                    let input = *x;
                    let mut y = 0.0;
                    for (b, v) in self.bands.iter_mut().zip(vol) {
                        y += b.run(ch, input) * v;
                    }
                    *x = sat(y * 1.6 * boost) / boost;
                }
            }
        } else {
            for i in 0..l.len() {
                for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                    let mut y = *x;
                    for b in self.bands.iter_mut() {
                        y = b.run(ch, y);
                    }
                    *x = y;
                }
            }
        }
        for b in &mut self.bands {
            b.tidy();
        }
    }
}

// ---------------------------------------------------------------------------
// Plate
// ---------------------------------------------------------------------------

/// DECAY (0..1) as a reverb time (to −60 dB), seconds: 0.35 s, about 2 s
/// in the middle, a minute at the top.
pub fn plate_rt60(d: f32) -> f32 {
    0.35 * 180f32.powf(d.clamp(0.0, 1.0).powf(1.8))
}

/// The tank's gain per stage for a reverb time: each branch (0.358 s of the
/// classic plate) passes it twice.
fn plate_feedback(rt60: f32) -> f32 {
    10f32
        .powf(-3.0 * 0.3577 / (2.0 * rt60.max(0.05)))
        .min(0.9995)
}

/// The tank's taps' level, so the wet signal stands about as loud as the dry
/// one at DRY/WET 50 % (on a sustained sound, DECAY in the middle).
const PLATE_WET: f32 = 0.6;

/// An all-pass diffuser on a delay line.
struct AllPass {
    line: DelayLine,
    len: f32,
}

impl AllPass {
    fn new(samples: f32) -> Self {
        Self {
            line: DelayLine::new(samples as usize + 64),
            len: samples,
        }
    }

    #[inline(always)]
    fn run(&mut self, x: f32, g: f32, extra: f32) -> f32 {
        let d = self.line.tap((self.len + extra).max(1.0));
        let v = x + g * d;
        self.line.push(v);
        d - g * v
    }
}

/// The plate: a figure-eight tank (two branches, each a modulated all-pass,
/// a delay, damping, a second all-pass and a delay) after four input
/// diffusers. Lengths are the classic plate's, scaled to the rate.
struct Plate {
    pre: DelayLine,
    bandwidth: f32,
    diff: [AllPass; 4],
    ap1: [AllPass; 2],
    d1: [DelayLine; 2],
    ap2: [AllPass; 2],
    d2: [DelayLine; 2],
    damp: [f32; 2],
    back: [f32; 2],
    scale: f32,
    phase: f32,
    tilt: [f32; 2],
}

impl Plate {
    fn new(sr: f32) -> Self {
        let k = sr / 29_761.0;
        let s = |n: f32| n * k;
        Self {
            pre: DelayLine::new((sr * 0.3) as usize),
            bandwidth: 0.0,
            diff: [
                AllPass::new(s(142.0)),
                AllPass::new(s(107.0)),
                AllPass::new(s(379.0)),
                AllPass::new(s(277.0)),
            ],
            ap1: [AllPass::new(s(672.0)), AllPass::new(s(908.0))],
            d1: [
                DelayLine::new(s(4453.0) as usize + 8),
                DelayLine::new(s(4217.0) as usize + 8),
            ],
            ap2: [AllPass::new(s(1800.0)), AllPass::new(s(2656.0))],
            d2: [
                DelayLine::new(s(3720.0) as usize + 8),
                DelayLine::new(s(3163.0) as usize + 8),
            ],
            damp: [0.0; 2],
            back: [0.0; 2],
            scale: k,
            phase: 0.0,
            tilt: [0.0; 2],
        }
    }

    fn clear(&mut self) {
        self.pre.clear();
        for a in self
            .diff
            .iter_mut()
            .chain(self.ap1.iter_mut())
            .chain(self.ap2.iter_mut())
        {
            a.line.clear();
        }
        for d in self.d1.iter_mut().chain(self.d2.iter_mut()) {
            d.clear();
        }
        self.damp = [0.0; 2];
        self.back = [0.0; 2];
    }

    #[allow(clippy::needless_range_loop)] // both branches of several arrays at once
    fn process(&mut self, s: &[f32; PARAM_COUNT], m: &Mods, sr: f32, l: &mut [f32], r: &mut [f32]) {
        let k = self.scale;
        let pre = (s[PLATE_PRE] * sr).clamp(1.0, sr * 0.29);
        let d = (s[PLATE_DECAY] + m[dest::PLATE_DECAY]).clamp(0.0, 1.0);
        let decay = plate_feedback(plate_rt60(d));
        let tone = s[PLATE_TONE].clamp(-1.0, 1.0);
        // Dark: more damping in the tank, and a tilt on what comes out.
        let damping = (0.08 + 0.55 * (0.5 - 0.5 * tone)).clamp(0.0, 0.9);
        let tilt_k = 1.0 - (-std::f32::consts::TAU * 1200.0 / sr).exp();
        let (lo, hi) = (1.0 - 0.6 * tone, 1.0 + 0.6 * tone);
        let mix = (s[PLATE_MIX] + m[dest::PLATE_MIX]).clamp(0.0, 1.0);
        let (dry, wet) = mix_gains(mix);
        let wet = wet * PLATE_WET;
        let excursion = 12.0 * k;
        let tap = |line: &DelayLine, n: f32| line.tap_at((n * k) as usize);
        for i in 0..l.len() {
            let x = 0.5 * (l[i] + r[i]);
            self.pre.push(x);
            let mut v = self.pre.tap(pre);
            self.bandwidth += (v - self.bandwidth) * 0.7;
            v = self.bandwidth;
            v = self.diff[0].run(v, 0.75, 0.0);
            v = self.diff[1].run(v, 0.75, 0.0);
            v = self.diff[2].run(v, 0.625, 0.0);
            v = self.diff[3].run(v, 0.625, 0.0);
            let wob = [
                fastmath::sin_turns(self.phase) * excursion,
                fastmath::cos_turns(self.phase) * excursion,
            ];
            for b in 0..2 {
                // Each branch takes the input and the other branch's end.
                let mut t = v + self.back[1 - b] * decay;
                t = self.ap1[b].run(t, -0.7, wob[b]);
                self.d1[b].push(t);
                let mut u = self.d1[b].tap_at((if b == 0 { 4453.0 } else { 4217.0 } * k) as usize);
                self.damp[b] += (u - self.damp[b]) * (1.0 - damping);
                u = self.damp[b] * decay;
                u = self.ap2[b].run(u, 0.5, 0.0);
                self.d2[b].push(u);
                self.back[b] =
                    self.d2[b].tap_at((if b == 0 { 3720.0 } else { 3163.0 } * k) as usize);
            }
            let (d1l, d1r, d2l, d2r) = (&self.d1[0], &self.d1[1], &self.d2[0], &self.d2[1]);
            let (a2l, a2r) = (&self.ap2[0].line, &self.ap2[1].line);
            let mut yl = tap(d1r, 266.0) + tap(d1r, 2974.0) - tap(a2r, 1913.0) + tap(d2r, 1996.0)
                - tap(d1l, 1990.0)
                - tap(a2l, 187.0)
                - tap(d2l, 1066.0);
            let mut yr = tap(d1l, 353.0) + tap(d1l, 3627.0) - tap(a2l, 1228.0) + tap(d2l, 2673.0)
                - tap(d1r, 2111.0)
                - tap(a2r, 335.0)
                - tap(d2r, 121.0);
            for (ch, y) in [&mut yl, &mut yr].into_iter().enumerate() {
                self.tilt[ch] += (*y - self.tilt[ch]) * tilt_k;
                *y = self.tilt[ch] * lo + (*y - self.tilt[ch]) * hi;
            }
            l[i] = l[i] * dry + yl * wet;
            r[i] = r[i] * dry + yr * wet;
            self.phase = (self.phase + 0.9 / sr).fract();
        }
        for v in self
            .damp
            .iter_mut()
            .chain(self.back.iter_mut())
            .chain(self.tilt.iter_mut())
        {
            if v.abs() < 1e-20 || !v.is_finite() {
                *v = 0.0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Conditioner
// ---------------------------------------------------------------------------

struct Conditioner {
    fast: f32,
    slow: f32,
    load: f32,
}

impl Conditioner {
    fn process(
        &mut self,
        s: &[f32; PARAM_COUNT],
        m: &Mods,
        sr: f32,
        l: &mut [f32],
        r: &mut [f32],
    ) -> f32 {
        let gain_knob = (s[COND_GAIN] + m[dest::COND_GAIN]).clamp(-1.0, 1.0);
        let gain = db(gain_knob * 18.0);
        let saturate = gain_knob > 0.0;
        let punch = s[COND_PUNCH].clamp(-1.0, 1.0);
        let width = (s[COND_WIDTH] + m[dest::COND_WIDTH] * 2.0).clamp(0.0, 2.0);
        let fa = 1.0 - (-1.0 / (0.0008 * sr)).exp();
        let fr = 1.0 - (-1.0 / (0.04 * sr)).exp();
        let sa = 1.0 - (-1.0 / (0.03 * sr)).exp();
        let mut load = 0.0f32;
        for i in 0..l.len() {
            let (mut a, mut b) = (l[i], r[i]);
            if punch != 0.0 {
                let level = 0.5 * (a.abs() + b.abs());
                self.fast += (level - self.fast) * if level > self.fast { fa } else { fr };
                self.slow += (level - self.slow) * sa;
                let ratio = (self.fast + 1e-6) / (self.slow + 1e-6);
                let g = fastmath::exp2(punch * 1.6 * fastmath::log2(ratio).clamp(-2.0, 2.0))
                    .clamp(0.25, 4.0);
                a *= g;
                b *= g;
            }
            let mid = 0.5 * (a + b);
            let side = 0.5 * (a - b) * width;
            a = (mid + side) * gain;
            b = (mid - side) * gain;
            if saturate {
                load = load.max(a.abs().max(b.abs()));
                a = sat(a * 0.9) / 0.9;
                b = sat(b * 0.9) / 0.9;
            }
            l[i] = a;
            r[i] = b;
        }
        self.load = (load - 0.9).clamp(0.0, 1.0);
        self.load
    }
}

// ---------------------------------------------------------------------------
// The chain
// ---------------------------------------------------------------------------

pub struct Chain {
    sr: f32,
    tape: Tape,
    echo: Echo,
    reseq: ResEq,
    plate: Plate,
    cond: Conditioner,
    was_on: [bool; 5],
}

impl Chain {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            tape: Tape::new(sr),
            echo: Echo::new(sr),
            reseq: ResEq::new(),
            plate: Plate::new(sr),
            cond: Conditioner {
                fast: 0.0,
                slow: 0.0,
                load: 0.0,
            },
            was_on: [false; 5],
        }
    }

    /// Runs the switched-in effects in `order` over the block; returns the
    /// conditioner's load.
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        s: &[f32; PARAM_COUNT],
        order: &[usize; 5],
        m: &Mods,
        ctx: &ProcessCtx,
        l: &mut [f32; BLOCK],
        r: &mut [f32; BLOCK],
        n: usize,
    ) -> f32 {
        let sr = self.sr;
        let tempo = ctx.tempo();
        let mut load = 0.0;
        let (l, r) = (&mut l[..n], &mut r[..n]);
        for &e in order {
            let on = s[SWITCHES[e]] > 0.5;
            // Switched back in: no stale tail.
            if on && !self.was_on[e] {
                match e {
                    ECHO => self.echo.clear(),
                    PLATE => self.plate.clear(),
                    _ => {}
                }
            }
            self.was_on[e] = on;
            if !on {
                continue;
            }
            match e {
                TAPE => self.tape.process(s, m, sr, l, r),
                ECHO => self.echo.process(s, m, sr, tempo, l, r),
                RESEQ => self.reseq.process(s, sr, l, r),
                PLATE => self.plate.process(s, m, sr, l, r),
                COND => load = self.cond.process(s, m, sr, l, r),
                _ => {}
            }
        }
        load
    }
}
