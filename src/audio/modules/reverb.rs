//! Reverb: an 8-line feedback delay network with input diffusion, slowly
//! modulated lines, frequency-dependent decay, an octave-up "shimmer" in the
//! loop and FREEZE for endless pads.

use std::f32::consts::TAU;
use std::sync::Arc;

use super::{DelayLine, ModuleDef, ModuleParams, approach, mix_gains, soft_ceiling, soft_limit, time_coeff, Quiet, silence};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{ParamSpec, Unit};

pub const SIZE: usize = 0;
pub const DECAY: usize = 1;
pub const DAMP: usize = 2;
pub const MOD: usize = 3;
pub const SHIMMER: usize = 4;
pub const MIX: usize = 5;

const PARAMS: [ParamSpec; 6] = [
    ParamSpec::lin("SIZE", 0.0, 1.0, 0.65, Unit::Percent),
    ParamSpec::log("DECAY", 0.3, 60.0, 7.0, Unit::Seconds),
    ParamSpec::lin("DAMP", 0.0, 1.0, 0.45, Unit::Percent),
    ParamSpec::lin("MOD", 0.0, 1.0, 0.4, Unit::Percent),
    ParamSpec::lin("SHIMMER", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("MIX", 0.0, 1.0, 0.4, Unit::Percent),
];

pub static DEF: ModuleDef = ModuleDef {
    name: "Reverb",
    prefix: "REVERB",
    hint: "hall · shimmer · freeze",
    inputs: &["IN"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: Some("FREEZE"),
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: 0,
};

const LINES: usize = 8;
/// Line lengths at full size, in milliseconds (roughly co-prime).
const LINE_MS: [f32; LINES] = [61.3, 73.7, 89.1, 101.9, 117.1, 131.7, 149.3, 167.9];
/// Input diffusers: (length ms, gain), applied in series to each channel.
const DIFFUSERS: [(f32, f32); 4] = [(4.77, 0.75), (3.59, 0.75), (12.73, 0.625), (9.31, 0.625)];
/// Line modulation rates in Hz.
const MOD_HZ: [f32; LINES] = [0.11, 0.17, 0.23, 0.29, 0.37, 0.43, 0.53, 0.61];

/// Length scale for the SIZE knob: small room to large hall.
fn size_scale(size: f32) -> f32 {
    0.25 + 0.75 * size.clamp(0.0, 1.0)
}

/// Schroeder all-pass on a delay line of `d` samples.
fn allpass(line: &mut DelayLine, d: usize, g: f32, x: f32) -> f32 {
    let z = line.tap_at(d - 1);
    let w = x + g * z;
    line.push(w);
    z - g * w
}

/// In-place fast Walsh–Hadamard transform, normalised (orthogonal).
fn hadamard(v: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        for i in (0..LINES).step_by(h * 2) {
            for j in i..i + h {
                let (a, b) = (v[j], v[j + h]);
                v[j] = a + b;
                v[j + h] = a - b;
            }
        }
        h *= 2;
    }
    let norm = 1.0 / (LINES as f32).sqrt();
    for x in v.iter_mut() {
        *x *= norm;
    }
}

/// Octave-up pitch shifter: two crossfaded taps sweeping through a short
/// delay at twice the write speed.
struct Shifter {
    line: DelayLine,
    window: f32,
    phase: f32,
}

impl Shifter {
    fn new(sample_rate: f32) -> Self {
        let window = 0.06 * sample_rate;
        Self {
            line: DelayLine::new(window as usize + 4),
            window,
            phase: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        self.line.push(x);
        self.phase = (self.phase + 1.0 / self.window).fract();
        let p2 = (self.phase + 0.5).fract();
        // sin(π·phase) windows: sin_turns takes half turns here.
        let a = self.line.tap((1.0 - self.phase) * self.window) * fastmath::sin_turns(self.phase * 0.5);
        let b = self.line.tap((1.0 - p2) * self.window) * fastmath::sin_turns(p2 * 0.5);
        a + b
    }
}

pub struct Reverb {
    params: Arc<ModuleParams>,
    quiet: Quiet,
    sr: f32,
    lines: [DelayLine; LINES],
    diffusers: [[DelayLine; 4]; 2],
    /// The diffusers' delays in whole samples (fixed: no interpolation).
    diffuser_len: [[usize; 4]; 2],
    damp: [f32; LINES],
    /// Quadrature oscillators for line modulation: (cos, sin).
    lfo: [(f32, f32); LINES],
    lfo_rot: [(f32, f32); LINES],
    shifter: Shifter,
    shimmer_hp: f32,
    shimmer_fb: f32,
    size: f32,
    size_glide: f32,
    mix: f32,
    feed: f32,
}

impl Reverb {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        let ms = sample_rate / 1000.0;
        let mod_margin = 0.002 * sample_rate;
        let lines = std::array::from_fn(|i| DelayLine::new((LINE_MS[i] * ms + mod_margin) as usize + 8));
        let diffusers = std::array::from_fn(|ch| {
            std::array::from_fn(|k| DelayLine::new((DIFFUSERS[k].0 * ms * 1.07 + 4.0) as usize + ch))
        });
        let lfo = std::array::from_fn(|i| {
            let a = i as f32 * 0.9;
            (a.cos(), a.sin())
        });
        let lfo_rot = std::array::from_fn(|i| {
            let w = TAU * MOD_HZ[i] / sample_rate;
            (w.cos(), w.sin())
        });
        Self {
            quiet: Quiet::new(lines.iter().map(DelayLine::len).max().unwrap_or(0) * 2),
            size: params.get(SIZE),
            mix: params.get(MIX),
            params,
            sr: sample_rate,
            lines,
            diffusers,
            diffuser_len: std::array::from_fn(|ch| {
                let stretch = if ch == 0 { 1.0 } else { 1.07 };
                std::array::from_fn(|k| ((DIFFUSERS[k].0 * ms * stretch).round() as usize).max(1))
            }),
            damp: [0.0; LINES],
            lfo,
            lfo_rot,
            shifter: Shifter::new(sample_rate),
            shimmer_hp: 0.0,
            shimmer_fb: 0.0,
            size_glide: time_coeff(0.3, sample_rate),
            feed: 1.0,
        }
    }
}

impl Processor for Reverb {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let frozen = p.toggled();
        let size_t = p.get(SIZE);
        let decay = p.get(DECAY);
        let damp = p.get(DAMP);
        let mod_depth = p.get(MOD) * 0.0015 * self.sr; // up to ±1.5 ms
        let shimmer = if frozen { 0.0 } else { p.get(SHIMMER) };
        let mix_t = p.get(MIX);
        let input = inputs.get(0);
        let silent_in = Quiet::silent(input, n);
        if silent_in && self.quiet.resting() {
            // Gone quiet: nothing to compute until input returns.
            silence(outputs, n);
            return;
        }

        let sr = self.sr;
        let ms = sr / 1000.0;
        // Damping low-pass in each line: open (18 kHz) to dark (~400 Hz).
        let damp_a = if frozen {
            1.0
        } else {
            (TAU * 18_000.0 * (-damp * 5.5).exp2() / sr).min(1.0)
        };
        let shimmer_hp_a = TAU * 300.0 / sr;
        // Long decays build up more energy: trim the wet level to match.
        let wet_trim = (2.5 / decay.max(2.5)).powf(0.3);
        // Freezing closes the input and makes the loop lossless.
        let feed_t = if frozen { 0.0 } else { 1.0 };

        // Keep the quadrature oscillators on the unit circle.
        for (c, s) in self.lfo.iter_mut() {
            let r = 1.0 / (*c * *c + *s * *s).sqrt();
            *c *= r;
            *s *= r;
        }

        // Per-line loop gain for the requested RT60 (per block: SIZE glides slowly).
        let gains: [f32; LINES] = std::array::from_fn(|k| {
            let secs = LINE_MS[k] * size_scale(self.size) / 1000.0;
            if frozen { 1.0 } else { 10f32.powf(-3.0 * secs / decay) }
        });

        let mut peak = 0.0f32;
        // The effect's own signal, for resting.
        let mut inner = 0.0f32;
        for i in 0..n {
            approach(&mut self.size, size_t, self.size_glide);
            approach(&mut self.mix, mix_t, ctx.smooth);
            approach(&mut self.feed, feed_t, ctx.smooth);
            let scale = size_scale(self.size);

            let (dry_l, dry_r) = match input {
                Some(b) => (b[0][i], b[1][i]),
                None => (0.0, 0.0),
            };

            // Input: diffuse each channel, plus the shimmer return.
            let shim = if shimmer > 0.0 { fastmath::tanh(self.shimmer_fb * shimmer * 0.7) } else { 0.0 };
            let mut x = [dry_l * self.feed + shim, dry_r * self.feed + shim];
            for (ch, xc) in x.iter_mut().enumerate() {
                for (k, &(_, g)) in DIFFUSERS.iter().enumerate() {
                    *xc = allpass(&mut self.diffusers[ch][k], self.diffuser_len[ch][k], g, *xc);
                }
            }

            // Read the lines (modulated), damp and apply the decay gain.
            let mut taps = [0.0f32; LINES];
            let mut v = [0.0f32; LINES];
            for k in 0..LINES {
                let (c, s) = self.lfo[k];
                let (rc, rs) = self.lfo_rot[k];
                self.lfo[k] = (c * rc - s * rs, s * rc + c * rs);
                let len = LINE_MS[k] * scale * ms;
                taps[k] = self.lines[k].tap(len + mod_depth * s - 1.0);
                self.damp[k] += damp_a * (taps[k] - self.damp[k]);
                v[k] = self.damp[k] * gains[k];
            }
            hadamard(&mut v);
            for (k, (line, vk)) in self.lines.iter_mut().zip(v).enumerate() {
                let inject = x[k % 2];
                line.push(soft_ceiling(vk + inject * 0.5));
            }

            // Stereo outputs from alternating lines.
            let wet_l = (taps[0] - taps[2] + taps[4] - taps[6]) * 0.5;
            let wet_r = (taps[1] - taps[3] + taps[5] - taps[7]) * 0.5;
            inner = inner.max(wet_l.abs()).max(wet_r.abs());

            // Shimmer: octave-up copy of the tail, high-passed, fed back
            // (skipped while SHIMMER is off; the shifter keeps listening).
            if shimmer > 0.0 {
                let shifted = self.shifter.process(0.5 * (wet_l + wet_r));
                self.shimmer_hp += shimmer_hp_a * (shifted - self.shimmer_hp);
                self.shimmer_fb = shifted - self.shimmer_hp;
            } else {
                self.shifter.line.push(0.5 * (wet_l + wet_r));
            }

            let (dry_g, wet_g) = mix_gains(self.mix);
            let wet_g = wet_g * wet_trim;
            let yl = soft_limit(dry_l * dry_g + wet_l * wet_g);
            let yr = soft_limit(dry_r * dry_g + wet_r * wet_g);
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak = peak.max(yl.abs()).max(yr.abs());
        }
        self.quiet.update(silent_in, inner, n);
        p.meter.fetch_max(peak);
    }
}
