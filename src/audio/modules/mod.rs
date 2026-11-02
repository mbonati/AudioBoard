//! Modules for ambient, drone and granular sound: generators, texture
//! processors and effects. Each one is described by a [`ModuleDef`] (knobs,
//! ports, optional mode selector and toggle), stores its settings in the
//! generic [`ModuleParams`] and has a single stereo output.

pub mod arpeggiator;
pub mod audio_in;
pub mod automa;
pub mod delay;
pub mod drone;
pub mod equalizer;
pub mod filter;
pub mod frakture;
pub mod granular;
pub mod magma;
pub mod midi_cc;
pub mod midi_in;
pub mod midicer;
pub mod pingpong;
pub mod pops;
pub mod midi_note;
pub mod nullpunkt;
pub mod oversample;
pub mod random;
pub mod resonator;
pub mod reverb;
pub mod sequencer;
pub mod steps;
pub mod tape;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::AudioHandle;
use super::fastmath;
use super::engine::{Buf, Processor};
use super::params::{AtomicF32, ParamSpec};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ModuleKind {
    Drone,
    Random,
    Pops,
    Nullpunkt,
    AudioIn,
    Granular,
    Frakture,
    Resonator,
    Filter,
    Magma,
    Equalizer,
    Tape,
    Delay,
    PingPong,
    Reverb,
    MidiCc,
    MidiNote,
    Sequencer,
    Arpeggiator,
    MidiIn,
    Midicer,
    Automa,
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
    /// The switch starts on.
    pub toggle_default: bool,
    /// Can drive any knob directly (MIDI nodes): shows a MAP button.
    pub map: bool,
    /// Takes MIDI on a MIDI cable.
    pub midi_in: bool,
    /// Sends MIDI on a MIDI cable.
    pub midi_out: bool,
    /// Extra values kept with the settings (a sequencer's steps), with
    /// their defaults.
    pub data: &'static [f32],
    /// Height of the display, in node units.
    pub display_h: f32,
    /// Height of extra controls under the display (buttons), if any.
    pub controls_h: f32,
    /// Number of values the processor publishes for the node's display.
    pub viz: usize,
}

impl ModuleKind {
    pub const ALL: [ModuleKind; 22] = [
        ModuleKind::Drone,
        ModuleKind::Random,
        ModuleKind::Pops,
        ModuleKind::Nullpunkt,
        ModuleKind::AudioIn,
        ModuleKind::Granular,
        ModuleKind::Frakture,
        ModuleKind::Resonator,
        ModuleKind::Filter,
        ModuleKind::Magma,
        ModuleKind::Equalizer,
        ModuleKind::Tape,
        ModuleKind::Delay,
        ModuleKind::PingPong,
        ModuleKind::Reverb,
        ModuleKind::MidiCc,
        ModuleKind::MidiNote,
        ModuleKind::Sequencer,
        ModuleKind::Arpeggiator,
        ModuleKind::MidiIn,
        ModuleKind::Midicer,
        ModuleKind::Automa,
    ];

    pub fn def(self) -> &'static ModuleDef {
        match self {
            ModuleKind::Drone => &drone::DEF,
            ModuleKind::Random => &random::DEF,
            ModuleKind::Pops => &pops::DEF,
            ModuleKind::Nullpunkt => &nullpunkt::DEF,
            ModuleKind::AudioIn => &audio_in::DEF,
            ModuleKind::Granular => &granular::DEF,
            ModuleKind::Frakture => &frakture::DEF,
            ModuleKind::Resonator => &resonator::DEF,
            ModuleKind::Filter => &filter::DEF,
            ModuleKind::Equalizer => &equalizer::DEF,
            ModuleKind::Magma => &magma::DEF,
            ModuleKind::Tape => &tape::DEF,
            ModuleKind::Delay => &delay::DEF,
            ModuleKind::PingPong => &pingpong::DEF,
            ModuleKind::Reverb => &reverb::DEF,
            ModuleKind::MidiCc => &midi_cc::DEF,
            ModuleKind::MidiNote => &midi_note::DEF,
            ModuleKind::Sequencer => &sequencer::DEF,
            ModuleKind::Arpeggiator => &arpeggiator::DEF,
            ModuleKind::MidiIn => &midi_in::DEF,
            ModuleKind::Midicer => &midicer::DEF,
            ModuleKind::Automa => &automa::DEF,
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
            ModuleKind::Pops => Box::new(pops::Pops::new(params, sample_rate)),
            ModuleKind::Nullpunkt => Box::new(nullpunkt::Nullpunkt::new(params, sample_rate)),
            ModuleKind::AudioIn => Box::new(audio_in::AudioIn::new(params, audio.shared.clone(), sample_rate)),
            ModuleKind::Granular => Box::new(granular::Granular::new(params, sample_rate)),
            ModuleKind::Frakture => Box::new(frakture::Frakture::new(params, sample_rate)),
            ModuleKind::Resonator => Box::new(resonator::Resonator::new(params, sample_rate)),
            ModuleKind::Filter => Box::new(filter::Filter::new(params, sample_rate)),
            ModuleKind::Equalizer => Box::new(equalizer::Equalizer::new(params, sample_rate)),
            ModuleKind::Magma => Box::new(magma::Magma::new(params, midi.clone(), sample_rate)),
            ModuleKind::Tape => Box::new(tape::Tape::new(params, sample_rate)),
            ModuleKind::Delay => Box::new(delay::Delay::new(params, sample_rate)),
            ModuleKind::PingPong => Box::new(pingpong::PingPong::new(params, sample_rate)),
            ModuleKind::Reverb => Box::new(reverb::Reverb::new(params, sample_rate)),
            ModuleKind::MidiCc => Box::new(midi_cc::MidiCc::new(params, midi, sample_rate)),
            ModuleKind::MidiNote => Box::new(midi_note::MidiNote::new(params, midi, sample_rate)),
            ModuleKind::Sequencer => Box::new(sequencer::Sequencer::new(params)),
            ModuleKind::Arpeggiator => Box::new(arpeggiator::Arpeggiator::new(params, midi)),
            ModuleKind::MidiIn => Box::new(midi_in::MidiIn::new(params, midi)),
            ModuleKind::Midicer => Box::new(midicer::Midicer::new(params, midi)),
            ModuleKind::Automa => Box::new(automa::Automa::new(params)),
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
    /// Extra settings beyond the knobs (`ModuleDef::data`), saved too.
    pub data: Box<[AtomicF32]>,
}

impl ModuleParams {
    pub fn new(kind: ModuleKind) -> Self {
        let def = kind.def();
        Self {
            kind,
            values: def.params.iter().map(|p| AtomicF32::new(p.default)).collect(),
            mode: AtomicU32::new(0),
            toggle: AtomicBool::new(def.toggle_default),
            meter: AtomicF32::new(0.0),
            viz: (0..def.viz).map(|_| AtomicF32::new(0.0)).collect(),
            data: def.data.iter().map(|&v| AtomicF32::new(v)).collect(),
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
        for (dst, src) in p.data.iter().zip(&self.data) {
            dst.set(src.get());
        }
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

    #[inline]
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in [-1, 1).
    #[inline]
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
        let s = 0.5 - 0.5 * fastmath::cos_turns(self.t * 0.5);
        self.from + (self.to - self.from) * s
    }
}

/// Circular buffer with fractional reads.
pub struct DelayLine {
    buf: Box<[f32]>,
    mask: usize,
    pos: usize,
    /// Longest delay `tap` accepts.
    max_d: f32,
}

impl DelayLine {
    pub fn new(min_len: usize) -> Self {
        let len = min_len.max(4).next_power_of_two();
        Self {
            buf: vec![0.0; len].into_boxed_slice(),
            mask: len - 1,
            pos: 0,
            max_d: (len - 2) as f32,
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    fn at(&self, i: usize) -> f32 {
        // SAFETY: the length is a power of two and `mask` is length − 1,
        // so the masked index is always in bounds.
        unsafe { *self.buf.get_unchecked(i & self.mask) }
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        self.pos = (self.pos + 1) & self.mask;
        // SAFETY: `pos` is masked as in `at`.
        unsafe { *self.buf.get_unchecked_mut(self.pos) = x };
    }

    /// The sample written `d` pushes ago (0 = the latest), for a whole
    /// number of samples: no interpolation.
    #[inline]
    pub fn tap_at(&self, d: usize) -> f32 {
        self.at(self.pos.wrapping_sub(d))
    }

    /// The sample written `d` pushes ago (0 = the latest), linearly
    /// interpolated. A feedback delay of `D` samples reads `tap(D - 1)`
    /// before pushing.
    #[inline]
    pub fn tap(&self, d: f32) -> f32 {
        let d = d.clamp(0.0, self.max_d);
        // Through u32: float to u64 conversion is slow on x86.
        let i = d as u32 as usize;
        let f = d - i as f32;
        let a = self.at(self.pos.wrapping_sub(i));
        let b = self.at(self.pos.wrapping_sub(i + 1));
        a + (b - a) * f
    }
}

/// Tells when an effect has gone quiet: its input silent and its own
/// signal (its echoes, its tail) below hearing (−100 dB) for as long as it
/// can remember sound (a delay line's length). Resting, it skips its work
/// and outputs silence until input returns, so idle effects cost nothing.
/// Anything that keeps sounding on its own (a frozen reverb, an endless
/// delay) never rests.
pub struct Quiet {
    /// Samples of silence so far, and how many make it rest.
    quiet: usize,
    memory: usize,
}

impl Quiet {
    /// Rests after `memory` samples of silence (and at least ~40 ms).
    pub fn new(memory: usize) -> Self {
        Self { quiet: 0, memory: memory.max(32 * crate::audio::engine::BLOCK) }
    }

    /// No input, or a silent one.
    pub fn silent(input: Option<&Buf>, n: usize) -> bool {
        input.is_none_or(|b| b[0][..n].iter().chain(&b[1][..n]).all(|s| s.abs() < 1e-6))
    }

    /// After a block of `n` samples: was its input silent, and how loud was
    /// the effect's own signal.
    pub fn update(&mut self, input_silent: bool, peak: f32, n: usize) {
        self.quiet = if input_silent && peak < 1e-5 { self.quiet.saturating_add(n) } else { 0 };
    }

    pub fn resting(&self) -> bool {
        self.quiet >= self.memory
    }
}

/// Silence on every output (for nodes that skip their work).
pub fn silence(outputs: &mut [Buf], n: usize) {
    for out in outputs.iter_mut() {
        for ch in out.iter_mut() {
            ch[..n].fill(0.0);
        }
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
    let turns = (pan.clamp(-1.0, 1.0) + 1.0) * 0.125;
    (fastmath::cos_turns(turns), fastmath::sin_turns(turns))
}

/// Equal-power dry/wet gains. Cheap enough to call per sample.
#[inline]
pub fn mix_gains(mix: f32) -> (f32, f32) {
    let turns = mix.clamp(0.0, 1.0) * 0.25;
    (fastmath::cos_turns(turns), fastmath::sin_turns(turns))
}

/// Transparent below 0.9, then bends smoothly towards ±1. Keeps module
/// outputs in range without colouring normal levels.
#[inline]
pub fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * fastmath::tanh((a - 0.9) * 10.0))
    }
}

/// Linear below 1, then saturates (to 1.5 at most). Guards feedback loops.
#[inline]
pub fn soft_ceiling(x: f32) -> f32 {
    let a = x.abs();
    if a <= 1.0 {
        x
    } else {
        x.signum() * (1.0 + 0.5 * fastmath::tanh((a - 1.0) * 2.0))
    }
}
