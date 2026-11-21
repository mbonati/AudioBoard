//! Delay: stereo tape-style echo. Time changes glide (pitching the repeats),
//! WOW adds slow wobble and flutter, TONE darkens each repeat and PING
//! bounces the echoes between left and right. With SYNC the time is a note
//! value (DIV) at the global tempo.

use std::f32::consts::TAU;
use std::sync::Arc;

use super::{
    DelayLine, ModuleDef, ModuleParams, Quiet, approach, mix_gains, silence, soft_limit, time_coeff,
};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{DEFAULT_TEMPO, ParamSpec, SYNC_DIVISIONS, Unit, sync_division};

pub const TIME: usize = 0;
pub const DIV: usize = 1;
pub const FEEDBACK: usize = 2;
pub const TONE: usize = 3;
pub const WOW: usize = 4;
pub const PING: usize = 5;
pub const MIX: usize = 6;

pub const MAX_TIME: f32 = 2.0;
/// Longest synced time (a whole note at 60 BPM).
pub const MAX_SYNC_TIME: f32 = 4.0;

const PARAMS: [ParamSpec; 7] = [
    ParamSpec::log("TIME", 0.01, MAX_TIME, 0.42, Unit::Seconds),
    ParamSpec::lin(
        "DIV",
        0.0,
        (SYNC_DIVISIONS.len() - 1) as f32,
        8.0,
        Unit::Sync,
    ),
    ParamSpec::lin("FEEDBACK", 0.0, 1.0, 0.55, Unit::Percent),
    ParamSpec::lin("TONE", 0.0, 1.0, 0.55, Unit::Percent),
    ParamSpec::lin("WOW", 0.0, 1.0, 0.25, Unit::Percent),
    ParamSpec::lin("PING", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("MIX", 0.0, 1.0, 0.35, Unit::Percent),
];

pub static DEF: ModuleDef = ModuleDef {
    name: "Delay",
    prefix: "DELAY",
    hint: "tape echo",
    inputs: &["IN"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: Some("SYNC"),
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: 0,
};

/// The delay time in seconds: TIME, or with SYNC the DIV note value at the
/// tempo (`seconds_per_beat`).
pub fn delay_seconds(synced: bool, time: f32, div: f32, seconds_per_beat: f64) -> f32 {
    if synced {
        (sync_division(div).1 * seconds_per_beat).clamp(0.01, MAX_SYNC_TIME as f64) as f32
    } else {
        time
    }
}

/// Seconds per beat at the engine's tempo.
pub fn seconds_per_beat(ctx: &ProcessCtx) -> f64 {
    if ctx.beats_per_sample > 0.0 {
        1.0 / (ctx.beats_per_sample * ctx.sample_rate as f64)
    } else {
        60.0 / DEFAULT_TEMPO as f64
    }
}

/// FEEDBACK knob (0..1) to loop gain; the top of the range self-oscillates
/// gently, held in check by the loop's saturation.
pub fn loop_gain(feedback: f32) -> f32 {
    feedback.clamp(0.0, 1.0) * 1.02
}

pub struct Delay {
    params: Arc<ModuleParams>,
    quiet: Quiet,
    sr: f32,
    lines: [DelayLine; 2],
    time: f32,
    glide: f32,
    lp: [f32; 2],
    hp: [f32; 2],
    wow_phase: [f32; 2],
    mix: f32,
}

impl Delay {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        let len = ((MAX_SYNC_TIME + 0.05) * sample_rate) as usize;
        Self {
            quiet: Quiet::new(len),
            // Set on the first block (synced, it depends on the tempo).
            time: 0.0,
            mix: params.get(MIX),
            params,
            sr: sample_rate,
            lines: [DelayLine::new(len), DelayLine::new(len)],
            glide: time_coeff(0.12, sample_rate),
            lp: [0.0; 2],
            hp: [0.0; 2],
            wow_phase: [0.0, 0.37],
        }
    }
}

impl Processor for Delay {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let time_t = delay_seconds(p.toggled(), p.get(TIME), p.get(DIV), seconds_per_beat(ctx));
        if self.time == 0.0 {
            self.time = time_t;
        }
        let fb = loop_gain(p.get(FEEDBACK));
        let tone = p.get(TONE);
        let wow = p.get(WOW);
        let ping = p.get(PING);
        let mix_t = p.get(MIX);
        let input = inputs.get(0);
        let silent_in = Quiet::silent(input, n);
        if silent_in && self.quiet.resting() {
            // Gone quiet: nothing to compute until input returns.
            silence(outputs, n);
            return;
        }

        let sr = self.sr;
        let lp_a = (TAU * 800.0 * (tone * 4.6).exp2() / sr).min(1.0);
        let hp_a = TAU * 40.0 / sr;
        // Wow: slow drift (±4 ms); flutter: faster, smaller (±0.4 ms).
        let wow_depth = wow * 0.004 * sr;
        let flutter_depth = wow * 0.0004 * sr;
        let max_d = (self.lines[0].len() - 4) as f32;

        let mut peak = 0.0f32;
        // The effect's own signal, for resting.
        let mut inner = 0.0f32;
        for i in 0..n {
            approach(&mut self.time, time_t, self.glide);
            approach(&mut self.mix, mix_t, ctx.smooth);
            self.wow_phase[0] = (self.wow_phase[0] + 0.43 / sr).fract();
            self.wow_phase[1] = (self.wow_phase[1] + 5.3 / sr).fract();
            let modulation = if wow > 0.0 {
                wow_depth * fastmath::sin_turns(self.wow_phase[0])
                    + flutter_depth * fastmath::sin_turns(self.wow_phase[1])
            } else {
                0.0
            };
            let d = (self.time * sr + modulation).clamp(1.0, max_d);

            let (in_l, in_r) = match input {
                Some(b) => (b[0][i], b[1][i]),
                None => (0.0, 0.0),
            };
            let wet = [self.lines[0].tap(d - 1.0), self.lines[1].tap(d - 1.0)];
            inner = inner.max(wet[0].abs()).max(wet[1].abs());

            // Feedback path: darken, remove rumble, cross-feed for ping-pong.
            let mut ret = [0.0; 2];
            for ch in 0..2 {
                self.lp[ch] += lp_a * (wet[ch] - self.lp[ch]);
                self.hp[ch] += hp_a * (self.lp[ch] - self.hp[ch]);
                ret[ch] = self.lp[ch] - self.hp[ch];
            }
            let fb_l = fb * ((1.0 - ping) * ret[0] + ping * ret[1]);
            let fb_r = fb * ((1.0 - ping) * ret[1] + ping * ret[0]);
            // With PING the input enters on the left only, so echoes alternate.
            let mid = 0.5 * (in_l + in_r);
            let feed_l = in_l + (mid - in_l) * ping;
            let feed_r = in_r * (1.0 - ping);
            self.lines[0].push(fastmath::tanh(feed_l + fb_l));
            self.lines[1].push(fastmath::tanh(feed_r + fb_r));

            let (dry_g, wet_g) = mix_gains(self.mix);
            let yl = soft_limit(in_l * dry_g + wet[0] * wet_g);
            let yr = soft_limit(in_r * dry_g + wet[1] * wet_g);
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak = peak.max(yl.abs()).max(yr.abs());
        }
        self.quiet.update(silent_in, inner, n);
        p.meter.fetch_max(peak);
    }
}
