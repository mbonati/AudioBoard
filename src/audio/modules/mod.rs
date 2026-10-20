//! Modules for ambient, drone and granular sound: generators, texture
//! processors and effects. Each one is described by a [`ModuleDef`] (knobs,
//! ports, optional mode selector and toggle), stores its settings in the
//! generic [`ModuleParams`] and has a single stereo output.

pub mod delay;
pub mod drone;
pub mod filter;
pub mod granular;
pub mod midi_cc;
pub mod midi_note;
pub mod random;
pub mod resonator;
pub mod reverb;

use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::AudioHandle;
use super::engine::Processor;
use super::params::{AtomicF32, ParamSpec};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ModuleKind {
    Drone,
    Random,
    Granular,
    Resonator,
    Filter,
    Delay,
    Reverb,
    MidiCc,
    MidiNote,
}

/// Static description of a module's panel and ports.
pub struct ModuleDef {
    /// Name in menus ("Reverb").
    pub name: &'static str,
    /// Node title prefix ("REVERB" → "REVERB 1").
    pub prefix: &'static str,
    /// Short description shown next to the name in menus.
    pub hint: &'static str,
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    pub params: &'static [ParamSpec],
    /// `(param, input)`: knobs that set how much a CV input modulates.
    pub cv_depth: &'static [(usize, usize)],
    /// Labels of the segmented mode selector (empty: none).
    pub modes: &'static [&'static str],
    /// Label of the on/off switch, if any.
    pub toggle: Option<&'static str>,
    /// Can drive any knob directly (MIDI nodes): shows a MAP button.
    pub map: bool,
    /// Number of values the processor publishes for the node's display.
    pub viz: usize,
}

impl ModuleKind {
    pub const ALL: [ModuleKind; 9] = [
        ModuleKind::Drone,
        ModuleKind::Random,
        ModuleKind::Granular,
        ModuleKind::Resonator,
        ModuleKind::Filter,
        ModuleKind::Delay,
        ModuleKind::Reverb,
        ModuleKind::MidiCc,
        ModuleKind::MidiNote,
    ];

    pub fn def(self) -> &'static ModuleDef {
        match self {
            ModuleKind::Drone => &drone::DEF,
            ModuleKind::Random => &random::DEF,
            ModuleKind::Granular => &granular::DEF,
            ModuleKind::Resonator => &resonator::DEF,
            ModuleKind::Filter => &filter::DEF,
            ModuleKind::Delay => &delay::DEF,
            ModuleKind::Reverb => &reverb::DEF,
            ModuleKind::MidiCc => &midi_cc::DEF,
            ModuleKind::MidiNote => &midi_note::DEF,
        }
    }

    /// Builds the audio-thread processor. Called on the UI thread, so
    /// buffers can be allocated here.
    pub fn processor(self, params: Arc<ModuleParams>, audio: &AudioHandle) -> Box<dyn Processor> {
        let sample_rate = audio.info.sample_rate as f32;
        let midi = audio.midi.clone();
        match self {
            ModuleKind::Drone => Box::new(drone::Drone::new(params, sample_rate)),
            ModuleKind::Random => Box::new(random::Random::new(params, sample_rate)),
            ModuleKind::Granular => Box::new(granular::Granular::new(params, sample_rate)),
            ModuleKind::Resonator => Box::new(resonator::Resonator::new(params, sample_rate)),
            ModuleKind::Filter => Box::new(filter::Filter::new(params, sample_rate)),
            ModuleKind::Delay => Box::new(delay::Delay::new(params, sample_rate)),
            ModuleKind::Reverb => Box::new(reverb::Reverb::new(params, sample_rate)),
            ModuleKind::MidiCc => Box::new(midi_cc::MidiCc::new(params, midi, sample_rate)),
            ModuleKind::MidiNote => Box::new(midi_note::MidiNote::new(params, midi, sample_rate)),
        }
    }
}

/// Settings shared lock-free between the UI and a module's processor.
pub struct ModuleParams {
    pub kind: ModuleKind,
    values: Box<[AtomicF32]>,
    mode: AtomicU32,
    toggle: AtomicBool,
    /// Output peak since the UI last looked.
    pub meter: AtomicF32,
    /// Values published by the processor for the node's display.
    pub viz: Box<[AtomicF32]>,
}

impl ModuleParams {
    pub fn new(kind: ModuleKind) -> Self {
        let def = kind.def();
        Self {
            kind,
            values: def.params.iter().map(|p| AtomicF32::new(p.default)).collect(),
            mode: AtomicU32::new(0),
            toggle: AtomicBool::new(false),
            meter: AtomicF32::new(0.0),
            viz: (0..def.viz).map(|_| AtomicF32::new(0.0)).collect(),
        }
    }

    pub fn get(&self, i: usize) -> f32 {
        self.values[i].get()
    }

    pub fn set(&self, i: usize, v: f32) {
        self.values[i].set(v);
    }

    pub fn mode(&self) -> usize {
        self.mode.load(Ordering::Relaxed) as usize
    }

    pub fn set_mode(&self, m: usize) {
        self.mode.store(m as u32, Ordering::Relaxed);
    }

    pub fn toggled(&self) -> bool {
        self.toggle.load(Ordering::Relaxed)
    }

    pub fn set_toggle(&self, on: bool) {
        self.toggle.store(on, Ordering::Relaxed);
    }

    /// A fresh copy of the settings (not the meters).
    pub fn duplicate(&self) -> Self {
        let p = Self::new(self.kind);
        for (dst, src) in p.values.iter().zip(&self.values) {
            dst.set(src.get());
        }
        p.set_mode(self.mode());
        p.set_toggle(self.toggled());
        p
    }
}

// ---------------------------------------------------------------------------
// Shared DSP helpers
// ---------------------------------------------------------------------------

/// xorshift32: cheap, allocation-free randomness for the audio thread.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in [-1, 1).
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

/// A value wandering smoothly in [-1, 1]: cosine interpolation between
/// random points. Drives the slow "drift" that makes sounds feel alive.
pub struct Wander {
    from: f32,
    to: f32,
    t: f32,
}

impl Wander {
    pub fn new(rng: &mut Rng) -> Self {
        Self {
            from: rng.bipolar(),
            to: rng.bipolar(),
            t: rng.unit(),
        }
    }

    /// Advances by `step` (fraction of one segment) and returns the value.
    pub fn tick(&mut self, rng: &mut Rng, step: f32) -> f32 {
        self.t += step;
        if self.t >= 1.0 {
            self.t -= self.t.floor();
            self.from = self.to;
            self.to = rng.bipolar();
        }
        let s = 0.5 - 0.5 * (self.t * PI).cos();
        self.from + (self.to - self.from) * s
    }
}

/// Circular buffer with fractional reads.
pub struct DelayLine {
    buf: Box<[f32]>,
    mask: usize,
    pos: usize,
}

impl DelayLine {
    pub fn new(min_len: usize) -> Self {
        let len = min_len.max(4).next_power_of_two();
        Self {
            buf: vec![0.0; len].into_boxed_slice(),
            mask: len - 1,
            pos: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn push(&mut self, x: f32) {
        self.pos = (self.pos + 1) & self.mask;
        self.buf[self.pos] = x;
    }

    /// The sample written `d` pushes ago (0 = the latest), linearly
    /// interpolated. A feedback delay of `D` samples reads `tap(D - 1)`
    /// before pushing.
    pub fn tap(&self, d: f32) -> f32 {
        let d = d.clamp(0.0, (self.buf.len() - 2) as f32);
        let i = d as usize;
        let f = d - i as f32;
        let a = self.buf[self.pos.wrapping_sub(i) & self.mask];
        let b = self.buf[self.pos.wrapping_sub(i + 1) & self.mask];
        a + (b - a) * f
    }
}

/// One-pole smoothing towards `target`.
#[inline]
pub fn approach(value: &mut f32, target: f32, k: f32) {
    *value += (target - *value) * k;
}

/// One-pole coefficient for a time constant in seconds.
pub fn time_coeff(seconds: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (seconds * sample_rate).max(1.0)).exp()
}

/// Equal-power (cos, sin) gains for a pan position in [-1, 1].
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let theta = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (theta.cos(), theta.sin())
}

/// Equal-power dry/wet gains.
pub fn mix_gains(mix: f32) -> (f32, f32) {
    let theta = mix.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
    (theta.cos(), theta.sin())
}

/// Transparent below 0.9, then bends smoothly towards ±1. Keeps module
/// outputs in range without colouring normal levels.
#[inline]
pub fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * ((a - 0.9) * 10.0).tanh())
    }
}

/// Linear below 1, then saturates (to 1.5 at most). Guards feedback loops.
#[inline]
pub fn soft_ceiling(x: f32) -> f32 {
    let a = x.abs();
    if a <= 1.0 {
        x
    } else {
        x.signum() * (1.0 + 0.5 * ((a - 1.0) * 2.0).tanh())
    }
}
