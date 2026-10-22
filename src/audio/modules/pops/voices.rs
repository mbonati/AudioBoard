//! The fifteen Pops 66 instruments, synthesized the way a 1966 rhythm box
//! made them: ringing resonators (damped sines) for the drums, bongos,
//! congas, claves and rim; square waves for the cowbell; filtered noise for
//! snares, hihat, cymbals and tambourine; gated bursts for the guiro and a
//! decaying rattle for the quijada.
//!
//! Three knobs reshape every voice's waveform: WAVE takes the oscillators
//! from sine through triangle to square (the noise instruments, from noise
//! to a metallic cluster of square waves), BEND sweeps the pitch at the
//! start of the hit, and DRIVE saturates it.

use crate::audio::fastmath;
use crate::audio::modules::Rng;

pub const BASE: usize = 0;
pub const SNARE1: usize = 1;
pub const SNARE2: usize = 2;
pub const BONGO_S: usize = 3;
pub const BONGO_L: usize = 4;
pub const CONGA: usize = 5;
pub const COWBELL: usize = 6;
pub const CLAVES: usize = 7;
pub const HIHAT: usize = 8;
pub const RIMSHOT: usize = 9;
pub const CYMBAL1: usize = 10;
pub const CYMBAL2: usize = 11;
pub const TAMBOURINE: usize = 12;
pub const GUIRO: usize = 13;
pub const QUIJADA: usize = 14;
pub const COUNT: usize = 15;

/// Whether the instrument rings with an oscillator (WAVE shapes it) or is
/// made of noise (WAVE turns the noise metallic).
pub const fn tonal(kind: usize) -> bool {
    !matches!(kind, HIHAT | CYMBAL1 | CYMBAL2 | TAMBOURINE | GUIRO | QUIJADA)
}

/// A voice's sound settings, read once per block: TUNE, ATTACK, DECAY,
/// three extras whose meaning depends on the instrument, then the waveform.
#[derive(Clone, Copy, Default)]
pub struct Sound {
    pub tune: f32,
    pub attack: f32,
    pub decay: f32,
    pub x: [f32; 3],
    /// 0 – 1: sine, triangle, square (or noise to metal).
    pub wave: f32,
    /// Octaves above (or below) TUNE where the hit starts.
    pub bend: f32,
    /// 0 – 1.
    pub drive: f32,
}

/// One cycle of the WAVE morph at `phase` (turns): a sine at 0, a triangle
/// at 0.5, then ever steeper sides up to a square at 1.
#[inline]
pub fn shape(phase: f32, wave: f32) -> f32 {
    if wave <= 0.0 {
        return fastmath::sin_turns(phase);
    }
    let tri = 1.0 - 4.0 * ((phase + 0.25).fract() - 0.5).abs();
    if wave < 0.5 {
        let sine = fastmath::sin_turns(phase);
        sine + (tri - sine) * wave * 2.0
    } else {
        (tri / (1.0 - (wave - 0.5) * 1.9)).clamp(-1.0, 1.0)
    }
}

/// The metallic source's six square waves, relative to the fifth (the
/// TR-808 cymbal's ratios).
const METAL: [f32; 6] = [0.380, 0.564, 0.684, 0.968, 1.0, 1.481];
/// Where they start, spread so the hit doesn't open on a jump.
const METAL_PHASES: [f32; 6] = [0.0, 0.31, 0.62, 0.13, 0.47, 0.78];

/// A 2-pole state-variable filter (Simper's).
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    fn tune(&mut self, f: f32, q: f32, sr: f32) {
        let g = fastmath::tan_turns(f.clamp(20.0, sr * 0.45) / (2.0 * sr));
        self.k = 1.0 / q.max(0.3);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// (low, band, high).
    #[inline]
    fn tick(&mut self, x: f32) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - self.k * v1 - v2)
    }
}

/// Attack ramp, then an exponential fall reaching −60 dB after `decay`.
#[derive(Clone, Copy, Default)]
struct Env {
    level: f32,
    rising: bool,
    rise: f32,
    fall: f32,
}

impl Env {
    fn start(&mut self, attack: f32, decay: f32, sr: f32) {
        self.rising = true;
        self.rise = 1.0 / (attack.max(0.0002) * sr);
        self.fall = (-6.907_755 / (decay.max(0.002) * sr)).exp();
    }

    #[inline]
    fn tick(&mut self) -> f32 {
        if self.rising {
            self.level += self.rise;
            if self.level >= 1.0 {
                self.level = 1.0;
                self.rising = false;
            }
        } else {
            self.level *= self.fall;
        }
        self.level
    }

    fn done(&self) -> bool {
        !self.rising && self.level < 1e-4
    }
}

/// An oscillator dying away: a ringing resonator (a sine), or whatever
/// WAVE makes of it.
#[derive(Clone, Copy, Default)]
struct Osc {
    phase: f32,
    inc: f32,
    env: Env,
}

impl Osc {
    fn start(&mut self, f: f32, attack: f32, decay: f32, sr: f32) {
        self.phase = 0.0;
        self.inc = f / sr;
        self.env.start(attack, decay, sr);
    }

    /// `pitch`: BEND's factor on the frequency.
    #[inline]
    fn tick(&mut self, wave: f32, pitch: f32) -> f32 {
        let e = self.env.tick();
        self.phase = (self.phase + self.inc * pitch).fract();
        shape(self.phase, wave) * e
    }
}

/// One instrument's sound generator.
#[derive(Clone, Default)]
pub struct Voice {
    kind: usize,
    active: bool,
    velocity: f32,
    sr: f32,
    tone: Osc,
    /// Second partial (conga, cowbell).
    tone2: Osc,
    noise: Env,
    click: f32,
    bp: Svf,
    hp: Svf,
    /// The filters' tuning before BEND: band-pass frequency and Q,
    /// high-pass frequency (0: unused).
    bp_f: f32,
    bp_q: f32,
    hp_f: f32,
    /// Guiro: open while > 0 (samples), pulse phase. Quijada: rattle phase.
    gate: f32,
    pulse: f32,
    rate: f32,
    density: f32,
    /// Second partial's level (conga, cowbell).
    level2: f32,
    /// Tambourine jingle: a held random level, and how often it changes.
    jingle: f32,
    jingle_phase: f32,
    jingle_inc: f32,
    wave: f32,
    /// The metallic source's phases, and the fifth one's step.
    metal: [f32; 6],
    metal_inc: f32,
    /// BEND: octaves at the start, how much of them is left, the fall per
    /// sample, and samples since the filters were last retuned.
    bend: f32,
    bend_left: f32,
    bend_fall: f32,
    retune: u32,
    /// DRIVE: gain into the saturator (0: off) and the gain after it.
    drive: f32,
    drive_norm: f32,
}

impl Voice {
    pub fn new(kind: usize) -> Self {
        Self { kind, ..Self::default() }
    }

    /// Strikes the instrument. `gate` (seconds) is how long the guiro
    /// scrapes.
    pub fn trigger(&mut self, s: &Sound, velocity: f32, gate: f32, sr: f32) {
        self.active = true;
        self.velocity = velocity;
        self.sr = sr;
        let (tune, attack, decay, x) = (s.tune, s.attack, s.decay, s.x);
        self.wave = s.wave.clamp(0.0, 1.0);
        self.metal = METAL_PHASES;
        self.metal_inc = tune * 0.07 / sr;
        self.bend = s.bend;
        self.bend_left = if s.bend != 0.0 { 1.0 } else { 0.0 };
        // The sweep takes a fifth of the decay, within 4 – 80 ms.
        self.bend_fall = (-1.0 / ((decay * 0.2).clamp(0.004, 0.08) * sr)).exp();
        self.retune = 0;
        // tanh(g·x) / tanh(g) is x itself for a small g, so DRIVE fades in.
        let g = 10.0 * s.drive.clamp(0.0, 1.0);
        (self.drive, self.drive_norm) = if g > 0.0 { (g, 1.0 / g.tanh()) } else { (0.0, 1.0) };
        let (mut bp, mut hp) = ((0.0, 0.7), 0.0);
        match self.kind {
            BASE | BONGO_S | BONGO_L | CLAVES => self.tone.start(tune, attack, decay, sr),
            SNARE1 | SNARE2 => {
                // RESO, TICK, HIGHP: a short drum tone under filtered noise.
                self.tone.start(tune, attack, decay * 0.35, sr);
                self.noise.start(attack, decay, sr);
                self.click = x[1];
                bp = (tune * 8.0, 0.7 + x[0] * 4.0);
                hp = x[2];
            }
            CONGA => {
                // LEVEL2, TUNE2, DECAY2: a second skin mode.
                self.tone.start(tune, attack, decay, sr);
                self.tone2.start(x[1], attack, x[2], sr);
                self.level2 = x[0];
            }
            COWBELL => {
                self.level2 = x[0];
                self.tone.start(tune, attack, decay, sr);
                self.tone2.start(x[1], attack, x[2], sr);
                bp = ((tune * x[1]).sqrt(), 2.5);
            }
            HIHAT => {
                self.noise.start(attack, decay, sr);
                bp = (tune, 0.8 + x[0] * 6.0);
                hp = tune * 0.6;
            }
            RIMSHOT => {
                self.tone.start(tune, attack, decay, sr);
                self.noise.start(attack, decay * 0.5, sr);
                hp = tune * 0.8;
            }
            CYMBAL1 | CYMBAL2 | TAMBOURINE => {
                self.noise.start(attack, decay, sr);
                bp = (tune, 0.6 + x[0] * 6.0);
                hp = x[1];
                self.jingle_phase = 0.0;
                self.jingle = 1.0;
                self.jingle_inc = 180.0 / sr;
            }
            GUIRO => {
                self.gate = gate * sr;
                self.pulse = 0.0;
                self.rate = x[0] / sr;
                self.density = x[1];
                bp = (tune, 4.0);
            }
            _ => {
                // Quijada: teeth rattling against the jaw.
                self.noise.start(attack, decay, sr);
                self.pulse = 0.0;
                self.rate = x[0] / sr;
                self.density = x[1];
                bp = (tune, 14.0);
            }
        }
        (self.bp_f, self.bp_q, self.hp_f) = (bp.0, bp.1, hp);
        self.tune_filters(fastmath::exp2(self.bend * self.bend_left));
    }

    /// The filters at their tuning times BEND's `pitch`.
    fn tune_filters(&mut self, pitch: f32) {
        if self.bp_f > 0.0 {
            self.bp.tune(self.bp_f * pitch, self.bp_q, self.sr);
        }
        if self.hp_f > 0.0 {
            self.hp.tune(self.hp_f * pitch, 0.7, self.sr);
        }
    }

    /// Lets go of a gated instrument (the guiro, played from MIDI).
    pub fn release(&mut self) {
        if self.kind == GUIRO {
            self.gate = self.gate.min(64.0);
        }
    }

    /// The noise instruments' source: white noise, turning (WAVE) into the
    /// metallic square waves.
    #[inline]
    fn source(&mut self, rng: &mut Rng, pitch: f32) -> f32 {
        let n = rng.bipolar();
        if self.wave <= 0.0 {
            return n;
        }
        let mut s = 0.0;
        for (phase, ratio) in self.metal.iter_mut().zip(METAL) {
            *phase = (*phase + self.metal_inc * ratio * pitch).fract();
            s += if *phase < 0.5 { 1.0 } else { -1.0 };
        }
        n + (s * (1.4 / 6.0) - n) * self.wave
    }

    /// One sample, mono.
    #[inline]
    pub fn tick(&mut self, rng: &mut Rng) -> f32 {
        if !self.active {
            return 0.0;
        }
        // BEND: the pitch glides back to TUNE; the filters follow it every
        // 16 samples.
        let pitch = if self.bend_left > 1e-4 {
            self.bend_left *= self.bend_fall;
            let m = fastmath::exp2(self.bend * self.bend_left);
            self.retune += 1;
            if self.retune >= 16 {
                self.retune = 0;
                self.tune_filters(m);
            }
            m
        } else {
            1.0
        };
        let (v, wave) = (self.velocity, self.wave);
        let y = match self.kind {
            BASE => self.tone.tick(wave, pitch) * 1.2,
            BONGO_S | BONGO_L => self.tone.tick(wave, pitch) * 0.8,
            CLAVES => self.tone.tick(wave, pitch) * 0.7,
            SNARE1 | SNARE2 => {
                let n = rng.bipolar() * self.noise.tick();
                let (_, band, _) = self.bp.tick(n);
                let (_, _, high) = self.hp.tick(n * 0.6 + band * 0.8);
                let tick = self.click;
                self.click *= 0.97;
                self.tone.tick(wave, pitch) * 0.5 + high * 0.9 + tick * rng.bipolar() * 0.8
            }
            CONGA => self.tone.tick(wave, pitch) * 0.8 + self.tone2.tick(wave, pitch) * 0.6 * self.level2,
            COWBELL => {
                let s = self.tone.tick(wave, pitch) + self.tone2.tick(wave, pitch) * self.level2;
                self.bp.tick(s).1 * 0.9
            }
            HIHAT => {
                let n = self.source(rng, pitch) * self.noise.tick();
                let (_, band, _) = self.bp.tick(n);
                self.hp.tick(band + n * 0.3).2 * 0.8
            }
            RIMSHOT => {
                let n = rng.bipolar() * self.noise.tick();
                self.tone.tick(wave, pitch) * 0.6 + self.hp.tick(n).2 * 0.5
            }
            CYMBAL1 | CYMBAL2 | TAMBOURINE => {
                let e = self.noise.tick();
                let mut n = self.source(rng, pitch) * e;
                if self.kind == TAMBOURINE {
                    // Jingles: the level jumps about ~180 times a second.
                    self.jingle_phase += self.jingle_inc;
                    if self.jingle_phase >= 1.0 {
                        self.jingle_phase -= 1.0;
                        self.jingle = 0.4 + 0.6 * rng.unit();
                    }
                    n *= self.jingle;
                }
                let high = self.hp.tick(n).2;
                let band = self.bp.tick(high).1;
                (high * 0.5 + band * 0.7) * 0.8
            }
            GUIRO => {
                if self.gate <= 0.0 {
                    self.active = self.bp.ic1.abs() > 1e-5;
                    0.0
                } else {
                    self.gate -= 1.0;
                    self.pulse = (self.pulse + self.rate).fract();
                    // Each ridge: a burst of noise for DENSITY of the cycle.
                    let on = self.pulse < 0.1 + self.density * 0.6;
                    let n = if on { self.source(rng, pitch) } else { 0.0 };
                    self.bp.tick(n).1 * 1.2
                }
            }
            _ => {
                let e = self.noise.tick();
                self.pulse += self.rate * (1.0 + (self.density - 0.5) * rng.bipolar());
                let mut kick = 0.0;
                if self.pulse >= 1.0 {
                    self.pulse -= 1.0;
                    kick = e * (0.6 + 0.4 * rng.unit());
                }
                // WAVE brings the metal up under the rattle.
                let bed = self.source(rng, pitch) * e * (0.05 * self.density + 0.3 * wave);
                self.bp.tick(kick * 6.0 + bed).1 * 0.9
            }
        };
        // Silent again: stop until the next hit.
        let done = match self.kind {
            BASE | BONGO_S | BONGO_L | CLAVES | CONGA | COWBELL => self.tone.env.done() && self.tone2.env.done(),
            GUIRO => false,
            _ => self.noise.done() && self.tone.env.done(),
        };
        if done {
            self.active = false;
        }
        let y = if self.drive > 0.0 { fastmath::tanh(y * self.drive) * self.drive_norm } else { y };
        y * v
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}
