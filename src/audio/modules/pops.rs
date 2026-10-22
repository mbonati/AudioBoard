//! Pops 66: a rhythm box in the spirit of the KORG Mini Pops-7 (1966).
//! Fifteen instruments are synthesized as the original made them (no
//! samples), each with TUNE, ATTACK, DECAY and its own extras (resonance,
//! tick, high-pass, second partial, rate, density), its waveform (WAVE,
//! BEND, DRIVE), LEVEL and PAN.
//!
//! Twenty preset rhythms sit in two rows; several of one row can play at
//! once, merged as the original's diode matrix did. Eight user patterns
//! (up to 32 steps of sixteenths or triplets) can be edited step by step,
//! starting from a preset or from scratch. The machine follows the board's
//! tempo and transport (SYNC), or runs on its own TEMPO with PLAY. MIDI
//! notes on the MIDI input play the instruments (General MIDI drum notes),
//! and the pattern goes out as notes on the MIDI output (channel 10).

pub mod rhythms;
pub mod voices;

use std::sync::Arc;

use super::steps::{MAX_STEPS, Step, StepClock};
use super::{ModuleDef, ModuleParams, Rng, approach, pan_gains, soft_limit};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::params::{ParamSpec, Unit, format_hz, format_seconds};
use rhythms::{MAX_LEN, RHYTHMS, ROW, Rhythm, combine};
use voices::{COUNT, Sound, Voice};

/// An instrument: its name, a short label, its General MIDI drum note and
/// what its three extra knobs do ("" when unused).
pub struct Instrument {
    pub name: &'static str,
    pub short: &'static str,
    pub note: u8,
    pub extras: [&'static str; 3],
    /// The guiro scrapes as long as its gate is open: no attack or decay.
    pub enveloped: bool,
}

const fn inst(name: &'static str, short: &'static str, note: u8, extras: [&'static str; 3]) -> Instrument {
    Instrument { name, short, note, extras, enveloped: true }
}

pub const INSTRUMENTS: [Instrument; COUNT] = [
    inst("Base", "BD", 36, ["", "", ""]),
    inst("Snare 1", "SN1", 38, ["RESO", "TICK", "HIGHP"]),
    inst("Snare 2", "SN2", 40, ["RESO", "TICK", "HIGHP"]),
    inst("Small Bongo", "BG S", 60, ["", "", ""]),
    inst("Large Bongo", "BG L", 61, ["", "", ""]),
    inst("Conga", "CONGA", 63, ["LEVEL2", "TUNE2", "DECAY2"]),
    inst("Cowbell", "BELL", 56, ["LEVEL2", "TUNE2", "DECAY2"]),
    inst("Claves", "CLAVES", 75, ["", "", ""]),
    inst("Hihat", "HIHAT", 42, ["RESO", "", ""]),
    inst("Rimshot", "RIM", 37, ["", "", ""]),
    inst("Cymbal 1", "CYM 1", 49, ["RESO", "HIGHP", ""]),
    inst("Cymbal 2", "CYM 2", 51, ["RESO", "HIGHP", ""]),
    inst("Tambourine", "TAMB", 54, ["RESO", "HIGHP", ""]),
    Instrument { name: "Guiro", short: "GUIRO", note: 73, extras: ["RATE", "DENSITY", ""], enveloped: false },
    inst("Quijada", "QUIJ", 58, ["RATE", "DENSITY", ""]),
];

/// An instrument's knobs, relative to its first (see [`ip`]).
pub mod knob {
    pub const TUNE: usize = 0;
    pub const ATTACK: usize = 1;
    pub const DECAY: usize = 2;
    /// The extras: see [`super::Instrument::extras`].
    pub const X1: usize = 3;
    /// The waveform: sine → triangle → square (noise → metal), the pitch
    /// sweep at the start, saturation.
    pub const WAVE: usize = 6;
    pub const BEND: usize = 7;
    pub const DRIVE: usize = 8;
    pub const LEVEL: usize = 9;
    pub const PAN: usize = 10;
    pub const COUNT: usize = 11;
}

/// BEND's range, either way.
pub const BEND_OCTAVES: f32 = 2.0;

pub const fn ip(i: usize, k: usize) -> usize {
    i * knob::COUNT + k
}

pub const VOLUME: usize = COUNT * knob::COUNT;
/// Base drum against the cymbal group (hihat, cymbals), as on the panel.
pub const BALANCE: usize = VOLUME + 1;
/// The machine's own tempo (without SYNC).
pub const TEMPO: usize = BALANCE + 1;
pub const SWING: usize = TEMPO + 1;
pub const PARAM_COUNT: usize = SWING + 1;

const fn unused(name: &'static str) -> ParamSpec {
    ParamSpec::lin(name, 0.0, 1.0, 0.0, Unit::Percent)
}
const fn amount(name: &'static str, default: f32) -> ParamSpec {
    ParamSpec::lin(name, 0.0, 1.0, default, Unit::Percent)
}
const fn hz(name: &'static str, lo: f32, hi: f32, default: f32) -> ParamSpec {
    ParamSpec::log(name, lo, hi, default, Unit::Hz)
}
const fn secs(name: &'static str, lo: f32, hi: f32, default: f32) -> ParamSpec {
    ParamSpec::log(name, lo, hi, default, Unit::Seconds)
}

/// TUNE (lo, hi, default), ATTACK, DECAY (lo, hi, default), three extras,
/// and the WAVE it starts from (a sine, or noise, unless given).
macro_rules! instrument {
    ($s:literal, $tune:expr, $attack:expr, $decay:expr, $x1:expr, $x2:expr, $x3:expr) => {
        instrument!($s, $tune, $attack, $decay, $x1, $x2, $x3, 0.0)
    };
    ($s:literal, $tune:expr, $attack:expr, $decay:expr, $x1:expr, $x2:expr, $x3:expr, $wave:expr) => {
        [
            hz(concat!($s, " TUNE"), $tune.0, $tune.1, $tune.2),
            secs(concat!($s, " ATTACK"), 0.0002, 0.05, $attack),
            secs(concat!($s, " DECAY"), $decay.0, $decay.1, $decay.2),
            $x1,
            $x2,
            $x3,
            ParamSpec::lin(concat!($s, " WAVE"), 0.0, 1.0, $wave, Unit::Percent),
            ParamSpec::lin(concat!($s, " BEND"), -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
            ParamSpec::lin(concat!($s, " DRIVE"), 0.0, 1.0, 0.0, Unit::Percent),
            ParamSpec::lin(concat!($s, " LEVEL"), 0.0, 1.0, 0.8, Unit::Percent),
            ParamSpec::lin(concat!($s, " PAN"), -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
        ]
    };
}

const PARAMS: [ParamSpec; PARAM_COUNT] = {
    const I: [[ParamSpec; knob::COUNT]; COUNT] = [
        instrument!("BD", (30.0, 150.0, 58.0), 0.002, (0.05, 1.5, 0.35), unused("BD X1"), unused("BD X2"), unused("BD X3")),
        instrument!(
            "SN1",
            (100.0, 400.0, 180.0),
            0.0005,
            (0.03, 0.8, 0.18),
            amount("SN1 RESO", 0.3),
            amount("SN1 TICK", 0.5),
            hz("SN1 HIGHP", 200.0, 10_000.0, 1200.0)
        ),
        instrument!(
            "SN2",
            (100.0, 400.0, 230.0),
            0.0005,
            (0.03, 0.8, 0.11),
            amount("SN2 RESO", 0.5),
            amount("SN2 TICK", 0.3),
            hz("SN2 HIGHP", 200.0, 10_000.0, 2500.0)
        ),
        instrument!("BGS", (200.0, 800.0, 420.0), 0.0008, (0.02, 0.5, 0.09), unused("BGS X1"), unused("BGS X2"), unused("BGS X3")),
        instrument!("BGL", (120.0, 500.0, 290.0), 0.0008, (0.02, 0.5, 0.12), unused("BGL X1"), unused("BGL X2"), unused("BGL X3")),
        instrument!(
            "CNG",
            (100.0, 500.0, 210.0),
            0.0008,
            (0.03, 0.8, 0.2),
            amount("CNG LEVEL2", 0.4),
            hz("CNG TUNE2", 100.0, 1000.0, 330.0),
            secs("CNG DECAY2", 0.01, 0.5, 0.08)
        ),
        instrument!(
            "CB",
            (300.0, 1200.0, 560.0),
            0.0005,
            (0.03, 1.0, 0.25),
            amount("CB LEVEL2", 0.8),
            hz("CB TUNE2", 400.0, 1600.0, 845.0),
            secs("CB DECAY2", 0.01, 0.6, 0.1),
            1.0
        ),
        instrument!("CLV", (1000.0, 5000.0, 2500.0), 0.0003, (0.005, 0.2, 0.04), unused("CLV X1"), unused("CLV X2"), unused("CLV X3")),
        instrument!("HH", (3000.0, 14_000.0, 8000.0), 0.0003, (0.01, 0.4, 0.05), amount("HH RESO", 0.3), unused("HH X2"), unused("HH X3")),
        instrument!("RIM", (800.0, 4000.0, 1700.0), 0.0003, (0.005, 0.15, 0.025), unused("RIM X1"), unused("RIM X2"), unused("RIM X3")),
        instrument!(
            "CY1",
            (3000.0, 14_000.0, 6500.0),
            0.0005,
            (0.05, 2.0, 0.35),
            amount("CY1 RESO", 0.2),
            hz("CY1 HIGHP", 1000.0, 10_000.0, 3500.0),
            unused("CY1 X3")
        ),
        instrument!(
            "CY2",
            (3000.0, 14_000.0, 9000.0),
            0.0005,
            (0.05, 2.0, 0.8),
            amount("CY2 RESO", 0.15),
            hz("CY2 HIGHP", 1000.0, 10_000.0, 5000.0),
            unused("CY2 X3")
        ),
        instrument!(
            "TAM",
            (3000.0, 14_000.0, 7500.0),
            0.0008,
            (0.02, 0.6, 0.12),
            amount("TAM RESO", 0.4),
            hz("TAM HIGHP", 1000.0, 10_000.0, 5000.0),
            unused("TAM X3")
        ),
        instrument!(
            "GUI",
            (800.0, 6000.0, 2200.0),
            0.0005,
            (0.05, 1.0, 0.3),
            hz("GUI RATE", 10.0, 120.0, 45.0),
            amount("GUI DENSITY", 0.4),
            unused("GUI X3")
        ),
        instrument!(
            "QUI",
            (1000.0, 8000.0, 3200.0),
            0.005,
            (0.1, 2.0, 0.7),
            hz("QUI RATE", 5.0, 60.0, 22.0),
            amount("QUI DENSITY", 0.5),
            unused("QUI X3")
        ),
    ];
    const FILL: ParamSpec = unused("");
    let mut out = [FILL; PARAM_COUNT];
    let mut i = 0;
    while i < COUNT {
        let mut k = 0;
        while k < knob::COUNT {
            out[ip(i, k)] = I[i][k];
            k += 1;
        }
        i += 1;
    }
    out[VOLUME] = ParamSpec::lin("VOLUME", 0.0, 1.0, 0.8, Unit::Percent);
    out[BALANCE] = ParamSpec::lin("BALANCE", -1.0, 1.0, 0.0, Unit::Percent).bipolar();
    out[TEMPO] = ParamSpec::lin("TEMPO", 40.0, 240.0, 110.0, Unit::Integer);
    out[SWING] = ParamSpec::lin("SWING", 0.0, 1.0, 0.0, Unit::Percent);
    out
};

/// User patterns kept with the node.
pub const PATTERNS: usize = 8;

/// What `ModuleParams::data` holds besides the knobs.
pub mod data {
    use super::{COUNT, PATTERNS};
    /// 0: the preset rhythms; 1: a user pattern.
    pub const SOURCE: usize = 0;
    /// The preset row shown and played (0 top, 1 bottom).
    pub const ROW: usize = 1;
    /// Which of the row's ten rhythms play.
    pub const RHYTHM: usize = 2;
    /// The user pattern played and edited.
    pub const PATTERN: usize = 12;
    /// How much the panel shows: 0 the machine, 1 + mixer, 2 + editor.
    pub const DETAIL: usize = 13;
    /// Follow the board's tempo and transport.
    pub const SYNC: usize = 14;
    pub const POWER: usize = 15;
    /// Playing on its own clock (without SYNC).
    pub const RUNNING: usize = 16;
    /// The instrument whose sound is shown.
    pub const SELECTED: usize = 17;
    pub const MUTE: usize = 18;
    /// PATTERNS × (length, feel (0 sixteenths, 1 triplets), then per
    /// instrument the low and high 16 steps as numbers).
    pub const USER: usize = MUTE + COUNT;
    pub const USER_LEN: usize = 2 + COUNT * 2;
    pub const LEN: usize = USER + PATTERNS * USER_LEN;
}

/// User patterns to start from: some of the presets, and an empty one.
const STARTERS: [usize; PATTERNS - 1] = [6, 5, 3, 15, 12, 1, 9];

const DATA: [f32; data::LEN] = {
    let mut d = [0.0; data::LEN];
    d[data::RHYTHM + 6] = 1.0; // ROCK 1
    d[data::SYNC] = 1.0;
    d[data::POWER] = 1.0;
    let mut k = 0;
    while k < PATTERNS {
        let at = data::USER + k * data::USER_LEN;
        if k < STARTERS.len() {
            let r = &RHYTHMS[STARTERS[k]];
            d[at] = r.len as f32;
            d[at + 1] = if r.per_beat == 3 { 1.0 } else { 0.0 };
            let mut i = 0;
            while i < COUNT {
                d[at + 2 + i * 2] = (r.rows[i] & 0xFFFF) as f32;
                d[at + 3 + i * 2] = (r.rows[i] >> 16) as f32;
                i += 1;
            }
        } else {
            d[at] = 16.0;
        }
        k += 1;
    }
    d
};

/// Layout of `ModuleParams::viz`.
pub mod viz {
    use super::COUNT;
    /// The step playing (−1: stopped) and the pattern's length.
    pub const STEP: usize = 0;
    pub const LEN: usize = 1;
    /// Hits per instrument so far (the panel flashes on a change).
    pub const HITS: usize = 2;
    /// From the panel: bumped to play an instrument once (its pad).
    pub const AUDITION: usize = HITS + COUNT;
    pub const END: usize = AUDITION + COUNT;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Pops 66",
    prefix: "POPS",
    hint: "1966 rhythm box · 15 analog voices",
    inputs: &[],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: true,
    midi_out: true,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::END,
};

pub fn on(p: &ModuleParams, i: usize) -> bool {
    p.data[i].get() > 0.5
}

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// The index (into [`RHYTHMS`]) of the selected row's rhythm `i`.
pub fn row_rhythm(p: &ModuleParams, i: usize) -> usize {
    usize::from(on(p, data::ROW)) * ROW + i
}

fn user_at(k: usize) -> usize {
    data::USER + k.min(PATTERNS - 1) * data::USER_LEN
}

/// User pattern `k`.
pub fn user(p: &ModuleParams, k: usize) -> Rhythm {
    let at = user_at(k);
    let len = (p.data[at].get().round() as usize).clamp(1, MAX_LEN);
    // Steps past the length are kept (shortening and lengthening again
    // gives them back); only the first `len` play.
    let rows = std::array::from_fn(|i| {
        let lo = p.data[at + 2 + i * 2].get().max(0.0) as u32 & 0xFFFF;
        let hi = p.data[at + 3 + i * 2].get().max(0.0) as u32 & 0xFFFF;
        lo | hi << 16
    });
    Rhythm { name: "USER", per_beat: if on(p, at + 1) { 3 } else { 4 }, len, rows }
}

/// Switches one step of user pattern `k` on or off.
pub fn set_hit(p: &ModuleParams, k: usize, inst: usize, step: usize, hit: bool) {
    let at = user_at(k) + 2 + inst * 2 + usize::from(step >= 16);
    let bit = 1u32 << (step % 16);
    let bits = p.data[at].get().max(0.0) as u32 & 0xFFFF;
    p.data[at].set((if hit { bits | bit } else { bits & !bit }) as f32);
}

/// User pattern `k`'s length and feel (triplets or sixteenths).
pub fn set_shape(p: &ModuleParams, k: usize, len: usize, triplets: bool) {
    let at = user_at(k);
    p.data[at].set(len.clamp(1, MAX_LEN) as f32);
    p.data[at + 1].set(if triplets { 1.0 } else { 0.0 });
}

/// Replaces user pattern `k`.
pub fn set_user(p: &ModuleParams, k: usize, r: &Rhythm) {
    let at = user_at(k);
    p.data[at].set(r.len as f32);
    p.data[at + 1].set(if r.per_beat == 3 { 1.0 } else { 0.0 });
    for (i, bits) in r.rows.iter().enumerate() {
        p.data[at + 2 + i * 2].set((bits & 0xFFFF) as f32);
        p.data[at + 3 + i * 2].set((bits >> 16) as f32);
    }
}

/// The pattern playing: a user pattern, or the selected presets merged.
pub fn current(p: &ModuleParams) -> Option<Rhythm> {
    if on(p, data::SOURCE) {
        return Some(user(p, p.data[data::PATTERN].get().round().max(0.0) as usize));
    }
    let selected: Vec<usize> = (0..ROW).filter(|&i| on(p, data::RHYTHM + i)).map(|i| row_rhythm(p, i)).collect();
    combine(&selected)
}

/// The same without allocating, for the audio thread.
fn current_rt(p: &ModuleParams) -> Option<Rhythm> {
    if on(p, data::SOURCE) {
        return Some(user(p, p.data[data::PATTERN].get().round().max(0.0) as usize));
    }
    let mut selected = [0usize; ROW];
    let mut n = 0;
    for i in 0..ROW {
        if on(p, data::RHYTHM + i) {
            selected[n] = row_rhythm(p, i);
            n += 1;
        }
    }
    combine(&selected[..n])
}

/// A knob's value as the panel shows it.
pub fn format(i: usize, v: f32) -> String {
    if i < VOLUME {
        let inst = &INSTRUMENTS[i / knob::COUNT];
        return match i % knob::COUNT {
            knob::TUNE => format_hz(v),
            knob::ATTACK | knob::DECAY => format_seconds(v),
            knob::WAVE if voices::tonal(i / knob::COUNT) => match v {
                v if v < 0.01 => "sine".to_owned(),
                v if (v - 0.5).abs() < 0.01 => "triangle".to_owned(),
                v if v > 0.99 => "square".to_owned(),
                v if v < 0.5 => format!("→tri {:.0}%", v * 200.0),
                v => format!("→sqr {:.0}%", (v - 0.5) * 200.0),
            },
            knob::WAVE if v < 0.01 => "noise".to_owned(),
            knob::WAVE if v > 0.99 => "metal".to_owned(),
            knob::WAVE => format!("metal {:.0}%", v * 100.0),
            knob::BEND if v.abs() < 0.005 => "none".to_owned(),
            knob::BEND => format!("{:+.1} st", v * BEND_OCTAVES * 12.0),
            knob::DRIVE if v < 0.005 => "off".to_owned(),
            knob::LEVEL | knob::DRIVE => format!("{:.0}%", v * 100.0),
            knob::PAN if v.abs() < 0.01 => "C".to_owned(),
            knob::PAN if v < 0.0 => format!("L{:.0}", -v * 100.0),
            knob::PAN => format!("R{:.0}", v * 100.0),
            k => match inst.extras[k - knob::X1] {
                "TUNE2" | "HIGHP" | "RATE" => format_hz(v),
                "DECAY2" => format_seconds(v),
                _ => format!("{:.0}%", v * 100.0),
            },
        };
    }
    match i {
        TEMPO => format!("{:.0} BPM", v.round()),
        BALANCE if v.abs() < 0.01 => "even".to_owned(),
        BALANCE if v < 0.0 => format!("BD +{:.0}", -v * 100.0),
        BALANCE => format!("CYM +{:.0}", v * 100.0),
        _ => format!("{:.0}%", v * 100.0),
    }
}

/// The instrument a General MIDI drum note plays.
pub fn instrument_for(note: u8) -> Option<usize> {
    INSTRUMENTS.iter().position(|i| i.note == note).or(match note {
        35 => Some(voices::BASE),
        39 | 31 => Some(voices::SNARE2),
        44 | 46 => Some(voices::HIHAT),
        52 | 55 | 57 => Some(voices::CYMBAL1),
        53 | 59 => Some(voices::CYMBAL2),
        62 | 64 => Some(voices::CONGA),
        69 | 70 => Some(voices::TAMBOURINE),
        74 => Some(voices::GUIRO),
        76 | 77 => Some(voices::CLAVES),
        _ => None,
    })
}

/// A hit to play this block: frame, instrument, velocity, gate (s), and
/// whether it goes out on MIDI.
#[derive(Clone, Copy)]
struct Hit {
    frame: usize,
    inst: usize,
    velocity: f32,
    gate: f32,
    send: bool,
}

const MAX_HITS: usize = 128;
/// How long a pad (and the panel's preview) lets the guiro scrape.
const PAD_GATE: f32 = 0.25;
/// MIDI notes sent out last this long.
const NOTE_SECONDS: f32 = 0.03;

pub struct Pops {
    params: Arc<ModuleParams>,
    sr: f32,
    voices: [Voice; COUNT],
    rng: Rng,
    clock: StepClock,
    /// The beat on the machine's own clock.
    free_beat: f64,
    /// SYNC and RUNNING last block (a change restarts the clock).
    mode: (bool, bool),
    was_playing: bool,
    out: MidiBuf,
    /// The panel's pad counters as last seen.
    auditioned: [f32; COUNT],
    /// Frames until each sent note ends (−1: none pending).
    note_off: [i64; COUNT],
    hits: [u32; COUNT],
    volume: f32,
    balance: f32,
}

impl Pops {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        params.viz[viz::STEP].set(-1.0);
        Self {
            volume: params.get(VOLUME),
            balance: params.get(BALANCE),
            auditioned: std::array::from_fn(|i| params.viz[viz::AUDITION + i].get()),
            params,
            sr: sample_rate,
            voices: std::array::from_fn(Voice::new),
            rng: Rng::new(0x0066_1966),
            clock: StepClock::new(),
            free_beat: 0.0,
            mode: (true, false),
            was_playing: false,
            out: MidiBuf::default(),
            note_off: [-1; COUNT],
            hits: [0; COUNT],
        }
    }
}

/// Instrument `i`'s sound, as its knobs set it.
fn sound(p: &ModuleParams, i: usize) -> Sound {
    let v = |k: usize| p.get(ip(i, k));
    Sound {
        tune: v(knob::TUNE),
        attack: v(knob::ATTACK),
        decay: v(knob::DECAY),
        x: [v(knob::X1), v(knob::X1 + 1), v(knob::X1 + 2)],
        wave: v(knob::WAVE),
        bend: v(knob::BEND) * BEND_OCTAVES,
        drive: v(knob::DRIVE),
    }
}

/// For the panel: how long instrument `i`'s hit lasts, in seconds.
pub fn hit_seconds(p: &ModuleParams, i: usize) -> f32 {
    if !INSTRUMENTS[i].enveloped {
        return PAD_GATE + 0.05;
    }
    let v = |k: usize| p.get(ip(i, k));
    let decay = match i {
        voices::CONGA | voices::COWBELL => v(knob::DECAY).max(v(knob::X1 + 2)),
        _ => v(knob::DECAY),
    };
    (v(knob::ATTACK) + decay).clamp(0.02, 2.0)
}

/// For the panel: instrument `i` struck once, as its pad plays it (before
/// LEVEL and PAN), at `sr` into `out`.
pub fn render(p: &ModuleParams, i: usize, sr: f32, out: &mut [f32]) {
    let mut voice = Voice::new(i);
    let mut rng = Rng::new(0x0066_1966);
    voice.trigger(&sound(p, i), 1.0, PAD_GATE, sr);
    for y in out {
        *y = voice.tick(&mut rng);
    }
}

impl Processor for Pops {
    #[allow(clippy::needless_range_loop)] // both channels, sample by sample
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        self.out.clear();
        let p = self.params.clone();
        let mut hits = [Hit { frame: 0, inst: 0, velocity: 0.0, gate: 0.0, send: false }; MAX_HITS];
        let mut count = 0;
        let mut releases = [usize::MAX; 16];
        let mut released = 0;

        // MIDI notes play the instruments.
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                let [status, note, velocity] = e.data;
                let Some(inst) = instrument_for(note) else { continue };
                match status & 0xF0 {
                    0x90 if velocity > 0 && count < MAX_HITS => {
                        hits[count] = Hit { frame: e.frame as usize, inst, velocity: velocity as f32 / 127.0, gate: 4.0, send: false };
                        count += 1;
                    }
                    0x80 | 0x90 if released < releases.len() => {
                        releases[released] = inst;
                        released += 1;
                    }
                    _ => {}
                }
            }
        }

        // The panel's pads.
        for (inst, seen) in self.auditioned.iter_mut().enumerate() {
            let v = p.viz[viz::AUDITION + inst].get();
            if v != *seen && count < MAX_HITS {
                *seen = v;
                hits[count] = Hit { frame: 0, inst, velocity: 1.0, gate: PAD_GATE, send: false };
                count += 1;
            }
        }

        // The sequencer: on the board's clock (SYNC) or its own.
        let sync = on(&p, data::SYNC);
        let running = on(&p, data::RUNNING);
        let started = ctx.playing && !self.was_playing;
        self.was_playing = ctx.playing;
        if (sync, running) != self.mode || ctx.restart || (sync && started) {
            self.mode = (sync, running);
            self.clock = StepClock::new();
            if !running {
                self.free_beat = 0.0;
            }
        }
        let rhythm = if on(&p, data::POWER) { current_rt(&p) } else { None };
        let mut steps = [Step { frame: 0, beat: 0.0, index: 0 }; MAX_STEPS];
        let mut found = 0;
        let swing = p.get(SWING);
        let mut step_seconds = 0.1;
        if let Some(r) = &rhythm {
            let per_beat = r.per_beat as f64;
            if sync && ctx.playing {
                found = self.clock.steps(ctx, n, per_beat, swing, &mut steps);
                step_seconds = (1.0 / (per_beat * ctx.beats_per_sample * self.sr as f64)) as f32;
            } else if !sync && running {
                let bps = p.get(TEMPO).clamp(20.0, 300.0) as f64 / (60.0 * self.sr as f64);
                let own = ProcessCtx { beat: self.free_beat, beats_per_sample: bps, playing: true, restart: false, ..*ctx };
                found = self.clock.steps(&own, n, per_beat, swing, &mut steps);
                self.free_beat += bps * n as f64;
                step_seconds = (1.0 / (per_beat * bps * self.sr as f64)) as f32;
            }
        }
        if !sync && !running {
            self.free_beat = 0.0;
        }
        let playing = rhythm.is_some() && ((sync && ctx.playing) || (!sync && running));
        if !playing {
            p.viz[viz::STEP].set(-1.0);
        }
        if let Some(r) = &rhythm {
            p.viz[viz::LEN].set(r.len as f32);
            for step in &steps[..found] {
                let s = step.index.rem_euclid(r.len as i64) as usize;
                p.viz[viz::STEP].set(s as f32);
                for (inst, bits) in r.rows.iter().enumerate() {
                    if bits >> s & 1 == 1 && !on(&p, data::MUTE + inst) && count < MAX_HITS {
                        hits[count] = Hit { frame: step.frame, inst, velocity: 1.0, gate: step_seconds * 0.9, send: true };
                        count += 1;
                    }
                }
            }
        }
        hits[..count].sort_by_key(|h| h.frame);

        // MIDI out: the pattern's hits as short notes on channel 10.
        for (inst, off) in self.note_off.iter_mut().enumerate() {
            if *off >= 0 && *off < n as i64 {
                self.out.push(*off as usize, [0x89, INSTRUMENTS[inst].note, 0]);
                *off = -1;
            } else if *off >= 0 {
                *off -= n as i64;
            }
        }
        for h in hits[..count].iter().filter(|h| h.send) {
            let note = INSTRUMENTS[h.inst].note;
            if self.note_off[h.inst] >= 0 {
                self.out.push(h.frame, [0x89, note, 0]);
            }
            self.out.push(h.frame, [0x99, note, 100]);
            self.note_off[h.inst] = h.frame as i64 + (NOTE_SECONDS * self.sr) as i64;
        }

        // Sound.
        for &inst in &releases[..released] {
            self.voices[inst].release();
        }
        let sounds: [Sound; COUNT] = std::array::from_fn(|i| sound(&p, i));
        let gains: [(f32, f32); COUNT] = std::array::from_fn(|i| {
            let level = p.get(ip(i, knob::LEVEL));
            let (l, r) = pan_gains(p.get(ip(i, knob::PAN)));
            (l * level * std::f32::consts::SQRT_2, r * level * std::f32::consts::SQRT_2)
        });
        let (vol_t, bal_t) = (p.get(VOLUME), p.get(BALANCE));
        let mut next = 0;
        let mut peak = 0.0f32;
        let idle = count == 0 && self.voices.iter().all(|v| !v.is_active());
        for i in 0..n {
            while next < count && hits[next].frame <= i {
                let h = hits[next];
                self.voices[h.inst].trigger(&sounds[h.inst], h.velocity, h.gate, self.sr);
                self.hits[h.inst] = self.hits[h.inst].wrapping_add(1);
                next += 1;
            }
            if idle {
                outputs[0][0][i] = 0.0;
                outputs[0][1][i] = 0.0;
                continue;
            }
            approach(&mut self.volume, vol_t, ctx.smooth);
            approach(&mut self.balance, bal_t, ctx.smooth);
            let bass = (1.0 - self.balance).min(1.0);
            let cymbals = (1.0 + self.balance).min(1.0);
            let (mut l, mut r) = (0.0, 0.0);
            for (k, voice) in self.voices.iter_mut().enumerate() {
                if !voice.is_active() {
                    continue;
                }
                let mut y = voice.tick(&mut self.rng);
                y *= match k {
                    voices::BASE => bass,
                    voices::HIHAT | voices::CYMBAL1 | voices::CYMBAL2 => cymbals,
                    _ => 1.0,
                };
                l += y * gains[k].0;
                r += y * gains[k].1;
            }
            let yl = soft_limit(l * self.volume);
            let yr = soft_limit(r * self.volume);
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak = peak.max(yl.abs()).max(yr.abs());
        }
        for (k, h) in self.hits.iter().enumerate() {
            p.viz[viz::HITS + k].set(*h as f32);
        }
        p.meter.fetch_max(peak);
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}

/// For the panel: a fresh user pattern from what is playing.
pub fn copy_current_to_user(p: &ModuleParams, k: usize) {
    if let Some(r) = current(p) {
        set_user(p, k, &r);
    }
}
