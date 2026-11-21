//! 2× oversampling for non-linear stages (Magma's High Quality, Tape's
//! saturation): stereo, a 63-tap half-band FIR each way.

/// Side taps of the half-band filter (its centre is ½, every other tap 0).
const HB: usize = 16;

/// 2× up and down sampling with a 63-tap half-band FIR (Kaiser window).
pub struct Oversampler {
    h: [f32; HB],
    up: [[f32; 2 * HB]; 2],
    up_pos: usize,
    down: [[f32; 4 * HB]; 2],
    down_pos: usize,
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..30 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
    }
    sum
}

impl Oversampler {
    pub fn new() -> Self {
        let n = (4 * HB - 1) as f64;
        let beta = 8.0;
        let mut h: [f32; HB] = std::array::from_fn(|j| {
            let d = (2 * j + 1) as f64;
            let sinc = (std::f64::consts::PI * d / 2.0).sin() / (std::f64::consts::PI * d / 2.0);
            let r = d / ((n - 1.0) / 2.0);
            let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta);
            (0.5 * sinc * w) as f32
        });
        // Unity gain at DC: ½ + 2·Σh = 1.
        let sum: f32 = h.iter().sum();
        for v in &mut h {
            *v *= 0.25 / sum;
        }
        Self {
            h,
            up: [[0.0; 2 * HB]; 2],
            up_pos: 0,
            down: [[0.0; 4 * HB]; 2],
            down_pos: 0,
        }
    }

    pub fn reset(&mut self) {
        self.up = [[0.0; 2 * HB]; 2];
        self.down = [[0.0; 4 * HB]; 2];
    }

    /// Both channels' next input sample → two samples each at twice the rate.
    pub fn up(&mut self, x: [f32; 2]) -> [[f32; 2]; 2] {
        self.up_pos = (self.up_pos + 1) & (2 * HB - 1);
        let mut out = [[0.0; 2]; 2];
        for ch in 0..2 {
            let ring = &mut self.up[ch];
            ring[self.up_pos] = x[ch];
            let at = |k: usize| ring[(self.up_pos + 2 * HB - k) & (2 * HB - 1)];
            let mut even = 0.0;
            for (j, &h) in self.h.iter().enumerate() {
                even += h * (at(HB - 1 - j) + at(HB + j));
            }
            out[0][ch] = 2.0 * even;
            out[1][ch] = at(HB - 1);
        }
        out
    }

    /// Two samples at twice the rate (in order) → one per channel.
    pub fn down(&mut self, y: [[f32; 2]; 2]) -> [f32; 2] {
        for s in y {
            self.down_pos = (self.down_pos + 1) & (4 * HB - 1);
            for (ring, v) in self.down.iter_mut().zip(s) {
                ring[self.down_pos] = v;
            }
        }
        std::array::from_fn(|ch| {
            let ring = &self.down[ch];
            let at = |k: usize| ring[(self.down_pos + 4 * HB - k) & (4 * HB - 1)];
            let mut sum = 0.5 * at(2 * HB - 1);
            for (j, &h) in self.h.iter().enumerate() {
                sum += h * (at(2 * HB - 2 - 2 * j) + at(2 * HB + 2 * j));
            }
            sum
        })
    }
}

impl Default for Oversampler {
    fn default() -> Self {
        Self::new()
    }
}
