//! Frakture: a granular rhythm effect after Excite Audio's Motion: Fractal.
//! Four effects in a chain whose order can be changed — GRAINS (two grain
//! layers playing from a tempo-synced buffer of the input), GATE (a trance
//! gate of up to 32 steps), REPEAT (a stutter repeating slices of the
//! input) and SPACE (a hall reverb) — between global IN, OUT and WET, with
//! a safety limiter, INPUT THRU and bypass.
//!
//! MUTATE pushes the grains further: FEEDBACK sends them back into the
//! buffer, SWARM spawns clouds of them, GLIDE bends each one's pitch,
//! SHIMMER throws some up an octave or a fifth, RESONATE makes them ring on
//! the harmonics of KEY, BLUR melts them together and ERODE wears some of
//! them down to a few bits.
//!
//! Every effect knob can be mapped to one of six macro sliders laid out as a
//! hexagon (each mapping with its own range), or to the input's level. A
//! magnet cursor moves the sliders' nodes: it attracts or repels them along
//! their sliders, and they return to their start positions after HOLD, over
//! RELEASE. The cursor is the mouse, or a drawn or recorded path played on
//! a timeline (see [`motion`]).

mod grains;
pub mod motion;
mod rhythm;
mod space;

use std::f32::consts::SQRT_2;
use std::sync::Arc;

use super::{ModuleDef, ModuleParams, Quiet, approach, silence, soft_limit, time_coeff};
use crate::audio::engine::{BLOCK, Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{ParamSpec, Unit, format_db, format_hz, format_seconds, gain_to_db};
use grains::{GrainSettings, Grains, LayerSettings, MutateSettings, Playhead, Rate};
use motion::{Motion, SLIDERS};
use rhythm::{Gate, GateSettings, Repeat, RepeatSettings};
use space::Space;

pub use grains::shape_ramps;

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

pub const IN: usize = 0;
pub const OUT: usize = 1;
pub const WET: usize = 2;
/// The grains' LO–HI band.
pub const LO: usize = 3;
pub const HI: usize = 4;
/// Index into [`BUFFERS`].
pub const BUFFER: usize = 5;
/// First parameter of grain layer 1; layer 2's follow (see [`layer`], [`lp`]).
pub const LAYER0: usize = 6;

/// A grain layer's parameters, relative to its first one.
pub mod layer {
    /// Meaning set by the playhead mode (see [`super::Playhead`]).
    pub const PLAYHEAD: usize = 0;
    pub const RANGE: usize = 1;
    /// Grains per second, or one per note value with DENSITY SYNC.
    pub const DENSITY: usize = 2;
    /// Grain length, or a note value with SIZE SYNC.
    pub const SIZE: usize = 3;
    pub const CHAOS: usize = 4;
    pub const DIRECTION: usize = 5;
    pub const PITCH: usize = 6;
    pub const SPREAD: usize = 7;
    pub const WIDTH: usize = 8;
    pub const SHAPE: usize = 9;
    pub const DRY: usize = 10;
    pub const WET: usize = 11;
    pub const COUNT: usize = 12;
}

/// Index of grain layer `layer`'s parameter `k`.
pub const fn lp(layer: usize, k: usize) -> usize {
    LAYER0 + layer * layer::COUNT + k
}

pub const GATE_STEPS: usize = 30;
/// Index into [`DIVS`].
pub const GATE_RATE: usize = 31;
/// Fractions of a step.
pub const GATE_ATTACK: usize = 32;
pub const GATE_RELEASE: usize = 33;
pub const GATE_MIX: usize = 34;
pub const REP_LO: usize = 35;
pub const REP_HI: usize = 36;
pub const REP_AMOUNT: usize = 37;
/// Index into [`DIVS`]: how often a slice is taken.
pub const REP_TRIGGER: usize = 38;
/// Where in the trigger period, as a fraction of it.
pub const REP_OFFSET: usize = 39;
pub const REP_DUCK: usize = 40;
/// The slice's length: a note value with SYNC, else milliseconds.
pub const REP_RATE: usize = 41;
pub const REP_VOLUME: usize = 42;
pub const SPACE_LENGTH: usize = 43;
pub const SPACE_MIX: usize = 44;
/// MUTATE: grains back into the buffer; grains per trigger; each grain's
/// pitch bend (bipolar); octave and fifth jumps; resonance on KEY's
/// harmonics, and KEY; the grains melted together; grains worn down.
pub const FEEDBACK: usize = 45;
pub const SWARM: usize = 46;
pub const GLIDE: usize = 47;
pub const SHIMMER: usize = 48;
pub const RESONATE: usize = 49;
pub const KEY: usize = 50;
pub const BLUR: usize = 51;
pub const ERODE: usize = 52;
/// Parameters below this are effect settings, which can be mapped to the
/// hexagon; the rest set the hexagon itself.
pub const MAPPABLE: usize = 53;
pub const AR_GAIN: usize = 53;
pub const AR_ATTACK: usize = 54;
pub const AR_RELEASE: usize = 55;
pub const AR_THRESHOLD: usize = 56;
pub const CUR_RANGE: usize = 57;
pub const CUR_STRENGTH: usize = 58;
/// At its maximum the sliders stay where the magnet left them.
pub const CUR_HOLD: usize = 59;
pub const CUR_RELEASE: usize = 60;
/// The path's duration: a note value with TIME SYNC, else seconds.
pub const TL_TIME: usize = 61;
/// Index into [`MULTIPLIERS`].
pub const TL_MULT: usize = 62;
pub const TL_OFFSET: usize = 63;
/// MUTATE's note values, used with SYNC: how far apart a swarm's grains
/// come, how long GLIDE's bend takes, how late FEEDBACK's grains return
/// (indices into [`DIVS`]).
pub const SWARM_RATE: usize = 64;
pub const GLIDE_TIME: usize = 65;
pub const FEEDBACK_TIME: usize = 66;
pub const PARAM_COUNT: usize = 67;

const M60: f32 = 0.001;
const M24: f32 = 0.063_095_73;
const M12: f32 = 0.251_188_64;
const P6: f32 = 1.995_262_3;
const P12: f32 = 3.981_071_7;
const P24: f32 = 15.848_932;

/// Note values for tempo-synced settings, shortest first: (label, beats).
pub const DIVS: [(&str, f64); 18] = [
    ("1/64", 0.0625),
    ("1/32T", 1.0 / 12.0),
    ("1/32", 0.125),
    ("1/16T", 1.0 / 6.0),
    ("1/16", 0.25),
    ("1/8T", 1.0 / 3.0),
    ("1/16D", 0.375),
    ("1/8", 0.5),
    ("1/4T", 2.0 / 3.0),
    ("1/8D", 0.75),
    ("1/4", 1.0),
    ("1/2T", 4.0 / 3.0),
    ("1/4D", 1.5),
    ("1/2", 2.0),
    ("1/2D", 3.0),
    ("1/1", 4.0),
    ("2/1", 8.0),
    ("4/1", 16.0),
];

/// The part of [`DIVS`] a synced setting picks from: (first, last).
pub type DivRange = (usize, usize);
pub const DENSITY_DIVS: DivRange = (0, 15);
pub const SIZE_DIVS: DivRange = (0, 15);
pub const OFFSET_DIVS: DivRange = (0, 17);
pub const GATE_DIVS: DivRange = (0, 10);
pub const TRIGGER_DIVS: DivRange = (10, 17);
pub const REPEAT_DIVS: DivRange = (0, 13);
pub const TIME_DIVS: DivRange = (4, 17);
pub const SPACE_DIVS: DivRange = (10, 17);
pub const SWARM_DIVS: DivRange = (0, 7);
pub const GLIDE_DIVS: DivRange = (4, 17);
pub const FEEDBACK_DIVS: DivRange = (4, 15);

/// SCAN's speeds with SYNC, times the buffer's own (so the scan stays
/// locked to the transport).
pub const SCAN_RATIOS: [(&str, f32); 8] = [
    ("−2×", -2.0),
    ("−1×", -1.0),
    ("−½×", -0.5),
    ("−¼×", -0.25),
    ("¼×", 0.25),
    ("½×", 0.5),
    ("1×", 1.0),
    ("2×", 2.0),
];
pub const SCAN_RANGE: DivRange = (0, 7);

/// Free (unsynced) ranges of the settings that can also be synced.
pub const DENSITY_HZ: (f32, f32) = (0.5, 100.0);
pub const SIZE_S: (f32, f32) = (0.005, 2.0);
pub const OFFSET_S: (f32, f32) = (0.001, 20.0);
pub const REPEAT_S: (f32, f32) = (0.01, 1.0);
pub const TIME_S: (f32, f32) = (0.1, 60.0);
/// SCAN speed at the ends of the PLAYHEAD knob, times real time.
pub const SCAN_SPEED: f32 = 2.0;

/// The note value a normalized knob picks within `range`.
pub fn pick(v: f32, (lo, hi): DivRange) -> usize {
    lo + (v.clamp(0.0, 1.0) * (hi - lo) as f32).round() as usize
}

/// The knob position that picks `DIVS[i]` within `range`.
pub const fn div_norm(i: usize, (lo, hi): DivRange) -> f32 {
    (i - lo) as f32 / (hi - lo) as f32
}

/// A normalized knob on a logarithmic range.
pub fn log_range(v: f32, (lo, hi): (f32, f32)) -> f32 {
    lo * (hi / lo).powf(v.clamp(0.0, 1.0))
}

/// Buffer lengths: (label, beats).
pub const BUFFERS: [(&str, f64); 5] = [
    ("1/4", 1.0),
    ("1/2", 2.0),
    ("1 bar", 4.0),
    ("2 bars", 8.0),
    ("4 bars", 16.0),
];

/// Timeline multipliers: (label, factor).
pub const MULTIPLIERS: [(&str, f64); 8] = [
    ("×1/4", 0.25),
    ("×1/2", 0.5),
    ("×1", 1.0),
    ("×2", 2.0),
    ("×3", 3.0),
    ("×4", 4.0),
    ("×8", 8.0),
    ("×16", 16.0),
];

const PARAMS: [ParamSpec; PARAM_COUNT] = [
    ParamSpec::log("IN", M24, P12, 1.0, Unit::Decibel),
    ParamSpec::log("OUT", M24, P12, 1.0, Unit::Decibel),
    ParamSpec::lin("WET", 0.0, 1.0, 1.0, Unit::Percent),
    ParamSpec::log("GRAINS LO", 20.0, 20_000.0, 20.0, Unit::Hz),
    ParamSpec::log("GRAINS HI", 20.0, 20_000.0, 20_000.0, Unit::Hz),
    ParamSpec::lin("BUFFER", 0.0, 4.0, 2.0, Unit::Integer),
    // Grain layer 1: a rhythmic granular delay a quarter note behind.
    ParamSpec::lin(
        "G1 PLAYHEAD",
        0.0,
        1.0,
        div_norm(10, OFFSET_DIVS),
        Unit::Percent,
    ),
    ParamSpec::lin("G1 RANGE", 0.0, 1.0, 0.1, Unit::Percent),
    ParamSpec::lin(
        "G1 DENSITY",
        0.0,
        1.0,
        div_norm(4, DENSITY_DIVS),
        Unit::Percent,
    ),
    ParamSpec::lin("G1 SIZE", 0.0, 1.0, div_norm(7, SIZE_DIVS), Unit::Percent),
    ParamSpec::lin("G1 CHAOS", 0.0, 1.0, 0.2, Unit::Percent),
    ParamSpec::lin("G1 DIRECTION", -1.0, 1.0, 1.0, Unit::Percent).bipolar(),
    ParamSpec::lin("G1 PITCH", -36.0, 36.0, 0.0, Unit::Semitones).bipolar(),
    ParamSpec::lin("G1 SPREAD", 0.0, 36.0, 0.0, Unit::Semitones),
    ParamSpec::lin("G1 WIDTH", 0.0, 1.0, 0.4, Unit::Percent),
    ParamSpec::lin("G1 SHAPE", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::log("G1 DRY", M60, P6, 1.0, Unit::Decibel),
    ParamSpec::log("G1 WET", M60, P6, 0.7, Unit::Decibel),
    // Grain layer 2: sparkles an octave up, scattered across the buffer.
    ParamSpec::lin("G2 PLAYHEAD", 0.0, 1.0, 0.25, Unit::Percent),
    ParamSpec::lin("G2 RANGE", 0.0, 1.0, 0.2, Unit::Percent),
    ParamSpec::lin(
        "G2 DENSITY",
        0.0,
        1.0,
        div_norm(7, DENSITY_DIVS),
        Unit::Percent,
    ),
    ParamSpec::lin("G2 SIZE", 0.0, 1.0, div_norm(4, SIZE_DIVS), Unit::Percent),
    ParamSpec::lin("G2 CHAOS", 0.0, 1.0, 0.4, Unit::Percent),
    ParamSpec::lin("G2 DIRECTION", -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
    ParamSpec::lin("G2 PITCH", -36.0, 36.0, 12.0, Unit::Semitones).bipolar(),
    ParamSpec::lin("G2 SPREAD", 0.0, 36.0, 12.0, Unit::Semitones),
    ParamSpec::lin("G2 WIDTH", 0.0, 1.0, 0.8, Unit::Percent),
    ParamSpec::lin("G2 SHAPE", 0.0, 1.0, 0.15, Unit::Percent),
    ParamSpec::log("G2 DRY", M60, P6, 1.0, Unit::Decibel),
    ParamSpec::log("G2 WET", M60, P6, 0.5, Unit::Decibel),
    ParamSpec::lin("GATE STEPS", 1.0, 32.0, 16.0, Unit::Integer),
    ParamSpec::lin("GATE RATE", 0.0, 10.0, 4.0, Unit::Integer),
    ParamSpec::lin("GATE ATTACK", 0.0, 1.0, 0.05, Unit::Percent),
    ParamSpec::lin("GATE RELEASE", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("GATE MIX", 0.0, 1.0, 1.0, Unit::Percent),
    ParamSpec::log("REPEAT LO", 20.0, 20_000.0, 20.0, Unit::Hz),
    ParamSpec::log("REPEAT HI", 20.0, 20_000.0, 20_000.0, Unit::Hz),
    ParamSpec::lin("REPEAT AMOUNT", 1.0, 16.0, 4.0, Unit::Integer),
    ParamSpec::lin("REPEAT TRIGGER", 10.0, 17.0, 15.0, Unit::Integer),
    ParamSpec::lin("REPEAT OFFSET", 0.0, 1.0, 0.25, Unit::Percent),
    ParamSpec::lin("REPEAT DUCK", 0.0, 1.0, 0.8, Unit::Percent),
    ParamSpec::lin(
        "REPEAT RATE",
        0.0,
        1.0,
        div_norm(4, REPEAT_DIVS),
        Unit::Percent,
    ),
    ParamSpec::log("REPEAT VOLUME", M60, P6, 1.0, Unit::Decibel),
    ParamSpec::log("SPACE LENGTH", 0.3, 20.0, 2.5, Unit::Seconds),
    ParamSpec::lin("SPACE MIX", 0.0, 1.0, 0.2, Unit::Percent),
    ParamSpec::lin("FEEDBACK", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("SWARM", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("GLIDE", -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
    ParamSpec::lin("SHIMMER", 0.0, 1.0, 0.15, Unit::Percent),
    ParamSpec::lin("RESONATE", 0.0, 1.0, 0.25, Unit::Percent),
    ParamSpec::log("KEY", 30.0, 600.0, 110.0, Unit::Hz),
    ParamSpec::lin("BLUR", 0.0, 1.0, 0.35, Unit::Percent),
    ParamSpec::lin("ERODE", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::log("AUDIO GAIN", M12, P24, 1.0, Unit::Decibel),
    ParamSpec::log("AUDIO ATTACK", 0.001, 0.5, 0.01, Unit::Seconds),
    ParamSpec::log("AUDIO RELEASE", 0.01, 5.0, 0.25, Unit::Seconds),
    ParamSpec::log("AUDIO THRESHOLD", M60, 1.0, 0.031_622_78, Unit::Decibel),
    ParamSpec::lin("MAGNET RANGE", 0.05, 1.0, 0.4, Unit::Percent),
    ParamSpec::lin("MAGNET STRENGTH", 0.0, 1.0, 0.6, Unit::Percent),
    ParamSpec::lin("HOLD", 0.0, 10.0, 0.5, Unit::Seconds),
    ParamSpec::log("RELEASE", 0.02, 10.0, 1.5, Unit::Seconds),
    ParamSpec::lin("TIME", 0.0, 1.0, div_norm(15, TIME_DIVS), Unit::Percent),
    ParamSpec::lin("MULTIPLIER", 0.0, 7.0, 5.0, Unit::Integer),
    ParamSpec::lin("PATH OFFSET", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin(
        "SWARM RATE",
        SWARM_DIVS.0 as f32,
        SWARM_DIVS.1 as f32,
        2.0,
        Unit::Integer,
    ),
    ParamSpec::lin(
        "GLIDE TIME",
        GLIDE_DIVS.0 as f32,
        GLIDE_DIVS.1 as f32,
        10.0,
        Unit::Integer,
    ),
    ParamSpec::lin(
        "FEEDBACK TIME",
        FEEDBACK_DIVS.0 as f32,
        FEEDBACK_DIVS.1 as f32,
        10.0,
        Unit::Integer,
    ),
];

/// HOLD at (or near) its maximum: the sliders never return.
pub const HOLD_FOREVER: f32 = 9.95;

// ---------------------------------------------------------------------------
// Settings beyond the knobs
// ---------------------------------------------------------------------------

/// The effects, as numbered in the routing.
pub const GRAINS: usize = 0;
pub const GATE: usize = 1;
pub const REPEAT: usize = 2;
pub const SPACE: usize = 3;
pub const SECTION_NAMES: [&str; 4] = ["Grains", "Gate", "Repeat", "Space"];

/// Playhead modes: where a layer's grains come from.
pub const MANUAL: usize = 0;
pub const SCAN: usize = 1;
pub const OFFSET_SYNC: usize = 2;
pub const OFFSET_MS: usize = 3;
pub const OFFSET_PCT: usize = 4;
/// (Button, long name).
pub const PLAYHEAD_MODES: [(&str, &str); 5] = [
    (
        "MAN",
        "Manual: grains come from the PLAYHEAD position in the buffer",
    ),
    (
        "SCAN",
        "Scan: the playhead moves through the buffer, forwards or backwards",
    ),
    (
        "SYNC",
        "Offset (sync): a note value behind the recording, a granular delay",
    ),
    ("MS", "Offset (ms): milliseconds behind the recording"),
    (
        "%",
        "Offset (%): a fraction of the buffer behind the recording",
    ),
];

/// Pitch modes: the intervals PITCH and SPREAD snap to.
pub const PITCH_MODES: [(&str, &str); 6] = [
    ("FREE", "Free: any pitch"),
    ("CHR", "Chromatic: semitones"),
    ("OCT", "Octaves"),
    ("4TH", "Fourths (and octaves)"),
    ("5TH", "Fifths (and octaves)"),
    ("4+5", "Fourths & fifths (and octaves)"),
];

/// What a hexagon mapping follows: 1…6 are the sliders.
pub const AUDIO_SLOT: usize = 7;

pub const GATE_MAX: usize = 32;
/// Points a drawn path can have.
pub const MAX_POINTS: usize = 32;
/// Samples a recorded path can have, and how many per second.
pub const MAX_REC: usize = 750;
pub const REC_HZ: f32 = 25.0;

/// What `ModuleParams::data` holds besides the knobs.
pub mod data {
    use super::{GATE_MAX, MAPPABLE, MAX_POINTS, MAX_REC};
    pub const BYPASS: usize = 0;
    pub const LIMITER: usize = 1;
    /// INPUT THRU: off, nothing new comes in and the grains play out.
    pub const THRU: usize = 2;
    /// On/off of each effect, by section number.
    pub const SECTION_ON: usize = 3;
    pub const RETRIG: usize = 7;
    pub const FREEZE: usize = 8;
    /// The effects' displays are shown.
    pub const DISPLAYS: usize = 9;
    /// Per grain layer.
    pub const LAYER_ON: usize = 10;
    pub const PLAYHEAD_MODE: usize = 12;
    pub const DENSITY_SYNC: usize = 14;
    pub const SIZE_SYNC: usize = 16;
    pub const PITCH_MODE: usize = 18;
    pub const REPEAT_SYNC: usize = 20;
    /// The four section numbers in processing order.
    pub const ROUTING: usize = 21;
    pub const GATE_PATTERN: usize = 25;
    /// 0 select, 1 attract, 2 repel (see [`super::motion`]).
    pub const MAGNET: usize = GATE_PATTERN + GATE_MAX;
    /// 0 constant, 1 while clicked, 2 while not clicked.
    pub const ACTIVATION: usize = MAGNET + 1;
    /// Where each slider rests.
    pub const SLIDER_START: usize = ACTIVATION + 1;
    pub const PATH_PLAYING: usize = SLIDER_START + 6;
    /// 0 the drawn path, 1 the recorded one.
    pub const PATH_KIND: usize = PATH_PLAYING + 1;
    /// 0 fixed (TIME × MULTIPLIER), 1 free (the recording's length).
    pub const DURATION: usize = PATH_KIND + 1;
    /// 0 the timeline's even speed, 1 the speed it was recorded at.
    pub const SPEED: usize = DURATION + 1;
    /// 0 off, 1 on transport start, 2 locked to the transport.
    pub const RETRIGGER: usize = SPEED + 1;
    pub const TIME_SYNC: usize = RETRIGGER + 1;
    /// 0 forwards, 1 backwards, 2 pendulum.
    pub const DIRECTION: usize = TIME_SYNC + 1;
    pub const LOOP: usize = DIRECTION + 1;
    /// Panel state: page, grain layer shown, mapping list shown (slider
    /// 0…5, 6 the input level) and path tool.
    pub const PAGE: usize = LOOP + 1;
    pub const LAYER_SHOWN: usize = PAGE + 1;
    pub const SELECTED: usize = LAYER_SHOWN + 1;
    pub const TOOL: usize = SELECTED + 1;
    /// The part of the recording played, as fractions.
    pub const REC_START: usize = TOOL + 1;
    pub const REC_END: usize = REC_START + 1;
    pub const DRAWN_COUNT: usize = REC_END + 1;
    pub const DRAWN_CLOSED: usize = DRAWN_COUNT + 1;
    /// MAX_POINTS × (x, y, bend, clicked): x and y in −1…1, y down.
    pub const DRAWN: usize = DRAWN_CLOSED + 1;
    pub const REC_COUNT: usize = DRAWN + MAX_POINTS * 4;
    /// MAX_REC × (x, y, clicked).
    pub const REC: usize = REC_COUNT + 1;
    /// MAPPABLE × (slot, min, max): slot 0 none, 1…6 a slider, 7 the
    /// input level; min and max as knob positions (0…1).
    pub const MAP: usize = REC + MAX_REC * 3;
    /// Per grain layer: MUTATE changes it.
    pub const MUTATE_LAYERS: usize = MAP + MAPPABLE * 3;
    /// Locked to the tempo: each layer's SCAN, SPACE's length, and
    /// MUTATE's SWARM, GLIDE and FEEDBACK.
    pub const SCAN_SYNC: usize = MUTATE_LAYERS + 2;
    pub const SPACE_SYNC: usize = SCAN_SYNC + 2;
    pub const SWARM_SYNC: usize = SPACE_SYNC + 1;
    pub const GLIDE_SYNC: usize = SWARM_SYNC + 1;
    pub const FEEDBACK_SYNC: usize = GLIDE_SYNC + 1;
    pub const LEN: usize = FEEDBACK_SYNC + 1;
}

/// Cosine and sine of 0°, 30°, … 330°, for the default circular path.
const CIRCLE: [(f32, f32); 12] = [
    (1.0, 0.0),
    (0.866_025_4, 0.5),
    (0.5, 0.866_025_4),
    (0.0, 1.0),
    (-0.5, 0.866_025_4),
    (-0.866_025_4, 0.5),
    (-1.0, 0.0),
    (-0.866_025_4, -0.5),
    (-0.5, -0.866_025_4),
    (0.0, -1.0),
    (0.5, -0.866_025_4),
    (0.866_025_4, -0.5),
];

/// Radius of the default path and of new circles.
pub const CIRCLE_R: f32 = 0.55;

/// A starting point that does something: a circling path, and six
/// mappings on the first grain layer (its pitch in fourths and fifths,
/// ±1 octave) and the reverb.
const DATA: [f32; data::LEN] = {
    let mut d = [0.0; data::LEN];
    d[data::LIMITER] = 1.0;
    d[data::THRU] = 1.0;
    d[data::SECTION_ON + GRAINS] = 1.0;
    d[data::SECTION_ON + SPACE] = 1.0;
    d[data::RETRIG] = 1.0;
    d[data::DISPLAYS] = 1.0;
    d[data::LAYER_ON] = 1.0;
    d[data::PLAYHEAD_MODE] = OFFSET_SYNC as f32;
    d[data::PLAYHEAD_MODE + 1] = MANUAL as f32;
    d[data::DENSITY_SYNC] = 1.0;
    d[data::DENSITY_SYNC + 1] = 1.0;
    d[data::SIZE_SYNC] = 1.0;
    d[data::SIZE_SYNC + 1] = 1.0;
    d[data::PITCH_MODE] = 5.0;
    d[data::PITCH_MODE + 1] = 1.0;
    d[data::REPEAT_SYNC] = 1.0;
    let mut i = 0;
    while i < 4 {
        d[data::ROUTING + i] = i as f32;
        i += 1;
    }
    const PATTERN: [f32; 16] = [
        1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0,
    ];
    let mut i = 0;
    while i < GATE_MAX {
        d[data::GATE_PATTERN + i] = PATTERN[i % 16];
        i += 1;
    }
    d[data::MAGNET] = motion::ATTRACT as f32;
    d[data::ACTIVATION] = motion::CONSTANT as f32;
    let mut i = 0;
    while i < SLIDERS {
        d[data::SLIDER_START + i] = 0.5;
        i += 1;
    }
    d[data::DURATION] = motion::FIXED as f32;
    d[data::SPEED] = motion::EVEN as f32;
    d[data::RETRIGGER] = motion::RETRIG_SYNC as f32;
    d[data::DIRECTION] = motion::FORWARD as f32;
    d[data::TIME_SYNC] = 1.0;
    d[data::LOOP] = 1.0;
    d[data::REC_END] = 1.0;
    d[data::DRAWN_COUNT] = 12.0;
    d[data::DRAWN_CLOSED] = 1.0;
    let mut i = 0;
    while i < 12 {
        let at = data::DRAWN + i * 4;
        d[at] = CIRCLE[i].0 * CIRCLE_R;
        d[at + 1] = CIRCLE[i].1 * CIRCLE_R;
        d[at + 3] = 1.0;
        i += 1;
    }
    // Slider by slider: what it moves, and over what range.
    let maps: [(usize, usize, f32, f32); 6] = [
        (1, lp(0, layer::DENSITY), 0.0, 0.6),
        (2, lp(0, layer::PITCH), 1.0 / 3.0, 2.0 / 3.0),
        (3, lp(0, layer::RANGE), 0.0, 0.6),
        (4, lp(0, layer::CHAOS), 0.0, 1.0),
        (5, SPACE_MIX, 0.05, 0.6),
        (6, lp(0, layer::WET), 0.75, 0.97),
    ];
    let mut s = 0;
    while s < maps.len() {
        let at = data::MAP + maps[s].1 * 3;
        d[at] = maps[s].0 as f32;
        d[at + 1] = maps[s].2;
        d[at + 2] = maps[s].3;
        s += 1;
    }
    d[data::MUTATE_LAYERS] = 1.0;
    d[data::MUTATE_LAYERS + 1] = 1.0;
    d
};

pub const VIZ_GRAINS: usize = 24;
/// Values shown per grain: position, pitch now, level, kind (see
/// [`grain_kind`]), pitch at its start.
pub const GRAIN_VIZ: usize = 5;

/// What a grain shown has been through, as bits of its kind value.
pub mod grain_kind {
    pub const RESONANT: u32 = 1;
    pub const ERODED: u32 = 2;
    pub const SHIMMER: u32 = 4;
}
pub const OVERVIEW: usize = 128;
pub const GATE_BINS: usize = 64;
pub const SLICE_BINS: usize = 48;

/// Layout of `ModuleParams::viz`: what the processor shows the panel, and a
/// few values the panel hands the processor (the mouse over the hexagon).
pub mod viz {
    use super::{GATE_BINS, GRAIN_VIZ, MAPPABLE, OVERVIEW, SLICE_BINS, VIZ_GRAINS};
    /// The six sliders' live values.
    pub const SLIDERS: usize = 0;
    /// The input-level follower, 0…1.
    pub const AUDIO_ENV: usize = 6;
    /// The magnet cursor: where, and 0 none, 1 present, 2 pulling/pushing.
    pub const CURSOR_X: usize = 7;
    pub const CURSOR_Y: usize = 8;
    pub const CURSOR_STATE: usize = 9;
    /// Position along the path while it plays (else −1), and whether a
    /// path that doesn't loop has ended.
    pub const PATH_POS: usize = 10;
    pub const PATH_DONE: usize = 11;
    /// Output level, smoothed.
    pub const LEVEL: usize = 12;
    /// From the panel: the mouse in hexagon space, over it, clicked, and
    /// a counter bumped every frame (a stale one means no mouse).
    pub const UI_X: usize = 13;
    pub const UI_Y: usize = 14;
    pub const UI_IN: usize = 15;
    pub const UI_DOWN: usize = 16;
    pub const UI_STAMP: usize = 17;
    /// Live knob positions of the mapped parameters (−1: not mapped).
    pub const EFFECTIVE: usize = 18;
    pub const BUF_SECONDS: usize = EFFECTIVE + MAPPABLE;
    /// Write head and each layer's playhead, as fractions of the buffer.
    pub const WRITE_HEAD: usize = BUF_SECONDS + 1;
    pub const PLAYHEADS: usize = WRITE_HEAD + 1;
    pub const GRAIN_COUNT: usize = PLAYHEADS + 2;
    /// Peak level per section of the buffer.
    pub const OVERVIEW_AT: usize = GRAIN_COUNT + 2;
    /// Per layer, VIZ_GRAINS × GRAIN_VIZ (position, pitch in semitones,
    /// level, kind, starting pitch; a negative level: none).
    pub const GRAINS: usize = OVERVIEW_AT + OVERVIEW;
    /// Position in the gate pattern (0…1), and the levels before and after
    /// the gate along it.
    pub const GATE_POS: usize = GRAINS + 2 * VIZ_GRAINS * GRAIN_VIZ;
    pub const GATE_IN: usize = GATE_POS + 1;
    pub const GATE_OUT: usize = GATE_IN + GATE_BINS;
    /// 0 idle, 1 taking a slice, 2 repeating it; repeats played; position
    /// in the slice; the slice's levels.
    pub const REP_STATE: usize = GATE_OUT + GATE_BINS;
    pub const REP_COUNT: usize = REP_STATE + 1;
    pub const REP_PROGRESS: usize = REP_COUNT + 1;
    pub const REP_SLICE: usize = REP_PROGRESS + 1;
    pub const LEN: usize = REP_SLICE + SLICE_BINS;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Frakture",
    prefix: "FRAKTURE",
    hint: "granular rhythms · hexagon",
    inputs: &["IN"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

pub fn on(p: &ModuleParams, i: usize) -> bool {
    p.data[i].get() > 0.5
}

/// A setting with `count` choices.
pub fn choice(p: &ModuleParams, i: usize, count: usize) -> usize {
    (p.data[i].get().round().max(0.0) as usize).min(count - 1)
}

/// The sections in processing order (a broken order falls back to the
/// default one).
pub fn routing(p: &ModuleParams) -> [usize; 4] {
    let order: [usize; 4] = std::array::from_fn(|i| choice(p, data::ROUTING + i, 4));
    let mut seen = [false; 4];
    for &s in &order {
        seen[s] = true;
    }
    if seen.iter().all(|&s| s) {
        order
    } else {
        [GRAINS, GATE, REPEAT, SPACE]
    }
}

/// A parameter's hexagon mapping: (slot, min, max).
pub fn mapping(p: &ModuleParams, i: usize) -> Option<(usize, f32, f32)> {
    if i >= MAPPABLE {
        return None;
    }
    let at = data::MAP + i * 3;
    let slot = p.data[at].get().round();
    (1.0..=AUDIO_SLOT as f32)
        .contains(&slot)
        .then(|| (slot as usize, p.data[at + 1].get(), p.data[at + 2].get()))
}

pub fn set_mapping(p: &ModuleParams, i: usize, m: Option<(usize, f32, f32)>) {
    if i >= MAPPABLE {
        return;
    }
    let at = data::MAP + i * 3;
    let (slot, lo, hi) = m.unwrap_or((0, 0.0, 1.0));
    p.data[at].set(slot as f32);
    p.data[at + 1].set(lo.clamp(0.0, 1.0));
    p.data[at + 2].set(hi.clamp(0.0, 1.0));
}

/// The pitch nearest `semis` that the mode allows.
pub fn quantize(semis: f32, mode: usize) -> f32 {
    let allowed: &[f32] = match mode {
        0 => return semis,
        1 => return semis.round(),
        2 => &[0.0, 12.0],
        3 => &[0.0, 5.0, 12.0],
        4 => &[0.0, 7.0, 12.0],
        _ => &[0.0, 5.0, 7.0, 12.0],
    };
    let octave = (semis / 12.0).floor() * 12.0;
    let mut best = octave;
    for &a in allowed {
        if (semis - (octave + a)).abs() < (semis - best).abs() {
            best = octave + a;
        }
    }
    best
}

/// A gain knob: its bottom is silence.
fn level(g: f32) -> f32 {
    if g <= M60 * 1.05 { 0.0 } else { g }
}

/// The DRY / WET / VOLUME readout.
fn format_level(g: f32) -> String {
    if level(g) == 0.0 {
        "-inf dB".to_owned()
    } else {
        format_db(gain_to_db(g))
    }
}

/// Short knob label.
pub fn label(i: usize) -> &'static str {
    if (LAYER0..GATE_STEPS).contains(&i) {
        return [
            "PLAYHEAD",
            "RANGE",
            "DENSITY",
            "SIZE",
            "CHAOS",
            "DIRECTION",
            "PITCH",
            "SPREAD",
            "WIDTH",
            "SHAPE",
            "DRY",
            "WET",
        ][(i - LAYER0) % layer::COUNT];
    }
    match i {
        IN => "IN",
        OUT => "OUT",
        WET => "WET",
        LO | REP_LO => "LO",
        HI | REP_HI => "HI",
        BUFFER => "BUFFER",
        GATE_STEPS => "STEPS",
        GATE_RATE | REP_RATE => "RATE",
        GATE_ATTACK | AR_ATTACK => "ATTACK",
        GATE_RELEASE | AR_RELEASE | CUR_RELEASE => "RELEASE",
        GATE_MIX | SPACE_MIX => "MIX",
        REP_AMOUNT => "AMOUNT",
        REP_TRIGGER => "TRIGGER",
        REP_OFFSET | TL_OFFSET => "OFFSET",
        SWARM_RATE => "SPACING",
        GLIDE_TIME => "BEND",
        FEEDBACK_TIME => "ECHO",
        REP_DUCK => "DUCK",
        REP_VOLUME => "VOLUME",
        SPACE_LENGTH => "LENGTH",
        FEEDBACK => "FEEDBACK",
        SWARM => "SWARM",
        GLIDE => "GLIDE",
        SHIMMER => "SHIMMER",
        RESONATE => "RESONATE",
        KEY => "KEY",
        BLUR => "BLUR",
        ERODE => "ERODE",
        AR_GAIN => "GAIN",
        AR_THRESHOLD => "THRESH",
        CUR_RANGE => "RANGE",
        CUR_STRENGTH => "STRENGTH",
        CUR_HOLD => "HOLD",
        TL_TIME => "TIME",
        TL_MULT => "MULTIPLY",
        _ => "",
    }
}

/// The section a parameter belongs to, for lists.
pub fn section(i: usize) -> &'static str {
    match i {
        IN | OUT | WET => "Global",
        LO | HI | BUFFER => "Grains",
        _ if i < LAYER0 + layer::COUNT => "Grains 1",
        _ if i < GATE_STEPS => "Grains 2",
        _ if i < REP_LO => "Gate",
        _ if i < SPACE_LENGTH => "Repeat",
        _ if i < FEEDBACK => "Space",
        _ if i < MAPPABLE => "Mutate",
        _ if i < CUR_RANGE => "Audio",
        _ if i < TL_TIME => "Cursor",
        _ if i < SWARM_RATE => "Timeline",
        _ => "Mutate",
    }
}

/// "Grains 1 · Pitch".
pub fn long_name(i: usize) -> String {
    let l = label(i);
    if l.is_empty() {
        return section(i).to_owned();
    }
    let mut name = l[..1].to_owned();
    name.push_str(&l[1..].to_lowercase());
    format!("{} · {name}", section(i))
}

/// A parameter's value as the panel shows it (some depend on modes).
pub fn format(p: &ModuleParams, i: usize, v: f32) -> String {
    let pct = |v: f32| format!("{:.0}%", v * 100.0);
    let div = |v: f32, r: DivRange| DIVS[pick(v, r)].0.to_owned();
    if (LAYER0..GATE_STEPS).contains(&i) {
        let k = (i - LAYER0) / layer::COUNT;
        return match (i - LAYER0) % layer::COUNT {
            layer::PLAYHEAD => match choice(p, data::PLAYHEAD_MODE + k, 5) {
                SCAN if on(p, data::SCAN_SYNC + k) => SCAN_RATIOS[pick(v, SCAN_RANGE)].0.to_owned(),
                SCAN => format!("{:+.2}×", (v - 0.5) * 2.0 * SCAN_SPEED),
                OFFSET_SYNC => div(v, OFFSET_DIVS),
                OFFSET_MS => format_seconds(log_range(v, OFFSET_S)),
                _ => pct(v),
            },
            layer::DENSITY if on(p, data::DENSITY_SYNC + k) => div(v, DENSITY_DIVS),
            layer::DENSITY => {
                let hz = log_range(v, DENSITY_HZ);
                if hz < 10.0 {
                    format!("{hz:.1}/s")
                } else {
                    format!("{hz:.0}/s")
                }
            }
            layer::SIZE if on(p, data::SIZE_SYNC + k) => div(v, SIZE_DIVS),
            layer::SIZE => format_seconds(log_range(v, SIZE_S)),
            layer::DIRECTION => format!("{:+.0}%", v * 100.0),
            layer::PITCH if choice(p, data::PITCH_MODE + k, 6) == 0 => format!("{v:+.1} st"),
            layer::PITCH => format!("{:+.0} st", quantize(v, choice(p, data::PITCH_MODE + k, 6))),
            layer::SPREAD => format!("±{v:.1} st"),
            layer::DRY | layer::WET => format_level(v),
            _ => pct(v),
        };
    }
    match i {
        IN | OUT | AR_GAIN | AR_THRESHOLD => format_db(gain_to_db(v)),
        LO | HI | REP_LO | REP_HI => format_hz(v),
        BUFFER => BUFFERS[(v.round().max(0.0) as usize).min(BUFFERS.len() - 1)]
            .0
            .to_owned(),
        GATE_STEPS | REP_AMOUNT => format!("{:.0}", v.round()),
        GATE_RATE | REP_TRIGGER => DIVS[(v.round().max(0.0) as usize).min(DIVS.len() - 1)]
            .0
            .to_owned(),
        REP_RATE if on(p, data::REPEAT_SYNC) => div(v, REPEAT_DIVS),
        REP_RATE => format_seconds(log_range(v, REPEAT_S)),
        REP_VOLUME => format_level(v),
        SPACE_LENGTH if on(p, data::SPACE_SYNC) => div(PARAMS[SPACE_LENGTH].to_norm(v), SPACE_DIVS),
        SPACE_LENGTH | AR_ATTACK | AR_RELEASE | CUR_RELEASE => format_seconds(v),
        SWARM_RATE | GLIDE_TIME | FEEDBACK_TIME => DIVS
            [(v.round().max(0.0) as usize).min(DIVS.len() - 1)]
        .0
        .to_owned(),
        CUR_HOLD if v >= HOLD_FOREVER => "∞".to_owned(),
        CUR_HOLD => format_seconds(v),
        TL_TIME if on(p, data::TIME_SYNC) => div(v, TIME_DIVS),
        TL_TIME => format_seconds(log_range(v, TIME_S)),
        TL_MULT => MULTIPLIERS[(v.round().max(0.0) as usize).min(MULTIPLIERS.len() - 1)]
            .0
            .to_owned(),
        GLIDE if v.abs() < 0.005 => "none".to_owned(),
        GLIDE => format!("{:+.0} st", v * GLIDE_OCTAVES * 12.0),
        SWARM => format!("×{}", swarm_most(v)),
        KEY => format!("{} · {}", format_hz(v), note_name(v)),
        FEEDBACK | SHIMMER | RESONATE | BLUR | ERODE if v <= 0.0 => "off".to_owned(),
        _ => pct(v),
    }
}

/// GLIDE's reach at full, either way.
pub const GLIDE_OCTAVES: f32 = 2.0;

/// The most grains SWARM spawns at a time.
pub fn swarm_most(v: f32) -> usize {
    1 + (v.clamp(0.0, 1.0) * 7.0).round() as usize
}

/// A note value's beats, from its index into [`DIVS`].
fn note(index: f32) -> f64 {
    DIVS[(index.round().max(0.0) as usize).min(DIVS.len() - 1)].1
}

/// The nearest note's name ("A2").
fn note_name(hz: f32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let n = (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32;
    format!(
        "{}{}",
        NAMES[n.rem_euclid(12) as usize],
        n.div_euclid(12) - 1
    )
}

/// A parameter's spec.
pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

// ---------------------------------------------------------------------------
// Shared DSP pieces
// ---------------------------------------------------------------------------

/// The transport for one block: the beat at its first sample (also while
/// stopped, when a free-running clock keeps the rhythms going).
#[derive(Clone, Copy)]
pub struct Clock {
    pub beat0: f64,
    pub bps: f64,
    pub samples_per_beat: f64,
    pub seconds_per_beat: f64,
    pub playing: bool,
    /// The transport has just started.
    pub started: bool,
}

impl Clock {
    #[inline]
    pub fn at(&self, i: usize) -> f64 {
        self.beat0 + self.bps * i as f64
    }
}

#[derive(Clone, Copy)]
struct Svf {
    a1: f32,
    a2: f32,
    a3: f32,
}

/// The LO–HI band: 12 dB/octave high-pass and low-pass, each skipped while
/// at the end of its range.
#[derive(Default)]
pub struct Band {
    hp: Option<Svf>,
    lp: Option<Svf>,
    /// (ic1, ic2) of the high-pass L/R, then the low-pass L/R.
    state: [[f32; 2]; 4],
    last: (f32, f32),
}

impl Band {
    pub fn set(&mut self, lo: f32, hi: f32, sr: f32) {
        if (lo, hi) == self.last {
            return;
        }
        self.last = (lo, hi);
        let svf = |fc: f32| {
            let g = fastmath::tan_turns(fc.clamp(10.0, sr * 0.45) / (2.0 * sr));
            let a1 = 1.0 / (1.0 + g * (g + SQRT_2));
            Svf {
                a1,
                a2: g * a1,
                a3: g * g * a1,
            }
        };
        self.hp = (lo > 21.0).then(|| svf(lo));
        self.lp = (hi < 19_500.0).then(|| svf(hi));
    }

    #[inline]
    fn tick(c: Svf, s: &mut [f32; 2], v0: f32) -> (f32, f32) {
        let v3 = v0 - s[1];
        let v1 = c.a1 * s[0] + c.a2 * v3;
        let v2 = s[1] + c.a2 * s[0] + c.a3 * v3;
        s[0] = 2.0 * v1 - s[0];
        s[1] = 2.0 * v2 - s[1];
        (v1, v2)
    }

    #[inline]
    pub fn process(&mut self, ch: usize, x: f32) -> f32 {
        let mut y = x;
        if let Some(c) = self.hp {
            let (v1, v2) = Self::tick(c, &mut self.state[ch], y);
            y = y - SQRT_2 * v1 - v2;
        }
        if let Some(c) = self.lp {
            y = Self::tick(c, &mut self.state[2 + ch], y).1;
        }
        y
    }
}

// ---------------------------------------------------------------------------
// Processor
// ---------------------------------------------------------------------------

pub struct Frakture {
    params: Arc<ModuleParams>,
    sr: f32,
    grains: Grains,
    gate: Gate,
    repeat: Repeat,
    space: Space,
    motion: Motion,
    /// This block's parameter values: the knobs, or where the hexagon
    /// puts the mapped ones.
    vals: [f32; PARAM_COUNT],
    /// Each section's on/off crossfade.
    fade: [f32; 4],
    bypass: f32,
    thru: f32,
    in_gain: f32,
    out_gain: f32,
    wet: f32,
    limit: f32,
    limit_release: f32,
    /// The beat while the transport is stopped.
    clock: f64,
    was_playing: bool,
    quiet: Quiet,
    level: f32,
    /// Blocks until the displays are refreshed.
    publish_in: u32,
}

impl Frakture {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        let grains = Grains::new(sample_rate);
        let memory = grains.capacity() + (sample_rate * 4.0) as usize;
        let lit = |i: usize| if on(&params, i) { 1.0 } else { 0.0 };
        Self {
            vals: std::array::from_fn(|i| params.get(i)),
            fade: std::array::from_fn(|s| lit(data::SECTION_ON + s)),
            bypass: lit(data::BYPASS),
            thru: lit(data::THRU),
            in_gain: params.get(IN),
            out_gain: params.get(OUT),
            wet: params.get(WET),
            motion: Motion::new(&params),
            grains,
            gate: Gate::new(),
            repeat: Repeat::new(sample_rate),
            space: Space::new(sample_rate),
            limit: 1.0,
            limit_release: time_coeff(0.08, sample_rate),
            clock: 0.0,
            was_playing: false,
            quiet: Quiet::new(memory),
            level: 0.0,
            publish_in: 0,
            params,
            sr: sample_rate,
        }
    }

    /// This block's values: each knob, or for a mapped one where its slider
    /// (or the input level) puts it within the mapping's range.
    fn resolve(&mut self) {
        let p = &self.params;
        for (i, v) in self.vals.iter_mut().enumerate() {
            *v = p.get(i);
        }
        for (i, spec) in PARAMS.iter().enumerate().take(MAPPABLE) {
            let Some((slot, lo, hi)) = mapping(p, i) else {
                p.viz[viz::EFFECTIVE + i].set(-1.0);
                continue;
            };
            let live = if slot == AUDIO_SLOT {
                self.motion.env
            } else {
                self.motion.values[slot - 1]
            };
            let norm = (lo + (hi - lo) * live).clamp(0.0, 1.0);
            self.vals[i] = spec.denorm(norm);
            p.viz[viz::EFFECTIVE + i].set(norm);
        }
    }

    fn grain_settings(&self, clock: &Clock) -> GrainSettings {
        let (p, v) = (&*self.params, &self.vals);
        let sr = self.sr as f64;
        let spb = clock.samples_per_beat;
        let layer = |k: usize| {
            let at = |j: usize| v[lp(k, j)];
            let density = if on(p, data::DENSITY_SYNC + k) {
                Rate::Grid(DIVS[pick(at(layer::DENSITY), DENSITY_DIVS)].1)
            } else {
                Rate::Hz(log_range(at(layer::DENSITY), DENSITY_HZ))
            };
            let size = if on(p, data::SIZE_SYNC + k) {
                DIVS[pick(at(layer::SIZE), SIZE_DIVS)].1 * spb
            } else {
                log_range(at(layer::SIZE), SIZE_S) as f64 * sr
            };
            let head = at(layer::PLAYHEAD);
            let playhead = match choice(p, data::PLAYHEAD_MODE + k, 5) {
                SCAN if on(p, data::SCAN_SYNC + k) => {
                    Playhead::ScanSync(SCAN_RATIOS[pick(head, SCAN_RANGE)].1)
                }
                SCAN => Playhead::Scan((head - 0.5) * 2.0 * SCAN_SPEED),
                OFFSET_SYNC => Playhead::Behind(DIVS[pick(head, OFFSET_DIVS)].1 * spb),
                OFFSET_MS => Playhead::Behind(log_range(head, OFFSET_S) as f64 * sr),
                OFFSET_PCT => Playhead::BehindFraction(head),
                _ => Playhead::At(head),
            };
            LayerSettings {
                on: on(p, data::LAYER_ON + k),
                playhead,
                range: at(layer::RANGE),
                density,
                size: size as f32,
                chaos: at(layer::CHAOS),
                direction: at(layer::DIRECTION),
                pitch: at(layer::PITCH),
                spread: at(layer::SPREAD),
                pitch_mode: choice(p, data::PITCH_MODE + k, PITCH_MODES.len()),
                width: at(layer::WIDTH),
                shape: at(layer::SHAPE),
                dry: level(at(layer::DRY)),
                wet: level(at(layer::WET)),
            }
        };
        GrainSettings {
            lo: v[LO],
            hi: v[HI],
            buffer_beats: BUFFERS[(v[BUFFER].round().max(0.0) as usize).min(BUFFERS.len() - 1)].1,
            record: !on(p, data::FREEZE) && on(p, data::THRU),
            refresh: clock.started && on(p, data::RETRIG),
            layers: [layer(0), layer(1)],
            mutate: MutateSettings {
                feedback: v[FEEDBACK],
                swarm: v[SWARM],
                glide: v[GLIDE] * GLIDE_OCTAVES,
                shimmer: v[SHIMMER],
                resonate: v[RESONATE],
                key: v[KEY],
                blur: v[BLUR],
                erode: v[ERODE],
                layers: [on(p, data::MUTATE_LAYERS), on(p, data::MUTATE_LAYERS + 1)],
                swarm_step: on(p, data::SWARM_SYNC).then(|| (note(v[SWARM_RATE]) * spb) as f32),
                glide_len: on(p, data::GLIDE_SYNC).then(|| (note(v[GLIDE_TIME]) * spb) as f32),
                feedback_delay: on(p, data::FEEDBACK_SYNC)
                    .then(|| (note(v[FEEDBACK_TIME]) * spb) as usize),
            },
        }
    }

    fn gate_settings(&self) -> GateSettings {
        let (p, v) = (&*self.params, &self.vals);
        GateSettings {
            steps: (v[GATE_STEPS].round() as usize).clamp(1, GATE_MAX),
            step_beats: DIVS[(v[GATE_RATE].round().max(0.0) as usize).min(GATE_DIVS.1)].1,
            attack: v[GATE_ATTACK],
            release: v[GATE_RELEASE],
            mix: v[GATE_MIX],
            pattern: std::array::from_fn(|i| on(p, data::GATE_PATTERN + i)),
        }
    }

    fn repeat_settings(&self, clock: &Clock) -> RepeatSettings {
        let (p, v) = (&*self.params, &self.vals);
        let slice = if on(p, data::REPEAT_SYNC) {
            DIVS[pick(v[REP_RATE], REPEAT_DIVS)].1 * clock.samples_per_beat
        } else {
            log_range(v[REP_RATE], REPEAT_S) as f64 * self.sr as f64
        };
        RepeatSettings {
            lo: v[REP_LO],
            hi: v[REP_HI],
            amount: (v[REP_AMOUNT].round() as usize).clamp(1, 16),
            trigger_beats: DIVS
                [(v[REP_TRIGGER].round().max(0.0) as usize).clamp(TRIGGER_DIVS.0, TRIGGER_DIVS.1)]
            .1,
            offset: v[REP_OFFSET],
            duck: v[REP_DUCK],
            slice: slice as usize,
            volume: level(v[REP_VOLUME]),
        }
    }

    fn publish(&self, p: &ModuleParams) {
        p.viz[viz::LEVEL].set(self.level);
        self.grains.publish(p);
        self.gate.publish(p);
        self.repeat.publish(p);
    }
}

impl Processor for Frakture {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let sr = self.sr;
        let bps = ctx.beats_per_sample.max(1e-9);
        let started = ctx.playing && !self.was_playing;
        self.was_playing = ctx.playing;
        let beat0 = if ctx.playing { ctx.beat } else { self.clock };
        self.clock = beat0 + bps * n as f64;
        let clock = Clock {
            beat0,
            bps,
            samples_per_beat: 1.0 / bps,
            seconds_per_beat: 1.0 / (bps * sr as f64),
            playing: ctx.playing,
            started,
        };

        let input = inputs.get(0);
        let in_peak = input.map_or(0.0, |b| {
            b.iter()
                .flat_map(|ch| &ch[..n])
                .fold(0.0f32, |m, s| m.max(s.abs()))
        });

        // The hexagon moves whether or not there is sound to process.
        self.motion
            .update(&p, &clock, in_peak * self.in_gain, n as f32 / sr);
        self.resolve();
        self.publish_in = self.publish_in.saturating_sub(1);
        let publish = self.publish_in == 0;
        if publish {
            self.publish_in = 4;
        }

        let bypass_t = if on(&p, data::BYPASS) { 1.0 } else { 0.0 };
        let silent_in = in_peak < 1e-6;
        let can_rest = !on(&p, data::FREEZE) && on(&p, data::THRU);
        let bypassed = bypass_t == 1.0 && self.bypass >= 0.999;
        if bypassed || (silent_in && can_rest && self.quiet.resting()) {
            if bypassed {
                self.bypass = 1.0;
                let mut peak = 0.0f32;
                for ch in 0..2 {
                    for i in 0..n {
                        let y = input.map_or(0.0, |b| soft_limit(b[ch][i]));
                        outputs[0][ch][i] = y;
                        peak = peak.max(y.abs());
                    }
                }
                p.meter.fetch_max(peak);
                self.level = peak;
            } else {
                // Gone quiet: nothing to compute until input returns.
                silence(outputs, n);
                self.level = 0.0;
            }
            if publish {
                self.publish(&p);
            }
            return;
        }

        // Into the chain: IN, and INPUT THRU.
        let (in_t, out_t, wet_t) = (self.vals[IN], self.vals[OUT], self.vals[WET]);
        let thru_t = if on(&p, data::THRU) { 1.0 } else { 0.0 };
        let mut x: Buf = [[0.0; BLOCK]; 2];
        for i in 0..n {
            approach(&mut self.in_gain, in_t, ctx.smooth);
            approach(&mut self.thru, thru_t, ctx.smooth);
            let g = self.in_gain * self.thru;
            if let Some(b) = input {
                x[0][i] = b[0][i] * g;
                x[1][i] = b[1][i] * g;
            }
        }
        let dry = x;

        let grain_settings = self.grain_settings(&clock);
        let gate_settings = self.gate_settings();
        let repeat_settings = self.repeat_settings(&clock);
        // SPACE's length, or with SYNC a note value at the tempo.
        let length = if on(&p, data::SPACE_SYNC) {
            let beats = DIVS[pick(
                PARAMS[SPACE_LENGTH].to_norm(self.vals[SPACE_LENGTH]),
                SPACE_DIVS,
            )]
            .1;
            (beats * clock.seconds_per_beat).clamp(0.3, 40.0) as f32
        } else {
            self.vals[SPACE_LENGTH]
        };
        let space_mix = self.vals[SPACE_MIX];
        for section in routing(&p) {
            let target = on(&p, data::SECTION_ON + section);
            if !target && self.fade[section] < 1e-4 {
                self.fade[section] = 0.0;
                continue;
            }
            let mut y = x;
            match section {
                GRAINS => self.grains.process(&mut y, n, &grain_settings, &clock),
                GATE => self.gate.process(&mut y, n, &gate_settings, &clock),
                REPEAT => self.repeat.process(&mut y, n, &repeat_settings, &clock),
                _ => self.space.process(&mut y, n, length, space_mix),
            }
            let t = if target { 1.0 } else { 0.0 };
            let f = &mut self.fade[section];
            for i in 0..n {
                approach(f, t, ctx.smooth);
                for ch in 0..2 {
                    x[ch][i] += *f * (y[ch][i] - x[ch][i]);
                }
            }
        }

        // Out: WET, OUT, the safety limiter, bypass.
        let limiter = on(&p, data::LIMITER);
        let mut peak = 0.0f32;
        let mut inner = 0.0f32;
        for i in 0..n {
            approach(&mut self.wet, wet_t, ctx.smooth);
            approach(&mut self.out_gain, out_t, ctx.smooth);
            approach(&mut self.bypass, bypass_t, ctx.smooth);
            inner = inner.max(x[0][i].abs()).max(x[1][i].abs());
            let mut l = (dry[0][i] + self.wet * (x[0][i] - dry[0][i])) * self.out_gain;
            let mut r = (dry[1][i] + self.wet * (x[1][i] - dry[1][i])) * self.out_gain;
            if limiter {
                let a = l.abs().max(r.abs());
                if a * self.limit > 0.99 {
                    self.limit = 0.99 / a;
                } else {
                    self.limit += (1.0 - self.limit) * self.limit_release;
                }
                l *= self.limit;
                r *= self.limit;
            }
            let (raw_l, raw_r) = input.map_or((0.0, 0.0), |b| (b[0][i], b[1][i]));
            let yl = soft_limit(l + (raw_l - l) * self.bypass);
            let yr = soft_limit(r + (raw_r - r) * self.bypass);
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak = peak.max(yl.abs()).max(yr.abs());
        }
        self.quiet.update(silent_in && can_rest, inner, n);
        p.meter.fetch_max(peak);
        self.level = peak.max(self.level * 0.97);
        if publish {
            self.publish(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_modes_snap_to_their_intervals() {
        assert_eq!(quantize(6.4, 0), 6.4);
        assert_eq!(quantize(6.4, 1), 6.0);
        assert_eq!(quantize(17.0, 2), 12.0);
        assert_eq!(quantize(6.4, 3), 5.0);
        assert_eq!(quantize(6.4, 4), 7.0);
        assert_eq!(quantize(9.0, 5), 7.0);
        assert_eq!(quantize(-4.0, 5), -5.0);
    }

    #[test]
    fn every_parameter_has_a_name_and_a_readout() {
        let p = ModuleParams::new(super::super::ModuleKind::Frakture);
        let mut names = std::collections::HashSet::new();
        for (i, spec) in PARAMS.iter().enumerate() {
            assert!(names.insert(spec.name), "{} twice", spec.name);
            assert!(!label(i).is_empty());
            for v in [spec.min, spec.default, spec.max] {
                assert!(!format(&p, i, v).is_empty());
            }
        }
        assert_eq!(routing(&p), [GRAINS, GATE, REPEAT, SPACE]);
        p.data[data::ROUTING].set(3.0);
        assert_eq!(
            routing(&p),
            [GRAINS, GATE, REPEAT, SPACE],
            "a broken order falls back"
        );
    }
}
