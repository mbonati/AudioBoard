//! Magma: four hot, analog-flavoured filters after FabFilter Volcano 3.
//! Each filter is a low/high/band pass, bell, low/high shelf, notch or all
//! pass at 6–48 dB/octave, in one of eleven non-linear styles that drive,
//! saturate and (most of them) self-oscillate. Per filter: DRIVE, FREQ with
//! a stereo frequency pan, PEAK (resonance, or gain for bells and shelves),
//! a DELAY of up to 50 ms, and LEVEL/PAN. The filters run in series,
//! parallel or a mix (six routings), in stereo or split across L/R or M/S.
//!
//! Almost every knob can be modulated: two each of XLFO (16 steps), envelope
//! generator, envelope follower, MIDI source and XY controller, connected by
//! up to 16 slots (whose levels can be modulated too). Envelopes trigger on
//! the input, the SC input or MIDI notes on the MIDI input.
//!
//! Globally: input and output gain and pan, MIX, High Quality (2×
//! oversampling), auto-mute of self-oscillation when the input stops, and
//! bypass.

mod filter;
pub mod modulation;

use std::sync::Arc;

use super::oversample::Oversampler;
use super::{DelayLine, ModuleDef, ModuleParams, Quiet, approach, silence, soft_limit};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit, format_db, format_hz, format_seconds};
use filter::{Complex, Filter, Setting};
use modulation::{SOURCES, Sources};

pub use filter::{BAND_PASS, BELL, BELL_DB, HIGH_PASS, HIGH_SHELF, LOW_PASS, LOW_SHELF, NOTCH, SHAPES, SLOPES, STYLES, db, mul};

pub const FILTERS: usize = 4;

/// A filter's parameters, relative to its first (see [`fp`]).
pub mod filt {
    pub const DRIVE: usize = 0;
    pub const FREQ: usize = 1;
    /// Frequency pan: the left channel's cutoff down, the right's up.
    pub const FPAN: usize = 2;
    /// Resonance, or gain for bells and shelves.
    pub const PEAK: usize = 3;
    pub const DELAY: usize = 4;
    pub const LEVEL: usize = 5;
    pub const PAN: usize = 6;
    pub const COUNT: usize = 7;
}

pub const fn fp(f: usize, k: usize) -> usize {
    f * filt::COUNT + k
}

pub const IN_GAIN: usize = FILTERS * filt::COUNT;
pub const IN_PAN: usize = IN_GAIN + 1;
pub const OUT_GAIN: usize = IN_PAN + 1;
pub const OUT_PAN: usize = OUT_GAIN + 1;
pub const MIX: usize = OUT_PAN + 1;

/// XLFO parameters and settings.
pub mod xlfo {
    /// Free-running rate (Hz), or with SYNC the cycle's multiplier.
    pub const RATE: usize = 0;
    pub const OFFSET: usize = 1;
    pub const BALANCE: usize = 2;
    pub const GLIDE: usize = 3;
    pub const PHASE: usize = 4;
    pub const COUNT: usize = 5;
    /// In `ModuleParams::data`, relative to [`super::xlfo_data`].
    pub const STEPS: usize = 0;
    /// 0 free, else 1 + an index into [`super::SYNC_LENGTHS`].
    pub const SYNC: usize = 1;
    /// 0 free, 1 retriggered by notes, 2 by the first note only (legato).
    pub const MIDI_TRIG: usize = 2;
    pub const SNAP: usize = 3;
    /// The step selected in the panel.
    pub const SELECTED: usize = 4;
    /// MAX_STEPS × (value, random, curve, glide).
    pub const STEP0: usize = 5;
    pub const DATA_LEN: usize = STEP0 + super::MAX_STEPS * 4;
}

pub const XLFO0: usize = MIX + 1;
pub const fn xp(x: usize, k: usize) -> usize {
    XLFO0 + x * xlfo::COUNT + k
}

/// Envelope generator parameters.
pub mod eg {
    pub const THRESHOLD: usize = 0;
    pub const DELAY: usize = 1;
    pub const ATTACK: usize = 2;
    pub const DECAY: usize = 3;
    pub const SUSTAIN: usize = 4;
    pub const HOLD: usize = 5;
    pub const RELEASE: usize = 6;
    pub const A_SLOPE: usize = 7;
    pub const D_SLOPE: usize = 8;
    pub const R_SLOPE: usize = 9;
    pub const COUNT: usize = 10;
}

pub const EG0: usize = XLFO0 + 2 * xlfo::COUNT;
pub const fn egp(e: usize, k: usize) -> usize {
    EG0 + e * eg::COUNT + k
}

/// Envelope follower parameters.
pub mod ef {
    pub const ATTACK: usize = 0;
    pub const RELEASE: usize = 1;
    pub const COUNT: usize = 2;
}

pub const EF0: usize = EG0 + 2 * eg::COUNT;
pub const fn efp(f: usize, k: usize) -> usize {
    EF0 + f * ef::COUNT + k
}

/// XY controllers: X, Y each.
pub const XY0: usize = EF0 + 2 * ef::COUNT;
pub const SLOTS: usize = 16;
/// Each slot's level (−100…100% of the target's range).
pub const SLOT0: usize = XY0 + 4;
pub const PARAM_COUNT: usize = SLOT0 + SLOTS;

pub const MAX_STEPS: usize = 16;
/// XLFO synced cycle lengths: (label, bars).
pub const SYNC_LENGTHS: [(&str, f64); 11] = [
    ("16", 16.0),
    ("8", 8.0),
    ("4", 4.0),
    ("2", 2.0),
    ("1", 1.0),
    ("1/2", 0.5),
    ("1/4", 0.25),
    ("1/8", 0.125),
    ("1/16", 0.0625),
    ("1/32", 0.031_25),
    ("1/64", 0.015_625),
];

macro_rules! filter_params {
    ($n:literal, $freq:expr, $peak:expr) => {
        [
            ParamSpec::lin(concat!("F", $n, " DRIVE"), 0.0, 1.0, 0.25, Unit::Percent),
            ParamSpec::log(concat!("F", $n, " FREQ"), 10.0, 20_000.0, $freq, Unit::Hz),
            ParamSpec::lin(concat!("F", $n, " FREQ PAN"), -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
            ParamSpec::lin(concat!("F", $n, " PEAK"), -1.0, 1.0, $peak, Unit::Percent).bipolar(),
            ParamSpec::lin(concat!("F", $n, " DELAY"), 0.0, 0.05, 0.0, Unit::Seconds),
            ParamSpec::lin(concat!("F", $n, " LEVEL"), -36.0, 12.0, 0.0, Unit::Db),
            ParamSpec::lin(concat!("F", $n, " PAN"), -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
        ]
    };
}

macro_rules! xlfo_params {
    ($n:literal) => {
        [
            ParamSpec::log(concat!("XLFO", $n, " RATE"), 0.02, 500.0, 1.0, Unit::Hz),
            ParamSpec::log(concat!("XLFO", $n, " OFFSET"), 0.5, 2.0, 1.0, Unit::Q),
            ParamSpec::lin(concat!("XLFO", $n, " BALANCE"), -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
            ParamSpec::lin(concat!("XLFO", $n, " GLIDE"), -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
            ParamSpec::lin(concat!("XLFO", $n, " PHASE"), 0.0, 1.0, 0.0, Unit::Percent),
        ]
    };
}

macro_rules! eg_params {
    ($n:literal) => {
        [
            ParamSpec::lin(concat!("EG", $n, " THRESHOLD"), -60.0, 0.0, -30.0, Unit::Db),
            ParamSpec::log(concat!("EG", $n, " DELAY"), 0.0005, 10.0, 0.0005, Unit::Seconds),
            ParamSpec::log(concat!("EG", $n, " ATTACK"), 0.0005, 10.0, 0.005, Unit::Seconds),
            ParamSpec::log(concat!("EG", $n, " DECAY"), 0.001, 20.0, 0.3, Unit::Seconds),
            ParamSpec::lin(concat!("EG", $n, " SUSTAIN"), 0.0, 1.0, 0.5, Unit::Percent),
            ParamSpec::log(concat!("EG", $n, " HOLD"), 0.0005, 10.0, 0.0005, Unit::Seconds),
            ParamSpec::log(concat!("EG", $n, " RELEASE"), 0.001, 20.0, 0.4, Unit::Seconds),
            ParamSpec::lin(concat!("EG", $n, " A SLOPE"), -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
            ParamSpec::lin(concat!("EG", $n, " D SLOPE"), -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
            ParamSpec::lin(concat!("EG", $n, " R SLOPE"), -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
        ]
    };
}

const PARAMS: [ParamSpec; PARAM_COUNT] = {
    const FILTER_PARAMS: [[ParamSpec; filt::COUNT]; FILTERS] = [
        filter_params!("1", 1500.0, 0.4),
        filter_params!("2", 200.0, 0.0),
        filter_params!("3", 1000.0, 0.3),
        filter_params!("4", 4000.0, 0.0),
    ];
    const XLFO_PARAMS: [[ParamSpec; xlfo::COUNT]; 2] = [xlfo_params!("1"), xlfo_params!("2")];
    const EG_PARAMS: [[ParamSpec; eg::COUNT]; 2] = [eg_params!("1"), eg_params!("2")];
    const FILL: ParamSpec = ParamSpec::lin("", 0.0, 1.0, 0.0, Unit::Percent);
    let mut out = [FILL; PARAM_COUNT];
    let mut f = 0;
    while f < FILTERS {
        let mut k = 0;
        while k < filt::COUNT {
            out[fp(f, k)] = FILTER_PARAMS[f][k];
            k += 1;
        }
        f += 1;
    }
    out[IN_GAIN] = ParamSpec::lin("IN", -36.0, 36.0, 0.0, Unit::Db).bipolar();
    out[IN_PAN] = ParamSpec::lin("IN PAN", -1.0, 1.0, 0.0, Unit::Percent).bipolar();
    out[OUT_GAIN] = ParamSpec::lin("OUT", -36.0, 36.0, 0.0, Unit::Db).bipolar();
    out[OUT_PAN] = ParamSpec::lin("OUT PAN", -1.0, 1.0, 0.0, Unit::Percent).bipolar();
    out[MIX] = ParamSpec::lin("MIX", 0.0, 1.0, 1.0, Unit::Percent);
    let mut i = 0;
    while i < 2 {
        let mut k = 0;
        while k < xlfo::COUNT {
            out[xp(i, k)] = XLFO_PARAMS[i][k];
            k += 1;
        }
        let mut k = 0;
        while k < eg::COUNT {
            out[egp(i, k)] = EG_PARAMS[i][k];
            k += 1;
        }
        i += 1;
    }
    out[efp(0, ef::ATTACK)] = ParamSpec::log("EF1 ATTACK", 0.0005, 1.0, 0.01, Unit::Seconds);
    out[efp(0, ef::RELEASE)] = ParamSpec::log("EF1 RELEASE", 0.005, 5.0, 0.2, Unit::Seconds);
    out[efp(1, ef::ATTACK)] = ParamSpec::log("EF2 ATTACK", 0.0005, 1.0, 0.01, Unit::Seconds);
    out[efp(1, ef::RELEASE)] = ParamSpec::log("EF2 RELEASE", 0.005, 5.0, 0.2, Unit::Seconds);
    out[XY0] = ParamSpec::lin("XY1 X", -1.0, 1.0, 0.0, Unit::Percent).bipolar();
    out[XY0 + 1] = ParamSpec::lin("XY1 Y", -1.0, 1.0, 0.0, Unit::Percent).bipolar();
    out[XY0 + 2] = ParamSpec::lin("XY2 X", -1.0, 1.0, 0.0, Unit::Percent).bipolar();
    out[XY0 + 3] = ParamSpec::lin("XY2 Y", -1.0, 1.0, 0.0, Unit::Percent).bipolar();
    const SLOT_NAMES: [&str; SLOTS] = [
        "SLOT1 LEVEL",
        "SLOT2 LEVEL",
        "SLOT3 LEVEL",
        "SLOT4 LEVEL",
        "SLOT5 LEVEL",
        "SLOT6 LEVEL",
        "SLOT7 LEVEL",
        "SLOT8 LEVEL",
        "SLOT9 LEVEL",
        "SLOT10 LEVEL",
        "SLOT11 LEVEL",
        "SLOT12 LEVEL",
        "SLOT13 LEVEL",
        "SLOT14 LEVEL",
        "SLOT15 LEVEL",
        "SLOT16 LEVEL",
    ];
    let mut s = 0;
    while s < SLOTS {
        out[SLOT0 + s] = ParamSpec::lin(SLOT_NAMES[s], -1.0, 1.0, 0.5, Unit::Percent).bipolar();
        s += 1;
    }
    out
};

/// Routings: (diagram, description); and what each does, as steps run in
/// series, each a set of parallel chains of filters in series.
pub const ROUTINGS: [(&str, &str); 6] = [
    ("1→2→3→4", "Serial: one filter after the other"),
    ("1|2|3|4", "Parallel: all filters side by side"),
    ("1→2 | 3→4", "Two serial pairs, in parallel"),
    ("1|2 → 3|4", "Two parallel pairs, in series"),
    ("1|2 → 3→4", "A parallel pair, then two in series"),
    ("1→2 → 3|4", "Two in series, then a parallel pair"),
];
type Chain = &'static [usize];
const ROUTES: [&[&[Chain]]; 6] = [
    &[&[&[0, 1, 2, 3]]],
    &[&[&[0], &[1], &[2], &[3]]],
    &[&[&[0, 1], &[2, 3]]],
    &[&[&[0], &[1]], &[&[2], &[3]]],
    &[&[&[0], &[1]], &[&[2, 3]]],
    &[&[&[0, 1]], &[&[2], &[3]]],
];
/// Stereo, or filters 1–2 on L (or mid) and 3–4 on R (or side).
pub const STEREO: usize = 0;
pub const LEFT_RIGHT: usize = 1;
pub const MID_SIDE: usize = 2;
pub const ROUTING_MODES: [(&str, &str); 3] = [
    ("ST", "Stereo: every filter on both channels"),
    ("L/R", "Filters 1–2 on the left, 3–4 on the right"),
    ("M/S", "Filters 1–2 on the mid, 3–4 on the side"),
];

/// What `ModuleParams::data` holds besides the knobs.
pub mod data {
    use super::{FILTERS, SLOTS, xlfo};
    pub const FILTER_ON: usize = 0;
    pub const SHAPE: usize = FILTER_ON + FILTERS;
    pub const SLOPE: usize = SHAPE + FILTERS;
    pub const STYLE: usize = SLOPE + FILTERS;
    pub const ROUTING: usize = STYLE + FILTERS;
    pub const ROUTING_MODE: usize = ROUTING + 1;
    pub const HQ: usize = ROUTING_MODE + 1;
    pub const AUTO_MUTE: usize = HQ + 1;
    pub const BYPASS: usize = AUTO_MUTE + 1;
    /// Panel state: the filter shown, piano and analyzer on, the source
    /// shown, the slots' page.
    pub const SELECTED: usize = BYPASS + 1;
    pub const PIANO: usize = SELECTED + 1;
    pub const ANALYZER: usize = PIANO + 1;
    pub const SOURCE_SHOWN: usize = ANALYZER + 1;
    pub const SLOT_PAGE: usize = SOURCE_SHOWN + 1;
    /// Two XLFOs (see [`xlfo`]).
    pub const XLFO: usize = SLOT_PAGE + 1;
    /// Two EGs: trigger (input, SC, MIDI), range (normal, neutral sustain).
    pub const EG: usize = XLFO + 2 * xlfo::DATA_LEN;
    /// Two EFs: trigger (input, SC), mode (envelope, transient).
    pub const EF: usize = EG + 4;
    /// Two MIDI sources: input, controller number, curve.
    pub const MIDI: usize = EF + 4;
    /// Two XY controllers: mode (XY, slider), bipolar.
    pub const XY: usize = MIDI + 6;
    /// Each slot's source (see [`super::modulation::SOURCE_NAMES`]) and
    /// target (0 none, else a parameter + 1).
    pub const SLOT_SOURCE: usize = XY + 4;
    pub const SLOT_TARGET: usize = SLOT_SOURCE + SLOTS;
    pub const LEN: usize = SLOT_TARGET + SLOTS;
}

pub const fn xlfo_data(x: usize) -> usize {
    data::XLFO + x * xlfo::DATA_LEN
}

const DATA: [f32; data::LEN] = {
    let mut d = [0.0; data::LEN];
    d[data::FILTER_ON] = 1.0;
    let shapes = [LOW_PASS, HIGH_PASS, BELL, BAND_PASS];
    let mut f = 0;
    while f < FILTERS {
        d[data::SHAPE + f] = shapes[f] as f32;
        d[data::SLOPE + f] = if f == 0 { 2.0 } else { 1.0 };
        f += 1;
    }
    d[data::AUTO_MUTE] = 1.0;
    d[data::ANALYZER] = 1.0;
    let mut x = 0;
    while x < 2 {
        let at = xlfo_data(x);
        d[at + xlfo::STEPS] = 2.0;
        // Two steps gliding along sine curves: a sine wave.
        d[at + xlfo::STEP0] = 1.0;
        d[at + xlfo::STEP0 + 2] = 3.0;
        d[at + xlfo::STEP0 + 3] = 1.0;
        d[at + xlfo::STEP0 + 4] = -1.0;
        d[at + xlfo::STEP0 + 6] = 3.0;
        d[at + xlfo::STEP0 + 7] = 1.0;
        x += 1;
    }
    d[data::MIDI + 1] = 74.0;
    d[data::MIDI + 4] = 1.0;
    d[data::XY + 1] = 1.0;
    d[data::XY + 3] = 1.0;
    d
};

pub const RTA_LEN: usize = 4096;

/// Layout of `ModuleParams::viz`.
pub mod viz {
    use super::{PARAM_COUNT, RTA_LEN, modulation::SOURCES};
    /// Live knob positions of modulated parameters (−1: not modulated).
    pub const EFFECTIVE: usize = 0;
    /// Each source's output.
    pub const SOURCES_AT: usize = EFFECTIVE + PARAM_COUNT;
    /// Trigger levels (dB) of EG 1, EG 2, EF 1, EF 2.
    pub const TRIGGER: usize = SOURCES_AT + SOURCES;
    pub const XLFO_PHASE: usize = TRIGGER + 4;
    pub const EG_STAGE: usize = XLFO_PHASE + 2;
    /// From the panel: the source whose side chain is auditioned (EG 1,
    /// EG 2, EF 1, EF 2 as 0…3), else −1.
    pub const AUDITION: usize = EG_STAGE + 2;
    pub const SR: usize = AUDITION + 1;
    pub const OUT_L: usize = SR + 1;
    pub const OUT_R: usize = OUT_L + 1;
    /// Auto-mute is holding the output.
    pub const MUTED: usize = OUT_R + 1;
    pub const RTA_WRITE: usize = MUTED + 1;
    pub const RTA: usize = RTA_WRITE + 1;
    pub const LEN: usize = RTA + RTA_LEN;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Magma",
    prefix: "MAGMA",
    hint: "4 hot filters · modulation",
    inputs: &["IN", "SC"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: true,
    midi_out: false,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

pub fn on(p: &ModuleParams, i: usize) -> bool {
    p.data[i].get() > 0.5
}

pub fn choice(p: &ModuleParams, i: usize, count: usize) -> usize {
    (p.data[i].get().round().max(0.0) as usize).min(count - 1)
}

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// A slot: (source, target parameter), if it does something.
pub fn slot(p: &ModuleParams, s: usize) -> Option<(usize, usize)> {
    let source = p.data[data::SLOT_SOURCE + s].get().round() as i32;
    let target = p.data[data::SLOT_TARGET + s].get().round() as i32;
    (source >= 1 && source as usize <= SOURCES && target >= 1 && (target as usize) <= PARAM_COUNT)
        .then(|| (source as usize, target as usize - 1))
}

/// A slot's raw settings (source, target + 1), even when incomplete.
pub fn slot_raw(p: &ModuleParams, s: usize) -> (usize, usize) {
    (
        (p.data[data::SLOT_SOURCE + s].get().round().max(0.0) as usize).min(SOURCES),
        (p.data[data::SLOT_TARGET + s].get().round().max(0.0) as usize).min(PARAM_COUNT),
    )
}

pub fn set_slot(p: &ModuleParams, s: usize, source: usize, target: usize) {
    p.data[data::SLOT_SOURCE + s].set(source as f32);
    p.data[data::SLOT_TARGET + s].set(target as f32);
}

/// A new slot from `source` to parameter `param`; None when all are used.
pub fn add_slot(p: &ModuleParams, source: usize, param: usize) -> Option<usize> {
    let free = (0..SLOTS).find(|&s| slot_raw(p, s) == (0, 0))?;
    set_slot(p, free, source, param + 1);
    // Frequencies span 11 octaves: start with a musical ±2.
    let level = if param < IN_GAIN && param % filt::COUNT == filt::FREQ { 0.18 } else { 0.5 };
    p.set(SLOT0 + free, level);
    Some(free)
}

// ---------------------------------------------------------------------------
// Names and readouts
// ---------------------------------------------------------------------------

pub fn label(i: usize) -> &'static str {
    if i < IN_GAIN {
        return ["DRIVE", "FREQ", "F.PAN", "PEAK", "DELAY", "LEVEL", "PAN"][i % filt::COUNT];
    }
    if (XLFO0..EG0).contains(&i) {
        return ["RATE", "OFFSET", "BALANCE", "GLIDE", "PHASE"][(i - XLFO0) % xlfo::COUNT];
    }
    if (EG0..EF0).contains(&i) {
        return ["THRESH", "DELAY", "ATTACK", "DECAY", "SUSTAIN", "HOLD", "RELEASE", "A.SLOPE", "D.SLOPE", "R.SLOPE"]
            [(i - EG0) % eg::COUNT];
    }
    if (EF0..XY0).contains(&i) {
        return ["ATTACK", "RELEASE"][(i - EF0) % ef::COUNT];
    }
    if (XY0..SLOT0).contains(&i) {
        return ["X", "Y"][(i - XY0) % 2];
    }
    match i {
        IN_GAIN => "IN",
        IN_PAN => "IN PAN",
        OUT_GAIN => "OUT",
        OUT_PAN => "OUT PAN",
        MIX => "MIX",
        _ => "LEVEL",
    }
}

/// "Filter 2 · Freq", "XLFO 1 · Rate", "Slot 3 · Level".
pub fn long_name(i: usize) -> String {
    let group = if i < IN_GAIN {
        format!("Filter {}", i / filt::COUNT + 1)
    } else if i < XLFO0 {
        "Global".to_owned()
    } else if i < EG0 {
        format!("XLFO {}", (i - XLFO0) / xlfo::COUNT + 1)
    } else if i < EF0 {
        format!("EG {}", (i - EG0) / eg::COUNT + 1)
    } else if i < XY0 {
        format!("EF {}", (i - EF0) / ef::COUNT + 1)
    } else if i < SLOT0 {
        format!("XY {}", (i - XY0) / 2 + 1)
    } else {
        format!("Slot {}", i - SLOT0 + 1)
    };
    let l = label(i);
    let mut name = l[..1].to_owned();
    name.push_str(&l[1..].to_lowercase());
    format!("{group} · {name}")
}

fn pan_label(v: f32) -> String {
    if v.abs() < 0.005 {
        "C".to_owned()
    } else if v < 0.0 {
        format!("L{:.0}", -v * 100.0)
    } else {
        format!("R{:.0}", v * 100.0)
    }
}

/// A parameter's value as the panel shows it.
pub fn format(p: &ModuleParams, i: usize, v: f32) -> String {
    if i < IN_GAIN {
        let f = i / filt::COUNT;
        return match i % filt::COUNT {
            filt::DRIVE => format!("{:.0}%", v * 100.0),
            filt::FREQ => format_hz(v),
            filt::FPAN => format!("{:+.1} oct", v * 2.0),
            filt::PEAK if matches!(choice(p, data::SHAPE + f, 8), BELL | LOW_SHELF | HIGH_SHELF) => {
                format!("{:+.1} dB", v * BELL_DB)
            }
            filt::PEAK => format!("{:.0}%", (v + 1.0) * 50.0),
            filt::DELAY => format_seconds(v),
            filt::LEVEL => format_db(v),
            _ => pan_label(v),
        };
    }
    match i {
        IN_GAIN | OUT_GAIN => format!("{v:+.1} dB"),
        IN_PAN | OUT_PAN => pan_label(v),
        _ if (XLFO0..EG0).contains(&i) => match (i - XLFO0) % xlfo::COUNT {
            xlfo::RATE => format_hz(v),
            xlfo::OFFSET => format!("×{v:.2}"),
            xlfo::PHASE => format!("{:.0}%", v * 100.0),
            _ => format!("{:+.0}%", v * 100.0),
        },
        _ if (EG0..EF0).contains(&i) => match (i - EG0) % eg::COUNT {
            eg::THRESHOLD => format!("{v:.1} dB"),
            eg::SUSTAIN => format!("{:.0}%", v * 100.0),
            eg::A_SLOPE | eg::D_SLOPE | eg::R_SLOPE => format!("{:+.0}%", v * 100.0),
            _ => format_seconds(v),
        },
        _ if (EF0..XY0).contains(&i) => format_seconds(v),
        _ => format!("{:+.0}%", v * 100.0),
    }
}

// ---------------------------------------------------------------------------
// Settings and responses (shared with the panel)
// ---------------------------------------------------------------------------

/// The transport for one block (see Frakture's: the same idea).
#[derive(Clone, Copy)]
pub struct Clock {
    pub beat0: f64,
    pub seconds_per_beat: f64,
    pub playing: bool,
}

/// Filter `f` as `vals` set it: per-channel frequencies spread by the
/// frequency pan (stereo routing only).
pub fn setting(p: &ModuleParams, vals: &[f32; PARAM_COUNT], f: usize) -> Setting {
    let stereo = choice(p, data::ROUTING_MODE, 3) == STEREO;
    let fc = vals[fp(f, filt::FREQ)];
    let pan = if stereo { vals[fp(f, filt::FPAN)] } else { 0.0 };
    let spread = fastmath::exp2(2.0 * pan);
    let shape = choice(p, data::SHAPE + f, SHAPES.len());
    let mut slope = choice(p, data::SLOPE + f, SLOPES.len());
    if slope == 0 && matches!(shape, BAND_PASS | NOTCH) {
        slope = 1;
    }
    Setting {
        shape,
        slope,
        style: choice(p, data::STYLE + f, STYLES.len()),
        freq: [fc / spread, fc * spread],
        peak: vals[fp(f, filt::PEAK)].clamp(-1.0, 1.0),
    }
}

/// The knob values (no modulation).
pub fn base_values(p: &ModuleParams) -> [f32; PARAM_COUNT] {
    std::array::from_fn(|i| p.get(i))
}

/// Filter `f`'s own response at `f_hz` on channel `ch`, with its level and
/// delay.
pub fn filter_response(p: &ModuleParams, vals: &[f32; PARAM_COUNT], f: usize, ch: usize, f_hz: f32, sr: f32) -> Complex {
    let s = setting(p, vals, f);
    let h = filter::response(&s, s.freq[ch], f_hz, sr);
    let gain = 10f32.powf(vals[fp(f, filt::LEVEL)] / 20.0);
    let phase = -std::f32::consts::TAU * f_hz * vals[fp(f, filt::DELAY)];
    mul(h, (gain * phase.cos(), gain * phase.sin()))
}

/// The whole response at `f_hz` for channel `ch` (in L/R and M/S modes, 0
/// is the first pair's channel, 1 the second's), routing and all.
pub fn total_response(p: &ModuleParams, vals: &[f32; PARAM_COUNT], ch: usize, f_hz: f32, sr: f32) -> Complex {
    let enabled = |f: usize| on(p, data::FILTER_ON + f);
    let response = |f: usize| filter_response(p, vals, f, ch, f_hz, sr);
    let routing = choice(p, data::ROUTING, ROUTINGS.len());
    if choice(p, data::ROUTING_MODE, 3) != STEREO {
        let pair = if ch == 0 { [0, 1] } else { [2, 3] };
        let active: Vec<usize> = pair.into_iter().filter(|&f| enabled(f)).collect();
        if active.is_empty() {
            return (1.0, 0.0);
        }
        return if routing == 0 {
            active.iter().fold((1.0, 0.0), |h, &f| mul(h, response(f)))
        } else {
            active.iter().fold((0.0, 0.0), |h, &f| {
                let r = response(f);
                (h.0 + r.0, h.1 + r.1)
            })
        };
    }
    let mut h = (1.0, 0.0);
    for step in ROUTES[routing] {
        let mut sum = (0.0, 0.0);
        let mut any = false;
        for chain in *step {
            if chain.iter().any(|&f| enabled(f)) {
                any = true;
                let c = chain.iter().filter(|&&f| enabled(f)).fold((1.0, 0.0), |acc, &f| mul(acc, response(f)));
                sum = (sum.0 + c.0, sum.1 + c.1);
            }
        }
        if any {
            h = mul(h, sum);
        }
    }
    h
}

// ---------------------------------------------------------------------------
// Processor
// ---------------------------------------------------------------------------

/// Samples per modulation update.
const CHUNK: usize = 16;
/// The longest filter delay, in seconds.
const MAX_DELAY: f32 = 0.05;

/// A filter with its output stage (delay, level, pan).
struct FilterSlot {
    filter: Filter,
    delay: [DelayLine; 2],
    drive: f32,
    delay_s: f32,
    level: f32,
    pan: f32,
}

pub struct Magma {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    sr: f32,
    slots: [FilterSlot; FILTERS],
    sources: Sources,
    /// This chunk's parameter values, modulation applied.
    vals: [f32; PARAM_COUNT],
    over: Oversampler,
    hq: bool,
    in_gain: f32,
    out_gain: f32,
    in_pan: f32,
    out_pan: f32,
    mix: f32,
    bypass: f32,
    mute: f32,
    silent_for: f32,
    clock: f64,
    quiet: Quiet,
    rta_write: usize,
    /// Some slot was working last time.
    modulated: bool,
}

/// dB to a gain factor.
#[inline]
fn gain(db: f32) -> f32 {
    fastmath::exp2(db * 0.166_096_4)
}

/// Balance gains for a pan in −1…1.
#[inline]
fn balance(pan: f32) -> (f32, f32) {
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}

impl Magma {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        params.viz[viz::SR].set(sample_rate);
        params.viz[viz::AUDITION].set(-1.0);
        // Nothing is modulated until the slots say so.
        for i in 0..PARAM_COUNT {
            params.viz[viz::EFFECTIVE + i].set(-1.0);
        }
        let delay_len = (MAX_DELAY * sample_rate * 2.0) as usize + 8;
        let vals = base_values(&params);
        Self {
            midi_cursor: midi.stream_head(),
            midi,
            slots: std::array::from_fn(|f| FilterSlot {
                filter: Filter::default(),
                delay: [DelayLine::new(delay_len), DelayLine::new(delay_len)],
                drive: 1.0,
                delay_s: vals[fp(f, filt::DELAY)],
                level: gain(vals[fp(f, filt::LEVEL)]),
                pan: vals[fp(f, filt::PAN)],
            }),
            sources: Sources::new(),
            over: Oversampler::new(),
            hq: false,
            in_gain: gain(vals[IN_GAIN]),
            out_gain: gain(vals[OUT_GAIN]),
            in_pan: vals[IN_PAN],
            out_pan: vals[OUT_PAN],
            mix: vals[MIX],
            bypass: if on(&params, data::BYPASS) { 1.0 } else { 0.0 },
            mute: 1.0,
            silent_for: 0.0,
            clock: 0.0,
            quiet: Quiet::new((sample_rate * 0.3) as usize),
            rta_write: 0,
            modulated: true,
            vals,
            params,
            sr: sample_rate,
        }
    }

    /// The knobs, plus whatever the slots add (in knob positions). Slots
    /// aimed at other slots' levels are worked out first. `publish` shows
    /// the live values to the panel.
    fn modulate(&mut self, slots: &[Option<(usize, usize)>; SLOTS], publish: bool) {
        let p = &*self.params;
        if slots.iter().all(Option::is_none) {
            for (i, v) in self.vals.iter_mut().enumerate() {
                *v = p.get(i);
            }
            if publish && self.modulated {
                for i in 0..PARAM_COUNT {
                    p.viz[viz::EFFECTIVE + i].set(-1.0);
                }
            }
            self.modulated = false;
            return;
        }
        self.modulated = true;
        let src = &self.sources.out;
        let mut delta = [0.0f32; PARAM_COUNT];
        let mut touched = [false; PARAM_COUNT];
        for pass in 0..2 {
            for (s, sl) in slots.iter().enumerate() {
                let Some((source, target)) = *sl else { continue };
                let on_slot = target >= SLOT0;
                if (pass == 0) != on_slot {
                    continue;
                }
                let level = if pass == 0 { p.get(SLOT0 + s) } else { self.vals[SLOT0 + s] };
                delta[target] += level * src[source - 1];
                touched[target] = true;
            }
            for i in 0..PARAM_COUNT {
                if (i >= SLOT0) != (pass == 0) {
                    continue;
                }
                if touched[i] {
                    let spec = &PARAMS[i];
                    let n = (spec.to_norm(p.get(i)) + delta[i]).clamp(0.0, 1.0);
                    self.vals[i] = spec.denorm(n);
                    if publish {
                        p.viz[viz::EFFECTIVE + i].set(n);
                    }
                } else {
                    self.vals[i] = p.get(i);
                    if publish {
                        p.viz[viz::EFFECTIVE + i].set(-1.0);
                    }
                }
            }
        }
    }

    /// New coefficients for every enabled filter (`rate`: processing rate).
    fn tune(&mut self, rate: f32) {
        let p = &*self.params;
        for (f, slot) in self.slots.iter_mut().enumerate() {
            if !on(p, data::FILTER_ON + f) {
                continue;
            }
            slot.filter.update(&setting(p, &self.vals, f), rate);
            slot.drive = gain(self.vals[fp(f, filt::DRIVE)] * 24.0);
        }
    }

    fn reset_filters(&mut self) {
        for slot in &mut self.slots {
            slot.filter.reset();
        }
        self.over.reset();
    }
}

/// Filter `f` on a stereo sample, with its delay, level and pan.
#[inline]
fn run_stereo(slot: &mut FilterSlot, x: (f32, f32), t: &Targets, smooth: f32, rate: f32) -> (f32, f32) {
    approach(&mut slot.delay_s, t.delay, smooth);
    approach(&mut slot.level, t.level, smooth);
    approach(&mut slot.pan, t.pan, smooth);
    let d = slot.delay_s * rate;
    let mut y = [slot.filter.process(0, x.0, slot.drive), slot.filter.process(1, x.1, slot.drive)];
    for (ch, v) in y.iter_mut().enumerate() {
        slot.delay[ch].push(*v);
        *v = slot.delay[ch].tap(d);
    }
    let (gl, gr) = balance(slot.pan);
    (y[0] * slot.level * gl, y[1] * slot.level * gr)
}

/// Filter `f` on one channel (L/R and M/S modes): no pan.
#[inline]
fn run_mono(slot: &mut FilterSlot, x: f32, t: &Targets, smooth: f32, rate: f32) -> f32 {
    approach(&mut slot.delay_s, t.delay, smooth);
    approach(&mut slot.level, t.level, smooth);
    let y = slot.filter.process(0, x, slot.drive);
    slot.delay[0].push(y);
    slot.delay[0].tap(slot.delay_s * rate) * slot.level
}

/// A filter's output-stage targets for the chunk.
#[derive(Clone, Copy, Default)]
struct Targets {
    delay: f32,
    level: f32,
    pan: f32,
}

impl Magma {
    /// The filters on one sample at the processing rate.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn chain(
        &mut self,
        x: (f32, f32),
        enabled: &[bool; FILTERS],
        targets: &[Targets; FILTERS],
        (routing, mode): (usize, usize),
        smooth: f32,
        rate: f32,
    ) -> (f32, f32) {
        let slots = &mut self.slots;
        if mode != STEREO {
            let (a, b) = if mode == LEFT_RIGHT { x } else { ((x.0 + x.1) * 0.5, (x.0 - x.1) * 0.5) };
            let mut half = |pair: [usize; 2], v: f32| {
                let (f0, f1) = (pair[0], pair[1]);
                match (enabled[f0], enabled[f1]) {
                    (false, false) => v,
                    (true, false) => run_mono(&mut slots[f0], v, &targets[f0], smooth, rate),
                    (false, true) => run_mono(&mut slots[f1], v, &targets[f1], smooth, rate),
                    (true, true) if routing == 0 => {
                        let y = run_mono(&mut slots[f0], v, &targets[f0], smooth, rate);
                        run_mono(&mut slots[f1], y, &targets[f1], smooth, rate)
                    }
                    (true, true) => {
                        run_mono(&mut slots[f0], v, &targets[f0], smooth, rate)
                            + run_mono(&mut slots[f1], v, &targets[f1], smooth, rate)
                    }
                }
            };
            let a = half([0, 1], a);
            let b = half([2, 3], b);
            return if mode == LEFT_RIGHT { (a, b) } else { (a + b, a - b) };
        }
        let mut v = x;
        for step in ROUTES[routing] {
            let mut sum = (0.0, 0.0);
            let mut any = false;
            for chain in *step {
                if !chain.iter().any(|&f| enabled[f]) {
                    continue;
                }
                any = true;
                let mut c = v;
                for &f in chain.iter() {
                    if enabled[f] {
                        c = run_stereo(&mut slots[f], c, &targets[f], smooth, rate);
                    }
                }
                sum = (sum.0 + c.0, sum.1 + c.1);
            }
            if any {
                v = sum;
            }
        }
        v
    }
}

impl Processor for Magma {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let sr = self.sr;
        let input = inputs.get(0);
        let side = inputs.get(1);
        let bps = ctx.beats_per_sample.max(1e-9);
        let beat0 = if ctx.playing { ctx.beat } else { self.clock };
        self.clock = beat0 + bps * n as f64;
        let clock = Clock { beat0, seconds_per_beat: 1.0 / (bps * sr as f64), playing: ctx.playing };

        // MIDI: the cable if patched, else the keyboards.
        let cable: &[crate::audio::engine::MidiMsg] = if inputs.midi_connected { inputs.midi.events() } else { &[] };
        if !inputs.midi_connected {
            let sources = &mut self.sources;
            self.midi.read_stream(&mut self.midi_cursor, |msg| sources.midi.handle(msg));
        } else {
            self.midi_cursor = self.midi.stream_head();
        }

        let in_peak = input.map_or(0.0, |b| b.iter().flat_map(|ch| &ch[..n]).fold(0.0f32, |m, s| m.max(s.abs())));
        let silent_in = in_peak < 1e-5;
        self.silent_for = if silent_in { self.silent_for + n as f32 / sr } else { 0.0 };
        let auto_mute = on(&p, data::AUTO_MUTE) && self.silent_for > 0.1;
        let bypass_t = if on(&p, data::BYPASS) { 1.0 } else { 0.0 };
        let audition = p.viz[viz::AUDITION].get();

        let hq = on(&p, data::HQ);
        if hq != self.hq {
            self.hq = hq;
            self.reset_filters();
        }
        let rate = if hq { sr * 2.0 } else { sr };
        let enabled: [bool; FILTERS] = std::array::from_fn(|f| on(&p, data::FILTER_ON + f));
        let layout = (choice(&p, data::ROUTING, ROUTINGS.len()), choice(&p, data::ROUTING_MODE, 3));
        let analyzer = on(&p, data::ANALYZER);

        // Only the sources a slot uses (or the panel shows) are worked out.
        let slots: [Option<(usize, usize)>; SLOTS] = std::array::from_fn(|s| slot(&p, s));
        if silent_in && self.quiet.resting() && side.is_none() {
            // Resting: no sound to make, but the knobs' live values stay true.
            self.modulate(&slots, true);
            silence(outputs, n);
            return;
        }

        let mut used = [false; SOURCES];
        for &(source, _) in slots.iter().flatten() {
            used[source - 1] = true;
        }
        let shown = choice(&p, data::SOURCE_SHOWN, 10);
        let tab_sources: [&[usize]; 10] = [&[0], &[1], &[2], &[3], &[4], &[5], &[6], &[7], &[8, 9], &[10, 11]];
        for &s in tab_sources[shown] {
            used[s] = true;
        }

        let mut peak = [0.0f32; 2];
        let mut inner = 0.0f32;
        let mut start = 0;
        while start < n {
            let end = (start + CHUNK).min(n);
            for e in cable.iter().filter(|e| (e.frame as usize) >= start && (e.frame as usize) < end) {
                self.sources.midi.handle(e.data);
            }
            let chunk_peak = |b: Option<&Buf>| {
                b.map_or(0.0, |b| b.iter().flat_map(|ch| &ch[start..end]).fold(0.0f32, |m, s| m.max(s.abs())))
            };
            let (main, sc) = (chunk_peak(input), chunk_peak(side));
            let dt = (end - start) as f32 / sr;
            self.sources.update(&p, &self.vals, &clock, dt, main, sc, &used);
            self.modulate(&slots, end == n);
            self.tune(rate);
            let v = &self.vals;
            let targets: [Targets; FILTERS] = std::array::from_fn(|f| Targets {
                delay: v[fp(f, filt::DELAY)],
                level: gain(v[fp(f, filt::LEVEL)]),
                pan: v[fp(f, filt::PAN)],
            });
            let (in_g, out_g) = (gain(v[IN_GAIN]), gain(v[OUT_GAIN]));
            let (in_p, out_p, mix_t) = (v[IN_PAN], v[OUT_PAN], v[MIX]);
            let mute_t = if auto_mute { 0.0 } else { 1.0 };
            let smooth = if hq { 1.0 - (1.0 - ctx.smooth).sqrt() } else { ctx.smooth };

            for i in start..end {
                let (dry_l, dry_r) = input.map_or((0.0, 0.0), |b| (b[0][i], b[1][i]));
                approach(&mut self.in_gain, in_g, ctx.smooth);
                approach(&mut self.in_pan, in_p, ctx.smooth);
                approach(&mut self.out_gain, out_g, ctx.smooth);
                approach(&mut self.out_pan, out_p, ctx.smooth);
                approach(&mut self.mix, mix_t, ctx.smooth);
                approach(&mut self.bypass, bypass_t, ctx.smooth);
                approach(&mut self.mute, mute_t, ctx.smooth * 0.5);
                let (il, ir) = balance(self.in_pan);
                let x = (dry_l * self.in_gain * il, dry_r * self.in_gain * ir);
                let y = if hq {
                    let up = self.over.up([x.0, x.1]);
                    let a = self.chain((up[0][0], up[0][1]), &enabled, &targets, layout, smooth, rate);
                    let b = self.chain((up[1][0], up[1][1]), &enabled, &targets, layout, smooth, rate);
                    let d = self.over.down([[a.0, a.1], [b.0, b.1]]);
                    (d[0], d[1])
                } else {
                    self.chain(x, &enabled, &targets, layout, smooth, rate)
                };
                let (ol, or) = balance(self.out_pan);
                let wet = (y.0 * self.out_gain * ol, y.1 * self.out_gain * or);
                let mut out = (dry_l + (wet.0 - dry_l) * self.mix, dry_r + (wet.1 - dry_r) * self.mix);
                out = (out.0 * self.mute, out.1 * self.mute);
                if audition >= 0.0 {
                    // Listening to an envelope's side chain.
                    out = side.map_or((0.0, 0.0), |b| (b[0][i], b[1][i]));
                }
                let l = soft_limit(out.0 + (dry_l - out.0) * self.bypass);
                let r = soft_limit(out.1 + (dry_r - out.1) * self.bypass);
                outputs[0][0][i] = l;
                outputs[0][1][i] = r;
                peak[0] = peak[0].max(l.abs());
                peak[1] = peak[1].max(r.abs());
                inner = inner.max(wet.0.abs()).max(wet.1.abs());
                if analyzer {
                    p.viz[viz::RTA + self.rta_write].set((l + r) * 0.5);
                    self.rta_write = (self.rta_write + 1) % RTA_LEN;
                }
            }
            start = end;
        }
        if self.mute < 1e-4 && auto_mute {
            // Silenced: let the oscillating loops die for good.
            self.reset_filters();
        }

        let viz = &p.viz;
        for (k, v) in self.sources.out.iter().enumerate() {
            viz[viz::SOURCES_AT + k].set(*v);
        }
        for e in 0..2 {
            let level = self.sources.levels[usize::from(choice(&p, data::EG + e * 2, 3) == 1)];
            viz[viz::TRIGGER + e].set(crate::audio::params::gain_to_db(level));
            viz[viz::EG_STAGE + e].set(self.sources.egs[e].stage as f32);
            viz[viz::XLFO_PHASE + e].set(self.sources.xlfos[e].shown_phase);
        }
        for f in 0..2 {
            let level = self.sources.levels[usize::from(choice(&p, data::EF + f * 2, 2) == 1)];
            viz[viz::TRIGGER + 2 + f].set(crate::audio::params::gain_to_db(level));
        }
        viz[viz::OUT_L].fetch_max(peak[0]);
        viz[viz::OUT_R].fetch_max(peak[1]);
        viz[viz::MUTED].set(if auto_mute { 1.0 } else { 0.0 });
        if analyzer {
            viz[viz::RTA_WRITE].set(self.rta_write as f32);
        }
        self.quiet.update(silent_in, inner, n);
        p.meter.fetch_max(peak[0].max(peak[1]));
    }
}
