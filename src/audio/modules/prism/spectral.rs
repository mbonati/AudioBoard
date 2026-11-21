//! Prism's spectral stage: short-time Fourier frames (Hann windows, 75%
//! overlap), each multiplied by a gain per frequency bin, then overlapped
//! back. It does three jobs:
//!
//! - the Linear Phase mode: the bands' combined curve, as real (zero-phase)
//!   gains, so phase is left untouched; dynamic bands add their live change;
//! - spectral dynamics: a spectral band measures every bin in its range
//!   against the threshold and turns down (or up) only the bins that cross
//!   it, the rest of the band left as it is;
//! - stereo placement in both: gains per left, right, mid and side.
//!
//! The curves come as dB on a log-frequency grid ([`GRID`] points from
//! 10 Hz to Nyquist), interpolated to the bins. A frame's work (transforms
//! and bins) is spread over the hop, so even the largest frames don't
//! weigh on one audio callback. Latency: the frame plus a hop.

use super::BANDS;
use super::fft::Fft;
use crate::audio::fastmath;
use crate::audio::modules::equalizer::dynamic_gain;

/// Points of the gain grids.
pub const GRID: usize = 1024;
pub const GRID_LO: f32 = 10.0;
/// Points of the spectral-activity curve shown on the display.
pub const CURVE: usize = 96;

/// The grids' channels.
pub const L: usize = 0;
pub const R: usize = 1;
pub const M: usize = 2;
pub const S: usize = 3;

/// Grid point `i`'s frequency.
pub fn grid_freq(i: usize, sr: f32) -> f32 {
    GRID_LO * (sr * 0.5 / GRID_LO).powf(i as f32 / (GRID - 1) as f32)
}

/// Where `f` falls on the grid (fractional index).
pub fn grid_pos(f: f32, sr: f32) -> f32 {
    let span = (sr * 0.5 / GRID_LO).log2();
    ((f.max(GRID_LO) / GRID_LO).log2() / span * (GRID - 1) as f32).clamp(0.0, (GRID - 1) as f32)
}

/// Linear interpolation on a grid.
#[inline]
pub fn at(grid: &[f32], pos: f32) -> f32 {
    let i = (pos as usize).min(GRID - 2);
    let f = pos - i as f32;
    grid[i] + (grid[i + 1] - grid[i]) * f
}

/// A spectral band, as the stage needs it.
#[derive(Clone, Copy, Default)]
pub struct SpecBand {
    pub active: bool,
    /// Stereo placement: 0 stereo, 1 left, 2 right, 3 mid, 4 side.
    pub place: usize,
    /// Triggered by the SC input.
    pub ext: bool,
    pub range: f32,
    pub threshold: f32,
    pub auto: bool,
    /// Seconds.
    pub attack: f32,
    pub release: f32,
    /// 0 wide (neighbouring bins trigger together) … 1 narrow.
    pub density: f32,
    /// +3 dB/octave on the trigger, so highs trigger a little sooner.
    pub tilt: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Idle,
    Gather,
    FftMain,
    FftSc,
    Detect,
    Smooth,
    Apply,
    Ifft,
    Overlap,
}

pub struct Stage {
    sr: f32,
    max_n: usize,
    /// The largest frame spectral bands may use (their state is this big).
    max_spec_n: usize,
    n: usize,
    hop: usize,
    fft: Fft,
    window: Box<[f32]>,
    /// Input rings (L, R, SC L, SC R) and the output's overlap-add ring.
    inp: [Box<[f32]>; 4],
    acc: [Box<[f32]>; 2],
    mask: usize,
    t: usize,
    since: usize,
    /// The frame in progress ends at this time.
    frame_t: usize,
    zr: Box<[f32]>,
    zi: Box<[f32]>,
    cr: Box<[f32]>,
    ci: Box<[f32]>,
    phase: Phase,
    pos: usize,
    band: usize,
    carry: f32,
    use_sc: bool,
    spectral_on: bool,
    total_units: usize,
    bin_pos: Box<[f32]>,
    /// Gains in dB on the grid, per channel; written by the processor.
    pub db: [Box<[f32]>; 4],
    /// Mid or side gains are in use.
    pub ms: bool,
    pub spec: [SpecBand; BANDS],
    /// Each band's weight on the grid (its shape at full range, 0…1).
    pub weight: Box<[Box<[f32]>]>,
    level: Box<[Box<[f32]>]>,
    avg: Box<[Box<[f32]>]>,
    tmp: Box<[f32]>,
    sdb: [Box<[f32]>; 4],
    acts: [f32; BANDS],
    /// Each spectral band's largest gain change in the last frame, dB.
    pub activity: [f32; BANDS],
    curve_acc: [f32; CURVE],
    /// The spectral bands' gain changes on [`CURVE`] log points, dB.
    pub curve: [f32; CURVE],
}

impl Stage {
    /// Frames up to `max_n` points, spectral bands up to `max_spec_n`.
    pub fn new(sr: f32, max_n: usize, max_spec_n: usize) -> Self {
        let max_n = max_n.next_power_of_two();
        let max_spec_n = max_spec_n.next_power_of_two().min(max_n);
        let ring = 2 * max_n;
        let zeros = |len: usize| vec![0.0f32; len].into_boxed_slice();
        let spec_bins = max_spec_n / 2 + 1;
        let mut s = Self {
            sr,
            max_n,
            max_spec_n,
            n: 0,
            hop: 1,
            fft: Fft::new(max_n),
            window: zeros(max_n),
            inp: std::array::from_fn(|_| zeros(ring)),
            acc: std::array::from_fn(|_| zeros(ring)),
            mask: ring - 1,
            t: 0,
            since: 0,
            frame_t: 0,
            zr: zeros(max_n),
            zi: zeros(max_n),
            cr: zeros(max_n),
            ci: zeros(max_n),
            phase: Phase::Idle,
            pos: 0,
            band: 0,
            carry: 0.0,
            use_sc: false,
            spectral_on: false,
            total_units: 1,
            bin_pos: zeros(max_n / 2 + 1),
            db: std::array::from_fn(|_| zeros(GRID)),
            ms: false,
            spec: [SpecBand::default(); BANDS],
            weight: (0..BANDS).map(|_| zeros(GRID)).collect(),
            level: (0..BANDS)
                .map(|_| vec![-120.0f32; spec_bins].into_boxed_slice())
                .collect(),
            avg: (0..BANDS)
                .map(|_| vec![-60.0f32; spec_bins].into_boxed_slice())
                .collect(),
            tmp: zeros(spec_bins),
            sdb: std::array::from_fn(|_| zeros(spec_bins)),
            acts: [0.0; BANDS],
            activity: [0.0; BANDS],
            curve_acc: [0.0; CURVE],
            curve: [0.0; CURVE],
        };
        s.resize(max_spec_n.min(2048));
        s
    }

    pub fn n(&self) -> usize {
        self.n
    }

    /// Samples from input to output.
    pub fn latency(&self) -> usize {
        self.n + self.hop
    }

    /// Spectral bands can work at this frame size.
    pub fn spectral_ok(&self) -> bool {
        self.n <= self.max_spec_n
    }

    /// A new frame size (clears everything).
    pub fn resize(&mut self, n: usize) {
        let n = n.next_power_of_two().clamp(256, self.max_n);
        if n == self.n {
            return;
        }
        self.n = n;
        self.hop = n / 4;
        for (i, w) in self.window[..n].iter_mut().enumerate() {
            *w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
        }
        for k in 0..=n / 2 {
            self.bin_pos[k] = grid_pos(k as f32 * self.sr / n as f32, self.sr);
        }
        self.reset();
    }

    /// Silence: empties the rings and the bands' memory.
    pub fn reset(&mut self) {
        for ring in self.inp.iter_mut().chain(self.acc.iter_mut()) {
            ring.fill(0.0);
        }
        for l in self.level.iter_mut() {
            l.fill(-120.0);
        }
        for a in self.avg.iter_mut() {
            a.fill(-60.0);
        }
        self.phase = Phase::Idle;
        self.since = 0;
        self.activity = [0.0; BANDS];
        self.curve = [0.0; CURVE];
    }

    /// Runs a block in place. `sc`: the side chain, if patched.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let len = l.len();
        let latency = self.latency();
        for i in 0..len {
            if self.since >= self.hop {
                self.since = 0;
                self.work(usize::MAX);
                self.start(sc.is_some());
            }
            let w = self.t & self.mask;
            self.inp[0][w] = l[i];
            self.inp[1][w] = r[i];
            let (a, b) = sc.map_or((0.0, 0.0), |(a, b)| (a[i], b[i]));
            self.inp[2][w] = a;
            self.inp[3][w] = b;
            let ri = self.t.wrapping_sub(latency) & self.mask;
            l[i] = self.acc[0][ri];
            r[i] = self.acc[1][ri];
            self.acc[0][ri] = 0.0;
            self.acc[1][ri] = 0.0;
            self.t = self.t.wrapping_add(1);
            self.since += 1;
        }
        // This block's share of the frame, with a margin.
        let budget = self.total_units * len / self.hop * 5 / 4 + 64;
        self.work(budget);
    }

    fn bins(&self) -> usize {
        self.n / 2 + 1
    }

    fn start(&mut self, sc: bool) {
        self.frame_t = self.t;
        self.spectral_on = self.spectral_ok() && self.spec.iter().any(|b| b.active);
        self.use_sc = sc && self.spectral_on && self.spec.iter().any(|b| b.active && b.ext);
        let n = self.n;
        let bins = self.bins();
        let fft = Fft::units(n);
        let bands = if self.spectral_on {
            self.spec.iter().filter(|b| b.active).count()
        } else {
            0
        };
        self.total_units = n * (1 + self.use_sc as usize)
            + fft * (2 + self.use_sc as usize)
            + bands * 3 * bins
            + 4 * bins
            + n;
        if self.spectral_on {
            for d in self.sdb.iter_mut() {
                d[..bins].fill(0.0);
            }
        }
        self.acts = [0.0; BANDS];
        self.curve_acc = [0.0; CURVE];
        self.phase = Phase::Gather;
        self.pos = 0;
    }

    /// Advances the frame by up to `budget` units.
    fn work(&mut self, mut budget: usize) {
        let n = self.n;
        let bins = self.bins();
        while budget > 0 && self.phase != Phase::Idle {
            match self.phase {
                Phase::Gather => {
                    let per = 1 + self.use_sc as usize;
                    let end = (self.pos + (budget / per).max(1)).min(n);
                    let base = self.frame_t.wrapping_sub(n);
                    for i in self.pos..end {
                        let ri = base.wrapping_add(i) & self.mask;
                        let w = self.window[i];
                        self.zr[i] = self.inp[0][ri] * w;
                        self.zi[i] = self.inp[1][ri] * w;
                        if self.use_sc {
                            self.cr[i] = self.inp[2][ri] * w;
                            self.ci[i] = self.inp[3][ri] * w;
                        }
                    }
                    budget = budget.saturating_sub((end - self.pos) * per);
                    self.pos = end;
                    if end == n {
                        self.next(Phase::FftMain);
                    }
                }
                Phase::FftMain | Phase::FftSc | Phase::Ifft => {
                    let units = Fft::units(n);
                    let take = budget.min(units - self.pos);
                    self.pos = match self.phase {
                        Phase::FftSc => {
                            self.fft
                                .step(&mut self.cr, &mut self.ci, n, false, self.pos, take)
                        }
                        Phase::Ifft => {
                            self.fft
                                .step(&mut self.zr, &mut self.zi, n, true, self.pos, take)
                        }
                        _ => self
                            .fft
                            .step(&mut self.zr, &mut self.zi, n, false, self.pos, take),
                    };
                    budget -= take;
                    if self.pos >= units {
                        let next = match self.phase {
                            Phase::FftMain if self.use_sc => Phase::FftSc,
                            Phase::FftMain | Phase::FftSc => {
                                if self.spectral_on {
                                    self.band = 0;
                                    self.skip_to_band(Phase::Detect)
                                } else {
                                    Phase::Apply
                                }
                            }
                            _ => Phase::Overlap,
                        };
                        self.next(next);
                    }
                }
                Phase::Detect => {
                    let end = (self.pos + budget).min(bins);
                    self.detect(self.pos, end);
                    budget -= end - self.pos;
                    self.pos = end;
                    if end == bins {
                        self.pos = 0;
                        self.carry = self.tmp[bins - 1];
                        self.phase = Phase::Smooth;
                    }
                }
                Phase::Smooth => {
                    let take = (budget / 2).max(1).min(bins - self.pos);
                    self.smooth(take);
                    budget = budget.saturating_sub(take * 2);
                    self.pos += take;
                    if self.pos >= bins {
                        self.band += 1;
                        let next = self.skip_to_band(Phase::Detect);
                        self.next(next);
                    }
                }
                Phase::Apply => {
                    let take = (budget / 4).max(1).min(bins - self.pos);
                    self.apply(self.pos, self.pos + take);
                    budget = budget.saturating_sub(take * 4);
                    self.pos += take;
                    if self.pos >= bins {
                        self.next(Phase::Ifft);
                    }
                }
                Phase::Overlap => {
                    let end = (self.pos + budget).min(n);
                    let base = self.frame_t.wrapping_sub(n);
                    let norm = 1.0 / (1.5 * n as f32);
                    for i in self.pos..end {
                        let wi = base.wrapping_add(i) & self.mask;
                        let w = self.window[i] * norm;
                        self.acc[0][wi] += self.zr[i] * w;
                        self.acc[1][wi] += self.zi[i] * w;
                    }
                    budget -= end - self.pos;
                    self.pos = end;
                    if end == n {
                        self.activity = self.acts;
                        self.curve = self.curve_acc;
                        self.phase = Phase::Idle;
                    }
                }
                Phase::Idle => {}
            }
        }
    }

    fn next(&mut self, phase: Phase) {
        self.phase = phase;
        self.pos = 0;
    }

    /// The next active spectral band from `self.band`, or on to Apply.
    fn skip_to_band(&mut self, phase: Phase) -> Phase {
        while self.band < BANDS && !self.spec[self.band].active {
            self.band += 1;
        }
        if self.band < BANDS {
            self.carry = 0.0;
            phase
        } else {
            Phase::Apply
        }
    }

    /// Bin k's two channels from the packed transform (L real, R imaginary).
    #[inline]
    fn unpack(re: &[f32], im: &[f32], n: usize, k: usize) -> ((f32, f32), (f32, f32)) {
        let m = (n - k) & (n - 1);
        let (a, b, c, d) = (re[k], im[k], re[m], im[m]);
        (
            ((a + c) * 0.5, (b - d) * 0.5),
            ((b + d) * 0.5, (c - a) * 0.5),
        )
    }

    /// The trigger's raw level per bin, smoothed across frequency upwards.
    fn detect(&mut self, from: usize, to: usize) {
        let b = self.spec[self.band];
        let n = self.n;
        let scale = 4.0 / n as f32;
        let bin_hz = self.sr / n as f32;
        // Neighbouring bins trigger together over a width set by DENSITY.
        let width_oct = 0.5 * fastmath::exp2(-7.0 * b.density);
        for k in from..to {
            let (xl, xr) = if b.ext {
                Self::unpack(&self.cr, &self.ci, n, k)
            } else {
                Self::unpack(&self.zr, &self.zi, n, k)
            };
            let mag = |c: (f32, f32)| (c.0 * c.0 + c.1 * c.1).sqrt();
            let v = match b.place {
                1 => mag(xl),
                2 => mag(xr),
                3 => mag(((xl.0 + xr.0) * 0.5, (xl.1 + xr.1) * 0.5)),
                4 => mag(((xl.0 - xr.0) * 0.5, (xl.1 - xr.1) * 0.5)),
                _ => mag(xl).max(mag(xr)),
            };
            let f = (k as f32 * bin_hz).max(bin_hz * 0.5);
            let mut db = 6.020_6 * fastmath::log2(v * scale + 1e-9);
            if b.tilt {
                db += 3.0 * fastmath::log2(f / 1000.0);
            }
            let span = (k as f32 * width_oct * std::f32::consts::LN_2).max(1.0);
            let a = 1.0 / span;
            self.carry = if k == 0 {
                db
            } else {
                self.carry + (db - self.carry) * a
            };
            self.tmp[k] = self.carry;
        }
    }

    /// Smoothing downwards, then over time, then the gain change: `take`
    /// bins from the top (self.pos counts them).
    fn smooth(&mut self, take: usize) {
        let bi = self.band;
        let b = self.spec[bi];
        let bins = self.bins();
        let width_oct = 0.5 * fastmath::exp2(-7.0 * b.density);
        let frames = self.hop as f32 / self.sr;
        let ka = 1.0 - (-frames / b.attack.max(1e-4)).exp();
        let kr = 1.0 - (-frames / b.release.max(1e-3)).exp();
        let kavg = 1.0 - (-frames / 2.5).exp();
        let channels: &[usize] = match b.place {
            1 => &[L],
            2 => &[R],
            3 => &[M],
            4 => &[S],
            _ => &[L, R],
        };
        for j in 0..take {
            let k = bins - 1 - (self.pos + j);
            let span = (k as f32 * width_oct * std::f32::consts::LN_2).max(1.0);
            self.carry += (self.tmp[k] - self.carry) * (1.0 / span);
            let raw = self.carry;
            let lvl = &mut self.level[bi][k];
            *lvl += (raw - *lvl) * if raw > *lvl { ka } else { kr };
            let avg = &mut self.avg[bi][k];
            if *lvl > -90.0 {
                *avg += (*lvl - *avg) * kavg;
            }
            let thr = if b.auto { *avg + 6.0 } else { b.threshold };
            let weight = at(&self.weight[bi], self.bin_pos[k]);
            if weight < 1e-3 {
                continue;
            }
            let g = dynamic_gain(*lvl - thr, b.range) * weight;
            for &c in channels {
                self.sdb[c][k] += g;
            }
            if g.abs() > self.acts[bi].abs() {
                self.acts[bi] = g;
            }
        }
    }

    /// Gains on bins `from..to`, in place.
    fn apply(&mut self, from: usize, to: usize) {
        let n = self.n;
        const DB2LOG: f32 = 0.166_096_4; // log2(10)/20
        for k in from..to {
            let pos = self.bin_pos[k];
            let spec = |c: usize| {
                if self.spectral_on {
                    self.sdb[c][k]
                } else {
                    0.0
                }
            };
            let gl = fastmath::exp2((at(&self.db[L], pos) + spec(L)) * DB2LOG);
            let gr = fastmath::exp2((at(&self.db[R], pos) + spec(R)) * DB2LOG);
            let ((mut lr, mut li), (mut rr, mut ri)) = Self::unpack(&self.zr, &self.zi, n, k);
            lr *= gl;
            li *= gl;
            rr *= gr;
            ri *= gr;
            if self.ms || self.spectral_on {
                let gm = fastmath::exp2((at(&self.db[M], pos) + spec(M)) * DB2LOG);
                let gs = fastmath::exp2((at(&self.db[S], pos) + spec(S)) * DB2LOG);
                let (mr, mi) = ((lr + rr) * 0.5 * gm, (li + ri) * 0.5 * gm);
                let (sr, si) = ((lr - rr) * 0.5 * gs, (li - ri) * 0.5 * gs);
                (lr, li, rr, ri) = (mr + sr, mi + si, mr - sr, mi - si);
            }
            if self.spectral_on {
                let c = ((pos / (GRID - 1) as f32) * (CURVE - 1) as f32).round() as usize;
                let v = [L, R, M, S]
                    .iter()
                    .map(|&ch| self.sdb[ch][k])
                    .fold(0.0f32, |a, b| if b.abs() > a.abs() { b } else { a });
                if v.abs() > self.curve_acc[c].abs() {
                    self.curve_acc[c] = v;
                }
            }
            let m = (n - k) & (n - 1);
            // Repack: out[k] = YL + j·YR, out[n−k] = conj(YL) + j·conj(YR).
            self.zr[k] = lr - ri;
            self.zi[k] = li + rr;
            if m != k {
                self.zr[m] = lr + ri;
                self.zi[m] = rr - li;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_gains_pass_the_signal_delayed() {
        let sr = 48_000.0;
        let mut st = Stage::new(sr, 4096, 2048);
        st.resize(1024);
        let lat = st.latency();
        let total = 8000;
        let x: Vec<f32> = (0..total)
            .map(|i| ((i as f32 * 0.013).sin() + (i as f32 * 0.21).cos() * 0.3) * 0.5)
            .collect();
        let mut out = vec![0.0f32; total];
        let mut out_r = vec![0.0f32; total];
        for chunk in 0..total / 64 {
            let mut l: Vec<f32> = x[chunk * 64..chunk * 64 + 64].to_vec();
            let mut r = l.iter().map(|v| -v).collect::<Vec<_>>();
            st.process(&mut l, &mut r, None);
            out[chunk * 64..chunk * 64 + 64].copy_from_slice(&l);
            out_r[chunk * 64..chunk * 64 + 64].copy_from_slice(&r);
        }
        for i in lat + 2000..total - 64 {
            assert!(
                (out[i] - x[i - lat]).abs() < 1e-3,
                "sample {i}: {} vs {}",
                out[i],
                x[i - lat]
            );
            assert!((out_r[i] + x[i - lat]).abs() < 1e-3);
        }
    }
}
