//! SPACE: a lush hall, the Reverb module's delay network at a fixed large
//! size and gentle damping; LENGTH sets its decay time.

use std::f32::consts::TAU;

use crate::audio::engine::Buf;
use crate::audio::modules::reverb::{DIFFUSERS, LINE_MS, LINES, MOD_HZ, allpass, hadamard};
use crate::audio::modules::{DelayLine, approach, mix_gains, soft_ceiling, time_coeff};

/// Line length scale: a large hall.
const SCALE: f32 = 0.92;
/// Line modulation depth, in seconds.
const MOD_DEPTH: f32 = 0.0006;

pub struct Space {
    smooth: f32,
    lines: [DelayLine; LINES],
    lengths: [f32; LINES],
    diffusers: [[DelayLine; 4]; 2],
    diffuser_len: [[usize; 4]; 2],
    damp: [f32; LINES],
    damp_a: f32,
    lfo: [(f32, f32); LINES],
    lfo_rot: [(f32, f32); LINES],
    mod_depth: f32,
    mix: f32,
    sr: f32,
}

impl Space {
    pub fn new(sample_rate: f32) -> Self {
        let ms = sample_rate / 1000.0;
        let margin = MOD_DEPTH * sample_rate + 8.0;
        Self {
            smooth: time_coeff(0.01, sample_rate),
            lines: std::array::from_fn(|i| {
                DelayLine::new((LINE_MS[i] * SCALE * ms + margin) as usize)
            }),
            lengths: std::array::from_fn(|i| LINE_MS[i] * SCALE * ms),
            diffusers: std::array::from_fn(|ch| {
                std::array::from_fn(|k| {
                    DelayLine::new((DIFFUSERS[k].0 * ms * 1.07 + 4.0) as usize + ch)
                })
            }),
            diffuser_len: std::array::from_fn(|ch| {
                let stretch = if ch == 0 { 1.0 } else { 1.07 };
                std::array::from_fn(|k| ((DIFFUSERS[k].0 * ms * stretch).round() as usize).max(1))
            }),
            damp: [0.0; LINES],
            damp_a: (TAU * 7_000.0 / sample_rate).min(1.0),
            lfo: std::array::from_fn(|i| {
                let a = i as f32 * 0.9;
                (a.cos(), a.sin())
            }),
            lfo_rot: std::array::from_fn(|i| {
                let w = TAU * MOD_HZ[i] / sample_rate;
                (w.cos(), w.sin())
            }),
            mod_depth: MOD_DEPTH * sample_rate,
            mix: 0.0,
            sr: sample_rate,
        }
    }

    /// Mixes the hall into `x`: `length` is the decay time (RT60) in
    /// seconds, `mix` the dry/wet balance.
    #[allow(clippy::needless_range_loop)] // both channels, sample by sample
    pub fn process(&mut self, x: &mut Buf, n: usize, length: f32, mix: f32) {
        for (c, s) in self.lfo.iter_mut() {
            let r = 1.0 / (*c * *c + *s * *s).sqrt();
            *c *= r;
            *s *= r;
        }
        let gains: [f32; LINES] =
            std::array::from_fn(|k| 10f32.powf(-3.0 * self.lengths[k] / self.sr / length.max(0.1)));
        let wet_trim = (2.5 / length.max(2.5)).powf(0.3);
        for i in 0..n {
            approach(&mut self.mix, mix, self.smooth);
            let (dry_l, dry_r) = (x[0][i], x[1][i]);
            let mut input = [dry_l, dry_r];
            for (ch, v) in input.iter_mut().enumerate() {
                for (k, &(_, g)) in DIFFUSERS.iter().enumerate() {
                    *v = allpass(&mut self.diffusers[ch][k], self.diffuser_len[ch][k], g, *v);
                }
            }
            let mut taps = [0.0f32; LINES];
            let mut v = [0.0f32; LINES];
            for k in 0..LINES {
                let (c, s) = self.lfo[k];
                let (rc, rs) = self.lfo_rot[k];
                self.lfo[k] = (c * rc - s * rs, s * rc + c * rs);
                taps[k] = self.lines[k].tap(self.lengths[k] + self.mod_depth * s - 1.0);
                self.damp[k] += self.damp_a * (taps[k] - self.damp[k]);
                v[k] = self.damp[k] * gains[k];
            }
            hadamard(&mut v);
            for (k, (line, vk)) in self.lines.iter_mut().zip(v).enumerate() {
                line.push(soft_ceiling(vk + input[k % 2] * 0.5));
            }
            let wet_l = (taps[0] - taps[2] + taps[4] - taps[6]) * 0.5;
            let wet_r = (taps[1] - taps[3] + taps[5] - taps[7]) * 0.5;
            let (dry_g, wet_g) = mix_gains(self.mix);
            x[0][i] = dry_l * dry_g + wet_l * wet_g * wet_trim;
            x[1][i] = dry_r * dry_g + wet_r * wet_g * wet_trim;
        }
    }
}
