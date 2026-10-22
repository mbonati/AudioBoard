//! The fifteen Pops 66 instruments, synthesized the way a 1966 rhythm box
//! made them: ringing resonators (damped sines) for the drums, bongos,
//! congas, claves and rim; square waves for the cowbell; filtered noise for
//! snares, hihat, cymbals and tambourine; gated bursts for the guiro and a
//! decaying rattle for the quijada.

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

/// A voice's sound settings, read once per block: TUNE, ATTACK, DECAY and
/// three extras whose meaning depends on the instrument.
#[derive(Clone, Copy, Default)]
pub struct Sound {
    pub tune: f32,
    pub attack: f32,
    pub decay: f32,
    pub x: [f32; 3],
}

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

/// A ringing resonator: a sine dying away.
#[derive(Clone, Copy, Default)]
struct Ring {
    phase: f32,
    inc: f32,
    env: Env,
}

impl Ring {
    fn start(&mut self, f: f32, attack: f32, decay: f32, sr: f32) {
        self.phase = 0.0;
        self.inc = f / sr;
        self.env.start(attack, decay, sr);
    }

    #[inline]
    fn tick(&mut self) -> f32 {
        let e = self.env.tick();
        self.phase = (self.phase + self.inc).fract();
        fastmath::sin_turns(self.phase) * e
    }
}

/// A square wave dying away (cowbell partials).
#[derive(Clone, Copy, Default)]
struct Square {
    phase: f32,
    inc: f32,
    env: Env,
}

impl Square {
    fn start(&mut self, f: f32, attack: f32, decay: f32, sr: f32) {
        self.inc = f / sr;
        self.env.start(attack, decay, sr);
    }

    #[inline]
    fn tick(&mut self) -> f32 {
        let e = self.env.tick();
        self.phase = (self.phase + self.inc).fract();
        if self.phase < 0.5 { e } else { -e }
    }
}

/// One instrument's sound generator.
#[derive(Clone, Default)]
pub struct Voice {
    kind: usize,
    active: bool,
    velocity: f32,
    tone: Ring,
    tone2: Ring,
    sq: [Square; 2],
    noise: Env,
    click: f32,
    bp: Svf,
    hp: Svf,
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
        let (tune, attack, decay, x) = (s.tune, s.attack, s.decay, s.x);
        match self.kind {
            BASE | BONGO_S | BONGO_L | CLAVES => self.tone.start(tune, attack, decay, sr),
            SNARE1 | SNARE2 => {
                // RESO, TICK, HIGHP: a short drum tone under filtered noise.
                self.tone.start(tune, attack, decay * 0.35, sr);
                self.noise.start(attack, decay, sr);
                self.click = x[1];
                self.bp.tune(tune * 8.0, 0.7 + x[0] * 4.0, sr);
                self.hp.tune(x[2], 0.7, sr);
            }
            CONGA => {
                // LEVEL2, TUNE2, DECAY2: a second skin mode.
                self.tone.start(tune, attack, decay, sr);
                self.tone2.start(x[1], attack, x[2], sr);
                self.level2 = x[0];
            }
            COWBELL => {
                self.level2 = x[0];
                self.sq[0].start(tune, attack, decay, sr);
                self.sq[1].start(x[1], attack, x[2], sr);
                self.bp.tune((tune * x[1]).sqrt(), 2.5, sr);
            }
            HIHAT => {
                self.noise.start(attack, decay, sr);
                self.bp.tune(tune, 0.8 + x[0] * 6.0, sr);
                self.hp.tune(tune * 0.6, 0.7, sr);
            }
            RIMSHOT => {
                self.tone.start(tune, attack, decay, sr);
                self.noise.start(attack, decay * 0.5, sr);
                self.hp.tune(tune * 0.8, 0.7, sr);
            }
            CYMBAL1 | CYMBAL2 | TAMBOURINE => {
                self.noise.start(attack, decay, sr);
                self.bp.tune(tune, 0.6 + x[0] * 6.0, sr);
                self.hp.tune(x[1], 0.7, sr);
                self.jingle_phase = 0.0;
                self.jingle = 1.0;
                self.jingle_inc = 180.0 / sr;
            }
            GUIRO => {
                self.gate = gate * sr;
                self.pulse = 0.0;
                self.rate = x[0] / sr;
                self.density = x[1];
                self.bp.tune(tune, 4.0, sr);
            }
            _ => {
                // Quijada: teeth rattling against the jaw.
                self.noise.start(attack, decay, sr);
                self.pulse = 0.0;
                self.rate = x[0] / sr;
                self.density = x[1];
                self.bp.tune(tune, 14.0, sr);
            }
        }
    }

    /// Lets go of a gated instrument (the guiro, played from MIDI).
    pub fn release(&mut self) {
        if self.kind == GUIRO {
            self.gate = self.gate.min(64.0);
        }
    }

    /// One sample, mono.
    #[inline]
    pub fn tick(&mut self, rng: &mut Rng) -> f32 {
        if !self.active {
            return 0.0;
        }
        let v = self.velocity;
        let y = match self.kind {
            BASE => self.tone.tick() * 1.2,
            BONGO_S | BONGO_L => self.tone.tick() * 0.8,
            CLAVES => self.tone.tick() * 0.7,
            SNARE1 | SNARE2 => {
                let n = rng.bipolar() * self.noise.tick();
                let (_, band, _) = self.bp.tick(n);
                let (_, _, high) = self.hp.tick(n * 0.6 + band * 0.8);
                let tick = self.click;
                self.click *= 0.97;
                self.tone.tick() * 0.5 + high * 0.9 + tick * rng.bipolar() * 0.8
            }
            CONGA => self.tone.tick() * 0.8 + self.tone2.tick() * 0.6 * self.level2,
            COWBELL => {
                let s = self.sq[0].tick() + self.sq[1].tick() * self.level2;
                self.bp.tick(s).1 * 0.9
            }
            HIHAT => {
                let n = rng.bipolar() * self.noise.tick();
                let (_, band, _) = self.bp.tick(n);
                self.hp.tick(band + n * 0.3).2 * 0.8
            }
            RIMSHOT => {
                let n = rng.bipolar() * self.noise.tick();
                self.tone.tick() * 0.6 + self.hp.tick(n).2 * 0.5
            }
            CYMBAL1 | CYMBAL2 | TAMBOURINE => {
                let e = self.noise.tick();
                let mut n = rng.bipolar() * e;
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
                    let n = if on { rng.bipolar() } else { 0.0 };
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
                self.bp.tick(kick * 6.0 + rng.bipolar() * e * 0.05 * self.density).1 * 0.9
            }
        };
        // Silent again: stop until the next hit.
        let done = match self.kind {
            BASE | BONGO_S | BONGO_L | CLAVES | CONGA => self.tone.env.done() && self.tone2.env.done(),
            COWBELL => self.sq[0].env.done() && self.sq[1].env.done(),
            GUIRO => false,
            _ => self.noise.done() && self.tone.env.done(),
        };
        if done {
            self.active = false;
        }
        y * v
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}
