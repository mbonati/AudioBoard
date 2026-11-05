//! What Vesper does to its layers: two motion filters, drive and chorus on
//! each ([`Strip`]), and the MixType over all of them ([`Master`]).

use std::f32::consts::TAU;

use crate::audio::engine::BLOCK;
use crate::audio::fastmath;
use crate::audio::modules::aether::space::Chorus;
use crate::audio::modules::{DelayLine, Rng, Wander};

/// What a motion filter can be.
pub const FILTERS: [&str; 4] = ["LOW", "HIGH", "BAND", "NOTCH"];
pub const LOW: usize = 0;
pub const HIGH: usize = 1;
pub const BAND: usize = 2;
pub const NOTCH: usize = 3;

/// A motion filter's reach at full MOTION, either way, in octaves.
const REACH: f32 = 2.5;
/// Filters past this do nothing worth computing.
const WIDE_OPEN: f32 = 17_000.0;
const SHUT_OFF: f32 = 21.0;

/// A knob's worth of filter this block.
#[derive(Clone, Copy)]
pub struct Sweep {
    pub kind: usize,
    pub hz: f32,
    /// How far it moves (0…1), and how fast (Hz).
    pub depth: f32,
    pub rate: f32,
    /// How far apart the two sides move (0…1).
    pub width: f32,
}

/// A state-variable filter (TPT) that moves on its own: a slow sine and a
/// slower wander, so that its path doesn't repeat, a little apart on the two
/// sides.
pub struct Motion {
    ic1: [f32; 2],
    ic2: [f32; 2],
    phase: f32,
    wander: Wander,
    rng: Rng,
}

impl Motion {
    pub fn new(seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        Self { ic1: [0.0; 2], ic2: [0.0; 2], phase: rng.unit(), wander: Wander::new(&mut rng), rng }
    }

    /// Filters a block. Returns where the cutoff is now, 0…1 on a log scale
    /// from 20 Hz to 20 kHz.
    pub fn process(&mut self, x: &mut [[f32; BLOCK]; 2], n: usize, s: &Sweep, sr: f32) -> f32 {
        let advance = s.rate * n as f32 / sr;
        self.phase = (self.phase + advance).fract();
        let w = self.wander.tick(&mut self.rng, advance * 0.6);
        let at = |phase: f32| s.hz * fastmath::exp2(s.depth * REACH * (0.7 * fastmath::sin_turns(phase.fract()) + 0.3 * w));
        let now = at(self.phase);
        let position = ((now / 20.0).log10() / 3.0).clamp(0.0, 1.0);
        let idle = s.depth <= 0.0 && ((s.kind == LOW && s.hz >= WIDE_OPEN) || (s.kind == HIGH && s.hz <= SHUT_OFF));
        if idle {
            return position;
        }
        let k = match s.kind {
            BAND => 0.7,
            NOTCH => 1.0,
            _ => 1.2,
        };
        for (ch, side) in x.iter_mut().enumerate() {
            let hz = at(self.phase + ch as f32 * 0.1 * s.width);
            let g = fastmath::tan_turns((hz / sr).clamp(1e-4, 0.23) * 0.5);
            let a1 = 1.0 / (1.0 + g * (g + k));
            let a2 = g * a1;
            let a3 = g * a2;
            let (mut ic1, mut ic2) = (self.ic1[ch], self.ic2[ch]);
            for y in &mut side[..n] {
                let v3 = *y - ic2;
                let v1 = a1 * ic1 + a2 * v3;
                let v2 = ic2 + a2 * ic1 + a3 * v3;
                ic1 = 2.0 * v1 - ic1;
                ic2 = 2.0 * v2 - ic2;
                *y = match s.kind {
                    LOW => v2,
                    HIGH => *y - k * v1 - v2,
                    BAND => v1 * k * 1.4,
                    _ => *y - k * v1,
                };
            }
            self.ic1[ch] = ic1;
            self.ic2[ch] = ic2;
        }
        position
    }
}

/// One layer's knobs this block.
#[derive(Clone, Copy)]
pub struct StripSettings {
    pub first: Sweep,
    pub second: Sweep,
    /// How far the first filter starts below itself on each new chord (0…1),
    /// and the layer's attack, which it opens with.
    pub bloom: f32,
    pub bloom_time: f32,
    pub drive: f32,
    pub chorus: f32,
    pub width: f32,
    pub level: f32,
    pub echo: f32,
    pub verb: f32,
}

/// One layer after its voices: the two filters, drive, chorus, width, then
/// its level and its two sends.
pub struct Strip {
    first: Motion,
    second: Motion,
    chorus: Chorus,
    level: f32,
    echo: f32,
    verb: f32,
    /// Blocks without voices: past [`Self::IDLE`], nothing is left to hear.
    idle: u32,
    /// BLOOM: how shut the first filter is now (0…1), and whether it's still
    /// closing (quickly, so that a tail doesn't lurch).
    shut: f32,
    closing: bool,
}

/// BLOOM's reach at the top of the knob, in octaves, and how long the filter
/// takes to close before it opens.
const BLOOM_REACH: f32 = 4.5;
const BLOOM_CLOSE: f32 = 0.03;

impl Strip {
    const IDLE: u32 = 200;

    pub fn new(seed: u32, sr: f32) -> Self {
        Self {
            first: Motion::new(seed),
            second: Motion::new(seed.wrapping_add(31)),
            chorus: Chorus::new(sr),
            level: 0.0,
            echo: 0.0,
            verb: 0.0,
            idle: Self::IDLE,
            shut: 0.0,
            closing: false,
        }
    }

    /// A new chord: the filter closes, and opens again as it comes in.
    pub fn bloom_again(&mut self) {
        self.closing = true;
    }

    pub fn note_activity(&mut self, sounding: bool) {
        self.idle = if sounding { 0 } else { self.idle.saturating_add(1) };
    }

    pub fn resting(&self) -> bool {
        self.idle >= Self::IDLE
    }

    /// Shapes `x` in place and adds it to the buses. Returns its peak (before
    /// the level, so that a meter shows what's there).
    #[allow(clippy::needless_range_loop)] // the two sides and the buses at once
    pub fn process(&mut self, x: &mut [[f32; BLOCK]; 2], s: &StripSettings, buses: &mut Buses, n: usize, sr: f32) -> (f32, f32, f32) {
        let secs = n as f32 / sr;
        if self.closing {
            self.shut += secs / BLOOM_CLOSE;
            if self.shut >= 1.0 {
                self.shut = 1.0;
                self.closing = false;
            }
        } else {
            self.shut *= (-secs / (0.15 + 0.7 * s.bloom_time)).exp();
        }
        let first = Sweep { hz: s.first.hz * fastmath::exp2(-BLOOM_REACH * s.bloom * self.shut), ..s.first };
        let f1 = self.first.process(x, n, &first, sr);
        let f2 = self.second.process(x, n, &s.second, sr);
        if s.drive > 1e-3 {
            let g = 1.0 + 7.0 * s.drive * s.drive;
            let trim = g.powf(-0.72);
            for y in x.iter_mut().flat_map(|side| side[..n].iter_mut()) {
                *y = fastmath::tanh(*y * g) * trim;
            }
        }
        if s.chorus > 1e-3 {
            self.chorus.begin(0.17 / sr, s.chorus, s.chorus, n);
            for i in 0..n {
                let y = self.chorus.process([x[0][i], x[1][i]]);
                (x[0][i], x[1][i]) = (y[0], y[1]);
            }
        }
        let inv = 1.0 / n as f32;
        let (dl, de, dv) = ((s.level - self.level) * inv, (s.echo - self.echo) * inv, (s.verb - self.verb) * inv);
        let side = s.width;
        let mut peak = 0.0f32;
        for i in 0..n {
            let (l, r) = (x[0][i], x[1][i]);
            let mid = 0.5 * (l + r);
            let wide = 0.5 * (l - r) * side;
            let (l, r) = (mid + wide, mid - wide);
            peak = peak.max(l.abs()).max(r.abs());
            self.level += dl;
            self.echo += de;
            self.verb += dv;
            let (l, r) = (l * self.level, r * self.level);
            buses.dry[0][i] += l;
            buses.dry[1][i] += r;
            buses.echo[0][i] += l * self.echo;
            buses.echo[1][i] += r * self.echo;
            buses.verb[0][i] += l * self.verb;
            buses.verb[1][i] += r * self.verb;
        }
        (peak, f1, f2)
    }
}

/// What the layers add up to, before the effects they share.
pub struct Buses {
    pub dry: [[f32; BLOCK]; 2],
    pub echo: [[f32; BLOCK]; 2],
    pub verb: [[f32; BLOCK]; 2],
}

/// A MixType: what it does to the whole mix.
#[derive(Clone, Copy)]
pub struct MixShape {
    /// Saturation (0…1).
    pub drive: f32,
    /// A two-pole low-pass, in Hz.
    pub dark: f32,
    /// A lift above 2.5 kHz (0: none).
    pub presence: f32,
    /// A slow, wide chorus over everything.
    pub haze: f32,
    /// Tape's wow and flutter.
    pub wow: f32,
    /// Added to SPACE, and how far the tail's highs are lifted.
    pub space: f32,
    pub air: f32,
    /// How far towards five bits.
    pub crush: f32,
}

/// The MixTypes: name, what it does, its shape.
pub const MIX_TYPES: [(&str, &str, MixShape); 4] = [
    (
        "CLEAN",
        "Neutral: the layers as they are",
        MixShape { drive: 0.0, dark: 20_000.0, presence: 0.0, haze: 0.0, wow: 0.0, space: 0.0, air: 0.25, crush: 0.0 },
    ),
    (
        "DREAM",
        "Hazy and far away: a slow chorus over everything, softer highs, a longer and brighter tail",
        MixShape { drive: 0.05, dark: 9000.0, presence: 0.0, haze: 0.6, wow: 0.15, space: 0.22, air: 0.6, crush: 0.0 },
    ),
    (
        "DUST",
        "Worn tape: warm saturation, wow and flutter, the top rolled off",
        MixShape { drive: 0.45, dark: 7000.0, presence: 0.0, haze: 0.0, wow: 0.8, space: 0.0, air: 0.12, crush: 0.0 },
    ),
    (
        "EDGE",
        "Harder and closer: driven, lifted presence, a touch of bit grit, a shorter tail",
        MixShape { drive: 0.8, dark: 20_000.0, presence: 0.5, haze: 0.0, wow: 0.0, space: -0.1, air: 0.35, crush: 0.28 },
    ),
];

/// A MixType coming in takes this long.
const MIX_GLIDE: f32 = 0.4;
/// The wow's delay: its middle, its reach, in seconds.
const WOW_DELAY: f32 = 0.009;
const WOW_REACH: f32 = 0.004;

/// The MixType's stage over the whole mix.
pub struct Master {
    sr: f32,
    now: MixShape,
    /// DC blocker after the saturation's bias: last input and output.
    dc: [(f32, f32); 2],
    dc_r: f32,
    haze: Chorus,
    wow: [DelayLine; 2],
    delay: f32,
    step: f32,
    slow: f32,
    fast: f32,
    dark: [[f32; 2]; 2],
    dark_a: f32,
    lift: [f32; 2],
    lift_a: f32,
}

impl Master {
    pub fn new(sr: f32) -> Self {
        let len = ((WOW_DELAY + WOW_REACH) * sr) as usize + 16;
        Self {
            sr,
            now: MIX_TYPES[0].2,
            dc: [(0.0, 0.0); 2],
            dc_r: 1.0 - TAU * 8.0 / sr,
            haze: Chorus::new(sr),
            wow: [DelayLine::new(len), DelayLine::new(len)],
            delay: WOW_DELAY * sr,
            step: 0.0,
            slow: 0.0,
            fast: 0.0,
            dark: [[0.0; 2]; 2],
            dark_a: 1.0,
            lift: [0.0; 2],
            lift_a: TAU * 2500.0 / sr,
        }
    }

    /// The shape now, on its way to the MixType picked.
    pub fn shape(&self) -> MixShape {
        self.now
    }

    pub fn begin(&mut self, which: usize, n: usize) {
        let secs = n as f32 / self.sr;
        let k = 1.0 - (-secs / MIX_GLIDE).exp();
        let to = MIX_TYPES[which.min(MIX_TYPES.len() - 1)].2;
        let now = &mut self.now;
        now.drive += (to.drive - now.drive) * k;
        // Frequencies move evenly in pitch.
        now.dark *= (to.dark / now.dark).powf(k);
        now.presence += (to.presence - now.presence) * k;
        now.haze += (to.haze - now.haze) * k;
        now.wow += (to.wow - now.wow) * k;
        now.space += (to.space - now.space) * k;
        now.air += (to.air - now.air) * k;
        now.crush += (to.crush - now.crush) * k;

        self.haze.begin(0.09 / self.sr, now.haze, now.haze, n);
        self.dark_a = 1.0 - (-TAU * now.dark.min(self.sr * 0.45) / self.sr).exp();

        self.slow = (self.slow + 0.37 * secs).fract();
        self.fast = (self.fast + 5.3 * secs).fract();
        let wobble = 0.85 * fastmath::sin_turns(self.slow) + 0.08 * fastmath::sin_turns(self.fast);
        let to_delay = (WOW_DELAY + now.wow * WOW_REACH * wobble) * self.sr;
        self.step = (to_delay - self.delay) / n as f32;
    }

    #[inline]
    pub fn process(&mut self, x: [f32; 2]) -> [f32; 2] {
        let now = self.now;
        let mut y = x;
        // The wow's delay always runs (no jump in time when it comes in);
        // the other stages step aside when they aren't wanted.
        self.delay += self.step;
        for (s, line) in y.iter_mut().zip(&mut self.wow) {
            line.push(*s);
            *s = line.tap(self.delay);
        }
        if now.haze > 1e-3 {
            y = self.haze.process(y);
        }
        if now.drive > 1e-3 {
            // A tanh with a little bias, faded in from nothing.
            let g = 1.0 + 7.0 * now.drive * now.drive;
            let bias = 0.22 * now.drive;
            let tb = fastmath::tanh(bias);
            let makeup = 1.0 / (g.powf(0.65) * (1.0 - tb * tb));
            let blend = (now.drive * 10.0).min(1.0);
            for s in &mut y {
                let shaped = (fastmath::tanh(g * *s + bias) - tb) * makeup;
                *s += blend * (shaped - *s);
            }
        }
        for (s, dc) in y.iter_mut().zip(&mut self.dc) {
            let out = *s - dc.0 + self.dc_r * dc.1;
            *dc = (*s, out);
            *s = out;
        }
        if self.now.dark < 19_000.0 {
            for (ch, s) in y.iter_mut().enumerate() {
                let [a, b] = &mut self.dark[ch];
                *a += self.dark_a * (*s - *a);
                *b += self.dark_a * (*a - *b);
                *s = *b;
            }
        }
        if now.presence > 1e-3 {
            for (ch, s) in y.iter_mut().enumerate() {
                self.lift[ch] += self.lift_a * (*s - self.lift[ch]);
                *s += now.presence * (*s - self.lift[ch]);
            }
        }
        if now.crush > 1e-3 {
            for s in &mut y {
                let held = (*s * 16.0).round() / 16.0;
                *s += now.crush * (held - *s);
            }
        }
        y
    }
}
