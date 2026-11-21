//! Audio In: the audio interface's inputs on a cable. INPUT picks the
//! channel; STEREO takes it and the next as left and right, MONO puts it on
//! both sides. GAIN sets the level. Which device is the input is set in
//! Settings; it opens while an Audio In node is on the board.

use std::sync::Arc;

use super::{ModuleDef, ModuleParams, approach};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::input::MAX_CHANNELS;
use crate::audio::params::{EngineShared, ParamSpec, Unit};

pub const INPUT: usize = 0;
pub const GAIN: usize = 1;

/// The second mode (the first is STEREO).
pub const MONO: usize = 1;

const PARAMS: [ParamSpec; 2] = [
    ParamSpec::lin("INPUT", 1.0, MAX_CHANNELS as f32, 1.0, Unit::Integer),
    ParamSpec::lin("GAIN", -24.0, 24.0, 0.0, Unit::Db).bipolar(),
];

/// What the processor publishes: the two sides' levels (falling slowly),
/// and the input's channels (0: no input open).
pub mod viz {
    pub const LEFT: usize = 0;
    pub const RIGHT: usize = 1;
    pub const CHANNELS: usize = 2;
    pub const LEN: usize = 3;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Audio In",
    prefix: "AUDIO IN",
    hint: "the interface's inputs",
    inputs: &[],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["STEREO", "MONO"],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

/// The channels (0-based) INPUT and the mode pick: (left, right).
pub fn channels(p: &ModuleParams) -> (usize, usize) {
    let first = (p.get(INPUT).round().max(1.0) as usize - 1).min(MAX_CHANNELS - 1);
    let second = if p.mode() == MONO {
        first
    } else {
        (first + 1).min(MAX_CHANNELS - 1)
    };
    (first, second)
}

pub struct AudioIn {
    params: Arc<ModuleParams>,
    shared: Arc<EngineShared>,
    /// The output buffer being played, and how far into it this node is.
    render: u32,
    at: usize,
    gain: f32,
    levels: [f32; 2],
    /// Per-sample fall of the displayed levels (~300 ms).
    fall: f32,
}

impl AudioIn {
    pub fn new(params: Arc<ModuleParams>, shared: Arc<EngineShared>, sample_rate: f32) -> Self {
        Self {
            gain: 10f32.powf(params.get(GAIN) / 20.0),
            params,
            render: shared.input.render_id(),
            shared,
            at: 0,
            levels: [0.0; 2],
            fall: (-1.0 / (0.3 * sample_rate)).exp(),
        }
    }
}

impl Processor for AudioIn {
    fn process(&mut self, ctx: &ProcessCtx, _inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let input = &self.shared.input;
        // Blocks come in order through each output buffer.
        let render = input.render_id();
        if render != self.render {
            self.render = render;
            self.at = 0;
        }
        let p = &self.params;
        let (left, right) = channels(p);
        let out = &mut outputs[0];
        input.read(self.at, n, left, right, out);
        self.at += n;

        let target = 10f32.powf(p.get(GAIN) / 20.0);
        let mut peak = [0.0f32; 2];
        let [out_l, out_r] = out;
        for (l, r) in out_l[..n].iter_mut().zip(&mut out_r[..n]) {
            approach(&mut self.gain, target, ctx.smooth);
            *l *= self.gain;
            *r *= self.gain;
            peak = [peak[0].max(l.abs()), peak[1].max(r.abs())];
        }
        p.meter.fetch_max(peak[0].max(peak[1]));
        let fall = self.fall.powi(n as i32);
        for (level, pk) in self.levels.iter_mut().zip(peak) {
            *level = pk.max(*level * fall);
        }
        p.viz[viz::LEFT].set(self.levels[0]);
        p.viz[viz::RIGHT].set(self.levels[1]);
        p.viz[viz::CHANNELS].set(input.channels() as f32);
    }
}
