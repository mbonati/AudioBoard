//! The trigger of a dynamic band: the node's input or the SC input, filtered
//! to the band's range (BAND: a 4th-order band pass for bells, low or high
//! pass for shelves) or between two free filters (FREE), on the
//! channel the band works on, into a peak follower. Attack and release
//! follow the band's frequency (slower low down, as a period lasts longer)
//! and its depth; their knobs scale that, the middle being the automatic
//! setting. The automatic threshold rides a few dB over the trigger's
//! average level (rising in about half a second, falling over two), so the
//! band reacts to what stands out of the music.

use super::design::{BELL, HIGH_SHELF, LOW_SHELF};
use crate::audio::fastmath;
use crate::audio::modules::equalizer::{Coeffs, Svf, dynamic_gain};

/// How a band's trigger is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trigger {
    pub shape: usize,
    /// 0 stereo, 1 left, 2 right, 3 mid, 4 side.
    pub place: usize,
    pub free: bool,
    pub freq: f32,
    pub q: f32,
    pub lo: f32,
    pub hi: f32,
    pub range: f32,
    pub threshold: f32,
    pub auto: bool,
    /// The knobs, 0…1 (½: automatic).
    pub attack: f32,
    pub release: f32,
}

/// The automatic threshold sits this far over the average level.
pub const AUTO_OVER: f32 = 3.0;

/// ×0.1 … ×10 around the automatic time.
pub fn time_factor(knob: f32) -> f32 {
    10f32.powf((knob.clamp(0.0, 1.0) - 0.5) * 2.0)
}

/// The automatic attack and release for a band at `freq`, in seconds.
pub fn auto_times(freq: f32) -> (f32, f32) {
    let f = freq.max(10.0);
    ((2.5 / f).clamp(0.000_5, 0.03), (25.0 / f).clamp(0.04, 0.4))
}

#[derive(Clone, Copy, Default)]
pub struct Detector {
    filt: [[Svf; 2]; 2],
    coeffs: [Coeffs; 2],
    stages: usize,
    made: Option<(usize, bool, u32, u32, u32, u32)>,
    env: f32,
    /// The trigger's level in dB, its slow average, the threshold in use
    /// and the gain change it causes.
    pub level: f32,
    pub avg: f32,
    pub threshold: f32,
    pub dynamic: f32,
    /// The trigger signal (for AUDITION).
    pub heard: (f32, f32),
}

impl Detector {
    pub fn new() -> Self {
        Self { level: -120.0, avg: -40.0, threshold: -40.0 + AUTO_OVER, ..Default::default() }
    }

    /// Remakes the filters when the settings moved.
    fn prepare(&mut self, t: &Trigger, sr: f32) {
        let key = (t.shape, t.free, t.freq.to_bits(), t.q.to_bits(), t.lo.to_bits(), t.hi.to_bits());
        if self.made == Some(key) {
            return;
        }
        self.made = Some(key);
        let g = |f: f32| fastmath::tan_turns(f.clamp(10.0, sr * 0.45) / (2.0 * sr));
        let k_bw = std::f32::consts::SQRT_2;
        if t.free {
            self.coeffs[0] = Coeffs::svf(g(t.lo), k_bw, (1.0, -k_bw, -1.0));
            self.coeffs[1] = Coeffs::svf(g(t.hi.max(t.lo)), k_bw, (0.0, 0.0, 1.0));
            self.stages = 2;
            return;
        }
        // Two stages: a 4th-order band, so loud sound outside the band's
        // range doesn't trip it.
        self.stages = 2;
        self.coeffs[0] = match t.shape {
            BELL => {
                let k = 1.0 / t.q.clamp(0.3, 12.0);
                Coeffs::svf(g(t.freq), k, (0.0, k, 0.0))
            }
            LOW_SHELF => Coeffs::svf(g(t.freq), k_bw, (0.0, 0.0, 1.0)),
            HIGH_SHELF => Coeffs::svf(g(t.freq), k_bw, (1.0, -k_bw, -1.0)),
            _ => {
                // Tilts listen to the full range.
                self.stages = 0;
                Coeffs::default()
            }
        };
        self.coeffs[1] = self.coeffs[0];
    }

    /// One sample of the trigger's source; `attack`, `release`: one-pole
    /// coefficients for this block.
    #[inline]
    pub fn tick(&mut self, t: &Trigger, sr: f32, (l, r): (f32, f32), attack: f32, release: f32) {
        self.prepare(t, sr);
        let (stages, coeffs, filt) = (self.stages, &self.coeffs, &mut self.filt);
        let mut run = |ch: usize, mut v: f32| {
            for s in 0..stages {
                v = filt[s][ch].apply(&coeffs[s], v);
            }
            v
        };
        let (a, b) = match t.place {
            1 => {
                let v = run(0, l);
                (v, v)
            }
            2 => {
                let v = run(0, r);
                (v, v)
            }
            3 => {
                let v = run(0, (l + r) * 0.5);
                (v, v)
            }
            4 => {
                let v = run(0, (l - r) * 0.5);
                (v, v)
            }
            _ => (run(0, l), run(1, r)),
        };
        self.heard = (a, b);
        let x = a.abs().max(b.abs());
        self.env += (x - self.env) * if x > self.env { attack } else { release };
    }

    /// At control rate: the level, the automatic threshold and the gain
    /// change. `avg_k`: the average's rising one-pole coefficient for this
    /// step (it falls four times slower).
    pub fn update(&mut self, t: &Trigger, avg_k: f32) {
        self.level = 6.020_6 * fastmath::log2(self.env + 1e-9);
        if self.level > -80.0 {
            // Rises in about half a second, falls over two: settles on what
            // is steady, still catches what jumps out.
            let k = if self.level > self.avg { avg_k } else { avg_k * 0.25 };
            self.avg += (self.level - self.avg) * k;
        }
        self.threshold = if t.auto { (self.avg + AUTO_OVER).clamp(-80.0, 0.0) } else { t.threshold };
        self.dynamic = dynamic_gain(self.level - self.threshold, t.range);
    }

    /// One-pole coefficients for attack and release at `sr`.
    pub fn coefficients(&self, t: &Trigger, sr: f32) -> (f32, f32) {
        let f = if t.free { (t.lo.max(10.0) * t.hi.max(10.0)).sqrt() } else { t.freq };
        let (a, r) = auto_times(f);
        let a = a * time_factor(t.attack);
        // Deeper changes let go more slowly, as a compressor's program release.
        let r = r * time_factor(t.release) * (1.0 + self.dynamic.abs() / 12.0);
        let k = |s: f32| 1.0 - (-1.0 / (s * sr).max(1.0)).exp();
        (k(a), k(r))
    }
}
