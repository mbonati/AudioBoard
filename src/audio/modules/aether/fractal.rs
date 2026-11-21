//! Fractal motion and the timbres it moves through.
//!
//! The motion is value noise summed over octaves (fractional Brownian
//! motion): each octave moves twice as fast as the one below and is `gain`
//! times as large, so the curve looks alike at every time scale — slow
//! swells carry faster ripples, which carry faster ones still. ROUGH sets
//! the gain: low, the slow swells rule; high, the fine detail shows.
//!
//! It is a pure function of the position (in periods of the slowest
//! octave) and a seed: the same position always gives the same value, so
//! Start replays the same evolution, and synced to the bars every octave
//! turns on a bar line or a fraction of one. [`Lane`] caches the knots it
//! needs, so the audio thread only hashes when a cell is crossed.

/// Octaves of motion: the slowest takes a whole EVOLVE period per cell,
/// the fastest 1/32 of it.
pub const OCTAVES: usize = 6;

/// A hash of (seed, lane, octave, cell) into [-1, 1).
#[inline]
fn knot(seed: u32, lane: u32, octave: u32, cell: i64) -> f32 {
    let mut h = seed
        ^ lane.wrapping_mul(0x9E37_79B9)
        ^ octave.wrapping_mul(0x85EB_CA6B)
        ^ (cell as u32).wrapping_mul(0xC2B2_AE35)
        ^ ((cell >> 32) as u32).wrapping_mul(0x27D4_EB2F);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    (h >> 8) as f32 / (1u32 << 23) as f32 - 1.0
}

/// Catmull-Rom through four knots at `t` in [0, 1): smooth, never stalling
/// on a knot. Can overshoot [-1, 1] a little.
#[inline]
fn spline([a, b, c, d]: [f32; 4], t: f32) -> f32 {
    let t2 = t * t;
    0.5 * (2.0 * b
        + (c - a) * t
        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
        + (3.0 * (b - c) + d - a) * t2 * t)
}

/// The knots around cell `cell` of one octave.
fn knots(seed: u32, lane: u32, octave: u32, cell: i64) -> [f32; 4] {
    std::array::from_fn(|k| knot(seed, lane, octave, cell - 1 + k as i64))
}

/// Octave `o` of lane `lane` at position `x` (no cache: for the panel).
pub fn octave(seed: u32, lane: u32, o: usize, x: f64) -> f32 {
    let x = x * (1u64 << o) as f64;
    let cell = x.floor();
    spline(knots(seed, lane, o as u32, cell as i64), (x - cell) as f32)
}

/// Octaves summed, each `gain` times the one below, normalised to about
/// [-1, 1] (no cache: for the panel).
pub fn fbm(seed: u32, lane: u32, x: f64, gain: f32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    for o in 0..OCTAVES {
        sum += amp * octave(seed, lane, o, x);
        norm += amp;
        amp *= gain;
    }
    (sum / norm).clamp(-1.0, 1.0)
}

/// Each octave's gain for a ROUGH value.
pub fn gain(rough: f32) -> f32 {
    0.32 + 0.52 * rough.clamp(0.0, 1.0)
}

/// One strand of motion, with its knots cached.
#[derive(Clone)]
pub struct Lane {
    lane: u32,
    seed: u32,
    cells: [i64; OCTAVES],
    knots: [[f32; 4]; OCTAVES],
}

impl Lane {
    pub fn new(lane: u32) -> Self {
        Self {
            lane,
            seed: 0,
            cells: [i64::MIN; OCTAVES],
            knots: [[0.0; 4]; OCTAVES],
        }
    }

    /// The motion at `x`, as [`fbm`] gives it, each octave weighted (0
    /// leaves it out: motion too fast to drift).
    pub fn value(&mut self, seed: u32, x: f64, gain: f32, weights: &[f32; OCTAVES]) -> f32 {
        if seed != self.seed {
            self.seed = seed;
            self.cells = [i64::MIN; OCTAVES];
        }
        let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
        for (o, &weight) in weights.iter().enumerate() {
            if weight <= 0.0 {
                amp *= gain;
                continue;
            }
            let xo = x * (1u64 << o) as f64;
            let cell = xo.floor();
            let c = cell as i64;
            if c != self.cells[o] {
                // Moving on by one cell (the usual case) reuses three knots.
                self.knots[o] = if c == self.cells[o].wrapping_add(1) {
                    let [_, b, cc, d] = self.knots[o];
                    [b, cc, d, knot(seed, self.lane, o as u32, c + 2)]
                } else {
                    knots(seed, self.lane, o as u32, c)
                };
                self.cells[o] = c;
            }
            let w = amp * weight;
            sum += w * spline(self.knots[o], (xo - cell) as f32);
            norm += w;
            amp *= gain;
        }
        (sum / norm.max(1e-6)).clamp(-1.0, 1.0)
    }
}

/// Partials per voice.
pub const PARTIALS: usize = 8;

/// The timbres COLOR moves through, as partial amplitudes: WARM (a soft
/// flute), SILK (strings, every partial at 1/k), HOLLOW (a reed, odd
/// partials) and CHOIR (a vowel: the 2nd and 3rd partials up, a formant
/// round the 6th).
pub const SPECTRA: [(&str, [f32; PARTIALS]); 4] = [
    ("WARM", [1.0, 0.38, 0.14, 0.06, 0.03, 0.015, 0.008, 0.004]),
    ("SILK", [1.0, 0.5, 0.333, 0.25, 0.2, 0.167, 0.143, 0.125]),
    ("HOLLOW", [1.0, 0.06, 0.36, 0.05, 0.21, 0.04, 0.15, 0.03]),
    ("CHOIR", [0.55, 1.0, 0.8, 0.32, 0.26, 0.48, 0.22, 0.09]),
];

/// The partial amplitudes at `color` (0…1 across [`SPECTRA`]).
pub fn spectrum(color: f32) -> [f32; PARTIALS] {
    let at = color.clamp(0.0, 1.0) * (SPECTRA.len() - 1) as f32;
    let i = (at as usize).min(SPECTRA.len() - 2);
    let t = at - i as f32;
    let (a, b) = (&SPECTRA[i].1, &SPECTRA[i + 1].1);
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// The nearest timbre's name.
pub fn spectrum_name(color: f32) -> &'static str {
    SPECTRA[((color.clamp(0.0, 1.0) * (SPECTRA.len() - 1) as f32).round() as usize)
        .min(SPECTRA.len() - 1)]
    .0
}
