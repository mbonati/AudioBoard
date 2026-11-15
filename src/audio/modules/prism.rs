//! Prism: a 24-band equalizer after FabFilter Pro-Q 4.
//!
//! - **Bands.** Up to 24, each of ten shapes (bell, low and high shelf, low
//!   and high cut, notch, band pass, tilt shelf, flat tilt, all pass) with
//!   slopes from 6 to 96 dB/octave and a brickwall for the cuts, FREQ
//!   10 Hz–30 kHz, GAIN ±30 dB, Q 0.025–40, and a stereo placement each:
//!   stereo, left, right, mid or side ([`design`]).
//! - **Processing modes.** ZERO LATENCY: the bands as analog-matched
//!   digital filters, no latency. NATURAL PHASE: the same filters at twice
//!   the rate, where they match the analog ones in phase as well as in
//!   magnitude up to the top of the audible range (half a millisecond of
//!   latency, from the oversampling filters). LINEAR PHASE: the curve as
//!   zero-phase gains on overlapping FFT frames, with RESOLUTION setting the
//!   frame (and the latency) ([`spectral`]).
//! - **Dynamic EQ.** Bells, shelves and tilts can move their gain by up to
//!   RANGE (±30 dB) when their trigger crosses the THRESHOLD, automatic or
//!   set, with ATTACK and RELEASE around automatic times, triggered by the
//!   band's own range or a FREE range, from the input or the SC input
//!   ([`dynamics`]).
//! - **Spectral dynamics.** A dynamic band can be SPECTRAL: it measures
//!   each frequency in its range and changes only those that cross the
//!   threshold, through the spectral stage (so with its latency).
//! - **Output.** CHARACTER (clean, subtle, warm saturation), AUTO GAIN (an
//!   estimate from the curve), GAIN SCALE (all gains and ranges), OUTPUT,
//!   PAN (left/right or mid/side) and phase invert; a soft limit keeps the
//!   output under full scale, as on every node.
//! - **For the panel.** SOLO (what a band works on), AUDITION (a dynamic
//!   band's trigger), and rings of samples for the analyzer (before, after,
//!   side chain) and EQ Match.

pub mod design;
pub mod dynamics;
pub mod fft;
pub mod fit;
pub mod spectral;

use std::sync::Arc;

use design::{Analog, BAND_PASS, BELL, BRICKWALL, Digital, HIGH_CUT, HIGH_SHELF, LOW_CUT, LOW_SHELF, MAX_SECTIONS, NOTCH, SHAPES, Shape, State};
use dynamics::{Detector, Trigger};
use spectral::{GRID, SpecBand, Stage};

use super::equalizer::{Coeffs, Svf};
use super::oversample::Oversampler;
use super::{ModuleDef, ModuleParams, Quiet, silence, soft_limit};
use crate::audio::engine::{BLOCK, Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{ParamSpec, Unit};

pub const BANDS: usize = 24;

/// A band's knobs, relative to its first one (see [`bp`]).
pub mod band {
    pub const FREQ: usize = 0;
    pub const GAIN: usize = 1;
    pub const Q: usize = 2;
    pub const RANGE: usize = 3;
    pub const THRESHOLD: usize = 4;
    pub const ATTACK: usize = 5;
    pub const RELEASE: usize = 6;
    pub const SC_LO: usize = 7;
    pub const SC_HI: usize = 8;
    pub const DENSITY: usize = 9;
    pub const COUNT: usize = 10;
}

/// Index of band `b`'s knob `k`.
pub const fn bp(b: usize, k: usize) -> usize {
    b * band::COUNT + k
}

pub const OUTPUT: usize = BANDS * band::COUNT;
pub const PAN: usize = OUTPUT + 1;
pub const SCALE: usize = PAN + 1;
pub const PARAM_COUNT: usize = SCALE + 1;

pub const FREQ_RANGE: (f32, f32) = (10.0, 30_000.0);
pub const GAIN_RANGE: f32 = 30.0;
pub const Q_RANGE: (f32, f32) = (0.025, 40.0);

macro_rules! band_params {
    ($n:literal) => {
        [
            ParamSpec::log(concat!("B", $n, " FREQ"), FREQ_RANGE.0, FREQ_RANGE.1, 1000.0, Unit::Hz),
            ParamSpec::lin(concat!("B", $n, " GAIN"), -GAIN_RANGE, GAIN_RANGE, 0.0, Unit::Db).bipolar(),
            ParamSpec::log(concat!("B", $n, " Q"), Q_RANGE.0, Q_RANGE.1, 1.0, Unit::Q),
            ParamSpec::lin(concat!("B", $n, " RANGE"), -GAIN_RANGE, GAIN_RANGE, 0.0, Unit::Db).bipolar(),
            ParamSpec::lin(concat!("B", $n, " THRESHOLD"), -60.0, 0.0, -20.0, Unit::Db),
            ParamSpec::lin(concat!("B", $n, " ATTACK"), 0.0, 1.0, 0.5, Unit::Percent),
            ParamSpec::lin(concat!("B", $n, " RELEASE"), 0.0, 1.0, 0.5, Unit::Percent),
            ParamSpec::log(concat!("B", $n, " SC LO"), FREQ_RANGE.0, FREQ_RANGE.1, 20.0, Unit::Hz),
            ParamSpec::log(concat!("B", $n, " SC HI"), FREQ_RANGE.0, FREQ_RANGE.1, 20_000.0, Unit::Hz),
            ParamSpec::lin(concat!("B", $n, " DENSITY"), 0.0, 1.0, 0.5, Unit::Percent),
        ]
    };
}

const PARAMS: [ParamSpec; PARAM_COUNT] = {
    const BAND_PARAMS: [[ParamSpec; band::COUNT]; BANDS] = [
        band_params!("1"),
        band_params!("2"),
        band_params!("3"),
        band_params!("4"),
        band_params!("5"),
        band_params!("6"),
        band_params!("7"),
        band_params!("8"),
        band_params!("9"),
        band_params!("10"),
        band_params!("11"),
        band_params!("12"),
        band_params!("13"),
        band_params!("14"),
        band_params!("15"),
        band_params!("16"),
        band_params!("17"),
        band_params!("18"),
        band_params!("19"),
        band_params!("20"),
        band_params!("21"),
        band_params!("22"),
        band_params!("23"),
        band_params!("24"),
    ];
    let mut out = [ParamSpec::lin("OUTPUT", -36.0, 36.0, 0.0, Unit::Db).bipolar(); PARAM_COUNT];
    let mut b = 0;
    while b < BANDS {
        let mut k = 0;
        while k < band::COUNT {
            out[bp(b, k)] = BAND_PARAMS[b][k];
            k += 1;
        }
        b += 1;
    }
    out[PAN] = ParamSpec::lin("PAN", -1.0, 1.0, 0.0, Unit::Percent).bipolar();
    out[SCALE] = ParamSpec::lin("GAIN SCALE", 0.0, 2.0, 1.0, Unit::Percent);
    out
};

/// Stereo placements.
pub const STEREO: usize = 0;
pub const LEFT: usize = 1;
pub const RIGHT: usize = 2;
pub const MID: usize = 3;
pub const SIDE: usize = 4;
pub const PLACES: usize = 5;

/// Processing modes.
pub const ZERO_LATENCY: usize = 0;
pub const NATURAL_PHASE: usize = 1;
pub const LINEAR_PHASE: usize = 2;

/// Linear-phase resolutions: (name, frame at 44.1/48 kHz).
pub const RESOLUTIONS: [(&str, usize); 5] = [("LOW", 2048), ("MEDIUM", 4096), ("HIGH", 8192), ("VERY HIGH", 16_384), ("MAX", 32_768)];
/// Dynamics and spectral bands work up to this resolution.
pub const DYNAMIC_RESOLUTIONS: usize = 3;

pub const CHARACTERS: [&str; 3] = ["CLEAN", "SUBTLE", "WARM"];

/// Analyzer settings: range (dB), resolution (FFT points), speed (seconds
/// for the display to fall 20 dB), tilt (dB/octave around 1 kHz).
pub const AN_RANGES: [f32; 3] = [60.0, 90.0, 120.0];
pub const AN_RESOLUTIONS: [(&str, usize); 4] = [("LOW", 1024), ("MED", 2048), ("HIGH", 4096), ("MAX", 8192)];
pub const AN_SPEEDS: [(&str, f32); 5] = [("V.FAST", 0.15), ("FAST", 0.35), ("MED", 0.8), ("SLOW", 1.8), ("V.SLOW", 4.0)];
pub const AN_TILTS: [f32; 5] = [0.0, 1.5, 3.0, 4.5, 6.0];
/// Display ranges of the EQ scale (± dB).
pub const DISPLAY_RANGES: [f32; 4] = [3.0, 6.0, 12.0, 30.0];

/// What `ModuleParams::data` holds besides the knobs.
pub mod data {
    use super::BANDS;
    /// Per band: the band exists, is on (not bypassed), its shape, slope
    /// (index into `design::SLOPES`), placement, automatic threshold,
    /// external trigger, free trigger range, spectral, spectral tilt.
    pub const USED: usize = 0;
    pub const ON: usize = USED + BANDS;
    pub const SHAPE: usize = ON + BANDS;
    pub const SLOPE: usize = SHAPE + BANDS;
    pub const PLACE: usize = SLOPE + BANDS;
    pub const AUTO: usize = PLACE + BANDS;
    pub const SC_EXT: usize = AUTO + BANDS;
    pub const SC_FREE: usize = SC_EXT + BANDS;
    pub const SPECTRAL: usize = SC_FREE + BANDS;
    pub const SPEC_TILT: usize = SPECTRAL + BANDS;
    /// Processing mode, linear-phase resolution, character.
    pub const MODE: usize = SPEC_TILT + BANDS;
    pub const RESOLUTION: usize = MODE + 1;
    pub const CHARACTER: usize = RESOLUTION + 1;
    pub const AUTO_GAIN: usize = CHARACTER + 1;
    pub const INVERT: usize = AUTO_GAIN + 1;
    /// PAN balances mid against side (else left against right).
    pub const PAN_MS: usize = INVERT + 1;
    /// Gain-Q interaction on bells.
    pub const GAIN_Q: usize = PAN_MS + 1;
    /// The panel's selected bands, as a bit mask (24 bits fit an f32).
    pub const SELECTED: usize = GAIN_Q + 1;
    /// The soloed band and the auditioned trigger (−1: none).
    pub const SOLO: usize = SELECTED + 1;
    pub const AUDITION: usize = SOLO + 1;
    /// Analyzer: pre, post and SC spectra shown, range, resolution, speed,
    /// tilt, freeze, spectrum grab.
    pub const AN_PRE: usize = AUDITION + 1;
    pub const AN_POST: usize = AN_PRE + 1;
    pub const AN_SC: usize = AN_POST + 1;
    pub const AN_RANGE: usize = AN_SC + 1;
    pub const AN_RES: usize = AN_RANGE + 1;
    pub const AN_SPEED: usize = AN_RES + 1;
    pub const AN_TILT: usize = AN_SPEED + 1;
    pub const AN_FREEZE: usize = AN_TILT + 1;
    pub const AN_GRAB: usize = AN_FREEZE + 1;
    /// The EQ scale (index into `DISPLAY_RANGES`), the piano strip.
    pub const DISPLAY_RANGE: usize = AN_GRAB + 1;
    pub const PIANO: usize = DISPLAY_RANGE + 1;
    /// EQ Match is listening: the rings keep filling with the analyzer off.
    pub const MATCH: usize = PIANO + 1;
    pub const LEN: usize = MATCH + 1;
}

const DATA: [f32; data::LEN] = {
    let mut d = [0.0; data::LEN];
    let mut b = 0;
    while b < BANDS {
        d[data::ON + b] = 1.0;
        d[data::SLOPE + b] = 1.0;
        d[data::AUTO + b] = 1.0;
        d[data::SPEC_TILT + b] = 1.0;
        b += 1;
    }
    d[data::RESOLUTION] = 1.0;
    d[data::SOLO] = -1.0;
    d[data::AUDITION] = -1.0;
    d[data::AN_PRE] = 1.0;
    d[data::AN_POST] = 1.0;
    d[data::AN_RANGE] = 1.0;
    d[data::AN_RES] = 2.0;
    d[data::AN_SPEED] = 2.0;
    d[data::AN_TILT] = 3.0;
    d[data::AN_GRAB] = 1.0;
    d[data::DISPLAY_RANGE] = 2.0;
    d
};

/// Samples in each analyzer ring.
pub const RTA_LEN: usize = 8192;
/// The rings: before the EQ, after it, the side chain.
pub const RING_PRE: usize = 0;
pub const RING_POST: usize = 1;
pub const RING_SC: usize = 2;

/// Layout of `ModuleParams::viz`.
pub mod viz {
    use super::{BANDS, RTA_LEN, spectral::CURVE};
    /// Per band: the live gain change from its dynamics (dB), its trigger's
    /// level and the threshold in use (dB).
    pub const DYN: usize = 0;
    pub const LEVEL: usize = DYN + BANDS;
    pub const THRESHOLD: usize = LEVEL + BANDS;
    /// Output peaks (taken by the panel).
    pub const OUT_L: usize = THRESHOLD + BANDS;
    pub const OUT_R: usize = OUT_L + 1;
    pub const SR: usize = OUT_R + 1;
    /// Samples of latency now.
    pub const LATENCY: usize = SR + 1;
    /// The auto gain applied (dB).
    pub const AUTO_GAIN: usize = LATENCY + 1;
    /// 1: dynamics and spectral bands are off at this resolution.
    pub const WARN: usize = AUTO_GAIN + 1;
    /// The spectral bands' gain changes along the spectrum (dB).
    pub const CURVE_AT: usize = WARN + 1;
    /// Where the next sample goes in each ring, then the rings.
    pub const RING_WRITE: usize = CURVE_AT + CURVE;
    pub const RINGS: usize = RING_WRITE + 3;
    pub const LEN: usize = RINGS + 3 * RTA_LEN;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Prism",
    prefix: "PRISM",
    hint: "24-band EQ · dynamic · spectral · linear phase",
    inputs: &["IN", "SC"],
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

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

pub fn on(p: &ModuleParams, i: usize) -> bool {
    p.data[i].get() > 0.5
}

/// A setting with `count` choices.
pub fn choice(p: &ModuleParams, i: usize, count: usize) -> usize {
    (p.data[i].get().round().max(0.0) as usize).min(count - 1)
}

/// A band index setting (SOLO, AUDITION): −1 is none.
pub fn band_setting(p: &ModuleParams, i: usize) -> Option<usize> {
    let v = p.data[i].get().round();
    (0.0..BANDS as f32).contains(&v).then_some(v as usize)
}

pub fn used(p: &ModuleParams, b: usize) -> bool {
    on(p, data::USED + b)
}

/// The band is there and not bypassed.
pub fn active(p: &ModuleParams, b: usize) -> bool {
    used(p, b) && on(p, data::ON + b)
}

pub fn shape(p: &ModuleParams, b: usize) -> usize {
    choice(p, data::SHAPE + b, SHAPES)
}

pub fn slope(p: &ModuleParams, b: usize) -> usize {
    design::fix_slope(shape(p, b), choice(p, data::SLOPE + b, BRICKWALL + 1))
}

pub fn place(p: &ModuleParams, b: usize) -> usize {
    choice(p, data::PLACE + b, PLACES)
}

/// The band has a dynamic range (and a shape that allows one).
pub fn dynamic(p: &ModuleParams, b: usize) -> bool {
    design::can_be_dynamic(shape(p, b)) && p.get(bp(b, band::RANGE)) != 0.0
}

/// The band is dynamic and spectral.
pub fn spectral(p: &ModuleParams, b: usize) -> bool {
    dynamic(p, b) && on(p, data::SPECTRAL + b)
}

/// Analog gain-Q interaction for bells: more gain narrows the bell a
/// little, and a narrow bell gains a touch.
pub fn gain_q(gain: f32, q: f32) -> (f32, f32) {
    let q2 = q * 2f32.powf(gain.abs() / 36.0);
    let g2 = gain * (1.0 + 0.12 * ((q / 2.0).log2() / 3.0).clamp(0.0, 1.0));
    (g2, q2)
}

/// What band `b` really is: GAIN SCALE and gain-Q interaction applied,
/// with `extra` dB of dynamics on top.
pub fn effective(p: &ModuleParams, b: usize, extra: f32) -> Shape {
    let sh = shape(p, b);
    let scale = p.get(SCALE);
    let mut gain = (p.get(bp(b, band::GAIN)) * scale + extra).clamp(-2.0 * GAIN_RANGE, 2.0 * GAIN_RANGE);
    let mut q = p.get(bp(b, band::Q));
    if sh == BELL && on(p, data::GAIN_Q) {
        (gain, q) = gain_q(gain, q);
    }
    Shape { shape: sh, slope: slope(p, b), freq: p.get(bp(b, band::FREQ)) as f64, gain: gain as f64, q: q as f64 }
}

/// The band's analog design now (for the panel).
pub fn analog(p: &ModuleParams, b: usize, extra: f32) -> Analog {
    Analog::design(&effective(p, b, extra))
}

/// The largest frame the spectral stage needs at `sr`.
fn rate_factor(sr: f32) -> usize {
    if sr <= 50_000.0 {
        1
    } else if sr <= 100_000.0 {
        2
    } else {
        4
    }
}

/// The frame for a resolution at `sr`.
pub fn frame(resolution: usize, sr: f32) -> usize {
    RESOLUTIONS[resolution.min(RESOLUTIONS.len() - 1)].1 * rate_factor(sr)
}

/// The latency of the oversampled Natural Phase path, in samples (the
/// half-band filters up and down: 15 + 15.5).
pub const NATURAL_LATENCY: f32 = 30.5;

/// Gives a band its settings when it is created.
pub fn create_band(p: &ModuleParams, b: usize, sh: usize, freq: f32, gain: f32, q: f32) {
    p.data[data::USED + b].set(1.0);
    p.data[data::ON + b].set(1.0);
    p.data[data::SHAPE + b].set(sh as f32);
    let slope = if matches!(sh, LOW_CUT | HIGH_CUT) { 3 } else { 1 };
    p.data[data::SLOPE + b].set(design::fix_slope(sh, slope) as f32);
    p.data[data::PLACE + b].set(0.0);
    p.data[data::AUTO + b].set(1.0);
    p.data[data::SC_EXT + b].set(0.0);
    p.data[data::SC_FREE + b].set(0.0);
    p.data[data::SPECTRAL + b].set(0.0);
    p.data[data::SPEC_TILT + b].set(1.0);
    p.set(bp(b, band::FREQ), freq.clamp(FREQ_RANGE.0, FREQ_RANGE.1));
    p.set(bp(b, band::GAIN), if design::has_gain(sh) { gain.clamp(-GAIN_RANGE, GAIN_RANGE) } else { 0.0 });
    p.set(bp(b, band::Q), q.clamp(Q_RANGE.0, Q_RANGE.1));
    p.set(bp(b, band::RANGE), 0.0);
    p.set(bp(b, band::THRESHOLD), -20.0);
    p.set(bp(b, band::ATTACK), 0.5);
    p.set(bp(b, band::RELEASE), 0.5);
    p.set(bp(b, band::SC_LO), (freq * 0.5).clamp(FREQ_RANGE.0, FREQ_RANGE.1));
    p.set(bp(b, band::SC_HI), (freq * 2.0).clamp(FREQ_RANGE.0, FREQ_RANGE.1));
    p.set(bp(b, band::DENSITY), 0.5);
}

/// The first free band slot.
pub fn free_band(p: &ModuleParams) -> Option<usize> {
    (0..BANDS).find(|&b| !used(p, b))
}

pub fn delete_band(p: &ModuleParams, b: usize) {
    p.data[data::USED + b].set(0.0);
    if band_setting(p, data::SOLO) == Some(b) {
        p.data[data::SOLO].set(-1.0);
    }
    if band_setting(p, data::AUDITION) == Some(b) {
        p.data[data::AUDITION].set(-1.0);
    }
    let mask = p.data[data::SELECTED].get() as u32 & !(1 << b);
    p.data[data::SELECTED].set(mask as f32);
}

// ---------------------------------------------------------------------------
// Processor
// ---------------------------------------------------------------------------

/// One-pole smoothing of `v` towards `t`.
#[inline]
fn glide(v: &mut f32, t: f32, k: f32) {
    *v += (t - *v) * k;
}

struct Band {
    /// Smoothed frequency, gain, Q and range.
    freq: f32,
    gain: f32,
    q: f32,
    fresh: bool,
    /// The digital design in use and what it was made for.
    made: Option<(Shape, bool)>,
    digital: Digital,
    states: [[State; 2]; MAX_SECTIONS],
    det: Detector,
    /// The static curve on the grid (dB), and what it is for.
    grid_key: Option<Shape>,
    grid: Box<[f32]>,
    /// The dynamic change last put into the linear-phase grids.
    lp_dyn: f32,
    lp_curve: Box<[f32]>,
    weight_key: Option<(Shape, u32)>,
    /// Running in the minimum-phase cascade last block.
    running: bool,
}

impl Band {
    fn new() -> Self {
        Self {
            freq: 1000.0,
            gain: 0.0,
            q: 1.0,
            fresh: true,
            made: None,
            digital: Digital::default(),
            states: [[State::default(); 2]; MAX_SECTIONS],
            det: Detector::new(),
            grid_key: None,
            grid: vec![0.0; GRID].into_boxed_slice(),
            lp_dyn: 0.0,
            lp_curve: vec![0.0; GRID].into_boxed_slice(),
            weight_key: None,
            running: false,
        }
    }

    fn reset_states(&mut self) {
        for s in self.states.iter_mut().flatten() {
            s.reset();
        }
    }

    #[inline]
    fn tick(&mut self, ch: usize, x: f64) -> f64 {
        let d = &self.digital;
        let mut y = x * d.gain;
        for (c, st) in d.secs[..d.n].iter().zip(self.states.iter_mut()) {
            y = st[ch].tick(c, y);
        }
        y
    }
}

/// A band's settings for this block.
#[derive(Clone, Copy)]
struct Set {
    active: bool,
    place: usize,
    /// Target design (no dynamics).
    target: Shape,
    dynamic: bool,
    spectral: bool,
    ext: bool,
    trigger: Trigger,
    density: f32,
    tilt: bool,
    /// The band changes nothing (a bell at 0 dB, no dynamics).
    identity: bool,
}

/// Saturation of the CHARACTER modes: unity gain for small signals, the
/// bias giving even harmonics.
#[inline]
fn saturate(x: f32, character: usize) -> f32 {
    let (drive, bias) = match character {
        1 => (0.55, 0.08),
        2 => (1.4, 0.22),
        _ => return x,
    };
    let tb = fastmath::tanh(bias);
    (fastmath::tanh(drive * x + bias) - tb) / (drive * (1.0 - tb * tb))
}

pub struct Prism {
    params: Arc<ModuleParams>,
    sr: f32,
    bands: Box<[Band]>,
    mode: usize,
    resolution: usize,
    os: Oversampler,
    stage: Box<Stage>,
    stage_on: bool,
    /// What fed the stage's grids last (bands' membership), and whether the
    /// curve needs summing again.
    stage_members: u128,
    grids_dirty: bool,
    lp_count: usize,
    /// Solo filter and its key.
    solo: [Svf; 2],
    solo_c: Coeffs,
    solo_key: Option<(usize, u32, u32, usize)>,
    /// DC blockers after the saturation.
    dc: [f32; 2],
    out: f32,
    pan: f32,
    auto_gain: f32,
    auto_target: f32,
    quiet: Quiet,
    rings: [usize; 3],
    work: [[f32; BLOCK]; 2],
}

impl Prism {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        params.viz[viz::SR].set(sample_rate);
        let f = rate_factor(sample_rate);
        Self {
            bands: (0..BANDS).map(|_| Band::new()).collect(),
            mode: usize::MAX,
            resolution: usize::MAX,
            os: Oversampler::new(),
            stage: Box::new(Stage::new(sample_rate, RESOLUTIONS[4].1 * f, RESOLUTIONS[DYNAMIC_RESOLUTIONS - 1].1 * f)),
            stage_on: false,
            stage_members: 0,
            grids_dirty: true,
            lp_count: 0,
            solo: [Svf::default(); 2],
            solo_c: Coeffs::default(),
            solo_key: None,
            dc: [0.0; 2],
            out: params.get(OUTPUT),
            pan: params.get(PAN),
            auto_gain: 0.0,
            auto_target: 0.0,
            // Long enough to let the largest frame's latency through.
            quiet: Quiet::new((sample_rate * 0.5) as usize + RESOLUTIONS[4].1 * f * 5 / 4 + 32),
            rings: [0; 3],
            work: [[0.0; BLOCK]; 2],
            params,
            sr: sample_rate,
        }
    }

    /// This block's band settings.
    fn settings(&self, mode: usize, resolution: usize) -> [Set; BANDS] {
        let p = &self.params;
        let dynamics_ok = mode != LINEAR_PHASE || resolution < DYNAMIC_RESOLUTIONS;
        let scale = p.get(SCALE);
        std::array::from_fn(|b| {
            let active = active(p, b);
            let target = effective(p, b, 0.0);
            let dynamic = active && dynamics_ok && dynamic(p, b);
            let spectral = dynamic && on(p, data::SPECTRAL + b);
            let place = place(p, b);
            let trigger = Trigger {
                shape: target.shape,
                place,
                free: on(p, data::SC_FREE + b),
                freq: target.freq as f32,
                q: target.q as f32,
                lo: p.get(bp(b, band::SC_LO)),
                hi: p.get(bp(b, band::SC_HI)),
                range: p.get(bp(b, band::RANGE)) * scale,
                threshold: p.get(bp(b, band::THRESHOLD)),
                auto: on(p, data::AUTO + b),
                attack: p.get(bp(b, band::ATTACK)),
                release: p.get(bp(b, band::RELEASE)),
            };
            let identity = design::has_gain(target.shape) && target.gain.abs() < 1e-4 && !dynamic;
            Set {
                active,
                place,
                target,
                dynamic,
                spectral,
                ext: on(p, data::SC_EXT + b),
                trigger,
                density: p.get(bp(b, band::DENSITY)),
                tilt: on(p, data::SPEC_TILT + b),
                identity,
            }
        })
    }

    /// Mode and resolution changes: start clean.
    fn configure(&mut self, mode: usize, resolution: usize, stage_on: bool, spectral_n: usize) {
        if mode != self.mode {
            self.mode = mode;
            self.os.reset();
            for b in self.bands.iter_mut() {
                b.made = None;
                b.reset_states();
            }
            self.stage.reset();
            self.grids_dirty = true;
        }
        let n = if mode == LINEAR_PHASE { frame(resolution, self.sr) } else { spectral_n };
        if n != self.stage.n() {
            self.stage.resize(n);
            self.grids_dirty = true;
        }
        self.resolution = resolution;
        if stage_on && !self.stage_on {
            self.stage.reset();
            self.grids_dirty = true;
        }
        self.stage_on = stage_on;
    }

    /// Band curves on the grid, the stage's gains and auto gain.
    /// `slow`: a few hundred samples have passed, time for the slower work
    /// (redrawing moved bands' curves, linear phase's live dynamics).
    fn update_curves(&mut self, sets: &[Set; BANDS], slow: bool) {
        let sr = self.sr;
        let lp = self.mode == LINEAR_PHASE;
        let mut changed = false;
        for (band, s) in self.bands.iter_mut().zip(sets) {
            // A band being dragged redraws its curve now and then, not every
            // block: steep bands are a few thousand evaluations.
            if s.active && band.grid_key != Some(s.target) && (slow || band.grid_key.is_none()) {
                band.grid_key = Some(s.target);
                let a = Analog::design(&s.target);
                for (i, g) in band.grid.iter_mut().enumerate() {
                    *g = a.db(spectral::grid_freq(i, sr) as f64) as f32;
                }
                changed = true;
            }
        }
        // Which bands feed the stage, and how.
        let mut members = 0u128;
        for (b, s) in sets.iter().enumerate() {
            if s.active && (lp || s.spectral) {
                members |= 1 << b;
                members |= (s.place as u128 + 1) << (24 + 3 * b);
            }
        }
        if members != self.stage_members {
            self.stage_members = members;
            changed = true;
        }
        // Linear phase: dynamic bands' live change, now and then.
        if lp && slow {
            for (band, s) in self.bands.iter_mut().zip(sets) {
                let d = if s.dynamic && !s.spectral { band.det.dynamic } else { 0.0 };
                if (d - band.lp_dyn).abs() > 0.05 || (d == 0.0 && band.lp_dyn != 0.0) {
                    band.lp_dyn = d;
                    if d != 0.0 {
                        let mut t = s.target;
                        t.gain += d as f64;
                        let a = Analog::design(&t);
                        for (i, g) in band.lp_curve.iter_mut().enumerate() {
                            *g = a.db(spectral::grid_freq(i, sr) as f64) as f32 - band.grid[i];
                        }
                    }
                    changed = true;
                }
            }
        }
        // Spectral bands' settings and weights reach the stage every block:
        // their knobs (threshold, times, density) don't move the curve.
        if self.stage_on {
            for (b, s) in sets.iter().enumerate() {
                if !s.spectral {
                    self.stage.spec[b].active = false;
                    continue;
                }
                let (a, r) = dynamics::auto_times(s.target.freq as f32);
                self.stage.spec[b] = SpecBand {
                    active: true,
                    place: s.place,
                    ext: s.ext,
                    range: s.trigger.range,
                    threshold: s.trigger.threshold,
                    auto: s.trigger.auto,
                    // Spectral frames are coarse in time: no faster than a hop.
                    attack: a * dynamics::time_factor(s.trigger.attack) * 4.0,
                    release: r * dynamics::time_factor(s.trigger.release) * 2.0,
                    density: s.density,
                    tilt: s.tilt,
                };
                // Its weight: the band's shape at full range, 0…1.
                let range = s.trigger.range;
                let key = (s.target, range.to_bits());
                let band = &mut self.bands[b];
                if band.weight_key != Some(key) && (slow || band.weight_key.is_none()) {
                    band.weight_key = Some(key);
                    let mut t = s.target;
                    t.gain = range as f64;
                    let a = Analog::design(&t);
                    for (i, v) in self.stage.weight[b].iter_mut().enumerate() {
                        *v = ((a.db(spectral::grid_freq(i, sr) as f64) as f32) / range).clamp(0.0, 1.0);
                    }
                }
            }
        }
        if !(changed || self.grids_dirty) {
            return;
        }
        self.grids_dirty = false;

        // The stage's gains per channel.
        if self.stage_on {
            let st = &mut self.stage;
            for g in st.db.iter_mut() {
                g.fill(0.0);
            }
            st.ms = false;
            for (band, s) in self.bands.iter().zip(sets) {
                let feeds = s.active && (lp || s.spectral);
                if !feeds {
                    continue;
                }
                let chans: &[usize] = match s.place {
                    LEFT => &[spectral::L],
                    RIGHT => &[spectral::R],
                    MID => &[spectral::M],
                    SIDE => &[spectral::S],
                    _ => &[spectral::L, spectral::R],
                };
                st.ms |= matches!(s.place, MID | SIDE);
                let live = lp && band.lp_dyn != 0.0;
                for &c in chans {
                    for i in 0..GRID {
                        st.db[c][i] += band.grid[i] + if live { band.lp_curve[i] } else { 0.0 };
                    }
                }
            }
        }

        // Auto gain: what the curve does to pink noise, 20 Hz–20 kHz, on
        // both sides (mid bands count fully, side bands not at all).
        let (lo, hi) = (spectral::grid_pos(20.0, sr) as usize, spectral::grid_pos(20_000.0, sr) as usize);
        let mut power = 0.0f64;
        let mut count = 0usize;
        for i in lo..=hi.min(GRID - 1) {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for (band, s) in self.bands.iter().zip(sets) {
                if !s.active {
                    continue;
                }
                match s.place {
                    LEFT => l += band.grid[i],
                    RIGHT => r += band.grid[i],
                    SIDE => {}
                    _ => {
                        l += band.grid[i];
                        r += band.grid[i];
                    }
                }
            }
            power += 0.5 * (10f64.powf(l as f64 / 10.0) + 10f64.powf(r as f64 / 10.0));
            count += 1;
        }
        self.auto_target = if count > 0 { (-10.0 * (power / count as f64).log10()).clamp(-24.0, 24.0) as f32 } else { 0.0 };
    }

    fn solo_filter(&mut self, s: &Set) {
        let f = s.target.freq as f32;
        let q = s.target.q as f32;
        let key = (s.target.shape, f.to_bits(), q.to_bits(), s.place);
        if self.solo_key == Some(key) {
            return;
        }
        if self.solo_key.is_none_or(|k| k.0 != key.0) {
            self.solo = [Svf::default(); 2];
        }
        self.solo_key = Some(key);
        let g = fastmath::tan_turns(f.clamp(10.0, self.sr * 0.45) / (2.0 * self.sr));
        let k2 = std::f32::consts::SQRT_2;
        self.solo_c = match s.target.shape {
            BELL | NOTCH | BAND_PASS => {
                let k = 1.0 / q.clamp(0.3, 30.0);
                Coeffs::svf(g, k, (0.0, k, 0.0))
            }
            // Shelves: the side they lift or cut; cuts: what they remove.
            LOW_SHELF | LOW_CUT => Coeffs::svf(g, k2, (0.0, 0.0, 1.0)),
            HIGH_SHELF | HIGH_CUT => Coeffs::svf(g, k2, (1.0, -k2, -1.0)),
            _ => Coeffs::svf(g, k2, (1.0, 0.0, 0.0)),
        };
    }
}

/// Into mid/side and back.
#[inline]
fn on_place(place: usize, x: (f64, f64), mut f: impl FnMut(usize, f64) -> f64) -> (f64, f64) {
    match place {
        LEFT => (f(0, x.0), x.1),
        RIGHT => (x.0, f(1, x.1)),
        MID | SIDE => {
            let (m, s) = ((x.0 + x.1) * 0.5, (x.0 - x.1) * 0.5);
            let (m, s) = if place == MID { (f(0, m), s) } else { (m, f(0, s)) };
            (m + s, m - s)
        }
        _ => (f(0, x.0), f(1, x.1)),
    }
}

/// Samples between dynamic updates.
const CONTROL: usize = 16;

impl Processor for Prism {
    #[allow(clippy::needless_range_loop)] // both channels, sample by sample
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let sr = self.sr;
        let input = inputs.get(0);
        let side = inputs.get(1);
        let mode = choice(&p, data::MODE, 3);
        let resolution = choice(&p, data::RESOLUTION, RESOLUTIONS.len());
        let sets = self.settings(mode, resolution);
        let any_spectral = sets.iter().any(|s| s.spectral);
        let stage_on = mode == LINEAR_PHASE || any_spectral;
        let spectral_n = frame(resolution.min(DYNAMIC_RESOLUTIONS - 1), sr);
        self.configure(mode, resolution, stage_on, spectral_n);
        let latency = if self.stage_on { self.stage.latency() as f32 } else { 0.0 } + if mode == NATURAL_PHASE { NATURAL_LATENCY } else { 0.0 };
        p.viz[viz::LATENCY].set(latency);
        p.viz[viz::WARN].set((mode == LINEAR_PHASE && resolution >= DYNAMIC_RESOLUTIONS && (0..BANDS).any(|b| active(&p, b) && dynamic(&p, b))) as u8 as f32);

        let silent_in = Quiet::silent(input, n);
        let listening = on(&p, data::AN_PRE) || on(&p, data::AN_POST) || on(&p, data::AN_SC) || on(&p, data::MATCH);
        if silent_in && self.quiet.resting() && !(listening && side.is_some()) {
            silence(outputs, n);
            for b in 0..BANDS {
                p.viz[viz::DYN + b].set(0.0);
                p.viz[viz::LEVEL + b].set(-120.0);
            }
            return;
        }

        // Smoothing for this block.
        let k_block = 1.0 - (1.0 - ctx.smooth).powi(n as i32);
        self.lp_count += n;
        let slow = self.lp_count >= 256;
        if slow {
            self.lp_count = 0;
        }
        self.update_curves(&sets, slow);
        glide(&mut self.auto_gain, if on(&p, data::AUTO_GAIN) { self.auto_target } else { 0.0 }, k_block);
        p.viz[viz::AUTO_GAIN].set(self.auto_gain);

        let os = mode == NATURAL_PHASE;
        let cascade = mode != LINEAR_PHASE;
        let rate = if os { sr * 2.0 } else { sr };
        for (band, s) in self.bands.iter_mut().zip(&sets) {
            let runs = cascade && s.active && !s.spectral && !s.identity;
            if !runs {
                if band.running {
                    band.running = false;
                    band.reset_states();
                    band.made = None;
                }
                band.fresh = true;
                continue;
            }
            if band.fresh {
                band.fresh = false;
                band.freq = s.target.freq as f32;
                band.gain = s.target.gain as f32;
                band.q = s.target.q as f32;
            } else {
                glide(&mut band.freq, s.target.freq as f32, k_block);
                glide(&mut band.gain, s.target.gain as f32, k_block);
                glide(&mut band.q, s.target.q as f32, k_block);
            }
            band.running = true;
        }

        // Dynamic bands' triggers run in every mode (spectral bands measure
        // in the stage instead).
        let avg_k = 1.0 - (-(CONTROL as f32) / (0.5 * sr)).exp();
        let triggers: [Option<(f32, f32)>; BANDS] = std::array::from_fn(|b| {
            let s = &sets[b];
            (s.dynamic && !s.spectral).then(|| self.bands[b].det.coefficients(&s.trigger, sr))
        });
        let character = choice(&p, data::CHARACTER, 3);
        let audition = band_setting(&p, data::AUDITION).filter(|&b| sets[b].dynamic);
        let mut heard = [[0.0f32; BLOCK]; 2];

        for i in 0..n {
            let x = input.map_or((0.0, 0.0), |b| (b[0][i], b[1][i]));
            let sc = side.map_or((0.0, 0.0), |b| (b[0][i], b[1][i]));
            let control = i % CONTROL == 0;
            for b in 0..BANDS {
                let Some((ka, kr)) = triggers[b] else { continue };
                let s = &sets[b];
                let band = &mut self.bands[b];
                band.det.tick(&s.trigger, sr, if s.ext { sc } else { x }, ka, kr);
                if control {
                    band.det.update(&s.trigger, avg_k);
                }
                if audition == Some(b) {
                    (heard[0][i], heard[1][i]) = band.det.heard;
                }
            }
            if control && cascade {
                // Redesign what moved.
                for (band, s) in self.bands.iter_mut().zip(&sets) {
                    if !band.running {
                        continue;
                    }
                    let dyn_db = if s.dynamic { band.det.dynamic } else { 0.0 };
                    let want = Shape { freq: band.freq as f64, gain: band.gain as f64 + dyn_db as f64, q: band.q as f64, ..s.target };
                    let stale = match band.made {
                        Some((m, o)) => {
                            o != os
                                || m.shape != want.shape
                                || m.slope != want.slope
                                || (m.freq / want.freq - 1.0).abs() > 1e-5
                                || (m.gain - want.gain).abs() > 0.01
                                || (m.q / want.q - 1.0).abs() > 1e-4
                        }
                        None => true,
                    };
                    if stale {
                        if band.made.is_some_and(|(m, _)| m.shape != want.shape || m.slope != want.slope) {
                            band.reset_states();
                        }
                        band.digital = Analog::design(&want).digital(rate as f64);
                        band.made = Some((want, os));
                    }
                }
            }

            // The minimum-phase cascade, at the rate of the mode.
            let mut y = (x.0 as f64, x.1 as f64);
            if cascade {
                if os {
                    let up = self.os.up([x.0, x.1]);
                    let mut down = [[0.0f32; 2]; 2];
                    for (k, u) in up.iter().enumerate() {
                        let mut v = (u[0] as f64, u[1] as f64);
                        for (band, s) in self.bands.iter_mut().zip(&sets) {
                            if band.running {
                                v = on_place(s.place, v, |ch, val| band.tick(ch, val));
                            }
                        }
                        down[k] = [saturate(v.0 as f32, character), saturate(v.1 as f32, character)];
                    }
                    let d = self.os.down(down);
                    y = (d[0] as f64, d[1] as f64);
                } else {
                    for (band, s) in self.bands.iter_mut().zip(&sets) {
                        if band.running {
                            y = on_place(s.place, y, |ch, val| band.tick(ch, val));
                        }
                    }
                }
            }
            self.work[0][i] = y.0 as f32;
            self.work[1][i] = y.1 as f32;
        }

        // The spectral stage: linear phase, spectral bands.
        if self.stage_on {
            let (l, r) = self.work.split_at_mut(1);
            let sc = side.map(|b| (&b[0][..n], &b[1][..n]));
            self.stage.process(&mut l[0][..n], &mut r[0][..n], sc);
            for (b, s) in sets.iter().enumerate() {
                if s.spectral {
                    p.viz[viz::DYN + b].set(self.stage.activity[b]);
                }
            }
            for (k, v) in self.stage.curve.iter().enumerate() {
                p.viz[viz::CURVE_AT + k].set(*v);
            }
        }

        // Output: character (in the base rate's modes), gains, pan, polarity.
        let invert = if on(&p, data::INVERT) { -1.0 } else { 1.0 };
        let pan_ms = on(&p, data::PAN_MS);
        let solo = band_setting(&p, data::SOLO).filter(|&b| sets[b].active);
        if let Some(b) = solo {
            let s = sets[b];
            self.solo_filter(&s);
        }
        let dc_k = 1.0 - (-std::f32::consts::TAU * 5.0 / sr).exp();
        let mut peak = [0.0f32; 2];
        let mut loud = 0.0f32;
        let listening_rings = listening;
        for i in 0..n {
            glide(&mut self.out, p.get(OUTPUT), ctx.smooth);
            glide(&mut self.pan, p.get(PAN), ctx.smooth);
            let gain = 10f32.powf((self.out + self.auto_gain) / 20.0) * invert;
            let (mut l, mut r) = (self.work[0][i], self.work[1][i]);
            if character != 0 {
                for (ch, v) in [&mut l, &mut r].into_iter().enumerate() {
                    // Natural Phase saturates at twice the rate, inside.
                    let y = if os { *v } else { saturate(*v, character) };
                    // DC blocker: the bias leaves an offset.
                    let state = &mut self.dc[ch];
                    *state += (y - *state) * dc_k;
                    *v = y - *state;
                }
            }
            let (gl, gr) = (1.0 - self.pan.max(0.0), 1.0 + self.pan.min(0.0));
            if pan_ms {
                let (m, s) = ((l + r) * 0.5 * gl, (l - r) * 0.5 * gr);
                (l, r) = (m + s, m - s);
            } else {
                (l, r) = (l * gl, r * gr);
            }
            l *= gain;
            r *= gain;
            let x = input.map_or((0.0, 0.0), |b| (b[0][i], b[1][i]));
            if let Some(b) = solo {
                let place = sets[b].place;
                let src = match place {
                    LEFT => (x.0, x.0),
                    RIGHT => (x.1, x.1),
                    MID => ((x.0 + x.1) * 0.5, (x.0 + x.1) * 0.5),
                    SIDE => ((x.0 - x.1) * 0.5, (x.1 - x.0) * 0.5),
                    _ => x,
                };
                l = self.solo[0].apply(&self.solo_c, src.0);
                r = self.solo[1].apply(&self.solo_c, src.1);
            } else if audition.is_some() {
                (l, r) = (heard[0][i], heard[1][i]);
            }
            // Like every node on the board, under full scale (transparent
            // below 0.9).
            (l, r) = (soft_limit(l), soft_limit(r));
            if !(l.is_finite() && r.is_finite()) {
                (l, r) = (0.0, 0.0);
                for band in self.bands.iter_mut() {
                    band.reset_states();
                }
                self.stage.reset();
            }
            outputs[0][0][i] = l;
            outputs[0][1][i] = r;
            peak[0] = peak[0].max(l.abs());
            peak[1] = peak[1].max(r.abs());
            loud = loud.max(l.abs()).max(r.abs());
            if listening_rings {
                let sc = side.map_or((0.0, 0.0), |b| (b[0][i], b[1][i]));
                for (ring, v) in [(RING_PRE, (x.0 + x.1) * 0.5), (RING_POST, (l + r) * 0.5), (RING_SC, (sc.0 + sc.1) * 0.5)] {
                    p.viz[viz::RINGS + ring * RTA_LEN + self.rings[ring]].set(v);
                    self.rings[ring] = (self.rings[ring] + 1) % RTA_LEN;
                }
            }
        }
        if listening_rings {
            for ring in 0..3 {
                p.viz[viz::RING_WRITE + ring].set(self.rings[ring] as f32);
            }
        }
        for (b, s) in sets.iter().enumerate() {
            if s.spectral {
                p.viz[viz::LEVEL + b].set(-120.0);
                continue;
            }
            let det = &self.bands[b].det;
            let live = s.dynamic;
            p.viz[viz::DYN + b].set(if live { det.dynamic } else { 0.0 });
            p.viz[viz::LEVEL + b].set(if live { det.level } else { -120.0 });
            p.viz[viz::THRESHOLD + b].set(det.threshold);
        }
        p.viz[viz::OUT_L].fetch_max(peak[0]);
        p.viz[viz::OUT_R].fetch_max(peak[1]);
        self.quiet.update(silent_in, loud, n);
        p.meter.fetch_max(peak[0].max(peak[1]));
    }
}
