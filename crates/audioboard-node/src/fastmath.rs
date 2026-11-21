//! Cheap stand-ins for the transcendental functions used per sample. Each
//! is a short polynomial or rational function, several times faster than
//! the libm call and accurate far beyond what can be heard (see the tests).

use std::f32::consts::{LN_2, TAU};

/// sin(2π·t) for a phase `t` in turns (any value). Error below 4e-6.
#[inline]
pub fn sin_turns(t: f32) -> f32 {
    // Into [-0.5, 0.5], then folded into [-0.25, 0.25] without branches
    // (sin(2π·x) = sin(2π·(±0.5 − x))), so loops over voices vectorize.
    let x = t - t.round_ties_even();
    let folded = 0.25 - (x.abs() - 0.25).abs();
    let r = folded.copysign(x) * TAU;
    let r2 = r * r;
    r * (1.0
        + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 * (1.0 / 362_880.0)))))
}

/// cos(2π·t) for a phase in turns.
#[inline]
pub fn cos_turns(t: f32) -> f32 {
    sin_turns(t + 0.25)
}

/// tan(2π·t) for t in [0, 0.24] (filter prewarping).
#[inline]
pub fn tan_turns(t: f32) -> f32 {
    sin_turns(t) / cos_turns(t)
}

/// 2^x, relative error below 4e-7 (a few f32 ulps; far under a cent).
#[inline]
pub fn exp2(x: f32) -> f32 {
    let x = x.clamp(-126.0, 126.0);
    let whole = x.round_ties_even();
    let f = x - whole;
    // 2^f for f in [-0.5, 0.5]: Taylor series of e^(f·ln 2).
    let p = 1.0
        + f * (LN_2
            + f * (0.240_226_5
                + f * (0.055_504_11
                    + f * (0.009_618_129 + f * (0.001_333_355_8 + f * 0.000_154_035_3)))));
    p * f32::from_bits(((whole as i32 + 127) as u32) << 23)
}

/// tanh(x), absolute error below 1e-6.
#[inline]
pub fn tanh(x: f32) -> f32 {
    // 1 − 2 / (e^2x + 1), with e^2x = 2^(2x·log2 e).
    1.0 - 2.0 / (exp2(x * 2.885_39) + 1.0)
}

/// log2(x) for x > 0 (tiny or non-positive x give about −100), absolute
/// error below 1e-6: level detectors turn amplitudes into dB with it.
#[inline]
pub fn log2(x: f32) -> f32 {
    let bits = x.max(1e-30).to_bits();
    let mut e = ((bits >> 23) & 0xff) as i32 - 127;
    // Mantissa in [1, 2), folded into [√½, √2) to keep the series short.
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if m > std::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let t = (m - 1.0) / (m + 1.0);
    let t2 = t * t;
    // 2·atanh(t) / ln 2.
    e as f32 + t * (2.885_39 + t2 * (0.961_797 + t2 * (0.577_078 + t2 * 0.412_199)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worst(f: impl Fn(f32) -> (f32, f32), from: f32, to: f32) -> f32 {
        (0..=200_000)
            .map(|i| {
                let x = from + (to - from) * i as f32 / 200_000.0;
                let (fast, exact) = f(x);
                (fast - exact).abs()
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn approximations_are_accurate() {
        assert!(worst(|t| (sin_turns(t), (t * TAU).sin()), -3.0, 3.0) < 4e-6);
        assert!(worst(|t| (cos_turns(t), (t * TAU).cos()), -3.0, 3.0) < 4e-6);
        let rel = worst(|t| (tan_turns(t) / (t * TAU).tan(), 1.0), 1e-4, 0.24);
        assert!(rel < 1e-4, "tan {rel}");
        let rel = worst(|x| (exp2(x) / x.exp2(), 1.0), -20.0, 20.0);
        assert!(rel < 4e-7, "exp2 {rel}");
        assert!(worst(|x| (tanh(x), x.tanh()), -12.0, 12.0) < 1e-6);
        assert!(worst(|x| (log2(x), x.log2()), 1e-3, 1e3) < 1e-6);
        assert_eq!(log2(1.0), 0.0);
        assert!(log2(0.0) < -90.0);
        assert_eq!(tanh(40.0), 1.0);
        assert_eq!(exp2(0.0), 1.0);
        assert_eq!(sin_turns(0.0), 0.0);
    }
}
