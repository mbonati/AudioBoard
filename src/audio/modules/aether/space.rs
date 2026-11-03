//! The warm, wide end of Aether: soft saturation (2× oversampled when it
//! must be), a three-voice stereo chorus and a diffuse space — all-pass
//! diffusion into an eight-line network, as in the Reverb but longer and
//! darker — lifted by AIR: a high shelf on the tail and a breath of noise
//! through narrow resonances tuned to the chord's upper harmonics, so even
//! the air is in tune.

use std::f32::consts::TAU;

use super::super::oversample::Oversampler;
use super::super::reverb::{DIFFUSERS, LINE_MS, LINES, MOD_HZ, allpass, hadamard};
use super::super::{DelayLine, Rng, soft_ceiling};
use crate::audio::fastmath;

/// Soft saturation: a tanh with a little bias (even harmonics, as from
/// tubes or tape). With high partials in the sound it runs at twice the
/// rate, so its harmonics don't fold back; without, it runs plain, delayed
/// to line up with the oversampled path, and the two crossfade.
pub struct Warmth {
    os: Oversampler,
    /// The plain path's input, a ring of the last [`RING`] samples.
    ring: [[f32; RING]; 2],
    pos: usize,
    /// Oversampling wanted; how far the crossfade to it has gone (0 plain,
    /// 1 oversampled); samples the oversampler has been fed since it
    /// started.
    hq: bool,
    blend: f32,
    warm: usize,
    /// DC blocker after the bias: last input and output per channel.
    dc: [(f32, f32); 2],
    dc_r: f32,
    g: f32,
    bias: f32,
    tb: f32,
    makeup: f32,
}

const RING: usize = 32;
/// The oversampler's latency is 30.5 samples: the plain path reads
/// halfway between these two.
const LATENCY: usize = 30;
/// Samples the oversampler needs to fill, and the crossfade's length.
const WARM_UP: usize = 64;
const FADE: f32 = 1.0 / 64.0;

impl Warmth {
    pub fn new(sample_rate: f32) -> Self {
        let mut w = Self {
            os: Oversampler::new(),
            ring: [[0.0; RING]; 2],
            pos: 0,
            hq: false,
            blend: 0.0,
            warm: 0,
            dc: [(0.0, 0.0); 2],
            dc_r: 1.0 - TAU * 8.0 / sample_rate,
            g: 1.0,
            bias: 0.0,
            tb: 0.0,
            makeup: 1.0,
        };
        w.set_drive(0.0, false);
        w
    }

    /// DRIVE 0…1: small signals come out g^0.35 louder, peaks round off.
    /// `high`: there are partials high enough to alias.
    pub fn set_drive(&mut self, drive: f32, high: bool) {
        let d = drive.clamp(0.0, 1.0);
        self.g = 1.0 + 7.0 * d * d;
        self.bias = 0.22 * d;
        self.tb = fastmath::tanh(self.bias);
        self.makeup = 1.0 / (self.g.powf(0.65) * (1.0 - self.tb * self.tb));
        self.hq = high && d > 0.02;
    }

    #[inline]
    fn shape(&self, x: f32) -> f32 {
        (fastmath::tanh(self.g * x + self.bias) - self.tb) * self.makeup
    }

    pub fn process(&mut self, x: [f32; 2]) -> [f32; 2] {
        self.pos = (self.pos + 1) % RING;
        for (ring, v) in self.ring.iter_mut().zip(x) {
            ring[self.pos] = v;
        }
        let at = |ring: &[f32; RING], d: usize| ring[(self.pos + RING - d) % RING];
        let plain = |w: &Self, ch: usize| w.shape(0.5 * (at(&w.ring[ch], LATENCY) + at(&w.ring[ch], LATENCY + 1)));

        if self.hq && self.warm == 0 {
            // Starting: from silence, so no stale history comes out.
            self.os.reset();
        }
        let y = if self.hq || self.blend > 0.0 {
            let up = self.os.up(x);
            let shaped = up.map(|s| s.map(|v| self.shape(v)));
            let over = self.os.down(shaped);
            self.warm += 1;
            if self.hq && self.warm >= WARM_UP {
                self.blend = (self.blend + FADE).min(1.0);
            } else if !self.hq {
                self.blend = (self.blend - FADE).max(0.0);
            }
            if self.blend >= 1.0 {
                over
            } else {
                std::array::from_fn(|ch| plain(self, ch) + (over[ch] - plain(self, ch)) * self.blend)
            }
        } else {
            self.warm = 0;
            [plain(self, 0), plain(self, 1)]
        };
        std::array::from_fn(|ch| {
            let (x1, y1) = self.dc[ch];
            let out = y[ch] - x1 + self.dc_r * y1;
            self.dc[ch] = (y[ch], out);
            out
        })
    }
}

/// Chorus taps, in milliseconds: three voices per side.
const TAPS_MS: [f32; 3] = [8.3, 12.9, 17.7];

/// Three slowly swaying copies per side, each read from alternating sides.
pub struct Chorus {
    lines: [DelayLine; 2],
    phase: f32,
    ms: f32,
    lp: [f32; 2],
    /// Each copy's delay (samples) and its step per sample this block: the
    /// sway is slow, so it is worked out per block and ramped.
    delay: [[f32; 2]; 3],
    step: [[f32; 2]; 3],
    mix: f32,
}

impl Chorus {
    pub fn new(sample_rate: f32) -> Self {
        let ms = sample_rate / 1000.0;
        let len = ((TAPS_MS[2] + 6.0) * ms) as usize + 8;
        let mut c = Self { lines: [DelayLine::new(len), DelayLine::new(len)], phase: 0.0, ms, lp: [0.0; 2], delay: [[0.0; 2]; 3], step: [[0.0; 2]; 3], mix: 0.0 };
        c.begin(0.0, 0.0, 0.0, 1);
        c.delay = std::array::from_fn(|t| [c.delay[t][0] + c.step[t][0], c.delay[t][1] + c.step[t][1]]);
        c
    }

    /// Sets up the next `n` samples: `rate` in cycles per sample, `depth`
    /// 0…1, `mix` 0…1.
    pub fn begin(&mut self, rate: f32, depth: f32, mix: f32, n: usize) {
        self.phase = (self.phase + rate * n as f32).fract();
        let sway = (0.6 + 3.4 * depth) * self.ms;
        for (t, &tap) in TAPS_MS.iter().enumerate() {
            for ch in 0..2 {
                let ph = self.phase + t as f32 / 3.0 + ch as f32 * 0.25;
                let to = tap * self.ms + sway * fastmath::sin_turns(ph);
                self.step[t][ch] = (to - self.delay[t][ch]) / n as f32;
            }
        }
        self.mix = mix;
    }

    pub fn process(&mut self, x: [f32; 2]) -> [f32; 2] {
        self.lines[0].push(x[0]);
        self.lines[1].push(x[1]);
        let mut wet = [0.0f32; 2];
        for t in 0..TAPS_MS.len() {
            for (ch, w) in wet.iter_mut().enumerate() {
                self.delay[t][ch] += self.step[t][ch];
                *w += self.lines[(ch + t) % 2].tap(self.delay[t][ch]);
            }
        }
        let mix = self.mix;
        // A touch darker than the dry: the copies sit behind it.
        for (ch, w) in wet.iter_mut().enumerate() {
            self.lp[ch] += 0.35 * (*w * (1.0 / 3.0) - self.lp[ch]);
        }
        let dry = 1.0 - 0.45 * mix;
        std::array::from_fn(|ch| x[ch] * dry + self.lp[ch] * mix * 0.95)
    }
}

/// A band-pass resonance (TPT state-variable filter), unity at its centre.
#[derive(Clone, Copy, Default)]
struct Resonance {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Resonance {
    fn tune(&mut self, hz: f32, q: f32, sample_rate: f32) {
        let g = fastmath::tan_turns((hz / sample_rate).clamp(1e-4, 0.23) * 0.5);
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v1 * self.k
    }
}

/// The space's line lengths against the Reverb's: a larger hall.
const SCALE: f32 = 1.3;

/// What the space does this block.
pub struct SpaceSettings {
    gains: [f32; LINES],
    damp_a: f32,
    mod_depth: f32,
    air: f32,
    air_a: f32,
    breath: f32,
    wet: f32,
}

/// Diffusion into a long, slowly moving network, with AIR.
pub struct Space {
    sr: f32,
    ms: f32,
    diffusers: [[DelayLine; 4]; 2],
    diffuser_len: [[usize; 4]; 2],
    lines: [DelayLine; LINES],
    damp: [f32; LINES],
    /// Quadrature oscillators moving the lines: (cos, sin).
    lfo: [(f32, f32); LINES],
    lfo_rot: [(f32, f32); LINES],
    air_lp: [f32; 2],
    breath: [Resonance; 2],
    rng: Rng,
}

impl Space {
    pub fn new(sample_rate: f32) -> Self {
        let ms = sample_rate / 1000.0;
        let margin = 0.003 * sample_rate;
        Self {
            sr: sample_rate,
            ms,
            diffusers: std::array::from_fn(|ch| std::array::from_fn(|k| DelayLine::new((DIFFUSERS[k].0 * ms * 1.6 + 8.0) as usize + ch))),
            diffuser_len: std::array::from_fn(|ch| {
                let stretch = if ch == 0 { 1.45 } else { 1.55 };
                std::array::from_fn(|k| ((DIFFUSERS[k].0 * ms * stretch).round() as usize).max(1))
            }),
            lines: std::array::from_fn(|k| DelayLine::new((LINE_MS[k] * SCALE * ms + margin) as usize + 8)),
            damp: [0.0; LINES],
            lfo: std::array::from_fn(|k| {
                let a = k as f32 * 1.3;
                (a.cos(), a.sin())
            }),
            lfo_rot: std::array::from_fn(|k| {
                let w = TAU * MOD_HZ[k] * 0.7 / sample_rate;
                (w.cos(), w.sin())
            }),
            air_lp: [0.0; 2],
            breath: [Resonance::default(); 2],
            rng: Rng::new(0xA1E7_4E55),
        }
    }

    /// SPACE 0…1 (size of the tail and how much of it), TONE 0…1 (how
    /// dark the tail gets), AIR 0…1, the two breath resonances (Hz), and
    /// how far the sound has swelled in (the breath goes with it).
    pub fn settings(&mut self, space: f32, tone: f32, air: f32, breath_hz: [f32; 2], run: f32) -> SpaceSettings {
        let decay = 2.5 + 10.0 * space * space;
        let gains = std::array::from_fn(|k| {
            let secs = LINE_MS[k] * SCALE / 1000.0;
            10f32.powf(-3.0 * secs / decay)
        });
        // The tail's damping opens with TONE and AIR: 2.5 kHz … 14 kHz.
        let damp_hz = 2500.0 * (2.5 * (0.6 * tone + 0.4 * air)).exp2();
        for (r, hz) in self.breath.iter_mut().zip(breath_hz) {
            r.tune(hz, 16.0, self.sr);
        }
        // Keep the quadrature oscillators on the unit circle.
        for (c, sn) in self.lfo.iter_mut() {
            let r = 1.0 / (*c * *c + *sn * *sn).sqrt();
            *c *= r;
            *sn *= r;
        }
        // Long tails pile up: trim them to match.
        let wet = space * (3.0 / decay.max(3.0)).powf(0.35);
        SpaceSettings {
            gains,
            damp_a: (TAU * damp_hz / self.sr).min(1.0),
            mod_depth: 0.0018 * self.sr,
            air: air * 1.6,
            air_a: TAU * 3000.0 / self.sr,
            breath: air * air * 0.9 * run,
            wet,
        }
    }

    /// Dry in, dry and tail out; also returns the tail's own level (for
    /// resting).
    pub fn process(&mut self, x: [f32; 2], s: &SpaceSettings) -> ([f32; 2], f32) {
        // Breath: noise through the tuned resonances, mostly into the tail.
        let breath = if s.breath > 0.0 {
            let n = self.rng.bipolar();
            let m = self.rng.bipolar();
            [self.breath[0].process(n) * s.breath, self.breath[1].process(m) * s.breath]
        } else {
            [0.0; 2]
        };
        let mut d = [x[0] + breath[0] * 2.0, x[1] + breath[1] * 2.0];
        for (ch, dc) in d.iter_mut().enumerate() {
            for (k, &(_, g)) in DIFFUSERS.iter().enumerate() {
                *dc = allpass(&mut self.diffusers[ch][k], self.diffuser_len[ch][k], g, *dc);
            }
        }
        let mut taps = [0.0f32; LINES];
        let mut v = [0.0f32; LINES];
        for k in 0..LINES {
            let (c, sn) = self.lfo[k];
            let (rc, rs) = self.lfo_rot[k];
            self.lfo[k] = (c * rc - sn * rs, sn * rc + c * rs);
            taps[k] = self.lines[k].tap(LINE_MS[k] * SCALE * self.ms + s.mod_depth * sn - 1.0);
            self.damp[k] += s.damp_a * (taps[k] - self.damp[k]);
            v[k] = self.damp[k] * s.gains[k];
        }
        hadamard(&mut v);
        for (k, (line, vk)) in self.lines.iter_mut().zip(v).enumerate() {
            line.push(soft_ceiling(vk + d[k % 2] * 0.5));
        }
        let tail = [(taps[0] - taps[2] + taps[4] - taps[6]) * 0.5, (taps[1] - taps[3] + taps[5] - taps[7]) * 0.5];
        // AIR: the tail's highs lifted.
        let mut out = [0.0f32; 2];
        let mut level = 0.0f32;
        for ch in 0..2 {
            self.air_lp[ch] += s.air_a * (tail[ch] - self.air_lp[ch]);
            let wet = tail[ch] + s.air * (tail[ch] - self.air_lp[ch]);
            level = level.max(wet.abs());
            out[ch] = x[ch] * (1.0 - 0.4 * s.wet) + (wet * s.wet + breath[ch] * 0.15) * 1.1;
        }
        (out, level)
    }
}
