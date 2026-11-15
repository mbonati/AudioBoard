//! A radix-2 FFT that can run in slices: a transform is a count of work
//! units (the bit-reversal swaps, then the butterflies), and [`Fft::step`]
//! does as many as it is given, so a large frame's transforms spread over
//! the blocks of a hop instead of landing in one audio callback.

pub struct Fft {
    max_n: usize,
    /// e^(−2πik/max_n), k < max_n/2.
    cos: Box<[f32]>,
    sin: Box<[f32]>,
}

impl Fft {
    /// Twiddles for transforms up to `max_n` points (a power of two).
    pub fn new(max_n: usize) -> Self {
        let max_n = max_n.next_power_of_two().max(4);
        let angle = |k: usize| -std::f64::consts::TAU * k as f64 / max_n as f64;
        Self {
            max_n,
            cos: (0..max_n / 2).map(|k| angle(k).cos() as f32).collect(),
            sin: (0..max_n / 2).map(|k| angle(k).sin() as f32).collect(),
        }
    }

    /// Work units of an `n`-point transform.
    pub fn units(n: usize) -> usize {
        n + n / 2 * n.trailing_zeros() as usize
    }

    /// Runs units `from..` (at most `budget`) of the `n`-point transform of
    /// `re`, `im` in place; returns how far it got. Unnormalised both ways.
    pub fn step(&self, re: &mut [f32], im: &mut [f32], n: usize, inverse: bool, from: usize, budget: usize) -> usize {
        debug_assert!(n.is_power_of_two() && n <= self.max_n);
        let log = n.trailing_zeros();
        let end = (from.saturating_add(budget)).min(Self::units(n));
        let mut i = from;
        while i < end && i < n {
            let j = i.reverse_bits() >> (usize::BITS - log);
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
            i += 1;
        }
        let half_n = n / 2;
        let sign = if inverse { -1.0 } else { 1.0 };
        while i < end {
            let u = i - n;
            let stage = (u / half_n) as u32;
            let j = u % half_n;
            let half = 1usize << stage;
            let k = j & (half - 1);
            let a = ((j >> stage) << (stage + 1)) + k;
            let b = a + half;
            let t = k * (self.max_n >> (stage + 1));
            let (wr, wi) = (self.cos[t], sign * self.sin[t]);
            let tr = re[b] * wr - im[b] * wi;
            let ti = re[b] * wi + im[b] * wr;
            re[b] = re[a] - tr;
            im[b] = im[a] - ti;
            re[a] += tr;
            im[a] += ti;
            i += 1;
        }
        end
    }

    /// The whole transform at once.
    #[cfg(test)]
    pub fn run(&self, re: &mut [f32], im: &mut [f32], n: usize, inverse: bool) {
        self.step(re, im, n, inverse, 0, usize::MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliced_transform_matches_the_dft_and_inverts() {
        let fft = Fft::new(64);
        let n = 32;
        let x: Vec<f32> = (0..n).map(|i| ((i * 7 % 11) as f32 - 5.0) * 0.1).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        // In slices of 5 units.
        let mut at = 0;
        while at < Fft::units(n) {
            at = fft.step(&mut re, &mut im, n, false, at, 5);
        }
        for k in 0..n {
            let (mut dr, mut di) = (0.0f64, 0.0f64);
            for (i, &v) in x.iter().enumerate() {
                let a = -std::f64::consts::TAU * (i * k) as f64 / n as f64;
                dr += v as f64 * a.cos();
                di += v as f64 * a.sin();
            }
            assert!((re[k] as f64 - dr).abs() < 1e-4 && (im[k] as f64 - di).abs() < 1e-4, "bin {k}");
        }
        fft.run(&mut re, &mut im, n, true);
        for (i, &v) in x.iter().enumerate() {
            assert!((re[i] / n as f32 - v).abs() < 1e-5 && im[i].abs() < 1e-4);
        }
    }
}
