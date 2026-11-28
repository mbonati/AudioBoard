//! Stormo: a motion-based swarm synthesizer. Every note is a swarm of
//! nodes, up to forty-eight signal paths, each one a four-operator FM voice
//! of Quartz's engine ([`super::fm`]), and each driven by where it is and
//! how it moves in its environment ([`swarm`]).
//!
//! A note lays its nodes out in a SHAPE round the attractor (the ✕) and
//! lets them go: the FIELD holds them (an orbit, a flock, a drifting
//! current or a storm), SPIN turns them, TILT leans the field, CHAOS kicks
//! them, the BOUNDARY keeps them in. Where a node is, is what it sounds
//! like: X and Y (bent by WARP) move its pitch (quantized to a SCALE on
//! the ROOT, or free), its pan, its brightness (the FM modulators' level),
//! its level or its detune; the distance from the attractor dims and
//! darkens it (DEPTH, FOCUS), its speed bends it (DOPPLER). Nodes come in
//! one by one (SCATTER) and play the FM voice the TIMBRE picks; TRIGGER
//! re-strikes them when they hit a wall or cross the orbit (HIT), or as a
//! sweep of the board's clock passes them (1/4 … 1/32). Each voice ends in
//! a resonant filter.
//!
//! Cost: nodes are FM voices, so the swarm is kept to [`NODE_BUDGET`]
//! nodes in all: with many voices, each gets fewer nodes.

pub mod swarm;

use std::f32::consts::TAU;
use std::sync::Arc;

use self::swarm::{Field, MAX_NODES, Swarm};
use super::fm::{self, FmPatch, FmVoice, Mods, SEG};
use super::{ModuleDef, ModuleParams, Rng, pan_gains, soft_limit};
use crate::audio::engine::{BLOCK, Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit};

pub const MAX_VOICES: usize = 8;
/// Nodes sounding at once, over every voice.
pub const NODE_BUDGET: usize = 64;

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

pub const NODES: usize = 0;
pub const VOICES: usize = 1;
pub const ROOT: usize = 2;
pub const SCALE: usize = 3;
pub const TRIGGER: usize = 4;
// TIMBRE
pub const TIMBRE: usize = 5;
pub const BRIGHT: usize = 6;
pub const STACK: usize = 7;
pub const DETUNE: usize = 8;
// FILTER
pub const F_MODE: usize = 9;
pub const CUTOFF: usize = 10;
pub const RESO: usize = 11;
pub const F_ENV: usize = 12;
pub const FOCUS: usize = 13;
// ENVELOPE
pub const ATTACK: usize = 14;
pub const DECAY: usize = 15;
pub const SUSTAIN: usize = 16;
pub const RELEASE: usize = 17;
pub const SCATTER: usize = 18;
// FIELD
pub const FIELD: usize = 19;
pub const GRAVITY: usize = 20;
pub const RADIUS: usize = 21;
pub const COHESION: usize = 22;
// MOTION
pub const SPEED: usize = 23;
pub const CHAOS: usize = 24;
pub const DAMPING: usize = 25;
// SPACE
pub const WIDTH: usize = 26;
pub const DEPTH: usize = 27;
pub const DOPPLER: usize = 28;
// X / Y
pub const X_DEST: usize = 29;
pub const X_AMT: usize = 30;
pub const Y_DEST: usize = 31;
pub const Y_AMT: usize = 32;
pub const RANGE: usize = 33;
pub const GLIDE: usize = 34;
// BOUNDARY
pub const BOUND_ON: usize = 35;
pub const BOUND: usize = 36;
pub const BOUNCE: usize = 37;
// SHAPE, SIZE, SPIN, TILT, WARP
pub const SHAPE: usize = 38;
pub const SIZE: usize = 39;
pub const SPIN: usize = 40;
pub const TILT_ANGLE: usize = 41;
pub const TILT: usize = 42;
pub const WARP: usize = 43;
pub const WARP_TYPE: usize = 44;
pub const VOLUME: usize = 45;
pub const PARAM_COUNT: usize = 46;

pub const SCALES: [(&str, &str, u16); 11] = [
    ("Off", "Free", 0xFFF),
    ("Maj", "Major", 0b1010_1011_0101),
    ("Min", "Minor", 0b0101_1010_1101),
    ("Dor", "Dorian", 0b0110_1010_1101),
    ("Phr", "Phrygian", 0b0101_1010_1011),
    ("Lyd", "Lydian", 0b1010_1101_0101),
    ("Mixo", "Mixolydian", 0b0110_1011_0101),
    ("Pent", "Pentatonic", 0b0010_1001_0101),
    ("mPen", "Minor pent.", 0b0100_1010_1001),
    ("Whl", "Whole tone", 0b0101_0101_0101),
    ("Chr", "Chromatic", 0xFFF),
];
pub const TRIGGERS: [&str; 6] = ["Off", "Hit", "/4", "/8", "/16", "/32"];
pub const STACKS: [&str; 5] = ["UNISON", "OCTAVES", "FIFTHS", "TRIAD", "HARMONIC"];
pub const F_MODES: [&str; 3] = ["LP", "BP", "HP"];
pub const DESTS: [&str; 5] = ["PAN", "PITCH", "BRIGHT", "LEVEL", "DETUNE"];
pub const WARPS: [&str; 3] = ["BEND", "FOLD", "STEP"];

const fn pct(name: &'static str, default: f32) -> ParamSpec {
    ParamSpec::lin(name, 0.0, 1.0, default, Unit::Percent)
}
const fn sel(name: &'static str, count: usize, default: usize) -> ParamSpec {
    ParamSpec::lin(name, 0.0, (count - 1) as f32, default as f32, Unit::Integer)
}

const PARAMS: [ParamSpec; PARAM_COUNT] = {
    let mut p = [pct("", 0.0); PARAM_COUNT];
    p[NODES] = ParamSpec::lin("NODES", 1.0, MAX_NODES as f32, 14.0, Unit::Integer);
    p[VOICES] = ParamSpec::lin("VOICES", 1.0, MAX_VOICES as f32, 4.0, Unit::Integer);
    p[ROOT] = ParamSpec::lin("ROOT", 0.0, 11.0, 0.0, Unit::NoteName);
    p[SCALE] = sel("SCALE", SCALES.len(), 3);
    p[TRIGGER] = sel("TRIGGER", TRIGGERS.len(), 0);
    p[TIMBRE] = sel("TIMBRE", 16, 12);
    p[BRIGHT] = ParamSpec::lin("BRIGHT", 0.0, 1.5, 0.8, Unit::Percent);
    p[STACK] = sel("STACK", STACKS.len(), 1);
    p[DETUNE] = ParamSpec::lin("DETUNE", 0.0, 50.0, 8.0, Unit::Cents);
    p[F_MODE] = sel("FILTER MODE", F_MODES.len(), 0);
    p[CUTOFF] = ParamSpec::log("CUTOFF", 60.0, 18_000.0, 6000.0, Unit::Hz);
    p[RESO] = pct("RESONANCE", 0.15);
    p[F_ENV] = ParamSpec::lin("FILTER ENV", -1.0, 1.0, 0.2, Unit::Percent).bipolar();
    p[FOCUS] = ParamSpec::lin("FOCUS", -1.0, 1.0, 0.4, Unit::Percent).bipolar();
    p[ATTACK] = ParamSpec::log("ATTACK", 0.002, 10.0, 0.05, Unit::Seconds);
    p[DECAY] = ParamSpec::log("DECAY", 0.005, 10.0, 0.8, Unit::Seconds);
    p[SUSTAIN] = pct("SUSTAIN", 0.75);
    p[RELEASE] = ParamSpec::log("RELEASE", 0.005, 20.0, 1.6, Unit::Seconds);
    p[SCATTER] = pct("SCATTER", 0.3);
    p[FIELD] = sel("FIELD", swarm::FIELDS.len(), 0);
    p[GRAVITY] = pct("GRAVITY", 0.5);
    p[RADIUS] = pct("RADIUS", 0.55);
    p[COHESION] = pct("COHESION", 0.4);
    p[SPEED] = ParamSpec::log("SPEED", 0.02, 4.0, 0.5, Unit::Plain);
    p[CHAOS] = pct("CHAOS", 0.25);
    p[DAMPING] = pct("DAMPING", 0.3);
    p[WIDTH] = pct("WIDTH", 0.8);
    p[DEPTH] = pct("DEPTH", 0.4);
    p[DOPPLER] = pct("DOPPLER", 0.1);
    p[X_DEST] = sel("X DEST", DESTS.len(), 0);
    p[X_AMT] = ParamSpec::lin("X AMOUNT", -1.0, 1.0, 1.0, Unit::Percent).bipolar();
    p[Y_DEST] = sel("Y DEST", DESTS.len(), 1);
    p[Y_AMT] = ParamSpec::lin("Y AMOUNT", -1.0, 1.0, 0.6, Unit::Percent).bipolar();
    p[RANGE] = ParamSpec::lin("PITCH RANGE", 0.0, 24.0, 12.0, Unit::Semitones);
    p[GLIDE] = pct("GLIDE", 0.15);
    p[BOUND_ON] = sel("BOUNDARY", 2, 1);
    p[BOUND] = sel("BOUNDARY MODE", swarm::BOUNDS.len(), 0);
    p[BOUNCE] = pct("BOUNCE", 0.8);
    p[SHAPE] = sel("SHAPE", swarm::SHAPES.len(), 1);
    p[SIZE] = pct("SIZE", 0.5);
    p[SPIN] = ParamSpec::lin("SPIN", -1.0, 1.0, 0.3, Unit::Percent).bipolar();
    p[TILT_ANGLE] = ParamSpec::lin("TILT ANGLE", -180.0, 180.0, 90.0, Unit::Plain).bipolar();
    p[TILT] = pct("TILT", 0.0);
    p[WARP] = ParamSpec::lin("WARP", -1.0, 1.0, 0.0, Unit::Percent).bipolar();
    p[WARP_TYPE] = sel("WARP TYPE", WARPS.len(), 0);
    p[VOLUME] = pct("VOLUME", 0.7);
    p
};

pub mod data {
    /// The page the panel edits (a tile, 0–13).
    pub const PAGE: usize = 0;
    /// The preset last loaded.
    pub const PRESET: usize = 1;
    /// The attractor (the ✕), −1..1.
    pub const ATTR_X: usize = 2;
    pub const ATTR_Y: usize = 3;
    pub const LEN: usize = 4;
}

pub mod viz {
    use super::{MAX_NODES, MAX_VOICES};
    /// Per voice: active, note, level, then x, y, level for each node.
    pub const VOICE: usize = 0;
    pub const VOICE_LEN: usize = 3 + MAX_NODES * 3;
    /// The idle swarm (shown while nothing plays): x, y per node.
    pub const GHOST: usize = VOICE + MAX_VOICES * VOICE_LEN;
    /// The newest voice's nodes: pitch (semitones from the key), brightness, pan.
    pub const LANES: usize = GHOST + MAX_NODES * 2;
    pub const LANE_LEN: usize = 3;
    /// Nodes each voice holds now.
    pub const COUNT: usize = LANES + MAX_NODES * LANE_LEN;
    /// The clock trigger's sweep, 0..1 of a turn (−1: off).
    pub const SWEEP: usize = COUNT + 1;
    /// The attractor as it is now (it wanders in a storm).
    pub const AX: usize = SWEEP + 1;
    pub const AY: usize = AX + 1;
    /// Written by the panel: the key it holds + 1 (0: none).
    pub const PANEL_KEY: usize = AY + 1;
    pub const PANIC: usize = PANEL_KEY + 1;
    pub const LEN: usize = PANIC + 1;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Stormo",
    prefix: "STORMO",
    hint: "swarm synth · every note a moving network of FM nodes",
    outputs: &["OUT"],
    params: &PARAMS,
    midi_in: true,
    data: &[0.0; data::LEN],
    viz: viz::LEN,
    ..ModuleDef::EMPTY
};

pub fn spec(i: usize) -> &'static ParamSpec {
    &DEF.params[i]
}

pub fn int_of(p: &ModuleParams, i: usize) -> i32 {
    let s = spec(i);
    p.get(i).round().clamp(s.min, s.max) as i32
}

/// Nodes each voice gets, within the budget.
pub fn nodes_per_voice(p: &ModuleParams) -> usize {
    let n = int_of(p, NODES).max(1) as usize;
    let v = int_of(p, VOICES).max(1) as usize;
    n.min(NODE_BUDGET / v).max(1)
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

pub struct Preset {
    pub name: &'static str,
    pub values: &'static [(usize, f32)],
}

pub const PRESETS: [Preset; 8] = [
    Preset {
        name: "Murmur",
        values: &[],
    },
    Preset {
        name: "Karst",
        values: &[
            (TIMBRE, 6.0),
            (FIELD, 1.0),
            (SCALE, 3.0),
            (TRIGGER, 4.0),
            (NODES, 14.0),
            (SPIN, 0.2),
            (CHAOS, 0.35),
            (RELEASE, 2.5),
            (SHAPE, 0.0),
        ],
    },
    Preset {
        name: "Lantern",
        values: &[
            (TIMBRE, 13.0),
            (FIELD, 0.0),
            (SCALE, 7.0),
            (TRIGGER, 1.0),
            (BOUND, 1.0),
            (RADIUS, 0.8),
            (SPIN, 0.6),
            (CHAOS, 0.4),
            (BRIGHT, 1.0),
        ],
    },
    Preset {
        name: "Tidepool",
        values: &[
            (TIMBRE, 12.0),
            (FIELD, 2.0),
            (SCALE, 2.0),
            (SPEED, 0.2),
            (ATTACK, 1.2),
            (RELEASE, 5.0),
            (SCATTER, 0.8),
            (NODES, 24.0),
            (VOICES, 3.0),
            (Y_AMT, 0.4),
            (CUTOFF, 3200.0),
        ],
    },
    Preset {
        name: "Hive",
        values: &[
            (TIMBRE, 7.0),
            (FIELD, 1.0),
            (SCALE, 0.0),
            (NODES, 32.0),
            (VOICES, 2.0),
            (DETUNE, 30.0),
            (Y_DEST, 2.0),
            (Y_AMT, 0.8),
            (COHESION, 0.8),
            (CHAOS, 0.5),
            (SPEED, 1.4),
        ],
    },
    Preset {
        name: "Comet",
        values: &[
            (TIMBRE, 9.0),
            (FIELD, 3.0),
            (SCALE, 6.0),
            (TRIGGER, 3.0),
            (DOPPLER, 0.6),
            (SHAPE, 4.0),
            (WARP, 0.5),
        ],
    },
    Preset {
        name: "Glass Rain",
        values: &[
            (TIMBRE, 2.0),
            (FIELD, 2.0),
            (SCALE, 1.0),
            (TRIGGER, 5.0),
            (TILT, 0.6),
            (TILT_ANGLE, 90.0),
            (BOUND, 0.0),
            (BOUNCE, 0.95),
            (NODES, 20.0),
            (WARP_TYPE, 2.0),
            (WARP, 0.4),
        ],
    },
    Preset {
        name: "Ember",
        values: &[
            (TIMBRE, 5.0),
            (FIELD, 0.0),
            (SCALE, 8.0),
            (STACK, 3.0),
            (ATTACK, 2.0),
            (RELEASE, 6.0),
            (SCATTER, 0.9),
            (F_MODE, 0.0),
            (CUTOFF, 1800.0),
            (RESO, 0.45),
            (F_ENV, 0.5),
            (SPEED, 0.15),
        ],
    },
];

/// Loads preset `k`: every setting back to its default, then the preset's.
pub fn load_preset(p: &ModuleParams, k: usize) {
    let k = k % PRESETS.len();
    for i in 0..PARAM_COUNT {
        if i == VOLUME {
            continue;
        }
        p.set(i, spec(i).default);
    }
    for &(i, v) in PRESETS[k].values {
        p.set(i, v);
    }
    p.data[data::PRESET].set(k as f32);
}

// ---------------------------------------------------------------------------
// The processor
// ---------------------------------------------------------------------------

/// The nearest note in `mask` (bit k: pitch class k above `root`) to `n`.
pub fn quantize(n: f32, root: i32, mask: u16) -> f32 {
    if mask == 0xFFF {
        return n.round();
    }
    let base = n.round() as i32;
    for d in 0..12 {
        for c in [base + d, base - d] {
            if mask & (1 << (c - root).rem_euclid(12)) != 0 {
                return c as f32;
            }
        }
    }
    n.round()
}

/// WARP: bends a position (−1..1) before it maps to anything.
pub fn warp(x: f32, amount: f32, kind: usize) -> f32 {
    if amount == 0.0 {
        return x;
    }
    match kind {
        0 => x.signum() * x.abs().powf(fastmath::exp2(amount * 1.6)),
        1 => {
            let k = 1.0 + 3.0 * amount.abs();
            let f = fastmath::sin_turns(x * k * 0.25);
            x + (f - x) * amount.abs()
        }
        _ => {
            let steps = 2.0 + (1.0 - amount.abs()) * 14.0;
            (x * steps).round() / steps
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Env {
    #[default]
    Idle,
    Wait,
    Attack,
    Decay,
    Release,
}

#[derive(Clone, Copy, Default)]
struct NodeVoice {
    fm: FmVoice,
    env: Env,
    amp: f32,
    /// Frames before it comes in (SCATTER).
    wait: u32,
    pitch: f32,
    fresh: bool,
    /// Gains now (ramped from them to the next block's).
    gl: f32,
    gr: f32,
    bright: f32,
    rel: f32,
    pan: f32,
}

struct Voice {
    swarm: Swarm,
    nodes: [NodeVoice; MAX_NODES],
    note: u8,
    vel: f32,
    gate: bool,
    active: bool,
    started: u64,
    sustained: bool,
    /// The voice's filter (left, right).
    svf: [(f32, f32); 2],
    level: f32,
}

impl Voice {
    fn new(rng: &mut Rng) -> Self {
        Self {
            swarm: Swarm::new(rng),
            nodes: [NodeVoice::default(); MAX_NODES],
            note: 60,
            vel: 0.0,
            gate: false,
            active: false,
            started: 0,
            sustained: false,
            svf: [(0.0, 0.0); 2],
            level: 0.0,
        }
    }
}

pub struct Stormo {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    sr: f32,
    voices: Vec<Voice>,
    ghost: Swarm,
    ghost_shape: (usize, usize),
    rng: Rng,
    clock: u64,
    pedal: bool,
    bend: f32,
    patch: FmPatch,
    timbre: usize,
    step: i64,
    free_beat: f64,
    events: Vec<(usize, [u8; 3])>,
    panel_key: u8,
    panic_seen: f32,
    buf: Box<[f32; BLOCK]>,
    dc: [f32; 2],
    volume: f32,
}

impl Stormo {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        let mut rng = Rng::new(0x5701_2A0E);
        let voices = (0..MAX_VOICES).map(|_| Voice::new(&mut rng)).collect();
        let ghost = Swarm::new(&mut rng);
        let timbre = int_of(&params, TIMBRE) as usize;
        Self {
            midi_cursor: midi.stream_head(),
            midi,
            sr: sample_rate,
            voices,
            ghost,
            ghost_shape: (usize::MAX, 0),
            rng,
            clock: 0,
            pedal: false,
            bend: 0.0,
            patch: super::quartz::voice_patch(timbre),
            timbre,
            step: -1,
            free_beat: 0.0,
            events: Vec::with_capacity(256),
            panel_key: 0,
            panic_seen: params.viz[viz::PANIC].get(),
            buf: Box::new([0.0; BLOCK]),
            dc: [0.0; 2],
            volume: 0.0,
            params,
        }
    }

    fn field(&self) -> Field {
        let p = &self.params;
        Field {
            mode: int_of(p, FIELD) as usize,
            gravity: p.get(GRAVITY),
            radius: p.get(RADIUS),
            cohesion: p.get(COHESION),
            speed: p.get(SPEED),
            chaos: p.get(CHAOS),
            damping: p.get(DAMPING),
            spin: p.get(SPIN),
            tilt_angle: p.get(TILT_ANGLE).to_radians() - std::f32::consts::FRAC_PI_2,
            tilt: p.get(TILT),
            bound_on: int_of(p, BOUND_ON) > 0,
            bound: int_of(p, BOUND) as usize,
            bounce: p.get(BOUNCE),
            size: p.get(SIZE),
            ax: p.data[data::ATTR_X].get().clamp(-1.0, 1.0),
            ay: p.data[data::ATTR_Y].get().clamp(-1.0, 1.0),
        }
    }

    fn all_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.active = false;
            v.gate = false;
            for n in v.nodes.iter_mut() {
                n.env = Env::Idle;
                n.amp = 0.0;
                n.fm.kill();
            }
        }
    }

    fn note_on(&mut self, note: u8, vel: f32) {
        self.clock += 1;
        let p = self.params.clone();
        let count = int_of(&p, VOICES).clamp(1, MAX_VOICES as i32) as usize;
        let nodes = nodes_per_voice(&p);
        let field = self.field();
        let pick = (0..count)
            .find(|&i| self.voices[i].active && self.voices[i].note == note)
            .or_else(|| (0..count).find(|&i| !self.voices[i].active))
            .or_else(|| {
                (0..count)
                    .filter(|&i| !self.voices[i].gate)
                    .min_by(|&a, &b| self.voices[a].level.total_cmp(&self.voices[b].level))
            })
            .unwrap_or_else(|| (0..count).min_by_key(|&i| self.voices[i].started).unwrap_or(0));
        let scatter = p.get(SCATTER);
        let sr = self.sr;
        let shape = int_of(&p, SHAPE) as usize;
        let rng = &mut self.rng;
        let v = &mut self.voices[pick];
        if !v.active {
            v.swarm.spawn(shape, nodes, &field, rng);
            v.svf = [(0.0, 0.0); 2];
        }
        v.note = note;
        v.vel = vel;
        v.gate = true;
        v.active = true;
        v.sustained = false;
        v.started = self.clock;
        for (i, n) in v.nodes.iter_mut().enumerate().take(nodes) {
            let u = v.swarm.nodes[i].u;
            let wait = (scatter * scatter * u * 2.0 * sr) as u32;
            if n.env == Env::Idle {
                n.fresh = true;
                n.amp = 0.0;
            }
            if wait > 0 {
                n.env = Env::Wait;
                n.wait = wait;
            } else {
                n.env = Env::Attack;
                n.fm.key_on(note as f32, vel, true);
            }
        }
    }

    fn release(v: &mut Voice) {
        v.gate = false;
        for n in v.nodes.iter_mut() {
            if n.env != Env::Idle {
                n.env = Env::Release;
                n.fm.key_off();
            }
        }
    }

    fn note_off(&mut self, note: u8) {
        let pedal = self.pedal;
        for v in self.voices.iter_mut() {
            if v.active && v.gate && v.note == note {
                if pedal {
                    v.sustained = true;
                } else {
                    Self::release(v);
                }
            }
        }
    }

    fn handle(&mut self, data: [u8; 3]) {
        let [status, d1, d2] = data;
        match status & 0xF0 {
            0x90 if d2 > 0 => self.note_on(d1 & 0x7F, d2 as f32 / 127.0),
            0x80 | 0x90 => self.note_off(d1 & 0x7F),
            0xB0 => match d1 {
                64 => {
                    self.pedal = d2 >= 64;
                    if !self.pedal {
                        for v in self.voices.iter_mut().filter(|v| v.sustained) {
                            v.sustained = false;
                            Self::release(v);
                        }
                    }
                }
                120 => self.all_off(),
                123 => {
                    for v in self.voices.iter_mut() {
                        Self::release(v);
                    }
                }
                _ => {}
            },
            0xE0 => {
                let v = ((d2 as i32) << 7 | d1 as i32) - 8192;
                self.bend = (v as f32 / 8192.0).clamp(-1.0, 1.0) * 2.0;
            }
            0xC0 => load_preset(&self.params, d1 as usize),
            _ => {}
        }
    }
}

const STACK_SEMIS: [[f32; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [0.0, 12.0, -12.0],
    [0.0, 7.0, 12.0],
    [0.0, 4.0, 7.0],
];

impl Processor for Stormo {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let panic = p.viz[viz::PANIC].get();
        if panic != self.panic_seen {
            self.panic_seen = panic;
            self.all_off();
        }
        let timbre = int_of(&p, TIMBRE) as usize;
        if timbre != self.timbre {
            self.timbre = timbre;
            self.patch = super::quartz::voice_patch(timbre);
        }

        // Events, applied at the start of the block (the swarm moves at
        // the block's pace).
        self.events.clear();
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                self.events.push((e.frame as usize, e.data));
            }
            self.midi_cursor = self.midi.stream_head();
        } else {
            let ev = &mut self.events;
            self.midi.read_stream(&mut self.midi_cursor, |m| ev.push((0, m)));
        }
        let panel_key = p.viz[viz::PANEL_KEY].get().round().clamp(0.0, 128.0) as u8;
        if panel_key != self.panel_key {
            if self.panel_key > 0 {
                self.events.push((0, [0x80, self.panel_key - 1, 0]));
            }
            if panel_key > 0 {
                self.events.push((0, [0x90, panel_key - 1, 100]));
            }
            self.panel_key = panel_key;
        }
        self.events.sort_by_key(|e| e.0);
        let events = std::mem::take(&mut self.events);
        for &(_, d) in &events {
            self.handle(d);
        }
        self.events = events;

        let sr = self.sr;
        let dt = n as f32 / sr;
        let field = self.field();
        let count = nodes_per_voice(&p);

        // The clock trigger's sweep: one sector a step, sixteen a turn.
        let trigger = int_of(&p, TRIGGER) as usize;
        let beat0 = if ctx.playing { ctx.beat } else { self.free_beat };
        self.free_beat = beat0 + ctx.beats_per_sample * n as f64;
        let mut sweep = None;
        if trigger >= 2 {
            let div = [1.0, 0.5, 0.25, 0.125][trigger - 2];
            let step = (beat0 / div).floor() as i64;
            let sector = step.rem_euclid(16) as f32;
            p.viz[viz::SWEEP].set(sector / 16.0);
            if step != self.step {
                self.step = step;
                sweep = Some(sector / 16.0);
            }
        } else {
            p.viz[viz::SWEEP].set(-1.0);
        }
        let sector_half = (0.5 / 16.0f32).max(0.5 / count as f32);

        // Settings for this block.
        let root = int_of(&p, ROOT);
        let mask = SCALES[int_of(&p, SCALE) as usize].2;
        let quant = int_of(&p, SCALE) > 0;
        let stack = int_of(&p, STACK) as usize;
        let detune = p.get(DETUNE);
        let bright0 = p.get(BRIGHT);
        let focus = p.get(FOCUS);
        let (att, dec, sus, rel) = (p.get(ATTACK), p.get(DECAY), p.get(SUSTAIN), p.get(RELEASE));
        let (width, depth, doppler) = (p.get(WIDTH), p.get(DEPTH), p.get(DOPPLER));
        let dests = [
            (int_of(&p, X_DEST) as usize, p.get(X_AMT)),
            (int_of(&p, Y_DEST) as usize, p.get(Y_AMT)),
        ];
        let range = p.get(RANGE);
        let glide = p.get(GLIDE);
        let glide_k = 1.0 - (-dt / (0.003 + glide * glide * 0.8)).exp();
        let (wamt, wkind) = (p.get(WARP), int_of(&p, WARP_TYPE) as usize);
        let pans = dests.iter().any(|d| d.0 == 0);
        let (f_mode, cutoff, reso, f_env) = (
            int_of(&p, F_MODE) as usize,
            p.get(CUTOFF),
            p.get(RESO),
            p.get(F_ENV),
        );
        let norm = 0.55 / (count as f32).sqrt();
        let vol_t = p.get(VOLUME);
        let att_step = dt / att.max(0.001);
        let dec_k = (-dt * 3.0 / dec.max(0.001)).exp();
        let rel_k = (-dt * 3.0 / rel.max(0.001)).exp();

        let patch = self.patch;
        let mods = Mods {
            pitch: self.bend,
            ..Mods::default()
        };
        let mut mix = [[0.0f32; BLOCK]; 2];
        let mut newest: Option<(u64, usize)> = None;
        let budget = NODE_BUDGET;
        let mut spent = 0usize;

        for (vi, v) in self.voices.iter_mut().enumerate() {
            if !v.active {
                continue;
            }
            if newest.is_none_or(|(s, _)| v.started > s) {
                newest = Some((v.started, vi));
            }
            v.swarm.step(count, &field, dt, &mut self.rng);
            let (ax, ay) = v.swarm.attractor(&field);
            let mut vl = [0.0f32; BLOCK];
            let mut vr = [0.0f32; BLOCK];
            let mut env_sum = 0.0;
            let mut any = false;
            let vel_gain = 0.35 + 0.65 * v.vel;
            for i in 0..count {
                let node = v.swarm.nodes[i];
                let hit = v.swarm.hits[i];
                let nv = &mut v.nodes[i];
                // The node's own envelope (and SCATTER's wait).
                match nv.env {
                    Env::Idle => continue,
                    Env::Wait => {
                        if nv.wait as usize <= n {
                            nv.wait = 0;
                            nv.env = Env::Attack;
                            nv.fm.key_on(v.note as f32, v.vel, true);
                        } else {
                            nv.wait -= n as u32;
                        }
                    }
                    Env::Attack => {
                        nv.amp += att_step;
                        if nv.amp >= 1.0 {
                            nv.amp = 1.0;
                            nv.env = Env::Decay;
                        }
                    }
                    Env::Decay => nv.amp = sus + (nv.amp - sus) * dec_k,
                    Env::Release => {
                        nv.amp *= rel_k;
                        if nv.amp < 1e-4 {
                            nv.amp = 0.0;
                            nv.env = Env::Idle;
                            nv.fm.kill();
                            continue;
                        }
                    }
                }
                any = true;
                env_sum += nv.amp;
                if spent >= budget {
                    continue;
                }
                spent += 1;

                // TRIGGER: re-strike on a hit, or as the sweep passes.
                if v.gate && nv.env != Env::Wait {
                    let strike = match trigger {
                        1 => hit,
                        2.. => sweep.is_some_and(|s| {
                            let a = (node.y - ay).atan2(node.x - ax) / TAU + 0.25;
                            let a = a.rem_euclid(1.0);
                            let d = (a - s - sector_half).rem_euclid(1.0);
                            d.min(1.0 - d) <= sector_half
                        }),
                        _ => false,
                    };
                    if strike {
                        nv.fm.key_on(v.note as f32, v.vel, false);
                    }
                }

                // Where it is, what it sounds like.
                let xw = warp(node.x.clamp(-1.0, 1.0), wamt, wkind);
                let yw = warp((-node.y).clamp(-1.0, 1.0), wamt, wkind);
                let (dx, dy) = (node.x - ax, node.y - ay);
                let d = (dx * dx + dy * dy).sqrt();
                let (mut pan, mut pitch, mut bright, mut level, mut det) =
                    (0.0, 0.0, 0.0, 0.0, 0.0);
                for (axis, &(dest, amt)) in [xw, yw].iter().zip(&dests) {
                    let val = axis * amt;
                    match dest {
                        0 => pan += val,
                        1 => pitch += val * range,
                        2 => bright += val,
                        3 => level += val * 0.7,
                        _ => det += val * 50.0,
                    }
                }
                if !pans {
                    pan = xw;
                }
                let group = i % 3;
                let stack_semis = if stack == 4 {
                    12.0 * fastmath::log2((i % 8 + 1) as f32)
                } else {
                    STACK_SEMIS[stack][group]
                };
                let mut target = v.note as f32 + stack_semis + pitch;
                if quant {
                    target = quantize(target, root, mask);
                }
                if nv.fresh {
                    nv.pitch = target;
                    nv.fresh = false;
                } else {
                    nv.pitch += (target - nv.pitch) * glide_k;
                }
                let vr_ = if d > 1e-4 { (dx * node.vx + dy * node.vy) / d } else { 0.0 };
                let cents = node.w * detune + det;
                let sounding = nv.pitch + cents / 100.0 - doppler * vr_ * 1.5;
                let b = (bright0 * (1.0 + focus * (0.5 - d) * 1.6 + bright)).clamp(0.0, 2.0);
                let lvl = (nv.amp * (1.0 - depth * d.min(1.0) * 0.85) * (1.0 + level))
                    .max(0.0)
                    * vel_gain
                    * norm;
                let (gl, gr) = pan_gains((pan * width).clamp(-1.0, 1.0));
                let (gl1, gr1) = (gl * lvl, gr * lvl);
                nv.bright = b;
                nv.rel = nv.pitch - v.note as f32;
                nv.pan = pan * width;

                // Render the node, ramping its gains over the block.
                let buf = &mut self.buf[..n];
                buf.fill(0.0);
                let mut f = 0;
                while f < n {
                    let m = (n - f).min(SEG);
                    nv.fm.render(&patch, &mods, sounding, b, sr, &mut buf[f..f + m]);
                    f += m;
                }
                let inv = 1.0 / n.max(1) as f32;
                let (dl, dr) = ((gl1 - nv.gl) * inv, (gr1 - nv.gr) * inv);
                let (mut a, mut c) = (nv.gl, nv.gr);
                for k in 0..n {
                    a += dl;
                    c += dr;
                    vl[k] += buf[k] * a;
                    vr[k] += buf[k] * c;
                }
                nv.gl = gl1;
                nv.gr = gr1;
            }
            if !any {
                v.active = false;
                v.level = 0.0;
                continue;
            }
            // The voice's filter.
            let env = env_sum / count as f32;
            v.level = env;
            let fc = (cutoff
                * fastmath::exp2(f_env * env * 5.0 + (v.note as f32 - 60.0) / 24.0))
                .clamp(30.0, sr * 0.45);
            let g = fastmath::tan_turns(fc / sr * 0.5);
            let kq = 2.0 - 1.96 * reso;
            let a1 = 1.0 / (1.0 + g * (g + kq));
            let a2 = g * a1;
            let a3 = g * a2;
            for (ch, buf) in [&mut vl, &mut vr].into_iter().enumerate() {
                let (mut ic1, mut ic2) = v.svf[ch];
                for (k, x) in buf.iter_mut().enumerate().take(n) {
                    let v3 = *x - ic2;
                    let v1 = a1 * ic1 + a2 * v3;
                    let v2 = ic2 + a2 * ic1 + a3 * v3;
                    ic1 = 2.0 * v1 - ic1;
                    ic2 = 2.0 * v2 - ic2;
                    let y = match f_mode {
                        0 => v2,
                        1 => v1 * kq.max(0.5),
                        _ => *x - kq * v1 - v2,
                    };
                    mix[ch][k] += y;
                }
                v.svf[ch] = (ic1, ic2);
            }
        }

        // The idle swarm, so the field moves with nothing playing.
        let shape = (int_of(&p, SHAPE) as usize, count);
        if shape != self.ghost_shape {
            self.ghost_shape = shape;
            self.ghost.spawn(shape.0, count, &field, &mut self.rng);
        }
        self.ghost.step(count, &field, dt, &mut self.rng);

        let dc_k = 1.0 - (-TAU * 10.0 / sr).exp();
        let mut peak = 0.0f32;
        for k in 0..n {
            self.volume += (vol_t - self.volume) * ctx.smooth;
            let g = self.volume * self.volume * 1.4;
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                let x = mix[ch][k] * g;
                self.dc[ch] += (x - self.dc[ch]) * dc_k;
                y[ch] = soft_limit(x - self.dc[ch]);
            }
            outputs[0][0][k] = y[0];
            outputs[0][1][k] = y[1];
            peak = peak.max(y[0].abs()).max(y[1].abs());
        }
        p.meter.fetch_max(peak);

        // What the panel shows.
        p.viz[viz::COUNT].set(count as f32);
        for (vi, v) in self.voices.iter().enumerate() {
            let base = viz::VOICE + vi * viz::VOICE_LEN;
            p.viz[base].set(if v.active { 1.0 } else { 0.0 });
            p.viz[base + 1].set(v.note as f32);
            p.viz[base + 2].set(v.level);
            if v.active {
                for i in 0..count {
                    let nd = &v.swarm.nodes[i];
                    let at = base + 3 + i * 3;
                    p.viz[at].set(nd.x);
                    p.viz[at + 1].set(nd.y);
                    p.viz[at + 2].set(v.nodes[i].amp);
                }
            }
        }
        for i in 0..count {
            let nd = &self.ghost.nodes[i];
            p.viz[viz::GHOST + i * 2].set(nd.x);
            p.viz[viz::GHOST + i * 2 + 1].set(nd.y);
        }
        let (ax, ay) = match newest {
            Some((_, vi)) => self.voices[vi].swarm.attractor(&field),
            None => self.ghost.attractor(&field),
        };
        p.viz[viz::AX].set(ax);
        p.viz[viz::AY].set(ay);
        if let Some((_, vi)) = newest {
            let v = &self.voices[vi];
            for i in 0..count {
                let at = viz::LANES + i * viz::LANE_LEN;
                p.viz[at].set(v.nodes[i].rel);
                p.viz[at + 1].set(v.nodes[i].bright);
                p.viz[at + 2].set(v.nodes[i].pan);
            }
        }
        let _ = fm::OPS;
    }
}
