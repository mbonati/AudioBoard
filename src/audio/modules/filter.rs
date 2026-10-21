//! Filter: stereo state-variable filter (Simper's trapezoidal SVF) with
//! low/band/high-pass and notch outputs, drive and cutoff modulation.

use std::sync::Arc;

use super::{ModuleDef, ModuleParams, approach, soft_limit};
use crate::audio::dsp::drive;
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{ParamSpec, Unit};

pub const CUTOFF: usize = 0;
pub const RESO: usize = 1;
pub const DRIVE: usize = 2;
pub const MOD: usize = 3;

const PARAMS: [ParamSpec; 4] = [
    ParamSpec::log("CUTOFF", 20.0, 20_000.0, 1200.0, Unit::Hz),
    ParamSpec::lin("RESO", 0.0, 1.0, 0.2, Unit::Percent),
    ParamSpec::lin("DRIVE", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("MOD", -4.0, 4.0, 0.0, Unit::Octaves).bipolar(),
];

pub const LOWPASS: usize = 0;
pub const BANDPASS: usize = 1;
pub const HIGHPASS: usize = 2;
pub const NOTCH: usize = 3;
pub const MODE_NAMES: [&str; 4] = ["Low-pass", "Band-pass", "High-pass", "Notch"];

pub static DEF: ModuleDef = ModuleDef {
    name: "Filter",
    prefix: "FILTER",
    hint: "LP · BP · HP · notch",
    inputs: &["IN", "CUT"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[(MOD, 1)],
    modes: &["LP", "BP", "HP", "NOTCH"],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &[],
    display_h: 42.0,
    viz: 0,
};

/// Resonance (0..1) to Q: 0.5 (no peak) up to 12.
pub fn q_of(reso: f32) -> f32 {
    0.5 * 24f32.powf(reso.clamp(0.0, 1.0))
}

pub struct Filter {
    params: Arc<ModuleParams>,
    sr: f32,
    cutoff: f32,
    ic1: [f32; 2],
    ic2: [f32; 2],
}

impl Filter {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        Self {
            cutoff: params.get(CUTOFF),
            params,
            sr: sample_rate,
            ic1: [0.0; 2],
            ic2: [0.0; 2],
        }
    }
}

impl Processor for Filter {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let mode = p.mode();
        let cutoff_t = p.get(CUTOFF);
        let k = 1.0 / q_of(p.get(RESO));
        let amount = p.get(DRIVE);
        let depth = p.get(MOD);
        let cv = inputs.cv(1);
        let Some(input) = inputs.get(0) else {
            for ch in outputs[0].iter_mut() {
                ch[..n].fill(0.0);
            }
            self.ic1 = [0.0; 2];
            self.ic2 = [0.0; 2];
            return;
        };

        let max_fc = self.sr * 0.45;
        let mut peak = 0.0f32;
        // Coefficients for the last cutoff (recomputed only when it moves).
        let (mut last_fc, mut a1, mut a2, mut a3) = (f32::NAN, 0.0, 0.0, 0.0);
        for i in 0..n {
            approach(&mut self.cutoff, cutoff_t, ctx.smooth);
            let mut fc = self.cutoff;
            if let Some(cv) = cv {
                fc *= fastmath::exp2(cv[i].clamp(-1.0, 1.0) * depth);
            }
            if fc != last_fc {
                last_fc = fc;
                let g = fastmath::tan_turns(fc.clamp(10.0, max_fc) / (2.0 * self.sr));
                a1 = 1.0 / (1.0 + g * (g + k));
                a2 = g * a1;
                a3 = g * a2;
            }

            for ch in 0..2 {
                let v0 = drive(input[ch][i], amount);
                let v3 = v0 - self.ic2[ch];
                let v1 = a1 * self.ic1[ch] + a2 * v3;
                let v2 = self.ic2[ch] + a2 * self.ic1[ch] + a3 * v3;
                self.ic1[ch] = 2.0 * v1 - self.ic1[ch];
                self.ic2[ch] = 2.0 * v2 - self.ic2[ch];
                let y = match mode {
                    LOWPASS => v2,
                    BANDPASS => v1 * k,
                    HIGHPASS => v0 - k * v1 - v2,
                    _ => v0 - k * v1,
                };
                let y = soft_limit(y);
                outputs[0][ch][i] = y;
                peak = peak.max(y.abs());
            }
        }
        p.meter.fetch_max(peak);
    }
}
