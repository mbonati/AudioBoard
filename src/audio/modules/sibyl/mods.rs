//! Sibyl's global modulation: the LFO (and the wheel's noise), and the
//! modulation matrix.
//!
//! The matrix has four slots (two pages of two, MM A and MM B). Each takes a
//! source, shapes it (curve, rectifier), scales it by DEPTH, quantizes it,
//! can sample & hold it on another signal's rising zero crossing and slew
//! it, and adds it to a destination. Sources and destinations that belong
//! to a voice (its envelopes, velocity, key, pitch, filter…) are worked out
//! per voice; the effects, the LFO's rate and the wheel's mix and limits
//! take the newest voice's sources.
//!
//! Offsets move the destination knob's position (0..1 of its travel; pitch
//! destinations in their own ranges), as if a hand turned it.

use super::voice::{Patch, Voice};
use super::{MAX_VOICES, PARAM_COUNT, SLOT_LEN, slot, sp};
use crate::audio::fastmath;
use audioboard_node::dsp::Rng;

pub const SLOTS: usize = 4;

pub const SOURCES: [&str; 16] = [
    "—",
    "Mod Wheel",
    "Pitch Wheel",
    "Control A",
    "Control B",
    "LFO",
    "Trigger",
    "Gate",
    "Key Follow",
    "Key + Bend",
    "Velocity",
    "Aftertouch",
    "Filter Env",
    "Amp Env",
    "Voice Index",
    "Voice Detune",
];

pub mod src {
    pub const NONE: usize = 0;
    pub const WHEEL: usize = 1;
    pub const BEND: usize = 2;
    pub const CC_A: usize = 3;
    pub const CC_B: usize = 4;
    pub const LFO: usize = 5;
    pub const TRIGGER: usize = 6;
    pub const GATE: usize = 7;
    pub const KEY: usize = 8;
    pub const KEY_BEND: usize = 9;
    pub const VELOCITY: usize = 10;
    pub const PRESSURE: usize = 11;
    pub const FENV: usize = 12;
    pub const AENV: usize = 13;
    pub const INDEX: usize = 14;
    pub const DETUNE: usize = 15;
}

pub mod dest {
    pub const NONE: usize = 0;
    pub const LFO_RATE: usize = 1;
    pub const PAN: usize = 2;
    pub const WM_MIX: usize = 3;
    pub const WM_LOW: usize = 4;
    pub const WM_HIGH: usize = 5;
    pub const GLIDE: usize = 6;
    pub const PITCH: usize = 7;
    pub const F_ATTACK: usize = 8;
    pub const F_DECAY: usize = 9;
    pub const F_SUSTAIN: usize = 10;
    pub const F_RELEASE: usize = 11;
    pub const F_VEL: usize = 12;
    pub const A_ATTACK: usize = 13;
    pub const A_DECAY: usize = 14;
    pub const A_SUSTAIN: usize = 15;
    pub const A_RELEASE: usize = 16;
    pub const A_VEL: usize = 17;
    pub const A_FREQ: usize = 18;
    pub const A_FINE: usize = 19;
    pub const A_PW: usize = 20;
    pub const B_FREQ: usize = 21;
    pub const B_FINE: usize = 22;
    pub const B_PW: usize = 23;
    pub const PM_FENV: usize = 24;
    pub const PM_OSCB: usize = 25;
    pub const MIX_A: usize = 26;
    pub const MIX_B: usize = 27;
    pub const MIX_NOISE: usize = 28;
    pub const CUTOFF: usize = 29;
    pub const RESO: usize = 30;
    pub const KEY_AMT: usize = 31;
    pub const ENV_AMT: usize = 32;
    pub const DIST_AMOUNT: usize = 33;
    pub const DIST_TONE: usize = 34;
    pub const DIST_MIX: usize = 35;
    pub const TAPE_GAIN: usize = 36;
    pub const ECHO_TIME: usize = 37;
    pub const ECHO_REGEN: usize = 38;
    pub const ECHO_MIX: usize = 39;
    pub const PLATE_DECAY: usize = 40;
    pub const PLATE_MIX: usize = 41;
    pub const COND_GAIN: usize = 42;
    pub const COND_WIDTH: usize = 43;
    pub const VOLUME: usize = 44;
    pub const COUNT: usize = 45;
}

pub const DESTS: [&str; dest::COUNT] = [
    "—",
    "LFO Rate",
    "Pan",
    "Wheel Mix",
    "Wheel Low",
    "Wheel High",
    "Glide",
    "Pitch",
    "F.Env Attack",
    "F.Env Decay",
    "F.Env Sustain",
    "F.Env Release",
    "F.Env Velocity",
    "A.Env Attack",
    "A.Env Decay",
    "A.Env Sustain",
    "A.Env Release",
    "A.Env Velocity",
    "Osc A Freq",
    "Osc A Fine",
    "Osc A PW",
    "Osc B Freq",
    "Osc B Fine",
    "Osc B PW",
    "PM Filt Env",
    "PM Osc B",
    "Mix Osc A",
    "Mix Osc B",
    "Mix Noise",
    "Cutoff",
    "Resonance",
    "Key Amount",
    "Env Amount",
    "Dist Amount",
    "Dist Tone",
    "Dist Mix",
    "Tape Gain",
    "Echo Time",
    "Echo Regen",
    "Echo Mix",
    "Plate Decay",
    "Plate Mix",
    "Cond Gain",
    "Cond Width",
    "Volume",
];

/// The destinations shared by all voices (taken from the newest voice).
pub fn is_global_dest(d: usize) -> bool {
    matches!(
        d,
        dest::LFO_RATE | dest::WM_MIX | dest::WM_LOW | dest::WM_HIGH
    ) || (dest::TAPE_GAIN..=dest::COND_WIDTH).contains(&d)
}

pub const CURVES: [&str; 5] = [
    "Linear",
    "Compressed",
    "Very compressed",
    "Expanded",
    "Very expanded",
];
pub const RECTIFIERS: [&str; 6] = [
    "None",
    "Half wave +",
    "Half wave −",
    "Full wave +",
    "Full wave −",
    "Unipolar",
];
pub const QUANTIZERS: [&str; 11] = [
    "None",
    "Integer",
    "Steps of 12",
    "Overtones",
    "Minor scale",
    "Major scale",
    "Minor chord",
    "Major chord",
    "Minor series",
    "Major series",
    "Fifths & octaves",
];
pub const SH_SOURCES: [&str; 8] = [
    "None",
    "Mod Wheel",
    "Pitch Wheel",
    "Control A",
    "Control B",
    "LFO",
    "Gate",
    "Aftertouch",
];
pub const SLEWS: [&str; 4] = ["None", "Fast", "Smooth", "Slow"];
/// The slews' time constants, seconds.
const SLEW_TIMES: [f32; 4] = [0.0, 0.012, 0.06, 0.3];

/// What every voice shares this segment.
#[derive(Clone, Copy, Default)]
pub struct Globals {
    pub wheel: f32,
    /// −1..1.
    pub bend: f32,
    pub pressure: f32,
    pub cc_a: f32,
    pub cc_b: f32,
    pub lfo: f32,
    /// The wheel-mod signal (LFO/noise mix times the wheel's depth).
    pub wm: f32,
    pub wm_noise: f32,
}

/// A slot's settings.
#[derive(Clone, Copy, Default)]
pub struct Slot {
    pub source: usize,
    pub dest: usize,
    pub depth: f32,
    pub curve: usize,
    pub rectify: usize,
    pub quantize: usize,
    pub sh: usize,
    pub slew: usize,
}

impl Slot {
    fn live(&self) -> bool {
        self.source != src::NONE && self.dest != dest::NONE && self.depth != 0.0
    }
}

/// A slot's memory (slew, sample & hold), per voice and for the shared
/// destinations.
#[derive(Clone, Copy, Default)]
pub struct VoiceState {
    slew: [f32; SLOTS],
    held: [f32; SLOTS],
    trig: [f32; SLOTS],
    started: [bool; SLOTS],
}

impl VoiceState {
    pub fn reset(&mut self) {
        self.started = [false; SLOTS];
    }
}

/// The offsets a voice (or the shared destinations) gets, by destination.
pub type Mods = [f32; dest::COUNT];

#[derive(Default)]
pub struct Matrix {
    pub slots: [Slot; SLOTS],
    /// Any live slot for the voices; for the shared destinations.
    voices_live: bool,
    globals_live: bool,
    global_state: VoiceState,
    fx_state: VoiceState,
}

/// The values a voice's sources take now.
struct Ctx {
    trigger: f32,
    gate: f32,
    key: f32,
    key_bend: f32,
    velocity: f32,
    fenv: f32,
    aenv: f32,
    index: f32,
    detune: f32,
}

impl Ctx {
    fn of(v: &Voice, g: &Globals, bend_semis: f32) -> Self {
        Ctx {
            trigger: if v.since < 0.004 { 1.0 } else { 0.0 },
            gate: if v.gate { 1.0 } else { 0.0 },
            key: (v.pitch() - 60.0) / 60.0,
            key_bend: (v.pitch() + g.bend * bend_semis - 60.0) / 60.0,
            velocity: v.velocity,
            fenv: v.fenv.level,
            aenv: v.aenv.level,
            index: v.index as f32 / (MAX_VOICES - 1) as f32,
            detune: v.detune_source(),
        }
    }

    fn none() -> Self {
        Ctx {
            trigger: 0.0,
            gate: 0.0,
            key: 0.0,
            key_bend: 0.0,
            velocity: 0.0,
            fenv: 0.0,
            aenv: 0.0,
            index: 0.0,
            detune: 0.0,
        }
    }
}

fn source(id: usize, c: &Ctx, g: &Globals) -> f32 {
    match id {
        src::WHEEL => g.wheel,
        src::BEND => g.bend,
        src::CC_A => g.cc_a,
        src::CC_B => g.cc_b,
        src::LFO => g.lfo,
        src::TRIGGER => c.trigger,
        src::GATE => c.gate,
        src::KEY => c.key,
        src::KEY_BEND => c.key_bend,
        src::VELOCITY => c.velocity,
        src::PRESSURE => g.pressure,
        src::FENV => c.fenv,
        src::AENV => c.aenv,
        src::INDEX => c.index,
        src::DETUNE => c.detune,
        _ => 0.0,
    }
}

fn sh_source(id: usize, c: &Ctx, g: &Globals) -> f32 {
    match id {
        1 => g.wheel - 0.5,
        2 => g.bend,
        3 => g.cc_a - 0.5,
        4 => g.cc_b - 0.5,
        5 => g.lfo,
        6 => c.gate - 0.5,
        7 => g.pressure - 0.5,
        _ => 0.0,
    }
}

/// The s-curves (a bipolar ramp in, the same range out).
pub fn curve(kind: usize, x: f32) -> f32 {
    let a = x.abs().min(1.0);
    let y = match kind {
        1 => 1.0 - (1.0 - a) * (1.0 - a),
        2 => 1.0 - (1.0 - a) * (1.0 - a) * (1.0 - a),
        3 => a * a,
        4 => a * a * a,
        _ => return x,
    };
    y.copysign(x)
}

pub fn rectify(kind: usize, x: f32) -> f32 {
    match kind {
        1 => x.max(0.0),
        2 => x.min(0.0),
        3 => x.abs(),
        4 => -x.abs(),
        5 => x * 0.5 + 0.5,
        _ => x,
    }
}

const OVERTONES: [f32; 16] = [
    0.0, 12.0, 19.02, 24.0, 27.86, 31.02, 33.69, 36.0, 38.04, 39.86, 41.51, 43.02, 44.4, 45.69,
    46.88, 48.0,
];
const MINOR: [f32; 7] = [0.0, 2.0, 3.0, 5.0, 7.0, 8.0, 10.0];
const MAJOR: [f32; 7] = [0.0, 2.0, 4.0, 5.0, 7.0, 9.0, 11.0];
const MINOR_CHORD: [f32; 3] = [0.0, 3.0, 7.0];
const MAJOR_CHORD: [f32; 3] = [0.0, 4.0, 7.0];
/// Stacked thirds, repeating every two octaves.
const MINOR_SERIES: [f32; 7] = [0.0, 3.0, 7.0, 10.0, 14.0, 17.0, 20.0];
const MAJOR_SERIES: [f32; 7] = [0.0, 4.0, 7.0, 11.0, 14.0, 17.0, 21.0];
const FIFTHS: [f32; 2] = [0.0, 7.0];

/// The nearest of `set` (repeating every `period` semitones) to `s`.
fn nearest(s: f32, set: &[f32], period: f32) -> f32 {
    let base = (s / period).floor() * period;
    let r = s - base;
    let mut best = set[0];
    for &x in set.iter().chain(std::iter::once(&period)) {
        if (x - r).abs() < (best - r).abs() {
            best = x;
        }
    }
    base + best
}

/// Quantizes an offset; the steps are semitones of a two-octave span (full
/// depth = ±24), so a pitch destination lands on the scale.
pub fn quantize(kind: usize, x: f32) -> f32 {
    if kind == 0 {
        return x;
    }
    let s = x * 24.0;
    let q = match kind {
        1 => s.round(),
        2 => (s / 12.0).round() * 12.0,
        3 => {
            let a = s.abs();
            let best = OVERTONES
                .iter()
                .copied()
                .min_by(|p, q| (p - a).abs().total_cmp(&(q - a).abs()))
                .unwrap_or(0.0);
            best.copysign(s)
        }
        4 => nearest(s, &MINOR, 12.0),
        5 => nearest(s, &MAJOR, 12.0),
        6 => nearest(s, &MINOR_CHORD, 12.0),
        7 => nearest(s, &MAJOR_CHORD, 12.0),
        8 => nearest(s, &MINOR_SERIES, 24.0),
        9 => nearest(s, &MAJOR_SERIES, 24.0),
        _ => nearest(s, &FIFTHS, 12.0),
    };
    q / 24.0
}

impl Matrix {
    pub fn update(&mut self, s: &[f32; PARAM_COUNT]) {
        let idx = |k: usize| s[k].round().max(0.0) as usize;
        self.voices_live = false;
        self.globals_live = false;
        for (k, sl) in self.slots.iter_mut().enumerate() {
            *sl = Slot {
                source: idx(sp(k, slot::SOURCE)).min(SOURCES.len() - 1),
                dest: idx(sp(k, slot::DEST)).min(dest::COUNT - 1),
                depth: s[sp(k, slot::DEPTH)],
                curve: idx(sp(k, slot::CURVE)).min(CURVES.len() - 1),
                rectify: idx(sp(k, slot::RECTIFY)).min(RECTIFIERS.len() - 1),
                quantize: idx(sp(k, slot::QUANTIZE)).min(QUANTIZERS.len() - 1),
                sh: idx(sp(k, slot::SH)).min(SH_SOURCES.len() - 1),
                slew: idx(sp(k, slot::SLEW)).min(SLEWS.len() - 1),
            };
            if sl.live() {
                if is_global_dest(sl.dest) {
                    self.globals_live = true;
                } else {
                    self.voices_live = true;
                }
            }
        }
        let _ = SLOT_LEN;
    }

    /// One slot's value through its modifiers, with its memory.
    fn run(sl: &Slot, k: usize, c: &Ctx, g: &Globals, st: &mut VoiceState, dt: f32) -> f32 {
        let x = rectify(sl.rectify, curve(sl.curve, source(sl.source, c, g)));
        let mut y = quantize(sl.quantize, x * sl.depth);
        if sl.sh != 0 {
            let t = sh_source(sl.sh, c, g);
            if !st.started[k] || (st.trig[k] <= 0.0 && t > 0.0) {
                st.held[k] = y;
            }
            st.trig[k] = t;
            y = st.held[k];
        }
        if sl.slew != 0 && st.started[k] {
            let a = 1.0 - fastmath::exp2(-std::f32::consts::LOG2_E * dt / SLEW_TIMES[sl.slew]);
            st.slew[k] += (y - st.slew[k]) * a;
            y = st.slew[k];
        } else {
            st.slew[k] = y;
        }
        st.started[k] = true;
        y
    }

    /// Some slot reaches the voices.
    pub fn voices_live(&self) -> bool {
        self.voices_live
    }

    /// A voice's offsets now, added into `m` (zeros).
    pub fn voice(&self, v: &mut Voice, g: &Globals, dt: f32, m: &mut Mods) {
        let c = Ctx::of(v, g, v.bend_range());
        for (k, sl) in self.slots.iter().enumerate() {
            if sl.live() && !is_global_dest(sl.dest) {
                m[sl.dest] += Self::run(sl, k, &c, g, &mut v.mods, dt);
            }
        }
    }

    /// The offset for a shared destination, from the newest voice's sources.
    pub fn global(
        &mut self,
        d: usize,
        g: &Globals,
        voices: &[Voice; MAX_VOICES],
        _clock: u64,
    ) -> f32 {
        if !self.globals_live {
            return 0.0;
        }
        let newest = voices
            .iter()
            .filter(|v| v.active())
            .max_by_key(|v| v.started);
        let c = newest.map_or_else(Ctx::none, |v| Ctx::of(v, g, v.bend_range()));
        let mut sum = 0.0;
        for k in 0..SLOTS {
            let sl = self.slots[k];
            if sl.live() && sl.dest == d {
                sum += Self::run(&sl, k, &c, g, &mut self.global_state, 0.0016);
            }
        }
        sum
    }

    /// The effects' offsets, once a block.
    pub fn fx_mods(&mut self, g: &Globals, voices: &[Voice; MAX_VOICES], _clock: u64) -> Mods {
        let mut m = [0.0; dest::COUNT];
        if !self.globals_live {
            return m;
        }
        let newest = voices
            .iter()
            .filter(|v| v.active())
            .max_by_key(|v| v.started);
        let c = newest.map_or_else(Ctx::none, |v| Ctx::of(v, g, v.bend_range()));
        for k in 0..SLOTS {
            let sl = self.slots[k];
            if sl.live() && (dest::TAPE_GAIN..=dest::COND_WIDTH).contains(&sl.dest) {
                m[sl.dest] += Self::run(&sl, k, &c, g, &mut self.fx_state, 0.0013);
            }
        }
        m
    }
}

// ---------------------------------------------------------------------------
// LFO
// ---------------------------------------------------------------------------

/// The global LFO: saw, triangle and square, which add up when several are
/// on; free from 0.03 to 27.5 Hz or synced to the board. Also makes the
/// wheel's noise: pink-ish, or sampled & held at the LFO's rate.
pub struct Lfo {
    phase: f64,
    pub value: f32,
    pub noise: f32,
    sh: f32,
    pink: [f32; 3],
}

impl Lfo {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            value: 0.0,
            noise: 0.0,
            sh: 0.0,
            pink: [0.0; 3],
        }
    }

    /// Moves on by `dt` seconds. `beat` is the board's position, `beats_per_second`
    /// its tempo (synced, the LFO follows the beat while the transport plays and
    /// keeps the tempo while it is stopped).
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        patch: &Patch,
        dt: f32,
        beat: f64,
        beats_per_second: f64,
        playing: bool,
        rate_mod: f32,
        rng: &mut Rng,
    ) {
        let before = self.phase;
        if patch.lfo_sync {
            if playing {
                self.phase = (beat / patch.lfo_beats).rem_euclid(1.0);
            } else {
                self.phase =
                    (self.phase + dt as f64 * beats_per_second / patch.lfo_beats).rem_euclid(1.0);
            }
        } else {
            let rate = if rate_mod == 0.0 {
                patch.lfo_rate
            } else {
                patch.lfo_rate * fastmath::exp2(rate_mod * (27.5f32 / 0.03).log2())
            };
            self.phase = (self.phase + (rate.clamp(0.01, 60.0) * dt) as f64).rem_euclid(1.0);
        }
        if self.phase < before {
            self.sh = rng.bipolar();
        }
        let t = self.phase as f32;
        let (lo, hi) = if patch.lfo_dc {
            (0.0, 1.0)
        } else {
            (-1.0, 1.0)
        };
        let mut v = 0.0;
        if patch.lfo_saw {
            let ramp = if patch.lfo_inv { 1.0 - t } else { t };
            v += lo + (hi - lo) * ramp;
        }
        if patch.lfo_tri {
            // Always bipolar, whatever the DC jumper says.
            v += 1.0 - 4.0 * (t - 0.5).abs();
        }
        if patch.lfo_sqr {
            v += if t < 0.5 { hi } else { lo };
        }
        self.value = v;
        // Pink-ish noise: three leaky integrators of white (a modulation
        // signal, so per segment is plenty).
        let w = rng.bipolar();
        self.pink[0] = 0.997 * self.pink[0] + 0.029 * w;
        self.pink[1] = 0.985 * self.pink[1] + 0.032 * w;
        self.pink[2] = 0.950 * self.pink[2] + 0.048 * w;
        let pink = (self.pink[0] + self.pink[1] + self.pink[2] + 0.05 * w) * 2.5;
        self.noise = if patch.wm_sh {
            self.sh
        } else {
            pink.clamp(-1.0, 1.0)
        };
    }
}
