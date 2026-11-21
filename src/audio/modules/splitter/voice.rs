//! A Splitter voice: one slice playing, from the sample or from an atom.
//!
//! Played straight, the voice reads its slice at its pitch (faster and
//! higher, or slower and lower, as a sampler does; an atom keeps its timing
//! and changes its frequency), forwards or reversed, once (ONE), as long as
//! it is held (GATE) or looping while held (LOOP), through an attack and a
//! release. A slice ends with its release: on a ONE slice the release is
//! the fade-out before its end.
//!
//! Granular, the voice sprays grains instead: Hann-windowed snippets of
//! SIZE, DENSITY a second (or locked to the beat grid with SYNC), each read
//! at the slice's pitch around a playhead that moves through the slice in
//! real time (so the pitch no longer changes the length), or stands still at
//! POS when FROZEN. SPRAY scatters where the grains start, JITTER their
//! pitch, the node's SPREAD their place in the stereo field.
//!
//! Last comes the crusher: the sample-and-hold of DOWN (holding each value
//! for that many samples), then BITS of resolution.

use super::atoms::{self, ATOM_LEN, ATOM_RATE};
use crate::audio::modules::{Rng, pan_gains};

/// Grains a voice can play at once.
pub const MAX_GRAINS: usize = 16;
/// Points of the grain window's table.
pub const WINDOW: usize = 512;
/// How long a choked voice takes to fall silent.
const CHOKE_SECONDS: f32 = 0.002;
/// Grain densities locked to the grid, in grains per beat.
pub const SYNC_DENSITIES: [f32; 12] = [
    1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0,
];

/// How a slice plays.
pub const ONE: usize = 0;
pub const GATE: usize = 1;
pub const LOOP: usize = 2;

/// What a voice reads: the sample, an atom, or nothing.
#[derive(Clone, Copy)]
pub enum Src<'a> {
    Sample {
        l: &'a [f32],
        r: &'a [f32],
        rate: f32,
    },
    Atom(usize),
    Empty,
}

impl Src<'_> {
    pub fn len(&self) -> f64 {
        match self {
            Src::Sample { l, .. } => l.len() as f64,
            Src::Atom(_) => ATOM_LEN,
            Src::Empty => 0.0,
        }
    }

    /// Frames a second.
    pub fn rate(&self) -> f32 {
        match self {
            Src::Sample { rate, .. } => *rate,
            _ => ATOM_RATE,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() < 2.0
    }

    /// Four-point Hermite between frames (an atom at any position).
    #[inline]
    fn cubic(&self, pos: f64, hz: f32, seed: u32) -> (f32, f32) {
        match self {
            Src::Sample { l, r, .. } => {
                let last = l.len() as isize - 1;
                let i = pos.floor();
                let f = (pos - i) as f32;
                let i = i as isize;
                let at = |b: &[f32], k: isize| b[k.clamp(0, last) as usize];
                let h = |b: &[f32]| hermite(at(b, i - 1), at(b, i), at(b, i + 1), at(b, i + 2), f);
                (h(l), h(r))
            }
            Src::Atom(k) => {
                let y = atoms::atom(*k, pos, hz, seed);
                (y, y)
            }
            Src::Empty => (0.0, 0.0),
        }
    }

    /// Straight-line between frames (grains: many at once).
    #[inline]
    fn linear(&self, pos: f64, hz: f32, seed: u32) -> (f32, f32) {
        match self {
            Src::Sample { l, r, .. } => {
                let last = l.len() - 1;
                let i = (pos.max(0.0) as usize).min(last);
                let j = (i + 1).min(last);
                let f = (pos - i as f64).clamp(0.0, 1.0) as f32;
                (l[i] + (l[j] - l[i]) * f, r[i] + (r[j] - r[i]) * f)
            }
            Src::Atom(k) => {
                let y = atoms::atom(*k, pos, hz, seed);
                (y, y)
            }
            Src::Empty => (0.0, 0.0),
        }
    }
}

#[inline]
fn hermite(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    let c1 = 0.5 * (c - a);
    let c2 = a - 2.5 * b + 2.0 * c - 0.5 * d;
    let c3 = 0.5 * (d - a) + 1.5 * (b - c);
    ((c3 * t + c2) * t + c1) * t + b
}

/// A slice's settings as a voice uses them (read at block rate).
#[derive(Clone, Copy, Debug, Default)]
pub struct Settings {
    pub source: usize,
    pub start: f32,
    pub end: f32,
    /// Semitones, the node's PITCH included.
    pub pitch: f32,
    /// Linear.
    pub gain: f32,
    pub pan: f32,
    pub reverse: bool,
    pub play: usize,
    /// Seconds.
    pub attack: f32,
    pub release: f32,
    /// Bits of resolution (16 or more: clean) and samples held (1: clean),
    /// the node's CRUSH included.
    pub bits: f32,
    pub down: f32,
    pub grain: bool,
    /// Seconds.
    pub size: f32,
    /// Grains a second.
    pub density: f32,
    pub pos: f32,
    pub spray: f32,
    pub freeze: bool,
    pub sync: bool,
    /// Semitones either way.
    pub jitter: f32,
    /// The node's SPREAD: how far grains and hits scatter across the field.
    pub spread: f32,
}

/// What a hit changes in its slice: from CHAOS, the stutter mode and the
/// repeats of a ratchet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Variation {
    /// Semitones added.
    pub pitch: f32,
    /// Played the other way round.
    pub flip: bool,
    /// The share of the slice skipped at its start.
    pub skip: f32,
    /// Linear gain.
    pub gain: f32,
}

impl Variation {
    pub const NONE: Variation = Variation {
        pitch: 0.0,
        flip: false,
        skip: 0.0,
        gain: 1.0,
    };
}

/// What holds a voice's note: a MIDI key, the panel's PLAY button, or a
/// time (a step's GATE), in samples.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Holder {
    Key(u8),
    Panel,
    Timed(u32),
}

/// The board's clock where a voice starts, for grains on the grid.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub beat: f64,
    pub beats_per_sample: f64,
    pub playing: bool,
}

#[derive(Clone, Copy, Default)]
struct Grain {
    pos: f64,
    inc: f64,
    hz: f32,
    phase: f32,
    step: f32,
    gl: f32,
    gr: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Attack,
    Hold,
    Release,
}

pub struct Voice {
    pub active: bool,
    pub slice: usize,
    pub source: usize,
    pub holder: Holder,
    /// When it started (the oldest is stolen first).
    pub age: u64,
    /// Its loudest moment in the last block, for the panel.
    pub level: f32,
    // The slice, in the source's frames.
    start: f64,
    end: f64,
    pos: f64,
    inc: f64,
    dir: f64,
    hz: f32,
    seed: u32,
    play: usize,
    /// Semitones on top of the slice's (CHAOS, a ratchet's ramp, a key).
    pitch_add: f32,
    vel_gain: f32,
    gl: f32,
    gr: f32,
    pan_jitter: f32,
    // Envelope.
    env: f32,
    stage: Stage,
    att_step: f32,
    rel_samples: f32,
    rel_step: f32,
    /// Distance from the end (source frames) where a ONE slice lets go.
    rel_dist: f64,
    choke: f32,
    choke_step: f32,
    gate_left: u32,
    // Crusher.
    q: f32,
    down: f32,
    hold_phase: f32,
    held: (f32, f32),
    // Grains.
    granular: bool,
    frozen: bool,
    sync: bool,
    head: f64,
    head_inc: f64,
    /// Samples left before a ONE granular slice lets go, of how many.
    life: f64,
    life_total: f64,
    grains: [Grain; MAX_GRAINS],
    spawn: f64,
    interval: f64,
    grain_samples: f32,
    spray: f32,
    jitter: f32,
    ratio: f32,
    /// Source frames per output sample at the source's own speed.
    real: f64,
    /// Grains a second asked for.
    density: f32,
    norm: f32,
    spread: f32,
    gain: f32,
    pan: f32,
    rng: Rng,
}

impl Voice {
    pub fn new(seed: u32) -> Self {
        Voice {
            active: false,
            slice: 0,
            source: 0,
            holder: Holder::Timed(0),
            age: 0,
            level: 0.0,
            start: 0.0,
            end: 1.0,
            pos: 0.0,
            inc: 0.0,
            dir: 1.0,
            hz: 440.0,
            seed,
            play: ONE,
            pitch_add: 0.0,
            vel_gain: 1.0,
            gl: 0.7,
            gr: 0.7,
            pan_jitter: 0.0,
            env: 0.0,
            stage: Stage::Attack,
            att_step: 1.0,
            rel_samples: 1.0,
            rel_step: 1.0,
            rel_dist: 0.0,
            choke: 1.0,
            choke_step: 0.0,
            gate_left: 0,
            q: 0.0,
            down: 1.0,
            hold_phase: 0.0,
            held: (0.0, 0.0),
            granular: false,
            frozen: false,
            sync: false,
            head: 0.0,
            head_inc: 0.0,
            life: 0.0,
            life_total: 0.0,
            grains: [Grain::default(); MAX_GRAINS],
            spawn: 0.0,
            interval: 1.0,
            grain_samples: 1.0,
            spray: 0.0,
            jitter: 0.0,
            ratio: 1.0,
            real: 1.0,
            density: 30.0,
            norm: 1.0,
            spread: 0.0,
            gain: 1.0,
            pan: 0.0,
            rng: Rng::new(seed | 1),
        }
    }

    /// Starts slice `slice` with settings `s`.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        slice: usize,
        s: &Settings,
        src: &Src,
        sr: f32,
        velocity: f32,
        holder: Holder,
        var: &Variation,
        clock: &Clock,
        age: u64,
    ) {
        let len = src.len();
        if len < 2.0 {
            self.active = false;
            return;
        }
        let (mut a, mut b) = (
            (s.start.min(s.end) as f64 * len).max(0.0),
            (s.start.max(s.end) as f64 * len).min(len - 1.0),
        );
        let shortest = (0.0005 * src.rate()) as f64;
        if b - a < shortest {
            b = (a + shortest).min(len - 1.0);
            a = (b - shortest).max(0.0);
        }
        self.dir = if s.reverse != var.flip { -1.0 } else { 1.0 };
        let skip = var.skip.clamp(0.0, 0.95) as f64 * (b - a);
        self.active = true;
        self.slice = slice;
        self.source = s.source;
        self.holder = holder;
        self.gate_left = if let Holder::Timed(n) = holder {
            n.max(1)
        } else {
            0
        };
        self.age = age;
        self.start = a;
        self.end = b;
        self.play = s.play.min(LOOP);
        self.pitch_add = var.pitch;
        let v = velocity.clamp(0.0, 1.0);
        self.vel_gain = v * v.sqrt() * var.gain;
        self.pan_jitter = self.rng.bipolar();
        self.seed = self.rng.next_u32();
        self.env = 0.0;
        self.stage = Stage::Attack;
        self.choke = 1.0;
        self.choke_step = 0.0;
        self.hold_phase = f32::MAX;
        self.held = (0.0, 0.0);
        self.granular = s.grain;
        self.grains = [Grain::default(); MAX_GRAINS];
        self.update(s, src, sr);
        if self.dir > 0.0 {
            self.pos = a + skip;
        } else {
            self.pos = b - skip;
        }
        let span = b - a;
        let real = (src.rate() / sr) as f64;
        self.head = if self.dir > 0.0 {
            a + s.pos as f64 * span
        } else {
            b - s.pos as f64 * span
        };
        self.head = (self.head + skip * self.dir).clamp(a, b);
        self.life = if self.frozen {
            span / real
        } else {
            (if self.dir > 0.0 {
                b - self.head
            } else {
                self.head - a
            }) / real
        };
        self.life_total = self.life;
        self.spawn = 0.0;
        self.sync_grains(clock, sr);
        self.att_step = 1.0 / (s.attack * sr).max(1.0);
        self.level = 0.0;
    }

    /// Takes the slice's settings as they are now (block rate): its level,
    /// place, pitch, crusher and grains follow the panel while it plays.
    pub fn update(&mut self, s: &Settings, src: &Src, sr: f32) {
        let semis = s.pitch + self.pitch_add;
        self.ratio = crate::audio::fastmath::exp2(semis / 12.0);
        let real = (src.rate() / sr) as f64;
        self.real = real;
        let atom = matches!(src, Src::Atom(_));
        if let Src::Atom(k) = src {
            self.hz = atoms::hz(*k, semis);
        }
        // An atom keeps its timing; a sample's pitch is its speed.
        let speed = if atom { real } else { real * self.ratio as f64 };
        self.inc = self.dir * speed;
        self.gain = s.gain;
        self.spread = s.spread;
        self.pan = (s.pan + s.spread * self.pan_jitter).clamp(-1.0, 1.0);
        let (l, r) = pan_gains(self.pan);
        self.gl = l * s.gain * self.vel_gain;
        self.gr = r * s.gain * self.vel_gain;
        self.rel_samples = (s.release * sr).max(1.0);
        // A release longer than the slice fades its second half.
        self.rel_dist = (self.rel_samples as f64 * speed).min((self.end - self.start) * 0.5);
        self.q = if s.bits < 15.5 {
            crate::audio::fastmath::exp2(s.bits.max(1.0) - 1.0)
        } else {
            0.0
        };
        self.down = s.down.max(1.0);
        // Grains.
        self.frozen = s.freeze;
        self.sync = s.sync;
        self.head_inc = if s.freeze { 0.0 } else { self.dir * real };
        if s.freeze && self.granular {
            let span = self.end - self.start;
            self.head = if self.dir > 0.0 {
                self.start + s.pos as f64 * span
            } else {
                self.end - s.pos as f64 * span
            };
        }
        self.grain_samples = (s.size * sr).max(16.0);
        self.spray = s.spray;
        self.jitter = s.jitter;
        self.density = s.density.max(0.1);
        if !self.sync {
            self.interval = (sr / self.density) as f64;
        }
        // Overlapping grains add up: keep the cloud near the level of the
        // slice played straight.
        let overlap = (s.density * s.size).max(1.0);
        self.norm = 1.0 / overlap.sqrt();
    }

    /// Grains on the grid: the spawn timer locks to the next grid line
    /// (each block, so it never drifts); off the grid, or with the
    /// transport stopped, they run on their own.
    pub fn sync_grains(&mut self, clock: &Clock, sr: f32) {
        if !self.granular || !self.sync {
            return;
        }
        let bps = clock.beats_per_sample.max(1e-9);
        let wanted = (self.density as f64 / (bps * sr as f64)) as f32;
        let per_beat = nearest_density(wanted) as f64;
        self.interval = 1.0 / (per_beat * bps);
        if clock.playing {
            // The first grid line from half a sample back: a line right at
            // the block's start is not skipped.
            let next = (((clock.beat - 0.5 * bps) * per_beat).floor() + 1.0) / per_beat;
            self.spawn = (next - clock.beat) / bps;
        }
    }

    /// Lets go: the release starts (a held note's end).
    pub fn release(&mut self) {
        if self.stage != Stage::Release {
            self.stage = Stage::Release;
            self.rel_step = self.env / self.rel_samples;
        }
    }

    /// Cut short in 2 ms (the slice struck again, or the voice stolen).
    pub fn choke(&mut self, sr: f32) {
        if self.choke_step == 0.0 {
            self.choke_step = 1.0 / (CHOKE_SECONDS * sr);
        }
    }

    /// Where it plays, as a share of the source (the playhead of grains).
    pub fn position(&self, len: f64) -> f32 {
        let at = if self.granular { self.head } else { self.pos };
        (at / len.max(1.0)) as f32
    }

    /// Its grains' positions, as shares of the source.
    pub fn grain_positions(&self, len: f64, out: &mut [f32]) -> usize {
        let mut n = 0;
        for g in self.grains.iter().filter(|g| g.step > 0.0) {
            if n == out.len() {
                break;
            }
            out[n] = (g.pos / len.max(1.0)) as f32;
            n += 1;
        }
        n
    }

    fn finish(&mut self) {
        self.active = false;
    }

    /// The envelope's next value (and the choke): `None` once silent.
    #[inline]
    fn envelope(&mut self) -> Option<f32> {
        match self.stage {
            Stage::Attack => {
                self.env += self.att_step;
                if self.env >= 1.0 {
                    self.env = 1.0;
                    self.stage = Stage::Hold;
                }
            }
            Stage::Hold => {}
            Stage::Release => {
                self.env -= self.rel_step;
                if self.env <= 0.0 {
                    return None;
                }
            }
        }
        if self.choke_step > 0.0 {
            self.choke -= self.choke_step;
            if self.choke <= 0.0 {
                return None;
            }
        }
        if self.gate_left > 0 {
            self.gate_left -= 1;
            if self.gate_left == 0 {
                self.release();
            }
        }
        Some(self.env * self.env * self.choke)
    }

    /// The crusher: hold, then quantize.
    #[inline]
    fn crush(&mut self, y: (f32, f32)) -> (f32, f32) {
        let mut y = y;
        if self.down > 1.0 {
            self.hold_phase += 1.0;
            if self.hold_phase >= self.down {
                self.hold_phase = if self.hold_phase == f32::MAX {
                    0.0
                } else {
                    self.hold_phase - self.down
                };
                self.held = y;
            }
            y = self.held;
        }
        if self.q > 0.0 {
            let q = self.q;
            y = ((y.0 * q).round() / q, (y.1 * q).round() / q);
        }
        y
    }

    /// Adds `l.len()` samples into `l` and `r`; false once it has ended.
    pub fn render(&mut self, src: &Src, l: &mut [f32], r: &mut [f32], window: &[f32]) -> bool {
        if !self.active {
            return false;
        }
        if src.is_empty() {
            self.finish();
            return false;
        }
        let mut peak = self.level * 0.5;
        if self.granular {
            self.render_grains(src, l, r, window, &mut peak);
        } else {
            self.render_straight(src, l, r, &mut peak);
        }
        self.level = peak;
        self.active
    }

    fn render_straight(&mut self, src: &Src, l: &mut [f32], r: &mut [f32], peak: &mut f32) {
        let looping = self.play == LOOP;
        let span = self.end - self.start;
        // Loops fade in and out over 2 ms (or a quarter of a short loop) at
        // their seam.
        let seam = (0.002 * src.rate() as f64).min(span * 0.25).max(1.0);
        for i in 0..l.len() {
            if self.dir > 0.0 {
                if self.pos >= self.end {
                    if looping {
                        self.pos -= span;
                    } else {
                        self.finish();
                        return;
                    }
                }
            } else if self.pos < self.start {
                if looping {
                    self.pos += span;
                } else {
                    self.finish();
                    return;
                }
            }
            // ONE and GATE slices fade out before their end.
            if !looping && self.stage != Stage::Release {
                let left = if self.dir > 0.0 {
                    self.end - self.pos
                } else {
                    self.pos - self.start
                };
                if left <= self.rel_dist {
                    self.stage = Stage::Release;
                    self.rel_step = self.env / ((left / self.inc.abs()) as f32).max(1.0);
                }
            }
            let Some(mut g) = self.envelope() else {
                self.finish();
                return;
            };
            if looping {
                let edge = (self.pos - self.start).min(self.end - self.pos);
                g *= ((edge / seam) as f32).min(1.0);
            }
            let y = src.cubic(self.pos, self.hz, self.seed);
            self.pos += self.inc;
            let (yl, yr) = self.crush((y.0 * g, y.1 * g));
            let (ol, or) = (yl * self.gl, yr * self.gr);
            l[i] += ol;
            r[i] += or;
            *peak = peak.max(ol.abs().max(or.abs()));
        }
    }

    fn render_grains(
        &mut self,
        src: &Src,
        l: &mut [f32],
        r: &mut [f32],
        window: &[f32],
        peak: &mut f32,
    ) {
        let span = self.end - self.start;
        let one = self.play == ONE;
        let wmax = (window.len() - 1) as f32;
        let atom = matches!(src, Src::Atom(_));
        for i in 0..l.len() {
            // A new grain, on the sample nearest its time.
            if self.spawn < 0.5 {
                self.spawn += self.interval.max(1.0);
                self.add_grain(atom);
            }
            self.spawn -= 1.0;
            // The playhead.
            if self.head_inc != 0.0 {
                self.head += self.head_inc;
                if self.head >= self.end {
                    self.head -= span;
                } else if self.head < self.start {
                    self.head += span;
                }
            }
            if one && self.stage != Stage::Release {
                self.life -= 1.0;
                if self.life <= (self.rel_samples as f64).min(self.life_total * 0.5) {
                    self.stage = Stage::Release;
                    self.rel_step = self.env / (self.life.max(1.0) as f32);
                }
            }
            let Some(g) = self.envelope() else {
                self.finish();
                return;
            };
            let (mut sl, mut sr) = (0.0f32, 0.0f32);
            for gr in self.grains.iter_mut() {
                if gr.step == 0.0 {
                    continue;
                }
                let w = window[(gr.phase * wmax) as usize];
                let (a, b) = src.linear(gr.pos, gr.hz, self.seed);
                sl += a * w * gr.gl;
                sr += b * w * gr.gr;
                gr.pos += gr.inc;
                if gr.pos >= self.end {
                    gr.pos -= span;
                } else if gr.pos < self.start {
                    gr.pos += span;
                }
                gr.phase += gr.step;
                if gr.phase >= 1.0 {
                    gr.step = 0.0;
                }
            }
            let k = g * self.norm * self.gain * self.vel_gain;
            let (yl, yr) = self.crush((sl * k, sr * k));
            l[i] += yl;
            r[i] += yr;
            *peak = peak.max(yl.abs().max(yr.abs()));
        }
    }

    fn add_grain(&mut self, atom: bool) {
        let Some(slot) = self.grains.iter_mut().position(|g| g.step == 0.0) else {
            return;
        };
        let span = self.end - self.start;
        let mut pos = self.head + self.rng.bipolar() as f64 * self.spray as f64 * span * 0.5;
        if pos >= self.end {
            pos -= span;
        }
        if pos < self.start {
            pos += span;
        }
        let pos = pos.clamp(self.start, self.end);
        let detune = if self.jitter > 0.0 {
            crate::audio::fastmath::exp2(self.rng.bipolar() * self.jitter / 12.0)
        } else {
            1.0
        };
        let pan = (self.pan + self.spread * self.rng.bipolar()).clamp(-1.0, 1.0);
        let (gl, gr) = pan_gains(pan);
        let speed = self.dir * self.real;
        let g = &mut self.grains[slot];
        g.pos = pos;
        // An atom's grain keeps time and moves its frequency; a sample's
        // grain reads faster or slower.
        g.inc = if atom {
            speed
        } else {
            speed * (self.ratio * detune) as f64
        };
        g.hz = self.hz * if atom { detune } else { 1.0 };
        g.phase = 0.0;
        g.step = 1.0 / self.grain_samples;
        g.gl = gl;
        g.gr = gr;
    }
}

/// The grid density nearest `per_beat` grains a beat.
pub fn nearest_density(per_beat: f32) -> f32 {
    let target = per_beat.max(0.01).ln();
    SYNC_DENSITIES
        .iter()
        .copied()
        .min_by(|a, b| (a.ln() - target).abs().total_cmp(&(b.ln() - target).abs()))
        .unwrap_or(4.0)
}

/// The grain window: a Hann curve of [`WINDOW`] + 1 points.
pub fn window() -> Vec<f32> {
    (0..=WINDOW)
        .map(|k| 0.5 - 0.5 * (std::f32::consts::TAU * k as f32 / WINDOW as f32).cos())
        .collect()
}
