//! Prism's filters. Every band is designed as an analog prototype: a cascade
//! of up to [`MAX_SECTIONS`] first- and second-order sections in s,
//! normalised to the band's frequency. That one design gives:
//!
//! - the exact response, for the display, the linear-phase stage and auto
//!   gain ([`Analog::db`]);
//! - the digital sections of the minimum-phase stages ([`Analog::digital`]):
//!   poles by the matched z-transform (z = e^sT), so they sit where the
//!   analog ones do, and numerators that match the analog magnitude at DC, at
//!   the section's own frequency and at Nyquist. Bells near Nyquist keep
//!   their analog shape instead of being squeezed (no "cramping"), which is
//!   how Pro-Q's Zero Latency mode behaves.
//!
//! The shapes:
//! - cuts: Butterworth of 1 to 16 poles (6–96 dB/octave), the strongest
//!   pole pair's Q scaled by the band's Q for a resonant knee; the brickwall
//!   is a 16-pole inverse Chebyshev (flat passband, 100 dB down within about
//!   0.4 octave);
//! - shelves: Butterworth-type shelves of order 1 to 16 (midpoint gain at
//!   the band's frequency for any order and Q), Q shaping the knee;
//! - bell, notch and band pass: the band-pass transform (s → (s² + 1)/(s/Q))
//!   of a shelf or Butterworth prototype, so higher slopes give flatter tops
//!   and steeper skirts while Q still sets the bandwidth at half the gain;
//! - tilt shelf: a high shelf centred on 0 dB (−gain/2 below, +gain/2 above);
//! - flat tilt: thirteen first-order shelves an octave apart from 5 Hz to
//!   40 kHz, a straight line of GAIN over 20 Hz–20 kHz through 0 dB at the
//!   band's frequency;
//! - all pass: B(−s)/B(s) of a Butterworth polynomial, phase only.

use std::f64::consts::PI;

pub const MAX_SECTIONS: usize = 16;

pub const BELL: usize = 0;
pub const LOW_SHELF: usize = 1;
pub const LOW_CUT: usize = 2;
pub const HIGH_SHELF: usize = 3;
pub const HIGH_CUT: usize = 4;
pub const NOTCH: usize = 5;
pub const BAND_PASS: usize = 6;
pub const TILT_SHELF: usize = 7;
pub const FLAT_TILT: usize = 8;
pub const ALL_PASS: usize = 9;
pub const SHAPES: usize = 10;

/// Slopes in dB/octave, by index; the last is the brickwall (cuts only).
pub const SLOPES: [u32; 10] = [6, 12, 18, 24, 30, 36, 48, 72, 96, 0];
pub const BRICKWALL: usize = 9;

/// The shape has a gain (bell, shelves, tilts).
pub fn has_gain(shape: usize) -> bool {
    matches!(
        shape,
        BELL | LOW_SHELF | HIGH_SHELF | TILT_SHELF | FLAT_TILT
    )
}

/// The shape can be dynamic (and spectral): those with a gain.
pub fn can_be_dynamic(shape: usize) -> bool {
    has_gain(shape)
}

/// The shape has a slope setting at all.
pub fn has_slope(shape: usize) -> bool {
    shape != FLAT_TILT
}

/// The shape's Q does something at this slope (not at 6 dB/octave, except
/// for bells, notches and band passes whose Q is their width).
pub fn has_q(shape: usize, slope: usize) -> bool {
    match shape {
        BELL | NOTCH | BAND_PASS => true,
        FLAT_TILT => false,
        _ => slope != 0 && slope != BRICKWALL,
    }
}

/// Whether slope index `i` exists for `shape`: bells and notches go in
/// steps of 12 dB/octave (pole pairs), only cuts have the brickwall.
pub fn slope_ok(shape: usize, i: usize) -> bool {
    match shape {
        BELL | NOTCH => i < BRICKWALL && SLOPES[i].is_multiple_of(12),
        LOW_CUT | HIGH_CUT => i <= BRICKWALL,
        FLAT_TILT => i == 1,
        _ => i < BRICKWALL,
    }
}

/// The nearest slope `shape` allows to index `i`.
pub fn fix_slope(shape: usize, i: usize) -> usize {
    if slope_ok(shape, i) {
        return i;
    }
    let target = if i == BRICKWALL {
        200
    } else {
        SLOPES[i.min(BRICKWALL - 1)] as i32
    };
    (0..=BRICKWALL)
        .filter(|&j| slope_ok(shape, j))
        .min_by_key(|&j| {
            let s = if j == BRICKWALL {
                200
            } else {
                SLOPES[j] as i32
            };
            ((s - target).abs(), j)
        })
        .unwrap_or(1)
}

/// The next allowed slope after `i` (wrapping), for Alt+Shift-click.
pub fn next_slope(shape: usize, i: usize) -> usize {
    (1..=BRICKWALL + 1)
        .map(|d| (i + d) % (BRICKWALL + 1))
        .find(|&j| slope_ok(shape, j))
        .unwrap_or(i)
}

/// One section: (b0·s² + b1·s + b2) / (a0·s² + a1·s + a2), s normalised to
/// the band's frequency. First-order sections have a0 = b0 = 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sec {
    pub b: [f64; 3],
    pub a: [f64; 3],
    /// An all-pass section: its digital numerator mirrors its denominator.
    pub allpass: bool,
}

impl Sec {
    fn new(b: [f64; 3], a: [f64; 3]) -> Self {
        Self {
            b,
            a,
            allpass: false,
        }
    }

    fn order(&self) -> usize {
        if self.a[0] != 0.0 { 2 } else { 1 }
    }

    /// |H(jw)|² at normalised frequency `w`.
    pub fn mag2(&self, w: f64) -> f64 {
        let (nr, ni) = (self.b[2] - self.b[0] * w * w, self.b[1] * w);
        let (dr, di) = (self.a[2] - self.a[0] * w * w, self.a[1] * w);
        (nr * nr + ni * ni) / (dr * dr + di * di).max(1e-300)
    }

    /// arg H(jw), in radians.
    pub fn phase(&self, w: f64) -> f64 {
        let (nr, ni) = (self.b[2] - self.b[0] * w * w, self.b[1] * w);
        let (dr, di) = (self.a[2] - self.a[0] * w * w, self.a[1] * w);
        ni.atan2(nr) - di.atan2(dr)
    }
}

/// A band's analog design.
#[derive(Clone, Copy, Debug)]
pub struct Analog {
    pub secs: [Sec; MAX_SECTIONS],
    pub n: usize,
    pub gain: f64,
    /// The band's frequency in Hz (what s is normalised to).
    pub f0: f64,
}

impl Default for Analog {
    fn default() -> Self {
        Self {
            secs: [Sec::default(); MAX_SECTIONS],
            n: 0,
            gain: 1.0,
            f0: 1000.0,
        }
    }
}

/// What a band is set to, as the design sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub shape: usize,
    pub slope: usize,
    pub freq: f64,
    /// dB, for the shapes with a gain.
    pub gain: f64,
    pub q: f64,
}

#[derive(Clone, Copy, Debug)]
struct C {
    re: f64,
    im: f64,
}

impl C {
    fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
    fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
    fn mul(self, o: C) -> C {
        C::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
    fn scale(self, k: f64) -> C {
        C::new(self.re * k, self.im * k)
    }
    fn inv(self) -> C {
        let d = self.re * self.re + self.im * self.im;
        C::new(self.re / d, -self.im / d)
    }
    fn norm2(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
    fn sqrt(self) -> C {
        let r = self.norm2().sqrt();
        let re = ((r + self.re) * 0.5).max(0.0).sqrt();
        let im = ((r - self.re) * 0.5).max(0.0).sqrt();
        C::new(re, if self.im < 0.0 { -im } else { im })
    }
}

/// s² + c1·s + c0 with roots `r`, `r̄`.
fn quad(r: C) -> [f64; 3] {
    [1.0, -2.0 * r.re, r.norm2()]
}

/// The band-pass transform of a root: the two roots of s² − p·β·s + 1.
fn bp_roots(p: C, beta: f64) -> (C, C) {
    let pb = p.scale(beta);
    let d = pb.mul(pb).sub(C::new(4.0, 0.0)).sqrt();
    (pb.add(d).scale(0.5), pb.sub(d).scale(0.5))
}

/// Butterworth pole-pair Qs for `n` poles; odd orders add a real pole.
fn butterworth_q(n: usize, k: usize) -> f64 {
    1.0 / (2.0 * ((2 * k + 1) as f64 * PI / (2 * n) as f64).sin())
}

/// The roots of s² + s/q + 1 (one of a complex pair, or two real roots).
fn pair_roots(q: f64) -> Result<C, (f64, f64)> {
    let h = 0.5 / q;
    if h < 1.0 {
        Ok(C::new(-h, (1.0 - h * h).sqrt()))
    } else {
        let d = (h * h - 1.0).sqrt();
        Err((-h + d, -h - d))
    }
}

/// dB to an amplitude ratio.
pub fn db_amp(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

impl Analog {
    fn push(&mut self, s: Sec) {
        if self.n < MAX_SECTIONS {
            self.secs[self.n] = s;
            self.n += 1;
        }
    }

    /// The design for `s`. Allocation-free: fine on the audio thread.
    pub fn design(s: &Shape) -> Self {
        let mut a = Analog {
            f0: s.freq,
            ..Default::default()
        };
        let slope = if s.slope == BRICKWALL {
            96
        } else {
            SLOPES[s.slope.min(BRICKWALL - 1)]
        } as usize;
        let q = s.q.clamp(0.01, 100.0);
        match s.shape {
            LOW_CUT | HIGH_CUT if s.slope == BRICKWALL => a.brickwall(s.shape == LOW_CUT),
            LOW_CUT | HIGH_CUT => a.cut(s.shape == LOW_CUT, slope / 6, q),
            LOW_SHELF | HIGH_SHELF => a.shelf(s.shape == HIGH_SHELF, slope / 6, db_amp(s.gain), q),
            TILT_SHELF => {
                a.shelf(true, slope / 6, db_amp(s.gain), q);
                a.gain /= db_amp(s.gain * 0.5);
            }
            BELL => a.bell((slope / 12).max(1), db_amp(s.gain), q),
            NOTCH => a.notch((slope / 12).max(1), q),
            BAND_PASS => a.band_pass(slope / 6, q),
            FLAT_TILT => a.flat_tilt(s.gain),
            ALL_PASS => a.all_pass(slope / 6, q),
            _ => {}
        }
        a
    }

    /// Butterworth high pass (`high_pass`) or low pass of `n` poles; the
    /// strongest pair's Q times `q` for resonance.
    fn cut(&mut self, high_pass: bool, n: usize, q: f64) {
        let n = n.clamp(1, 16);
        for k in 0..n / 2 {
            let qk = butterworth_q(n, k) * if k == 0 { q } else { 1.0 };
            let num = if high_pass {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 0.0, 1.0]
            };
            self.push(Sec::new(num, [1.0, 1.0 / qk, 1.0]));
        }
        if n % 2 == 1 {
            let num = if high_pass {
                [0.0, 1.0, 0.0]
            } else {
                [0.0, 0.0, 1.0]
            };
            self.push(Sec::new(num, [0.0, 1.0, 1.0]));
        }
    }

    /// 16-pole inverse Chebyshev, 100 dB stopband, −3 dB at the frequency.
    fn brickwall(&mut self, high_pass: bool) {
        const N: usize = 16;
        let eps = 1.0 / (10f64.powf(10.0) - 1.0).sqrt();
        let mu = (1.0 / eps).asinh() / N as f64;
        // Where the low pass is 3 dB down, relative to its stopband edge.
        let w3 = 1.0 / ((1.0 / eps).acosh() / N as f64).cosh();
        for k in 0..N / 2 {
            let theta = (2 * k + 1) as f64 * PI / (2 * N) as f64;
            let cheby = C::new(-mu.sinh() * theta.sin(), mu.cosh() * theta.cos());
            let mut pole = cheby.inv().scale(1.0 / w3);
            let mut wz = 1.0 / (theta.cos() * w3);
            if high_pass {
                pole = pole.inv();
                wz = 1.0 / wz;
            }
            let den = quad(pole);
            // Unity gain in the passband (DC for the low pass, ∞ for the high).
            let num = if high_pass {
                [1.0, 0.0, wz * wz]
            } else {
                [den[2] / (wz * wz), 0.0, den[2]]
            };
            self.push(Sec::new(num, den));
        }
    }

    /// Butterworth-type shelf of order `n`: low (DC gain `g`) or high (HF
    /// gain `g`), midpoint √g at the frequency.
    fn shelf(&mut self, high: bool, n: usize, g: f64, q: f64) {
        let n = n.clamp(1, 16);
        let g = g.max(1e-6);
        let a = g.powf(0.5 / n as f64);
        let b = 1.0 / a;
        let (z, p) = if high { (b, a) } else { (a, b) };
        for k in 0..n / 2 {
            let qk = butterworth_q(n, k) * if k == 0 { q } else { 1.0 };
            self.push(Sec::new([1.0, z / qk, z * z], [1.0, p / qk, p * p]));
        }
        if n % 2 == 1 {
            self.push(Sec::new([0.0, 1.0, z], [0.0, 1.0, p]));
        }
        if high {
            self.gain *= g;
        }
    }

    /// The band-pass transform of a low-shelf prototype of order `m` (DC
    /// gain `g`): gain `g` at the centre, Q the width at half the gain.
    fn bell(&mut self, m: usize, g: f64, q: f64) {
        let m = m.clamp(1, 8);
        let beta = 1.0 / q;
        let g = g.max(1e-6);
        let a = g.powf(0.5 / m as f64);
        let b = 1.0 / a;
        for k in 0..m / 2 {
            let u = pair_roots(butterworth_q(m, k)).unwrap_or(C::new(-1.0, 0.0));
            let (z1, z2) = bp_roots(u.scale(a), beta);
            let (p1, p2) = bp_roots(u.scale(b), beta);
            self.push(Sec::new(quad(z1), quad(p1)));
            self.push(Sec::new(quad(z2), quad(p2)));
        }
        if m % 2 == 1 {
            self.push(Sec::new([1.0, a * beta, 1.0], [1.0, b * beta, 1.0]));
        }
    }

    /// The band-stop transform of a Butterworth low pass of order `m`.
    fn notch(&mut self, m: usize, q: f64) {
        let m = m.clamp(1, 8);
        let beta = 1.0 / q;
        for k in 0..m / 2 {
            let u = pair_roots(butterworth_q(m, k)).unwrap_or(C::new(-1.0, 0.0));
            let (p1, p2) = bp_roots(u.inv(), beta);
            self.push(Sec::new([1.0, 0.0, 1.0], quad(p1)));
            self.push(Sec::new([1.0, 0.0, 1.0], quad(p2)));
        }
        if m % 2 == 1 {
            self.push(Sec::new([1.0, 0.0, 1.0], [1.0, beta, 1.0]));
        }
    }

    /// The band-pass transform of a Butterworth low pass of order `n`:
    /// 0 dB at the centre, skirts of 6·n dB/octave.
    fn band_pass(&mut self, n: usize, q: f64) {
        let n = n.clamp(1, 16);
        let beta = 1.0 / q;
        for k in 0..n / 2 {
            let u = pair_roots(butterworth_q(n, k)).unwrap_or(C::new(-1.0, 0.0));
            let (p1, p2) = bp_roots(u, beta);
            self.push(Sec::new([0.0, beta, 0.0], quad(p1)));
            self.push(Sec::new([0.0, beta, 0.0], quad(p2)));
        }
        if n % 2 == 1 {
            self.push(Sec::new([0.0, beta, 0.0], [1.0, beta, 1.0]));
        }
    }

    /// Butterworth all pass: B(−s)/B(s).
    fn all_pass(&mut self, n: usize, q: f64) {
        let n = n.clamp(1, 16);
        for k in 0..n / 2 {
            let qk = butterworth_q(n, k) * if k == 0 { q } else { 1.0 };
            self.push(Sec {
                allpass: true,
                ..Sec::new([1.0, -1.0 / qk, 1.0], [1.0, 1.0 / qk, 1.0])
            });
        }
        if n % 2 == 1 {
            self.push(Sec {
                allpass: true,
                ..Sec::new([0.0, -1.0, 1.0], [0.0, 1.0, 1.0])
            });
        }
    }

    /// GAIN over 20 Hz–20 kHz (log2(1000) octaves), through 0 dB at f0.
    fn flat_tilt(&mut self, gain: f64) {
        const COUNT: usize = 13;
        let spacing = 13.0 / 12.0;
        let per_octave = gain / 1000f64.log2();
        let k = db_amp(per_octave * spacing);
        for i in 0..COUNT {
            let wc = 5.0 * 2f64.powf(i as f64 * spacing) / self.f0;
            // High shelf of step k, midpoint at wc: k·(s + wc/√k)/(s + wc·√k).
            self.push(Sec::new(
                [0.0, 1.0, wc / k.sqrt()],
                [0.0, 1.0, wc * k.sqrt()],
            ));
            self.gain *= k;
        }
        self.gain /= self.mag(1.0);
    }

    /// |H| at normalised frequency `w` (f / f0).
    pub fn mag(&self, w: f64) -> f64 {
        let mut m2 = 1.0;
        for s in &self.secs[..self.n] {
            m2 *= s.mag2(w);
        }
        m2.sqrt() * self.gain.abs()
    }

    /// The response in dB at `f` Hz.
    pub fn db(&self, f: f64) -> f64 {
        20.0 * self.mag(f / self.f0).max(1e-15).log10()
    }

    /// The phase in radians at `f` Hz.
    pub fn phase(&self, f: f64) -> f64 {
        let w = f / self.f0;
        self.secs[..self.n].iter().map(|s| s.phase(w)).sum::<f64>()
            + if self.gain < 0.0 { PI } else { 0.0 }
    }

    /// The digital sections at `sr` (minimum-phase stages).
    pub fn digital(&self, sr: f64) -> Digital {
        let w0 = 2.0 * PI * self.f0 / sr;
        let mut d = Digital {
            n: self.n,
            gain: self.gain,
            ..Default::default()
        };
        for (out, s) in d.secs.iter_mut().zip(&self.secs[..self.n]) {
            *out = to_digital(s, w0);
        }
        d
    }
}

/// A digital biquad: (b0 + b1·z⁻¹ + b2·z⁻²) / (1 + a1·z⁻¹ + a2·z⁻²).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Biquad {
    pub const IDENTITY: Biquad = Biquad {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// |H(e^jω)|.
    #[cfg(test)]
    pub fn mag(&self, w: f64) -> f64 {
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let (nr, ni) = (
            self.b0 + self.b1 * c1 + self.b2 * c2,
            -(self.b1 * s1 + self.b2 * s2),
        );
        let (dr, di) = (
            1.0 + self.a1 * c1 + self.a2 * c2,
            -(self.a1 * s1 + self.a2 * s2),
        );
        ((nr * nr + ni * ni) / (dr * dr + di * di).max(1e-300)).sqrt()
    }
}

/// A band's digital cascade.
#[derive(Clone, Copy, Debug)]
pub struct Digital {
    pub secs: [Biquad; MAX_SECTIONS],
    pub n: usize,
    pub gain: f64,
}

impl Default for Digital {
    fn default() -> Self {
        Self {
            secs: [Biquad::IDENTITY; MAX_SECTIONS],
            n: 0,
            gain: 1.0,
        }
    }
}

#[cfg(test)]
impl Digital {
    /// |H| at ω (radians per sample).
    pub fn mag(&self, w: f64) -> f64 {
        self.secs[..self.n]
            .iter()
            .map(|s| s.mag(w))
            .product::<f64>()
            * self.gain.abs()
    }
}

/// Matched poles stop short of this angle (beyond Nyquist they would fold
/// back into the audible range).
const MATCH_LIMIT: f64 = 0.98 * PI;

/// One analog section (normalised, the band at `w0` rad/sample) to a
/// digital biquad.
fn to_digital(s: &Sec, w0: f64) -> Biquad {
    // Scale to rad/sample: the coefficient of s^k times w0^(order − k).
    let order = s.order();
    let scale = |c: [f64; 3]| -> [f64; 3] {
        if order == 2 {
            [c[0], c[1] * w0, c[2] * w0 * w0]
        } else {
            [0.0, c[1], c[2] * w0]
        }
    };
    let (b, a) = (scale(s.b), scale(s.a));
    let target = |w: f64| Sec::new(b, a).mag2(w);

    // Poles: by the matched z-transform, unless they would fold back.
    let (d1, d2, wn) = if order == 1 {
        let wn = a[2] / a[1];
        if s.allpass {
            let d1 = -(-wn).exp();
            return Biquad {
                b0: d1,
                b1: 1.0,
                b2: 0.0,
                a1: d1,
                a2: 0.0,
            };
        }
        // A biquad with one pole: its second numerator term fits the
        // analog curve better than a first-order section could.
        (-(-wn).exp(), 0.0, wn)
    } else {
        let (a1n, a0n) = (a[1] / a[0], a[2] / a[0]);
        let wn = a0n.max(1e-30).sqrt();
        let zeta = a1n / (2.0 * wn);
        let (d1, d2) = if zeta >= 1.0 {
            let r = wn * (zeta * zeta - 1.0).sqrt();
            let (s1, s2) = (wn * zeta - r, wn * zeta + r);
            (-((-s1).exp() + (-s2).exp()), (-(s1 + s2)).exp())
        } else {
            // Past Nyquist the pole would fold back: it stays just below,
            // as damped as it is, and the numerator fits the skirt.
            let theta = (wn * (1.0 - zeta * zeta).sqrt()).min(MATCH_LIMIT);
            let r = (-zeta * wn).exp();
            (-2.0 * r * theta.cos(), r * r)
        };
        (d1, d2, wn)
    };
    if s.allpass {
        return Biquad {
            b0: d2,
            b1: d1,
            b2: 1.0,
            a1: d1,
            a2: d2,
        };
    }

    // Numerator: matches |H|² at DC, at Nyquist and at a third frequency
    // (the zeros' own for notches, else the poles'), minimum phase.
    let wm = if b[1].abs() < 1e-12 && b[0] > 0.0 && b[2] > 0.0 {
        let wz = (b[2] / b[0]).sqrt();
        if wz < 0.98 * PI { wz } else { 0.5 * PI }
    } else if wn < 0.9 * PI {
        wn.max(1e-4)
    } else {
        0.5 * PI
    };
    let a_sq = |w: f64| {
        let p1 = (w * 0.5).sin().powi(2);
        let p0 = 1.0 - p1;
        let (a0q, a1q, a2q) = ((1.0 + d1 + d2).powi(2), (1.0 - d1 + d2).powi(2), -4.0 * d2);
        (a0q * p0 + a1q * p1 + a2q * 4.0 * p0 * p1, p0, p1)
    };
    let big_b0 = target(0.0) * (1.0 + d1 + d2).powi(2);
    let mut big_b1 = target(PI) * (1.0 - d1 + d2).powi(2);
    let (am, p0m, p1m) = a_sq(wm);
    let phi2 = 4.0 * p0m * p1m;
    let mut big_b2 = if phi2 > 1e-12 {
        (target(wm) * am - big_b0 * p0m - big_b1 * p1m) / phi2
    } else {
        0.0
    };
    let notch = b[1].abs() < 1e-12 && b[0] > 0.0 && b[2] > 0.0;
    if !notch && wn > 0.25 * PI {
        // High up, refine the Nyquist and middle terms by least squares on
        // the relative error over the section's range, DC kept exact: the
        // three-point match alone drifts by most of a dB between its
        // points there. Lower down it is already exact to a hair.
        let lo = wn.min(PI) / 16.0;
        let (mut s11, mut s12, mut s22, mut r1, mut r2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for i in 0..=25 {
            // The section's own frequency weighs most: a bell's peak stays put.
            let (w, weight) = if i == 25 {
                (wm, 8.0)
            } else {
                (lo * (PI / lo).powf(i as f64 / 24.0), 1.0)
            };
            let (aw, p0, p1) = a_sq(w);
            let t = target(w) * aw;
            let y = t - big_b0 * p0;
            let f2 = 4.0 * p0 * p1;
            let wt = weight / (t * t).max(1e-24);
            s11 += wt * p1 * p1;
            s12 += wt * p1 * f2;
            s22 += wt * f2 * f2;
            r1 += wt * p1 * y;
            r2 += wt * f2 * y;
        }
        let det = s11 * s22 - s12 * s12;
        if det.abs() > 1e-30 {
            let (x1, x2) = ((r1 * s22 - r2 * s12) / det, (s11 * r2 - s12 * r1) / det);
            if x1 >= 0.0 && x1.is_finite() && x2.is_finite() {
                (big_b1, big_b2) = (x1, x2);
            }
        }
    }
    let (r0, r1) = (big_b0.max(0.0).sqrt(), big_b1.max(0.0).sqrt());
    let w = 0.5 * (r0 + r1);
    let disc = (w * w + big_b2).max(0.0);
    let c0 = 0.5 * (w + disc.sqrt());
    let c2 = w - c0;
    let c1 = 0.5 * (r0 - r1);
    Biquad {
        b0: c0,
        b1: c1,
        b2: c2,
        a1: d1,
        a2: d2,
    }
}

/// Transposed direct form II, in f64: steady at low frequencies and high Q.
#[derive(Clone, Copy, Default)]
pub struct State {
    s1: f64,
    s2: f64,
}

impl State {
    #[inline]
    pub fn tick(&mut self, c: &Biquad, x: f64) -> f64 {
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(shape: usize, slope: usize, freq: f64, gain: f64, q: f64) -> Analog {
        Analog::design(&Shape {
            shape,
            slope,
            freq,
            gain,
            q,
        })
    }

    #[test]
    fn bells_peak_at_their_gain_with_any_slope() {
        for slope in [1, 3, 5, 6, 7, 8] {
            let a = shape(BELL, slope, 1000.0, 9.0, 1.0);
            assert!(
                (a.db(1000.0) - 9.0).abs() < 1e-6,
                "slope {slope}: {}",
                a.db(1000.0)
            );
            let d = a.digital(48_000.0);
            let peak = 20.0 * d.mag(2.0 * PI * 1000.0 / 48_000.0).log10();
            assert!(
                (peak - 9.0).abs() < 0.05,
                "slope {slope} digital peak: {peak}"
            );
            assert!(a.db(30.0).abs() < 0.2, "slope {slope} far below");
            assert!(a.db(30_000.0).abs() < 0.2, "slope {slope} far above");
            // Q is the width at half the gain: the edges sit at 4.5 dB.
            let beta = 1.0f64;
            let hi = 1000.0 * (beta / 2.0 + (beta * beta / 4.0 + 1.0).sqrt());
            assert!(
                (a.db(hi) - 4.5).abs() < 0.05,
                "slope {slope} half-gain edge: {}",
                a.db(hi)
            );
        }
        // Steeper slopes give a flatter top and steeper skirts.
        let (soft, steep) = (
            shape(BELL, 1, 1000.0, 12.0, 1.0),
            shape(BELL, 8, 1000.0, 12.0, 1.0),
        );
        assert!(steep.db(1300.0) > soft.db(1300.0));
        assert!(steep.db(3000.0) < soft.db(3000.0));
    }

    #[test]
    fn cuts_have_their_slope_and_resonance() {
        for (i, &s) in SLOPES[..BRICKWALL].iter().enumerate() {
            let a = shape(LOW_CUT, i, 1000.0, 0.0, 1.0);
            assert!(
                (a.db(1000.0) + 3.01).abs() < 0.05,
                "{s} dB/oct corner: {}",
                a.db(1000.0)
            );
            // An octave of the asymptote, short of the 300 dB floor.
            let far = if s <= 24 { 16.0 } else { 2.0 };
            let drop = a.db(1000.0 / far) - a.db(500.0 / far);
            assert!((drop - s as f64).abs() < 0.4, "{s} dB/oct: {drop}");
            let h = shape(HIGH_CUT, i, 1000.0, 0.0, 1.0);
            assert!((h.db(1000.0 * far) - h.db(2000.0 * far) - s as f64).abs() < 0.4);
            assert!(h.db(100.0).abs() < 0.05);
        }
        // Q above 1 lifts the knee into a resonant peak.
        let res = shape(LOW_CUT, 1, 1000.0, 0.0, 6.0);
        assert!(res.db(1000.0) > 10.0);
        // The brickwall: flat, 3 dB down at the corner, 100 dB down soon after.
        let bw = shape(HIGH_CUT, BRICKWALL, 1000.0, 0.0, 1.0);
        assert!((bw.db(1000.0) + 3.01).abs() < 0.05, "{}", bw.db(1000.0));
        assert!(bw.db(700.0).abs() < 0.01);
        assert!(bw.db(1400.0) < -95.0, "{}", bw.db(1400.0));
        let bwl = shape(LOW_CUT, BRICKWALL, 1000.0, 0.0, 1.0);
        assert!((bwl.db(1000.0) + 3.01).abs() < 0.05);
        assert!(bwl.db(700.0) < -95.0);
        assert!(bwl.db(1500.0).abs() < 0.01);
    }

    #[test]
    fn shelves_tilts_notch_band_pass_and_all_pass() {
        for i in 0..BRICKWALL {
            let ls = shape(LOW_SHELF, i, 300.0, -8.0, 1.0);
            assert!(
                (ls.db(300.0) + 4.0).abs() < 1e-6,
                "midpoint at half the gain"
            );
            assert!((ls.db(10.0) + 8.0).abs() < 0.3);
            assert!(ls.db(20_000.0).abs() < 0.05);
            let hs = shape(HIGH_SHELF, i, 3000.0, 6.0, 1.0);
            assert!((hs.db(3000.0) - 3.0).abs() < 1e-6);
            assert!((hs.db(30_000.0) - 6.0).abs() < 0.3);
            let tilt = shape(TILT_SHELF, i, 1000.0, 10.0, 1.0);
            assert!(tilt.db(1000.0).abs() < 1e-6);
            assert!((tilt.db(20.0) + 5.0).abs() < 0.2 && (tilt.db(50_000.0) - 5.0).abs() < 0.2);
            let ap = shape(ALL_PASS, i, 1000.0, 0.0, 1.0);
            assert!(ap.db(123.0).abs() < 1e-9 && ap.db(4567.0).abs() < 1e-9);
        }
        let notch = shape(NOTCH, 1, 1000.0, 0.0, 4.0);
        assert!(notch.db(1000.0) < -100.0);
        assert!(notch.db(500.0).abs() < 0.3 && notch.db(2000.0).abs() < 0.3);
        let bp = shape(BAND_PASS, 3, 1000.0, 0.0, 2.0);
        assert!(bp.db(1000.0).abs() < 1e-6);
        let skirt = bp.db(62.5) - bp.db(125.0);
        assert!((skirt + 24.0).abs() < 0.5, "24 dB/oct skirts: {skirt}");
        // Flat tilt: a straight line through 0 dB at the pivot.
        let flat = shape(FLAT_TILT, 1, 632.0, 10.0, 1.0);
        assert!(flat.db(632.0).abs() < 1e-6);
        let per_octave = 10.0 / 1000f64.log2();
        for f in [40.0, 100.0, 1000.0, 5000.0, 12_000.0] {
            let want = per_octave * (f / 632.0f64).log2();
            assert!(
                (flat.db(f) - want).abs() < 0.25,
                "flat tilt at {f}: {} vs {want}",
                flat.db(f)
            );
        }
    }

    #[test]
    fn digital_sections_follow_the_analog_curve() {
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            for (sh, slope, f, g, q) in [
                (BELL, 1, 1000.0, 12.0, 1.0),
                (BELL, 1, 15_000.0, 9.0, 1.5),
                (BELL, 1, 18_000.0, -12.0, 0.7),
                (BELL, 5, 300.0, -10.0, 3.0),
                (BELL, 1, 40.0, 8.0, 8.0),
                (LOW_SHELF, 1, 120.0, 6.0, 1.0),
                (HIGH_SHELF, 3, 12_000.0, -6.0, 1.0),
                (HIGH_SHELF, 1, 16_000.0, 6.0, 1.0),
                (LOW_CUT, 3, 30.0, 0.0, 1.0),
                (LOW_CUT, 1, 200.0, 0.0, 4.0),
                (HIGH_CUT, 1, 10_000.0, 0.0, 1.0),
                (HIGH_CUT, 2, 25_000.0, 0.0, 1.0),
                (BAND_PASS, 1, 2000.0, 0.0, 2.0),
                (TILT_SHELF, 1, 1000.0, 6.0, 1.0),
                (FLAT_TILT, 1, 1000.0, 9.0, 1.0),
                (BELL, 1, 28_000.0, 10.0, 1.0),
            ] {
                let a = shape(sh, slope, f, g, q);
                let d = a.digital(sr);
                // Close to analog up to 0.75 × Nyquist (or 20 kHz), within a
                // dB to 0.9 × (Pro-Q's Zero Latency drifts there too).
                let (mut worst, mut top, mut at): (f64, f64, f64) = (0.0, 0.0, 0.0);
                for k in 1..200 {
                    let fk = 20.0 * (0.9 * sr / 2.0 / 20.0f64).powf(k as f64 / 200.0);
                    let w = 2.0 * PI * fk / sr;
                    let (ad, dd) = (a.db(fk), 20.0 * d.mag(w).log10());
                    // Deep stopbands only need to stay deep.
                    if ad > -40.0 {
                        // Skirts below −20 dB may drift a little more.
                        let e = (ad - dd).abs() * if ad < -20.0 { 0.33 } else { 1.0 };
                        if fk < (0.75 * sr / 2.0).min(20_000.0) {
                            if e > worst {
                                (worst, at) = (e, fk);
                            }
                        } else {
                            top = top.max(e);
                        }
                    }
                }
                assert!(
                    worst < 0.5 && top < 1.0,
                    "shape {sh} slope {slope} at {f} Hz, sr {sr}: {worst:.2} dB off at {at:.0} Hz, {top:.2} near Nyquist"
                );
            }
        }
    }

    #[test]
    fn notches_stay_deep_and_all_passes_stay_flat_digitally() {
        let sr = 48_000.0;
        let n = shape(NOTCH, 1, 1000.0, 0.0, 2.0).digital(sr);
        assert!(20.0 * n.mag(2.0 * PI * 1000.0 / sr).log10() < -60.0);
        let ap = shape(ALL_PASS, 3, 2000.0, 0.0, 1.0).digital(sr);
        for f in [50.0, 2000.0, 15_000.0] {
            assert!((ap.mag(2.0 * PI * f / sr) - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn slopes_per_shape() {
        assert!(slope_ok(BELL, 1) && !slope_ok(BELL, 2) && !slope_ok(BELL, BRICKWALL));
        assert!(slope_ok(LOW_CUT, BRICKWALL) && !slope_ok(HIGH_SHELF, BRICKWALL));
        assert_eq!(fix_slope(BELL, 0), 1);
        assert_eq!(fix_slope(BELL, 2), 1);
        assert_eq!(fix_slope(LOW_SHELF, BRICKWALL), 8);
        assert_eq!(next_slope(BELL, 1), 3);
        assert_eq!(next_slope(LOW_CUT, 8), BRICKWALL);
    }
}
