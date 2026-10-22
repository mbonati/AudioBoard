//! Granular: records the input into an 8-second loop and plays it back as a
//! cloud of short, enveloped grains. FREEZE stops recording so the cloud
//! keeps scanning the captured sound; the POS input sweeps through it.

use std::sync::Arc;

use super::{ModuleDef, ModuleParams, Rng, approach, mix_gains, pan_gains, soft_limit, Quiet, silence};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::params::{ParamSpec, Unit};

pub const POSITION: usize = 0;
pub const SIZE: usize = 1;
pub const DENSITY: usize = 2;
pub const PITCH: usize = 3;
pub const JITTER: usize = 4;
pub const SPRAY: usize = 5;
pub const SPREAD: usize = 6;
pub const MIX: usize = 7;

/// How far back POS can reach, in seconds.
pub const MAX_POSITION: f32 = 6.0;

const PARAMS: [ParamSpec; 8] = [
    ParamSpec::lin("POS", 0.0, MAX_POSITION, 0.5, Unit::Seconds),
    ParamSpec::log("SIZE", 0.01, 1.0, 0.15, Unit::Seconds),
    ParamSpec::log("DENSITY", 1.0, 100.0, 14.0, Unit::PerSecond),
    ParamSpec::lin("PITCH", -24.0, 24.0, 0.0, Unit::Semitones).bipolar(),
    ParamSpec::lin("JITTER", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("SPRAY", 0.0, 12.0, 0.0, Unit::Semitones),
    ParamSpec::lin("SPREAD", 0.0, 1.0, 0.7, Unit::Percent),
    ParamSpec::lin("MIX", 0.0, 1.0, 0.8, Unit::Percent),
];

pub const FORWARD: usize = 0;
pub const REVERSE: usize = 1;

/// Grains shown in the display, and resolution of the buffer overview.
pub const VIZ_GRAINS: usize = 24;
pub const OVERVIEW: usize = 128;

/// Layout of `ModuleParams::viz`.
pub mod viz {
    use super::{OVERVIEW, VIZ_GRAINS};
    /// Number of grains playing.
    pub const COUNT: usize = 0;
    /// Write position as a fraction of the loop.
    pub const HEAD: usize = 1;
    /// Loop length in seconds.
    pub const LOOP_SECONDS: usize = 2;
    /// Seconds behind the write head of each shown grain (negative: none).
    pub const GRAIN_AGE: usize = 3;
    /// Current envelope level of each shown grain.
    pub const GRAIN_AMP: usize = GRAIN_AGE + VIZ_GRAINS;
    /// Peak level per section of the loop.
    pub const OVERVIEW_AT: usize = GRAIN_AMP + VIZ_GRAINS;
    pub const LEN: usize = OVERVIEW_AT + OVERVIEW;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Granular",
    prefix: "GRAINS",
    hint: "live grain cloud",
    inputs: &["IN", "POS"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["FWD", "REV", "BOTH"],
    toggle: Some("FREEZE"),
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

const MAX_GRAINS: usize = 64;
const WINDOW: usize = 512;
const LOOP_SECONDS: f32 = 8.0;

#[derive(Clone, Copy, Default)]
struct Grain {
    active: bool,
    /// Read position in absolute sample time (see `Granular::write`).
    pos: f64,
    inc: f64,
    age: u32,
    len: u32,
    /// Window entries per sample of age (`WINDOW / len`).
    win_step: f32,
    gl: f32,
    gr: f32,
}

pub struct Granular {
    params: Arc<ModuleParams>,
    quiet: Quiet,
    sr: f32,
    buf: Box<[f32]>,
    mask: usize,
    /// Samples recorded so far; the newest sits at `write - 1`.
    write: u64,
    grains: [Grain; MAX_GRAINS],
    /// One past the highest slot that may be playing.
    top: usize,
    countdown: f32,
    rng: Rng,
    window: [f32; WINDOW + 1],
    norm: f32,
    mix: f32,
    overview: [f32; OVERVIEW],
    bin_len: usize,
    bin: usize,
}

impl Granular {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        let len = ((LOOP_SECONDS * sample_rate) as usize).next_power_of_two();
        let window = std::array::from_fn(|i| {
            let x = i as f32 / WINDOW as f32;
            0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
        });
        params.viz[viz::LOOP_SECONDS].set(len as f32 / sample_rate);
        Self {
            quiet: Quiet::new(len),
            mix: params.get(MIX),
            params,
            sr: sample_rate,
            buf: vec![0.0; len].into_boxed_slice(),
            mask: len - 1,
            write: 0,
            grains: [Grain::default(); MAX_GRAINS],
            top: 0,
            countdown: 0.0,
            rng: Rng::new(0x6A41_0001),
            window,
            norm: 1.0,
            overview: [0.0; OVERVIEW],
            bin_len: len / OVERVIEW,
            bin: 0,
        }
    }

    fn read(&self, pos: f64) -> f32 {
        let i = pos.floor();
        let f = (pos - i) as f32;
        let i = i as i64 as usize; // two's complement wraps correctly under the mask
        let a = self.buf[i & self.mask];
        let b = self.buf[i.wrapping_add(1) & self.mask];
        a + (b - a) * f
    }

    fn envelope(&self, g: &Grain) -> f32 {
        let x = g.age as f32 * g.win_step;
        let i = (x as usize).min(WINDOW - 1);
        let f = x - i as f32;
        self.window[i] + (self.window[i + 1] - self.window[i]) * f
    }

    fn spawn(&mut self, frozen: bool, position: f32, settings: &Settings) {
        let Some(slot) = self.grains.iter().position(|g| !g.active) else {
            return;
        };
        let sr = self.sr;
        let rng = &mut self.rng;
        let len = (settings.size * (1.0 + 0.5 * settings.jitter * rng.bipolar()) * sr).max(32.0);
        let semis = settings.pitch + settings.spray * rng.bipolar();
        let ratio = (semis / 12.0).exp2();
        let reverse = match settings.direction {
            FORWARD => false,
            REVERSE => true,
            _ => rng.unit() < 0.5,
        };

        // Distance behind the write head changes by `slope` per sample;
        // keep the whole grain inside the loop and behind the head.
        let head_speed = if frozen { 0.0 } else { 1.0 };
        let slope = head_speed + if reverse { ratio } else { -ratio };
        let ring = self.buf.len() as f32;
        let lo = 2.0 + (-slope * len).max(0.0);
        let hi = ring - 4.0 - (slope * len).max(0.0);
        if lo > hi {
            return;
        }
        let scatter = settings.jitter * rng.unit() * (0.25 * sr + len);
        let d0 = (position * sr + scatter + 2.0).clamp(lo, hi);
        let (gl, gr) = pan_gains(settings.spread * rng.bipolar());

        self.grains[slot] = Grain {
            active: true,
            pos: self.write as f64 - 1.0 - d0 as f64,
            inc: if reverse { -ratio } else { ratio } as f64,
            age: 0,
            len: len as u32,
            win_step: WINDOW as f32 / len as u32 as f32,
            gl,
            gr,
        };
        self.top = self.top.max(slot + 1);
    }

    fn publish(&self) {
        let viz = &self.params.viz;
        let mut shown = 0;
        let mut count = 0;
        let head = self.write as f64 - 1.0;
        for g in self.grains.iter().filter(|g| g.active) {
            count += 1;
            if shown < VIZ_GRAINS {
                viz[viz::GRAIN_AGE + shown].set(((head - g.pos) / self.sr as f64) as f32);
                viz[viz::GRAIN_AMP + shown].set(self.envelope(g));
                shown += 1;
            }
        }
        for k in shown..VIZ_GRAINS {
            viz[viz::GRAIN_AGE + k].set(-1.0);
        }
        viz[viz::COUNT].set(count as f32);
        viz[viz::HEAD].set((self.write as usize & self.mask) as f32 / self.buf.len() as f32);
        for (k, v) in self.overview.iter().enumerate() {
            viz[viz::OVERVIEW_AT + k].set(*v);
        }
    }
}

struct Settings {
    size: f32,
    pitch: f32,
    jitter: f32,
    spray: f32,
    spread: f32,
    direction: usize,
}

impl Processor for Granular {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let frozen = p.toggled();
        let settings = Settings {
            size: p.get(SIZE),
            pitch: p.get(PITCH).round(),
            jitter: p.get(JITTER),
            spray: p.get(SPRAY),
            spread: p.get(SPREAD),
            direction: p.mode(),
        };
        let density = p.get(DENSITY);
        let position = p.get(POSITION);
        let mix_t = p.get(MIX);
        let input = inputs.get(0);
        let silent_in = Quiet::silent(input, n);
        if silent_in && self.quiet.resting() {
            // Gone quiet: nothing to compute until input returns.
            silence(outputs, n);
            return;
        }
        let pos_cv = inputs.cv(1);
        // Overlapping grains add up: keep the cloud's level steady.
        let norm_t = 1.8 / (density * settings.size).max(1.0).sqrt();

        let mut peak = 0.0f32;
        // The effect's own signal, for resting.
        let mut inner = 0.0f32;
        for i in 0..n {
            approach(&mut self.mix, mix_t, ctx.smooth);
            approach(&mut self.norm, norm_t, ctx.smooth);
            let (dry_l, dry_r) = match input {
                Some(b) => (b[0][i], b[1][i]),
                None => (0.0, 0.0),
            };

            if !frozen {
                let idx = self.write as usize & self.mask;
                let x = 0.5 * (dry_l + dry_r);
                self.buf[idx] = x;
                let bin = idx / self.bin_len;
                if bin != self.bin {
                    self.bin = bin;
                    self.overview[bin] = 0.0;
                }
                self.overview[bin] = self.overview[bin].max(x.abs());
                self.write += 1;
            }

            self.countdown -= 1.0;
            if self.countdown <= 0.0 {
                let gap = self.sr / density * (1.0 + settings.jitter * 0.9 * self.rng.bipolar());
                self.countdown += gap.max(1.0);
                // POS input: ±1 sweeps across the whole reachable range.
                let cv = pos_cv.map_or(0.0, |c| c[i].clamp(-1.0, 1.0));
                let pos = (position + cv * MAX_POSITION * 0.5).clamp(0.0, MAX_POSITION);
                self.spawn(frozen, pos, &settings);
            }

            // Only the slots up to the highest one playing.
            let (mut wl, mut wr) = (0.0, 0.0);
            let mut top = 0;
            for k in 0..self.top {
                let g = self.grains[k];
                if !g.active {
                    continue;
                }
                let s = self.read(g.pos) * self.envelope(&g);
                wl += s * g.gl;
                wr += s * g.gr;
                let g = &mut self.grains[k];
                g.pos += g.inc;
                g.age += 1;
                g.active = g.age < g.len;
                if g.active {
                    top = k + 1;
                }
            }
            self.top = top;

            let (dry_g, wet_g) = mix_gains(self.mix);
            let wet = self.norm * wet_g;
            inner = inner.max(wl.abs()).max(wr.abs());
            let yl = soft_limit(dry_l * dry_g + wl * wet);
            let yr = soft_limit(dry_r * dry_g + wr * wet);
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak = peak.max(yl.abs()).max(yr.abs());
        }
        self.publish();
        self.quiet.update(silent_in, inner, n);
        p.meter.fetch_max(peak);
    }
}
