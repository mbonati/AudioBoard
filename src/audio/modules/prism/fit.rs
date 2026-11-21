//! Fits bands to a curve: EQ Match (the difference between a reference's
//! spectrum and the input's) and EQ Sketch (a curve drawn on the display).
//! Greedy: the largest deviation left gets a band (a bell, or a shelf when
//! it runs off either end), sized by where the deviation falls to half;
//! then the gains are refined together, a few passes.

use super::design::{Analog, BELL, HIGH_SHELF, LOW_SHELF, Shape};

/// A fitted band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fitted {
    pub shape: usize,
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
}

fn curve(b: &Fitted, freqs: &[f32]) -> Vec<f32> {
    let a = Analog::design(&Shape {
        shape: b.shape,
        slope: 1,
        freq: b.freq as f64,
        gain: b.gain as f64,
        q: b.q as f64,
    });
    freqs.iter().map(|&f| a.db(f as f64) as f32).collect()
}

/// Up to `max` bands that together approach `target` (dB at `freqs`,
/// log-spaced, low to high). Deviations under `floor` dB are left alone.
pub fn fit(freqs: &[f32], target: &[f32], max: usize, floor: f32) -> Vec<Fitted> {
    let n = freqs.len();
    if n < 3 || max == 0 {
        return Vec::new();
    }
    let target: Vec<f32> = target.iter().map(|v| v.clamp(-24.0, 24.0)).collect();
    let mut residual = target.clone();
    let mut bands: Vec<Fitted> = Vec::new();
    let mut curves: Vec<Vec<f32>> = Vec::new();
    while bands.len() < max {
        let (peak, &v) = residual
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .expect("not empty");
        if v.abs() < floor {
            break;
        }
        // Where the deviation falls to half (or turns), each side.
        let half =
            |i: usize| residual[i].signum() == v.signum() && residual[i].abs() >= v.abs() * 0.5;
        let mut lo = peak;
        while lo > 0 && half(lo - 1) {
            lo -= 1;
        }
        let mut hi = peak;
        while hi + 1 < n && half(hi + 1) {
            hi += 1;
        }
        let band = if lo == 0 && hi + 1 < n && peak < n / 3 {
            Fitted {
                shape: LOW_SHELF,
                freq: freqs[hi],
                gain: v,
                q: 1.0,
            }
        } else if hi + 1 == n && lo > 0 && peak > 2 * n / 3 {
            Fitted {
                shape: HIGH_SHELF,
                freq: freqs[lo],
                gain: v,
                q: 1.0,
            }
        } else {
            let (f_lo, f_hi) = (freqs[lo.saturating_sub(1)], freqs[(hi + 1).min(n - 1)]);
            let f0 = freqs[peak];
            // Q from the half-gain width (the bells' own definition).
            let q = (f0 / (f_hi - f_lo).max(1e-3)).clamp(0.3, 12.0);
            Fitted {
                shape: BELL,
                freq: f0,
                gain: v,
                q,
            }
        };
        let c = curve(&band, freqs);
        for (r, x) in residual.iter_mut().zip(&c) {
            *r -= x;
        }
        bands.push(band);
        curves.push(c);
    }
    // Refine the gains together: each band's in turn, against what the
    // others leave, along its own shape (nearly linear in the gain).
    for _ in 0..4 {
        for i in 0..bands.len() {
            let others: Vec<f32> = (0..n)
                .map(|k| {
                    curves
                        .iter()
                        .enumerate()
                        .filter(|&(j, _)| j != i)
                        .map(|(_, c)| c[k])
                        .sum()
                })
                .collect();
            let g = bands[i].gain;
            if g.abs() < 1e-3 {
                continue;
            }
            let (mut num, mut den) = (0.0f32, 0.0f32);
            for k in 0..n {
                let shape = curves[i][k] / g;
                num += shape * (target[k] - others[k]);
                den += shape * shape;
            }
            if den > 1e-6 {
                bands[i].gain = (num / den).clamp(-24.0, 24.0);
                curves[i] = curve(&bands[i], freqs);
            }
        }
    }
    bands.retain(|b| b.gain.abs() >= floor * 0.5);
    bands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_finds_the_bands_that_made_a_curve() {
        let freqs: Vec<f32> = (0..160)
            .map(|i| 20.0 * 1000f32.powf(i as f32 / 159.0))
            .collect();
        let made = [
            Fitted {
                shape: BELL,
                freq: 300.0,
                gain: -6.0,
                q: 2.0,
            },
            Fitted {
                shape: BELL,
                freq: 3000.0,
                gain: 5.0,
                q: 1.0,
            },
            Fitted {
                shape: HIGH_SHELF,
                freq: 10_000.0,
                gain: 4.0,
                q: 1.0,
            },
        ];
        let target: Vec<f32> = (0..freqs.len())
            .map(|k| made.iter().map(|b| curve(b, &freqs)[k]).sum())
            .collect();
        let bands = fit(&freqs, &target, 8, 0.3);
        assert!(!bands.is_empty() && bands.len() <= 8);
        let total: Vec<f32> = (0..freqs.len())
            .map(|k| bands.iter().map(|b| curve(b, &freqs)[k]).sum())
            .collect();
        let worst = total
            .iter()
            .zip(&target)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1.0, "fit within a dB: {worst}");
        assert!(
            bands
                .iter()
                .any(|b| b.shape == BELL && (b.freq / 300.0).log2().abs() < 0.2 && b.gain < -4.0)
        );
    }
}
