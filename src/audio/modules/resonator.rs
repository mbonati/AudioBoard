//! Resonator: a bank of eight tuned modes (strings, pipes, bars, bells)
//! excited by whatever is patched in. Noise or grains in give organic,
//! physical-sounding textures.

use std::f32::consts::TAU;
use std::sync::Arc;

use super::{ModuleDef, ModuleParams, approach, mix_gains, pan_gains, soft_limit, time_coeff};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::params::{ParamSpec, Unit};

pub const PITCH: usize = 0;
pub const DECAY: usize = 1;
pub const BRIGHT: usize = 2;
pub const SPREAD: usize = 3;
pub const MOD: usize = 4;
pub const MIX: usize = 5;

const PARAMS: [ParamSpec; 6] = [
    ParamSpec::log("PITCH", 20.0, 2000.0, 110.0, Unit::Hz),
    ParamSpec::log("DECAY", 0.05, 20.0, 2.5, Unit::Seconds),
    ParamSpec::lin("BRIGHT", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("SPREAD", 0.0, 1.0, 0.6, Unit::Percent),
    ParamSpec::lin("P.MOD", 0.0, 2.0, 0.0, Unit::Octaves),
    ParamSpec::lin("MIX", 0.0, 1.0, 1.0, Unit::Percent),
];

pub const MODES: usize = 8;

/// Partial ratios per material.
pub const MATERIALS: [(&str, [f32; MODES]); 4] = [
    ("String", [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]),
    ("Pipe", [1.0, 3.0, 5.0, 7.0, 9.0, 11.0, 13.0, 15.0]),
    ("Bar", [1.0, 2.756, 5.404, 8.933, 13.344, 18.638, 24.815, 31.873]),
    ("Bell", [0.5, 1.0, 1.183, 1.506, 2.0, 2.514, 2.662, 3.011]),
];

pub static DEF: ModuleDef = ModuleDef {
    name: "Resonator",
    prefix: "RESON",
    hint: "string · pipe · bar · bell",
    inputs: &["IN", "PITCH"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[(MOD, 1)],
    modes: &["STRING", "PIPE", "BAR", "BELL"],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: 0,
};

/// Relative level and decay time of mode `k` (0-based) for BRIGHT.
pub fn mode_shape(k: usize, bright: f32) -> (f32, f32) {
    let dull = 1.0 - bright.clamp(0.0, 1.0);
    let amp = 1.0 / (1.0 + k as f32).powf(0.3 + 1.7 * dull);
    let decay = 1.0 / (1.0 + k as f32 * dull * 0.8);
    (amp, decay)
}

/// Two-pole resonator normalised so that white noise in gives roughly the
/// same level out whatever the decay time.
#[derive(Default, Clone, Copy)]
struct Mode {
    b0: f32,
    a1: f32,
    a2: f32,
    y1: f32,
    y2: f32,
}

impl Mode {
    fn tune(&mut self, freq: f32, t60: f32, sr: f32) {
        let w = TAU * freq / sr;
        let r = (-6.91 / (t60 * sr)).exp();
        self.a1 = 2.0 * r * w.cos();
        self.a2 = -r * r;
        self.b0 = w.sin() * (2.0 * (1.0 - r * r)).sqrt();
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.a1 * self.y1 + self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

pub struct Resonator {
    params: Arc<ModuleParams>,
    sr: f32,
    modes: [Mode; MODES],
    pitch: f32,
    mix: f32,
    /// Peak follower for the automatic gain control.
    env: f32,
    release: f32,
    /// Settings the modes were last tuned for, with their pan gains:
    /// retuning costs a dozen transcendental calls per mode.
    tuned: Option<(f32, usize, f32, f32, f32)>,
    gains: ([f32; MODES], [f32; MODES]),
}

impl Resonator {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        Self {
            pitch: params.get(PITCH),
            mix: params.get(MIX),
            params,
            sr: sample_rate,
            modes: [Mode::default(); MODES],
            env: 0.0,
            release: time_coeff(0.25, sample_rate),
            tuned: None,
            gains: ([0.0; MODES], [0.0; MODES]),
        }
    }
}

impl Processor for Resonator {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let ratios = MATERIALS[p.mode().min(MATERIALS.len() - 1)].1;
        let decay = p.get(DECAY);
        let bright = p.get(BRIGHT);
        let spread = p.get(SPREAD);
        let mix_t = p.get(MIX);
        let input = inputs.get(0);

        // Tuning is updated once per block (pitch CV at the block start).
        approach(&mut self.pitch, p.get(PITCH), 0.3);
        let mut pitch = self.pitch;
        if let Some(cv) = inputs.cv(1) {
            pitch *= (cv[0].clamp(-1.0, 1.0) * p.get(MOD)).exp2();
        }
        let settings = (pitch, p.mode(), decay, bright, spread);
        if self.tuned != Some(settings) {
            self.tuned = Some(settings);
            let (gl, gr) = &mut self.gains;
            for k in 0..MODES {
                let f = pitch * ratios[k];
                let (amp, decay_k) = mode_shape(k, bright);
                if f >= self.sr * 0.45 {
                    (gl[k], gr[k]) = (0.0, 0.0);
                    continue;
                }
                self.modes[k].tune(f, (decay * decay_k).max(0.02), self.sr);
                let side = if k % 2 == 0 { -1.0 } else { 1.0 };
                let (l, r) = pan_gains(side * spread * (0.3 + 0.7 * k as f32 / MODES as f32));
                gl[k] = amp * l;
                gr[k] = amp * r;
            }
        }
        let (gl, gr) = self.gains;

        let mut peak = 0.0f32;
        for i in 0..n {
            approach(&mut self.mix, mix_t, ctx.smooth);
            let (dry_l, dry_r) = match input {
                Some(b) => (b[0][i], b[1][i]),
                None => (0.0, 0.0),
            };
            let x = 0.5 * (dry_l + dry_r);
            let (mut wl, mut wr) = (0.0, 0.0);
            for k in 0..MODES {
                if gl[k] == 0.0 && gr[k] == 0.0 {
                    continue;
                }
                let y = self.modes[k].tick(x);
                wl += y * gl[k];
                wr += y * gr[k];
            }
            // Automatic gain: a pitched input landing on a mode can ring
            // very loudly, so hold the wet signal under ~0.7.
            let level = wl.abs().max(wr.abs());
            self.env = if level > self.env { level } else { self.env + (level - self.env) * self.release };
            let agc = if self.env > 0.7 { 0.7 / self.env } else { 1.0 };

            let (dry_g, wet_g) = mix_gains(self.mix);
            let yl = soft_limit(dry_l * dry_g + wl * agc * wet_g);
            let yr = soft_limit(dry_r * dry_g + wr * agc * wet_g);
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak = peak.max(yl.abs()).max(yr.abs());
        }
        p.meter.fetch_max(peak);
    }
}
