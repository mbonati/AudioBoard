//! Random: a modulation source that jumps or glides between random values,
//! with optionally irregular timing. Patch it into any CV input.

use std::sync::Arc;

use super::{ModuleDef, ModuleParams, Rng};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{ParamSpec, Unit};

pub const RATE: usize = 0;
pub const CHAOS: usize = 1;
pub const SMOOTH: usize = 2;
pub const LEVEL: usize = 3;

const PARAMS: [ParamSpec; 4] = [
    ParamSpec::log("RATE", 0.02, 20.0, 0.4, Unit::Hz),
    ParamSpec::lin("CHAOS", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("SMOOTH", 0.0, 1.0, 0.8, Unit::Percent),
    ParamSpec::lin("LEVEL", 0.0, 1.0, 1.0, Unit::Percent),
];

/// `viz[VALUE]` is the current output, for the node's trace.
pub const VALUE: usize = 0;

pub static DEF: ModuleDef = ModuleDef {
    name: "Random",
    prefix: "RANDOM",
    hint: "smooth / stepped CV",
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
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: 1,
};

pub struct Random {
    params: Arc<ModuleParams>,
    sr: f32,
    rng: Rng,
    /// Position within the current segment, 0..1.
    t: f32,
    /// Length multiplier of the current segment (CHAOS).
    stretch: f32,
    from: f32,
    to: f32,
}

impl Random {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        let mut rng = Rng::new(0xC4A0_5EED);
        let to = rng.bipolar();
        Self {
            params,
            sr: sample_rate,
            rng,
            t: 0.0,
            stretch: 1.0,
            from: 0.0,
            to,
        }
    }
}

impl Processor for Random {
    fn process(&mut self, _ctx: &ProcessCtx, _inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let (rate, chaos, smooth, level) = (p.get(RATE), p.get(CHAOS), p.get(SMOOTH), p.get(LEVEL));
        let mut y = 0.0;
        let [out_l, out_r] = &mut outputs[0];
        for (ol, or) in out_l[..n].iter_mut().zip(&mut out_r[..n]) {
            self.t += rate / (self.sr * self.stretch);
            if self.t >= 1.0 {
                self.t -= self.t.floor();
                self.from = self.to;
                // Chaos makes the timing irregular.
                self.stretch = 1.0 + chaos * 0.9 * self.rng.bipolar();
                self.to = self.rng.bipolar();
            }
            // SMOOTH = 0: sample & hold; 1: glide over the whole segment.
            let g = (self.t / smooth.max(1e-3)).min(1.0);
            let s = 0.5 - 0.5 * fastmath::cos_turns(g * 0.5);
            y = (self.from + (self.to - self.from) * s) * level;
            *ol = y;
            *or = y;
        }
        p.viz[VALUE].set(y);
        p.meter.fetch_max(y.abs());
    }
}
