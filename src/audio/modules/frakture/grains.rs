//! GRAINS: a stereo buffer recording the input in a loop locked to the
//! tempo (BUFFER long), and two layers of grains playing from it. Each
//! layer finds its grains around a playhead (fixed, scanning, or trailing
//! the recording like a delay), spawns them at a rate or on a note grid,
//! and gives each its own pitch, direction, pan and envelope shape; CHAOS
//! randomizes the last three.

use std::f32::consts::SQRT_2;

use super::{Band, Clock, OVERVIEW, VIZ_GRAINS, quantize, viz};
use crate::audio::engine::{BLOCK, Buf};
use crate::audio::fastmath;
use crate::audio::modules::{ModuleParams, Rng, approach, pan_gains, time_coeff};

const MAX_GRAINS: usize = 64;
/// The longest buffer, in seconds (4 bars at 40 BPM).
const MAX_SECONDS: f32 = 24.0;

/// How often a layer spawns grains.
#[derive(Clone, Copy)]
pub enum Rate {
    /// One per note value (in beats), on the transport's grid.
    Grid(f64),
    Hz(f32),
}

/// Where a layer's grains come from.
#[derive(Clone, Copy)]
pub enum Playhead {
    /// A fraction of the buffer.
    At(f32),
    /// Moving through the buffer at this speed (times real time).
    Scan(f32),
    /// Samples behind the recording.
    Behind(f64),
    /// A fraction of the buffer behind the recording.
    BehindFraction(f32),
}

#[derive(Clone, Copy)]
pub struct LayerSettings {
    pub on: bool,
    pub playhead: Playhead,
    /// Spread of grain positions around the playhead, as a fraction of the
    /// buffer.
    pub range: f32,
    pub density: Rate,
    /// Grain length in samples.
    pub size: f32,
    pub chaos: f32,
    /// −1 all reversed … 1 all forwards.
    pub direction: f32,
    pub pitch: f32,
    pub spread: f32,
    pub pitch_mode: usize,
    pub width: f32,
    pub shape: f32,
    pub dry: f32,
    pub wet: f32,
}

#[derive(Clone, Copy)]
pub struct GrainSettings {
    pub lo: f32,
    pub hi: f32,
    pub buffer_beats: f64,
    /// Recording (not frozen, input coming through).
    pub record: bool,
    /// RETRIG on transport start: the buffer starts over.
    pub refresh: bool,
    pub layers: [LayerSettings; 2],
}

/// A grain's envelope ramps for SHAPE, as fractions of the grain: (attack,
/// release). Morphs from a percussive decay through a smooth bell and a
/// trapezoid to a square gate.
pub fn shape_ramps(shape: f32) -> (f32, f32) {
    const KEYS: [(f32, f32); 4] = [(0.0, 1.0), (0.5, 0.5), (0.15, 0.15), (0.0, 0.0)];
    let x = shape.clamp(0.0, 1.0) * 3.0;
    let i = (x as usize).min(2);
    let f = x - i as f32;
    let (a0, r0) = KEYS[i];
    let (a1, r1) = KEYS[i + 1];
    (a0 + (a1 - a0) * f, r0 + (r1 - r0) * f)
}

/// Smoothstep: an S-shaped ramp from 0 to 1.
#[inline]
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Copy, Default)]
struct Grain {
    active: bool,
    /// Frame in the buffer.
    pos: f64,
    inc: f64,
    age: u32,
    len: u32,
    inv_len: f32,
    attack: f32,
    release: f32,
    inv_attack: f32,
    inv_release: f32,
    /// Gains for the mid signal (pan) and the side signal (width kept).
    gl: f32,
    gr: f32,
    side: f32,
    /// For the display.
    pitch: f32,
}

impl Grain {
    #[inline]
    fn envelope(&self) -> f32 {
        let x = self.age as f32 * self.inv_len;
        if x < self.attack {
            smooth(x * self.inv_attack)
        } else if x > 1.0 - self.release {
            smooth(((1.0 - x) * self.inv_release).max(0.0))
        } else {
            1.0
        }
    }
}

struct Layer {
    grains: [Grain; MAX_GRAINS],
    /// One past the highest slot that may be playing.
    top: usize,
    countdown: f32,
    /// The last grid line crossed (synced density).
    grid: i64,
    /// SCAN's position, in frames.
    scan: f64,
    rng: Rng,
    norm: f32,
    wet: f32,
    playhead: Playhead,
}

impl Layer {
    fn new(seed: u32) -> Self {
        Self {
            grains: [Grain::default(); MAX_GRAINS],
            top: 0,
            countdown: 0.0,
            grid: i64::MIN,
            scan: 0.0,
            rng: Rng::new(seed),
            norm: 1.0,
            wet: 0.0,
            playhead: Playhead::At(0.0),
        }
    }

    /// The playhead's frame, with the write head at `w`.
    fn head(&self, w: usize, len: usize) -> f64 {
        let lenf = len as f64;
        let at = match self.playhead {
            Playhead::At(v) => v as f64 * lenf,
            Playhead::Scan(_) => self.scan,
            Playhead::Behind(d) => w as f64 - d,
            Playhead::BehindFraction(f) => w as f64 - f as f64 * lenf,
        };
        at.rem_euclid(lenf)
    }

    fn spawn(&mut self, s: &LayerSettings, w: usize, len: usize) {
        let Some(slot) = self.grains.iter().position(|g| !g.active) else {
            return;
        };
        let lenf = len as f64;
        let head = self.head(w, len);
        let rng = &mut self.rng;
        let mut start = (head + (s.range * rng.bipolar() * 0.5) as f64 * lenf).rem_euclid(lenf);
        let semis = quantize(s.pitch + s.spread * rng.bipolar(), s.pitch_mode).clamp(-48.0, 48.0);
        let ratio = fastmath::exp2(semis / 12.0) as f64;
        // CHAOS pulls direction, shape and width towards random.
        let forward = (s.direction + 1.0) * 0.5;
        let forward = forward + (0.5 - forward) * s.chaos;
        let reverse = rng.unit() >= forward;
        let shape = (s.shape + s.chaos * rng.bipolar() * 0.5).clamp(0.0, 1.0);
        let width = (s.width + s.chaos * rng.bipolar() * 0.5).clamp(0.0, 1.0);
        let pan = if rng.unit() < width {
            rng.bipolar().signum() * (0.35 + 0.65 * rng.unit())
        } else {
            0.15 * width * rng.bipolar()
        };
        let len_g = s.size.clamp(64.0, 16.0 * 48_000.0) as u32;
        let (mut a, mut r) = shape_ramps(shape);
        let min_ramp = (48.0 / len_g as f32).min(0.5);
        a = a.max(min_ramp);
        r = r.max(min_ramp);
        if a + r > 1.0 {
            let k = 1.0 / (a + r);
            a *= k;
            r *= k;
        }
        // Trailing the recording, a grain sped up must not catch it.
        if matches!(s.playhead, Playhead::Behind(_) | Playhead::BehindFraction(_)) && !reverse && ratio > 1.0 {
            let need = len_g as f64 * (ratio - 1.0) + 2.0;
            if (w as f64 - start).rem_euclid(lenf) < need && need < lenf {
                start = (w as f64 - need).rem_euclid(lenf);
            }
        }
        let (cl, cr) = pan_gains(pan);
        self.grains[slot] = Grain {
            active: true,
            pos: start,
            inc: if reverse { -ratio } else { ratio },
            age: 0,
            len: len_g,
            inv_len: 1.0 / len_g as f32,
            attack: a,
            release: r,
            inv_attack: 1.0 / a,
            inv_release: 1.0 / r,
            gl: cl * SQRT_2,
            gr: cr * SQRT_2,
            side: 1.0 - pan.abs(),
            pitch: semis,
        };
        self.top = self.top.max(slot + 1);
    }
}

pub struct Grains {
    sr: f32,
    smooth: f32,
    /// Interleaved L/R.
    buf: Box<[f32]>,
    cap: usize,
    /// The loop's length in frames, and the write head.
    len: usize,
    w: usize,
    /// Frames recorded since the buffer started over (beyond them it holds
    /// nothing yet).
    written: usize,
    layers: [Layer; 2],
    dry: f32,
    band: Band,
    overview: [f32; OVERVIEW],
    bin: usize,
}

impl Grains {
    pub fn new(sample_rate: f32) -> Self {
        let cap = (MAX_SECONDS * sample_rate) as usize;
        Self {
            sr: sample_rate,
            smooth: time_coeff(0.01, sample_rate),
            buf: vec![0.0; cap * 2].into_boxed_slice(),
            cap,
            len: cap,
            w: 0,
            written: 0,
            layers: [Layer::new(0x6A41_0F01), Layer::new(0x1B7C_93E5)],
            dry: 1.0,
            band: Band::default(),
            overview: [0.0; OVERVIEW],
            bin: 0,
        }
    }

    /// Frames the buffer can hold.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    #[allow(clippy::needless_range_loop)] // both channels, sample by sample
    pub fn process(&mut self, x: &mut Buf, n: usize, s: &GrainSettings, clock: &Clock) {
        let len = ((s.buffer_beats * clock.samples_per_beat).round() as usize).clamp(BLOCK * 4, self.cap);
        if len != self.len {
            self.len = len;
            self.w %= len;
            self.written = self.written.min(len);
            for layer in &mut self.layers {
                layer.scan = layer.scan.rem_euclid(len as f64);
                for g in layer.grains.iter_mut().filter(|g| g.active) {
                    g.pos = g.pos.rem_euclid(len as f64);
                }
            }
        }
        let lenf = len as f64;
        if s.refresh {
            self.written = 0;
            self.w = 0;
            for layer in &mut self.layers {
                layer.scan = 0.0;
                layer.grid = i64::MIN;
            }
        }
        // The write head follows the tempo: back in step whenever it drifts.
        let target = (((clock.beat0 / s.buffer_beats).rem_euclid(1.0) * lenf) as usize).min(len - 1);
        let drift = (self.w as i64 - target as i64).rem_euclid(len as i64);
        if drift.min(len as i64 - drift) > 256 {
            self.w = target;
        }
        self.band.set(s.lo, s.hi, self.sr);

        let sr = self.sr;
        let spb = clock.samples_per_beat as f32;
        let mut dry_sum = 0.0;
        let mut dry_n = 0;
        let mut norm_t = [1.0f32; 2];
        for (k, (layer, set)) in self.layers.iter_mut().zip(&s.layers).enumerate() {
            layer.playhead = set.playhead;
            if set.on {
                dry_sum += set.dry;
                dry_n += 1;
            }
            // Overlapping grains add up: keep each layer's level steady.
            let per_second = match set.density {
                Rate::Grid(beats) => sr / (beats as f32 * spb),
                Rate::Hz(hz) => hz,
            };
            norm_t[k] = 1.0 / (per_second * set.size / sr * 0.7).max(1.0).sqrt();
        }
        let dry_t = if dry_n > 0 { dry_sum / dry_n as f32 } else { 1.0 };

        let Grains { buf, layers, band, overview, .. } = self;
        let bin_len = (len / OVERVIEW).max(1);
        for i in 0..n {
            let (in_l, in_r) = (x[0][i], x[1][i]);
            if s.record {
                buf[self.w * 2] = in_l;
                buf[self.w * 2 + 1] = in_r;
                let bin = (self.w / bin_len).min(OVERVIEW - 1);
                if bin != self.bin {
                    self.bin = bin;
                    overview[bin] = 0.0;
                }
                overview[bin] = overview[bin].max(in_l.abs().max(in_r.abs()));
                self.written = (self.written + 1).min(self.cap);
            }
            self.w += 1;
            if self.w >= len {
                self.w = 0;
            }
            let valid = self.written.min(len);
            let beat = clock.at(i);

            let (mut wl, mut wr) = (0.0f32, 0.0f32);
            for (k, layer) in layers.iter_mut().enumerate() {
                let set = &s.layers[k];
                approach(&mut layer.norm, norm_t[k], self.smooth);
                approach(&mut layer.wet, if set.on { set.wet } else { 0.0 }, self.smooth);
                if let Playhead::Scan(speed) = set.playhead {
                    layer.scan = (layer.scan + speed as f64).rem_euclid(lenf);
                }
                if set.on {
                    let due = match set.density {
                        Rate::Grid(beats) => {
                            let g = (beat / beats).floor() as i64;
                            let due = g != layer.grid;
                            layer.grid = g;
                            due
                        }
                        Rate::Hz(hz) => {
                            layer.countdown -= 1.0;
                            let due = layer.countdown <= 0.0;
                            if due {
                                layer.countdown += (sr / hz).max(1.0);
                            }
                            due
                        }
                    };
                    if due {
                        layer.spawn(set, self.w, len);
                    }
                }
                if layer.top == 0 {
                    continue;
                }
                let (mut ll, mut lr) = (0.0f32, 0.0f32);
                let mut top = 0;
                for (slot, g) in layer.grains[..layer.top].iter_mut().enumerate() {
                    if !g.active {
                        continue;
                    }
                    let env = g.envelope();
                    let (l, r) = read(buf, len, valid, g.pos);
                    let mid = (l + r) * 0.5;
                    let side = (l - r) * 0.5 * g.side;
                    ll += env * (mid * g.gl + side);
                    lr += env * (mid * g.gr - side);
                    g.pos += g.inc;
                    if g.pos >= lenf {
                        g.pos -= lenf;
                    } else if g.pos < 0.0 {
                        g.pos += lenf;
                    }
                    g.age += 1;
                    g.active = g.age < g.len;
                    if g.active {
                        top = slot + 1;
                    }
                }
                layer.top = top;
                let gain = layer.wet * layer.norm;
                wl += ll * gain;
                wr += lr * gain;
            }

            approach(&mut self.dry, dry_t, self.smooth);
            x[0][i] = in_l * self.dry + band.process(0, wl);
            x[1][i] = in_r * self.dry + band.process(1, wr);
        }
    }

    pub fn publish(&self, p: &ModuleParams) {
        let viz = &p.viz;
        let lenf = self.len as f64;
        viz[viz::BUF_SECONDS].set(self.len as f32 / self.sr);
        viz[viz::WRITE_HEAD].set(self.w as f32 / self.len as f32);
        for (k, layer) in self.layers.iter().enumerate() {
            viz[viz::PLAYHEADS + k].set((layer.head(self.w, self.len) / lenf) as f32);
            let mut shown = 0;
            let mut count = 0;
            for g in layer.grains[..layer.top].iter().filter(|g| g.active) {
                count += 1;
                if shown < VIZ_GRAINS {
                    let at = viz::GRAINS + (k * VIZ_GRAINS + shown) * 3;
                    viz[at].set((g.pos / lenf) as f32);
                    viz[at + 1].set(g.pitch);
                    viz[at + 2].set(g.envelope() * layer.wet.min(1.0));
                    shown += 1;
                }
            }
            for s in shown..VIZ_GRAINS {
                viz[viz::GRAINS + (k * VIZ_GRAINS + s) * 3 + 2].set(-1.0);
            }
            viz[viz::GRAIN_COUNT + k].set(count as f32);
        }
        for (k, v) in self.overview.iter().enumerate() {
            viz[viz::OVERVIEW_AT + k].set(*v);
        }
    }
}

/// The buffer at a fractional frame, linearly interpolated. Frames not
/// recorded since the buffer started over read as silence.
#[inline]
fn read(buf: &[f32], len: usize, valid: usize, pos: f64) -> (f32, f32) {
    let i = (pos as usize).min(len - 1);
    let f = (pos - i as f64) as f32;
    let j = if i + 1 >= len { 0 } else { i + 1 };
    let at = |k: usize| if k < valid { (buf[k * 2], buf[k * 2 + 1]) } else { (0.0, 0.0) };
    let (al, ar) = at(i);
    let (bl, br) = at(j);
    (al + (bl - al) * f, ar + (br - ar) * f)
}
