//! Ping Pong: a stereo delay whose echoes bounce from one side to the other.
//! The input (summed to mono) enters one delay line; each line feeds the
//! other, so every echo lands on the opposite side. WIDTH sets how far
//! apart the sides are, TONE darkens each repeat, DUCK lowers the echoes
//! while the input is playing so they bloom in the gaps. With SYNC the time
//! is a note value (DIV) at the global tempo.

use std::f32::consts::TAU;
use std::sync::Arc;

use super::delay::{MAX_SYNC_TIME, delay_seconds, seconds_per_beat};
use super::{DelayLine, ModuleDef, ModuleParams, approach, mix_gains, soft_limit, time_coeff};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{ParamSpec, SYNC_DIVISIONS, Unit};

pub const TIME: usize = 0;
pub const DIV: usize = 1;
pub const FEEDBACK: usize = 2;
pub const WIDTH: usize = 3;
pub const TONE: usize = 4;
pub const DUCK: usize = 5;
pub const MIX: usize = 6;

const PARAMS: [ParamSpec; 7] = [
    ParamSpec::log("TIME", 0.01, super::delay::MAX_TIME, 0.3, Unit::Seconds),
    ParamSpec::lin("DIV", 0.0, (SYNC_DIVISIONS.len() - 1) as f32, 7.0, Unit::Sync),
    ParamSpec::lin("FEEDBACK", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("WIDTH", 0.0, 1.0, 1.0, Unit::Percent),
    ParamSpec::lin("TONE", 0.0, 1.0, 0.6, Unit::Percent),
    ParamSpec::lin("DUCK", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("MIX", 0.0, 1.0, 0.35, Unit::Percent),
];

/// Which side the first echo lands on.
pub const LEFT_FIRST: usize = 0;
pub const RIGHT_FIRST: usize = 1;

pub static DEF: ModuleDef = ModuleDef {
    name: "Ping Pong",
    prefix: "PINGPONG",
    hint: "echoes bouncing L ⇄ R",
    inputs: &["IN"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["L → R", "R → L"],
    toggle: Some("SYNC"),
    toggle_default: true,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: 0,
};

/// FEEDBACK knob to loop gain: just short of endless at the top.
pub fn loop_gain(feedback: f32) -> f32 {
    feedback.clamp(0.0, 1.0) * 0.98
}

pub struct PingPong {
    params: Arc<ModuleParams>,
    sr: f32,
    lines: [DelayLine; 2],
    time: f32,
    glide: f32,
    lp: [f32; 2],
    hp: [f32; 2],
    /// Input level, for DUCK.
    env: f32,
    duck: f32,
    mix: f32,
    width: f32,
}

impl PingPong {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        let len = ((MAX_SYNC_TIME + 0.05) * sample_rate) as usize;
        Self {
            time: 0.0,
            mix: params.get(MIX),
            width: params.get(WIDTH),
            params,
            sr: sample_rate,
            lines: [DelayLine::new(len), DelayLine::new(len)],
            glide: time_coeff(0.08, sample_rate),
            lp: [0.0; 2],
            hp: [0.0; 2],
            env: 0.0,
            duck: 1.0,
        }
    }
}

impl Processor for PingPong {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let time_t = delay_seconds(p.toggled(), p.get(TIME), p.get(DIV), seconds_per_beat(ctx));
        if self.time == 0.0 {
            self.time = time_t;
        }
        let fb = loop_gain(p.get(FEEDBACK));
        let tone = p.get(TONE);
        let duck_amount = p.get(DUCK);
        let mix_t = p.get(MIX);
        let width_t = p.get(WIDTH);
        let first = usize::from(p.mode() != LEFT_FIRST);
        let input = inputs.get(0);

        let sr = self.sr;
        let lp_a = (TAU * 900.0 * fastmath::exp2(tone * 4.4) / sr).min(1.0);
        let hp_a = TAU * 60.0 / sr;
        // The follower rises fast and falls over ~250 ms; DUCK at full takes
        // the echoes down by about 24 dB under a loud input.
        let (attack, release) = (time_coeff(0.005, sr), time_coeff(0.25, sr));
        let max_d = (self.lines[0].len() - 4) as f32;

        let mut peak = 0.0f32;
        for i in 0..n {
            approach(&mut self.time, time_t, self.glide);
            approach(&mut self.mix, mix_t, ctx.smooth);
            approach(&mut self.width, width_t, ctx.smooth);
            let d = (self.time * sr).clamp(1.0, max_d);

            let (in_l, in_r) = match input {
                Some(b) => (b[0][i], b[1][i]),
                None => (0.0, 0.0),
            };
            let mono = 0.5 * (in_l + in_r);
            let wet = [self.lines[0].tap(d - 1.0), self.lines[1].tap(d - 1.0)];

            // Each line feeds the other, darkened and without rumble.
            let mut ret = [0.0; 2];
            for ch in 0..2 {
                self.lp[ch] += lp_a * (wet[ch] - self.lp[ch]);
                self.hp[ch] += hp_a * (self.lp[ch] - self.hp[ch]);
                ret[ch] = self.lp[ch] - self.hp[ch];
            }
            let mut feed = [fb * ret[1], fb * ret[0]];
            feed[first] += mono;
            self.lines[0].push(fastmath::tanh(feed[0]));
            self.lines[1].push(fastmath::tanh(feed[1]));

            // Ducking: the echoes step back while the input plays.
            let level = mono.abs();
            let k = if level > self.env { attack } else { release };
            self.env += (level - self.env) * k;
            let duck_t = 1.0 - duck_amount * (self.env * 4.0).min(1.0) * 0.94;
            approach(&mut self.duck, duck_t, ctx.smooth);

            // WIDTH 0 folds both sides to the middle.
            let (side, other) = (0.5 + 0.5 * self.width, 0.5 - 0.5 * self.width);
            let wl = wet[0] * side + wet[1] * other;
            let wr = wet[1] * side + wet[0] * other;
            let (dry_g, wet_g) = mix_gains(self.mix);
            let wet_g = wet_g * self.duck;
            let yl = soft_limit(in_l * dry_g + wl * wet_g);
            let yr = soft_limit(in_r * dry_g + wr * wet_g);
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak = peak.max(yl.abs()).max(yr.abs());
        }
        p.meter.fetch_max(peak);
    }
}
