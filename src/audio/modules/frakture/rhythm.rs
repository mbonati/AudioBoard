//! GATE, a trance gate: the steps that are off drop the sound out, with an
//! attack and release per step. REPEAT, a stutter: once per TRIGGER period
//! (OFFSET into it) it takes a slice of the input RATE long and plays it
//! AMOUNT more times, ducking the dry signal meanwhile.

use super::{Band, Clock, GATE_BINS, GATE_MAX, SLICE_BINS, viz};
use crate::audio::engine::Buf;
use crate::audio::modules::{ModuleParams, approach, time_coeff};

pub struct GateSettings {
    pub steps: usize,
    pub step_beats: f64,
    /// Fractions of a step.
    pub attack: f32,
    pub release: f32,
    pub mix: f32,
    pub pattern: [bool; GATE_MAX],
}

pub struct Gate {
    env: f32,
    /// Position in the pattern (0…1), and levels along it.
    pos: f32,
    levels_in: [f32; GATE_BINS],
    levels_out: [f32; GATE_BINS],
    bin: usize,
}

/// One-pole coefficient reaching ~98% in `samples`.
fn ramp(samples: f64) -> f32 {
    1.0 - (-4.0 / samples.max(24.0)).exp() as f32
}

impl Gate {
    pub fn new() -> Self {
        Self {
            env: 1.0,
            pos: 0.0,
            levels_in: [0.0; GATE_BINS],
            levels_out: [0.0; GATE_BINS],
            bin: 0,
        }
    }

    #[allow(clippy::needless_range_loop)] // both channels, sample by sample
    pub fn process(&mut self, x: &mut Buf, n: usize, s: &GateSettings, clock: &Clock) {
        let step = s.step_beats * clock.samples_per_beat;
        let (k_attack, k_release) = (ramp(s.attack as f64 * step), ramp(s.release as f64 * step));
        let steps = s.steps as f64;
        for i in 0..n {
            let pos = (clock.at(i) / s.step_beats).rem_euclid(steps);
            let target = if s.pattern[(pos as usize).min(s.steps - 1)] { 1.0 } else { 0.0 };
            let k = if target > self.env { k_attack } else { k_release };
            self.env += (target - self.env) * k;
            let g = 1.0 - s.mix * (1.0 - self.env);

            self.pos = (pos / steps) as f32;
            let bin = ((self.pos * GATE_BINS as f32) as usize).min(GATE_BINS - 1);
            if bin != self.bin {
                self.bin = bin;
                self.levels_in[bin] = 0.0;
                self.levels_out[bin] = 0.0;
            }
            let a = x[0][i].abs().max(x[1][i].abs());
            self.levels_in[bin] = self.levels_in[bin].max(a);
            self.levels_out[bin] = self.levels_out[bin].max(a * g);
            x[0][i] *= g;
            x[1][i] *= g;
        }
    }

    pub fn publish(&self, p: &ModuleParams) {
        p.viz[viz::GATE_POS].set(self.pos);
        for k in 0..GATE_BINS {
            p.viz[viz::GATE_IN + k].set(self.levels_in[k]);
            p.viz[viz::GATE_OUT + k].set(self.levels_out[k]);
        }
    }
}

pub struct RepeatSettings {
    pub lo: f32,
    pub hi: f32,
    pub amount: usize,
    pub trigger_beats: f64,
    pub offset: f32,
    pub duck: f32,
    /// Slice length in samples.
    pub slice: usize,
    pub volume: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Idle,
    /// Recording the slice: frames so far.
    Taking(usize),
    /// Playing it again: position, and repeats finished.
    Repeating(usize, usize),
}

/// Longest slice, in seconds.
const MAX_SLICE: f32 = 4.0;

pub struct Repeat {
    sr: f32,
    smooth: f32,
    /// Interleaved L/R.
    slice: Box<[f32]>,
    cap: usize,
    len: usize,
    amount: usize,
    state: State,
    /// The last trigger period seen.
    period: i64,
    duck: f32,
    volume: f32,
    band: Band,
    levels: [f32; SLICE_BINS],
}

impl Repeat {
    pub fn new(sample_rate: f32) -> Self {
        let cap = (MAX_SLICE * sample_rate) as usize;
        Self {
            sr: sample_rate,
            smooth: time_coeff(0.004, sample_rate),
            slice: vec![0.0; cap * 2].into_boxed_slice(),
            cap,
            len: 1,
            amount: 1,
            state: State::Idle,
            period: i64::MIN,
            duck: 1.0,
            volume: 1.0,
            band: Band::default(),
            levels: [0.0; SLICE_BINS],
        }
    }

    #[allow(clippy::needless_range_loop)] // both channels, sample by sample
    pub fn process(&mut self, x: &mut Buf, n: usize, s: &RepeatSettings, clock: &Clock) {
        self.band.set(s.lo, s.hi, self.sr);
        for i in 0..n {
            let period = (clock.at(i) / s.trigger_beats - s.offset as f64).floor() as i64;
            if period != self.period {
                let first = self.period == i64::MIN;
                self.period = period;
                if !first {
                    self.len = s.slice.clamp(64, self.cap);
                    self.amount = s.amount;
                    self.state = State::Taking(0);
                    self.levels = [0.0; SLICE_BINS];
                }
            }
            let (in_l, in_r) = (x[0][i], x[1][i]);
            let (mut yl, mut yr) = (0.0, 0.0);
            let mut repeating = false;
            match self.state {
                State::Taking(k) => {
                    self.slice[k * 2] = in_l;
                    self.slice[k * 2 + 1] = in_r;
                    let bin = k * SLICE_BINS / self.len;
                    self.levels[bin] = self.levels[bin].max(in_l.abs().max(in_r.abs()));
                    self.state = if k + 1 >= self.len { State::Repeating(0, 0) } else { State::Taking(k + 1) };
                }
                State::Repeating(k, done) => {
                    // Short fades at each repeat's ends, against clicks.
                    let fade = (self.len / 4).clamp(1, 96) as f32;
                    let g = ((k + 1) as f32 / fade).min((self.len - k) as f32 / fade).min(1.0);
                    yl = self.slice[k * 2] * g;
                    yr = self.slice[k * 2 + 1] * g;
                    repeating = true;
                    self.state = if k + 1 < self.len {
                        State::Repeating(k + 1, done)
                    } else if done + 1 < self.amount {
                        State::Repeating(0, done + 1)
                    } else {
                        State::Idle
                    };
                }
                State::Idle => {}
            }
            approach(&mut self.duck, if repeating { 1.0 - s.duck } else { 1.0 }, self.smooth);
            approach(&mut self.volume, s.volume, self.smooth);
            x[0][i] = in_l * self.duck + self.band.process(0, yl) * self.volume;
            x[1][i] = in_r * self.duck + self.band.process(1, yr) * self.volume;
        }
    }

    pub fn publish(&self, p: &ModuleParams) {
        let (state, count, progress) = match self.state {
            State::Idle => (0.0, 0.0, 0.0),
            State::Taking(k) => (1.0, 0.0, k as f32 / self.len as f32),
            State::Repeating(k, done) => (2.0, (done + 1) as f32, k as f32 / self.len as f32),
        };
        p.viz[viz::REP_STATE].set(state);
        p.viz[viz::REP_COUNT].set(count);
        p.viz[viz::REP_PROGRESS].set(progress);
        for (k, v) in self.levels.iter().enumerate() {
            p.viz[viz::REP_SLICE + k].set(*v);
        }
    }
}
