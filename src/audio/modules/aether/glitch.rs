//! Harmonic glitches: now and then, on the board's grid, a glitch lands on a
//! harmonic of a chord tone — never noise, always in tune.
//!
//! - PING: a glassy sine at the harmonic. A touch of phase modulation by
//!   itself (so only its own harmonics) fades as it rings: it starts like a
//!   bell and ends pure.
//! - SKIP: the sound sticks, like a CD skipping. The slice it loops is an
//!   octave multiple (or fraction) of the period of a harmonic of the root,
//!   so the loop itself sings at an octave of that harmonic.
//!
//! REPEAT ratchets a glitch in subdivisions of the step (in time, triplets
//! too). Half the ratchets climb: pings up the harmonic series, skips an
//! octave higher each time.

use super::super::steps::{Step, block_end, frame_of, roll};
use super::super::{Rng, pan_gains};
use crate::audio::engine::ProcessCtx;
use crate::audio::fastmath;

/// The harmonics a glitch may land on: the series without its sour members
/// (7, 11, 13, 14, 15).
pub const HARMONICS: [u32; 10] = [2, 3, 4, 5, 6, 8, 9, 10, 12, 16];

/// How many of [`HARMONICS`] a HARM value allows (those up to it).
pub fn allowed(harm: f32) -> usize {
    HARMONICS
        .iter()
        .filter(|&&h| h as f32 <= harm.round())
        .count()
        .max(1)
}

/// What a glitch was (as the panel is told).
pub const PING: u8 = 1;
pub const SKIP: u8 = 2;

const SPARKS: usize = 8;
pub const QUEUE: usize = 32;
/// Samples of the sound kept for skips to loop (170 ms at 48 kHz).
const RING: usize = 1 << 13;
/// The longest and shortest loops, and the crossfade that closes them.
pub const LOOP_MAX: usize = 4096;
const LOOP_MIN: usize = 160;
const XFADE: usize = 64;

/// What the knobs ask of the glitches this block.
pub struct Settings {
    /// The chance a step glitches.
    pub chance: f32,
    /// How many of [`HARMONICS`] may be used.
    pub allowed: usize,
    pub repeat: f32,
    /// The share of glitches that are skips.
    pub skip: f32,
    pub voices: usize,
}

/// A glitch waiting for its beat.
#[derive(Clone, Copy, Default, Debug)]
pub struct Pending {
    pub beat: f64,
    pub skip: bool,
    pub voice: u8,
    /// An index into [`HARMONICS`].
    pub harmonic: u8,
    /// Its level (ratchets fade).
    pub level: f32,
    /// Octaves up (climbing skips).
    pub warp: u8,
}

/// A glitch due in this block, at a frame.
#[derive(Clone, Copy, Default)]
pub struct Fire {
    pub frame: usize,
    pub what: Pending,
}

#[derive(Clone, Copy, Default)]
struct Spark {
    phase: f32,
    inc: f32,
    env: f32,
    fall: f32,
    attack: f32,
    rise: f32,
    gain: f32,
    gl: f32,
    gr: f32,
    index: f32,
}

impl Spark {
    #[inline]
    fn tick(&mut self) -> [f32; 2] {
        if self.env < 1e-5 {
            return [0.0; 2];
        }
        self.attack = (self.attack + self.rise).min(1.0);
        let pm = self.index * self.env * fastmath::sin_turns(self.phase);
        let s = fastmath::sin_turns(self.phase + pm) * self.env * self.attack * self.gain;
        self.env *= self.fall;
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        [s * self.gl, s * self.gr]
    }
}

/// A loop playing a captured slice: fades in, holds, fades out.
struct Head {
    buf: Box<[[f32; 2]]>,
    len: usize,
    pos: usize,
    env: f32,
    hold: usize,
    rise: f32,
    fall: f32,
    level: f32,
}

impl Head {
    fn new() -> Self {
        Self {
            buf: vec![[0.0; 2]; LOOP_MAX].into_boxed_slice(),
            len: 0,
            pos: 0,
            env: 0.0,
            hold: 0,
            rise: 0.0,
            fall: 0.0,
            level: 0.0,
        }
    }

    /// The loop's next sample and how much it covers the sound.
    #[inline]
    fn tick(&mut self) -> ([f32; 2], f32) {
        if self.len == 0 {
            return ([0.0; 2], 0.0);
        }
        if self.hold > 0 {
            self.hold -= 1;
            self.env = (self.env + self.rise).min(1.0);
        } else {
            self.env -= self.fall;
            if self.env <= 0.0 {
                self.env = 0.0;
                self.len = 0;
                return ([0.0; 2], 0.0);
            }
        }
        let s = self.buf[self.pos];
        self.pos += 1;
        if self.pos >= self.len {
            self.pos = 0;
        }
        let g = self.env * self.level;
        ([s[0] * g, s[1] * g], g)
    }
}

pub struct Glitch {
    sr: f32,
    sparks: [Spark; SPARKS],
    queue: [Pending; QUEUE],
    queued: usize,
    ring: Box<[[f32; 2]]>,
    ring_pos: usize,
    heads: [Head; 2],
    current: usize,
}

impl Glitch {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            sparks: [Spark::default(); SPARKS],
            queue: [Pending::default(); QUEUE],
            queued: 0,
            ring: vec![[0.0; 2]; RING].into_boxed_slice(),
            ring_pos: 0,
            heads: [Head::new(), Head::new()],
            current: 0,
        }
    }

    /// Forgets the glitches still to come (the ringing ones ring on).
    pub fn clear(&mut self) {
        self.queued = 0;
    }

    fn push(&mut self, p: Pending) {
        if self.queued < QUEUE {
            self.queue[self.queued] = p;
            self.queued += 1;
        }
    }

    /// Rolls the dice on a step `len` beats long: maybe a glitch, maybe
    /// ratcheted.
    pub fn step(&mut self, rng: &mut Rng, step: &Step, len: f64, s: &Settings) {
        if s.chance <= 0.0 || !roll(rng, s.chance) {
            return;
        }
        let skip = rng.unit() < s.skip;
        let voice = ((rng.unit() * s.voices as f32) as usize).min(s.voices.max(1) - 1) as u8;
        // Lower harmonics more often: they sit closer to the chord.
        let first = ((rng.unit() * rng.unit() * s.allowed as f32) as usize).min(s.allowed - 1);
        let (count, sub) = if rng.unit() < s.repeat {
            const SUBS: [usize; 5] = [2, 3, 4, 6, 8];
            let sub = SUBS[((rng.unit() * (1.0 + 4.0 * s.repeat)) as usize).min(SUBS.len() - 1)];
            // Very high REPEAT may run on into the next step.
            let steps = if rng.unit() < (s.repeat - 0.6) * 2.5 {
                2
            } else {
                1
            };
            (sub * steps, sub)
        } else {
            (1, 1)
        };
        let climb = count > 1 && rng.unit() < 0.5;
        for r in 0..count {
            let harmonic = if climb {
                (first + r).min(s.allowed - 1)
            } else {
                first
            };
            self.push(Pending {
                beat: step.beat + len * r as f64 / sub as f64,
                skip,
                voice,
                harmonic: harmonic as u8,
                level: 0.8f32.powi(r as i32),
                warp: if skip && climb { r.min(3) as u8 } else { 0 },
            });
        }
    }

    /// Takes the glitches due in this block into `out`, in order.
    pub fn due(&mut self, ctx: &ProcessCtx, n: usize, out: &mut [Fire; QUEUE]) -> usize {
        let end = block_end(ctx, n);
        let mut count = 0;
        let mut k = 0;
        while k < self.queued {
            if self.queue[k].beat < end {
                out[count] = Fire {
                    frame: frame_of(ctx, n, self.queue[k].beat),
                    what: self.queue[k],
                };
                count += 1;
                self.queued -= 1;
                self.queue[k] = self.queue[self.queued];
            } else {
                k += 1;
            }
        }
        out[..count].sort_unstable_by_key(|f| f.frame);
        count
    }

    /// A ping at `hz`, folded down by octaves into a warm range; returns
    /// the frequency it plays.
    pub fn ping(&mut self, rng: &mut Rng, hz: f32, level: f32, decay: f32, width: f32) -> f32 {
        let mut hz = hz;
        while hz > 7000.0 {
            hz *= 0.5;
        }
        let slot = (0..SPARKS)
            .min_by(|&a, &b| self.sparks[a].env.total_cmp(&self.sparks[b].env))
            .unwrap_or(0);
        let (gl, gr) = pan_gains(rng.bipolar() * width);
        self.sparks[slot] = Spark {
            phase: 0.0,
            inc: hz / self.sr,
            env: 1.0,
            fall: (-6.9 / (decay.max(0.01) * self.sr)).exp(),
            attack: 0.0,
            rise: 1.0 / (0.003 * self.sr),
            gain: level,
            gl,
            gr,
            index: 0.05 + 0.12 * rng.unit(),
        };
        hz
    }

    /// A skip at an octave of harmonic `harmonic` of `root_hz`, `warp`
    /// octaves higher still; returns the loop's pitch.
    pub fn skip(
        &mut self,
        rng: &mut Rng,
        root_hz: f32,
        harmonic: usize,
        warp: u8,
        level: f32,
        hold: f32,
    ) -> f32 {
        let period =
            self.sr / (root_hz.max(10.0) * HARMONICS[harmonic.min(HARMONICS.len() - 1)] as f32);
        // A slice of 10–60 ms, halved per warp, rounded to an octave of the period.
        let target = (0.01 + 0.05 * rng.unit()) * self.sr * 0.5f32.powi(warp as i32);
        let mut octaves = (target / period).log2().round() as i32;
        let len_at = |o: i32| period * 2f32.powi(o);
        while len_at(octaves) > LOOP_MAX as f32 && octaves > -12 {
            octaves -= 1;
        }
        while len_at(octaves) < LOOP_MIN as f32 && octaves < 12 {
            octaves += 1;
        }
        let len = (len_at(octaves).round() as usize).clamp(LOOP_MIN, LOOP_MAX);

        // The playing loop fades out quickly; the other takes the slice.
        let old = &mut self.heads[self.current];
        old.hold = 0;
        old.fall = 1.0 / (0.004 * self.sr);
        self.current = 1 - self.current;
        let head = &mut self.heads[self.current];
        let mask = RING - 1;
        // The last `len` samples; their end crossfades into the ones just
        // before them, so the wrap is seamless.
        let start = (self.ring_pos + RING + 1 - len) & mask;
        for (i, s) in head.buf[..len].iter_mut().enumerate() {
            *s = self.ring[(start + i) & mask];
        }
        let fade = XFADE.min(len / 2);
        for k in 0..fade {
            let w = (k as f32 + 0.5) / fade as f32;
            let before = self.ring[(start + RING - fade + k) & mask];
            for (s, b) in head.buf[len - fade + k].iter_mut().zip(before) {
                *s += (b - *s) * w;
            }
        }
        head.len = len;
        head.pos = 0;
        head.env = 0.0;
        head.hold = (hold.clamp(0.03, 1.5) * self.sr) as usize;
        head.rise = 1.0 / (0.002 * self.sr);
        head.fall = 1.0 / (0.02 * self.sr);
        head.level = level.clamp(0.0, 1.0);
        self.sr / len as f32
    }

    /// The pings' next sample.
    #[inline]
    pub fn sparks(&mut self) -> [f32; 2] {
        let mut out = [0.0f32; 2];
        for s in &mut self.sparks {
            let [l, r] = s.tick();
            out[0] += l;
            out[1] += r;
        }
        out
    }

    /// Records `x` and returns it with any loop playing over it.
    #[inline]
    pub fn skipping(&mut self, x: [f32; 2]) -> [f32; 2] {
        self.ring_pos = (self.ring_pos + 1) & (RING - 1);
        self.ring[self.ring_pos] = x;
        let mut wet = [0.0f32; 2];
        let mut cover = 0.0f32;
        for h in &mut self.heads {
            let (s, g) = h.tick();
            wet[0] += s[0];
            wet[1] += s[1];
            cover += g;
        }
        let dry = 1.0 - cover.min(1.0);
        [x[0] * dry + wet[0], x[1] * dry + wet[1]]
    }

    /// Anything still sounding.
    pub fn active(&self) -> bool {
        self.sparks.iter().any(|s| s.env >= 1e-5) || self.heads.iter().any(|h| h.len > 0)
    }
}
