//! DSP helpers every node can share. None of them allocates or locks after
//! construction, so all are safe on the audio thread.

use crate::buffer::{BLOCK, Buf};
use crate::fastmath;

/// xorshift32: cheap, allocation-free randomness for the audio thread.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
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
    #[inline]
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
    /// Allocates: build it in the node's constructor, not while processing.
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

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Silence, without reallocating.
    pub fn clear(&mut self) {
        self.buf.fill(0.0);
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
        Self {
            quiet: 0,
            memory: memory.max(32 * BLOCK),
        }
    }

    /// No input, or a silent one.
    pub fn silent(input: Option<&Buf>, n: usize) -> bool {
        input.is_none_or(|b| b[0][..n].iter().chain(&b[1][..n]).all(|s| s.abs() < 1e-6))
    }

    /// After a block of `n` samples: was its input silent, and how loud was
    /// the effect's own signal.
    pub fn update(&mut self, input_silent: bool, peak: f32, n: usize) {
        self.quiet = if input_silent && peak < 1e-5 {
            self.quiet.saturating_add(n)
        } else {
            0
        };
    }

    pub fn resting(&self) -> bool {
        self.quiet >= self.memory
    }
}

/// A parameter glided towards its target by a one-pole filter, per sample
/// or per block, so knob moves never click.
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    pub value: f32,
}

impl Smoothed {
    pub const fn new(value: f32) -> Self {
        Self { value }
    }

    /// One step towards `target` with coefficient `k` (see
    /// [`ProcessCtx::smooth`](crate::ProcessCtx::smooth), [`time_coeff`]).
    #[inline]
    pub fn next(&mut self, target: f32, k: f32) -> f32 {
        approach(&mut self.value, target, k);
        self.value
    }
}

/// One-pole low-pass (and, by difference, high-pass) filter.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePole {
    z: f32,
}

impl OnePole {
    /// Coefficient for a cutoff in Hz.
    pub fn coeff(hz: f32, sample_rate: f32) -> f32 {
        1.0 - fastmath::exp2(-std::f32::consts::TAU * hz / sample_rate * std::f32::consts::LOG2_E)
    }

    #[inline]
    pub fn lowpass(&mut self, x: f32, k: f32) -> f32 {
        self.z += (x - self.z) * k;
        self.z
    }

    #[inline]
    pub fn highpass(&mut self, x: f32, k: f32) -> f32 {
        x - self.lowpass(x, k)
    }

    pub fn reset(&mut self) {
        self.z = 0.0;
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

/// The loudest sample of the first `n` frames of a block.
#[inline]
pub fn peak(buf: &Buf, n: usize) -> f32 {
    buf[0][..n]
        .iter()
        .chain(&buf[1][..n])
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

/// One-pole smoothing towards `target`.
#[inline]
pub fn approach(value: &mut f32, target: f32, k: f32) {
    *value += (target - *value) * k;
}

/// One-pole coefficient for a time constant in seconds.
#[inline]
pub fn time_coeff(seconds: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (seconds * sample_rate).max(1.0)).exp()
}

/// Equal-power (cos, sin) gains for a pan position in [-1, 1].
#[inline]
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

/// Transparent below 0.9, then bends smoothly towards ±1. Keeps node
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

/// The frequency of a MIDI note (A4 = 69 = 440 Hz).
#[inline]
pub fn note_hz(note: f32) -> f32 {
    440.0 * fastmath::exp2((note - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_line_reads_back() {
        let mut d = DelayLine::new(10);
        for i in 0..16 {
            d.push(i as f32);
        }
        assert_eq!(d.tap_at(0), 15.0);
        assert_eq!(d.tap_at(3), 12.0);
        assert!((d.tap(2.5) - 12.5).abs() < 1e-6);
    }

    #[test]
    fn rng_stays_in_range() {
        let mut r = Rng::new(0);
        for _ in 0..10_000 {
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
            assert!((-1.0..1.0).contains(&r.bipolar()));
        }
    }

    #[test]
    fn helpers_behave() {
        let (l, r) = pan_gains(0.0);
        assert!((l - r).abs() < 1e-5 && (l * l + r * r - 1.0).abs() < 1e-4);
        assert!((note_hz(69.0) - 440.0).abs() < 1e-3);
        assert!(soft_limit(5.0) <= 1.0 && soft_limit(0.5) == 0.5);
        let mut lp = OnePole::default();
        let k = OnePole::coeff(100.0, 48_000.0);
        for _ in 0..48_000 {
            lp.lowpass(1.0, k);
        }
        assert!((lp.lowpass(1.0, k) - 1.0).abs() < 1e-3);
        let mut q = Quiet::new(0);
        for _ in 0..40 {
            q.update(true, 0.0, BLOCK);
        }
        assert!(q.resting());
    }
}
