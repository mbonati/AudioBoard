//! Fjord: a four-channel drum synthesizer after the Clavia Nord Drum (the
//! first one, 2012), with a step sequencer.
//!
//! Each channel blends Tone, Noise and Click (see [`voice`]) with the
//! Nord Drum's parameters and ranges: twenty-four per channel, the eight
//! "red" ones (filter type, sweep times, gates, punch, bend time) beside
//! the others, plus two envelopes of our own on the tone: an ADSR that can
//! shape its level instead of DECAY, and an ADSR on its filter (see
//! [`voice`]). Ninety-nine programs hold the four channels; they are kept
//! with the node, P1–P34 filled with our own factory sounds and the rest
//! empty, and can be overwritten with Store unless Memory Protect is on.
//! The knobs are the edit buffer: a program loads into them, the panel
//! shows when they differ from the stored program.
//!
//! The system settings follow the Nord Drum too: a MIDI channel (or Off,
//! or learned from the next message), a MIDI note per channel (General MIDI
//! 36, 38, 50, 47 to start, or learned), Local On/Off, program change sent
//! and/or received, and per channel the Trig Type, Dyn curve, threshold
//! and sensitivity of the Trig inputs, here four audio inputs (see
//! [`trigger`]). Pads can pick the channel to edit (FOLLOW), until one is
//! picked on the panel. Hits from the panel, the Trig inputs and the sequencer go
//! out as MIDI notes, as the pads' hits do on the hardware; with Local Off
//! they only go out.
//!
//! The sequencer has a lane per channel on the board's grid and transport:
//! up to 32 steps of a note value, each off, soft, medium or accented, held
//! for GATE. Notes are held (for the envelopes' sustain) until a MIDI
//! note-off, while TRIG is down, for a step's GATE or 80 ms from a Trig
//! input.
//! Each channel plays one strike at a time; a new strike fades the old one
//! out in 2 ms.

pub mod alea;
pub mod bank;
pub mod trigger;
pub mod voice;

use std::sync::Arc;

use super::steps::{MAX_STEPS, Step, StepClock, steps_per_beat};
use super::{ModuleDef, ModuleParams, soft_limit};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::params::{ParamSpec, Unit};
use bank::{FACTORY, INIT, PROGRAMS};
use trigger::{Detector, Settings};
use voice::{ALL_PARTS, Channel, Parts, Sound};

/// The four channels.
pub const CHANNELS: usize = 4;

/// A channel's knobs, in this order (see [`ch`]).
pub mod knob {
    pub const N_COLOR: usize = 0;
    pub const N_FILTER: usize = 1;
    /// Red: the noise filter's type (1…7, more resonance).
    pub const N_TYPE: usize = 2;
    pub const N_SWEEP: usize = 3;
    /// Red: the noise sweep's time.
    pub const N_TIME: usize = 4;
    pub const N_DECAY: usize = 5;
    /// Red: the noise decay's shape (n, S, G).
    pub const N_GATE: usize = 6;
    pub const T_WAVE: usize = 7;
    pub const T_FILTER: usize = 8;
    /// Red: a short, velocity-controlled attack.
    pub const T_PUNCH: usize = 9;
    pub const T_SWEEP: usize = 10;
    /// Red: the tone sweep's time.
    pub const T_TIME: usize = 11;
    pub const T_DECAY: usize = 12;
    /// Red: the tone decay's shape.
    pub const T_GATE: usize = 13;
    /// MIDI note number in half semitones (69 = A 440).
    pub const PITCH: usize = 14;
    /// −9…30: negative bends up to the pitch, positive down.
    pub const BEND: usize = 15;
    /// Red: the bend's time.
    pub const BEND_TIME: usize = 16;
    pub const C_TYPE: usize = 17;
    pub const C_DECAY: usize = 18;
    /// Red: the click decay's shape.
    pub const C_GATE: usize = 19;
    /// The mixer: tone, noise and click levels, the channel's level.
    pub const TONE: usize = 20;
    pub const NOISE: usize = 21;
    pub const CLICK: usize = 22;
    pub const LEVEL: usize = 23;
    /// Beyond the Nord Drum: the tone's amplitude follows its DECAY and
    /// GATE (0) or an ADSR (1), held as long as the note.
    pub const AMP_ENV: usize = 24;
    pub const AMP_A: usize = 25;
    pub const AMP_D: usize = 26;
    pub const AMP_S: usize = 27;
    pub const AMP_R: usize = 28;
    /// An ADSR on the tone filter, opening it by AMOUNT (−10…10: closing
    /// it when negative).
    pub const FLT_A: usize = 29;
    pub const FLT_D: usize = 30;
    pub const FLT_S: usize = 31;
    pub const FLT_R: usize = 32;
    pub const FLT_AMT: usize = 33;
    pub const COUNT: usize = 34;
}

/// Channel `c`'s knob `k`.
pub const fn ch(c: usize, k: usize) -> usize {
    c * knob::COUNT + k
}

/// Overall level (not stored in programs, as on the hardware).
pub const MASTER: usize = CHANNELS * knob::COUNT;
/// The sequencer's swing.
pub const SWING: usize = MASTER + 1;
pub const PARAM_COUNT: usize = SWING + 1;

/// Name, lowest, highest and how many steps per unit of each knob.
pub const RANGES: [(&str, f32, f32, f32); knob::COUNT] = [
    ("COLOR", -10.0, 10.0, 1.0),
    ("FILTER", 0.0, 99.0, 1.0),
    ("TYPE", 1.0, 7.0, 1.0),
    ("SWEEP", 0.0, 10.0, 1.0),
    ("TIME", 0.0, 9.0, 1.0),
    ("DECAY", 0.0, 50.0, 1.0),
    ("GATE", 0.0, 2.0, 1.0),
    ("WAVE", 0.0, 16.0, 1.0),
    ("FILTER", 0.0, 99.0, 1.0),
    ("PUNCH", 0.0, 9.0, 1.0),
    ("SWEEP", 0.0, 10.0, 1.0),
    ("TIME", 0.0, 9.0, 1.0),
    ("DECAY", 0.0, 50.0, 1.0),
    ("GATE", 0.0, 2.0, 1.0),
    ("PITCH", 11.0, 98.5, 2.0),
    ("BEND", -9.0, 30.0, 1.0),
    ("BEND TIME", 0.0, 9.0, 1.0),
    ("TYPE", 0.0, 26.0, 1.0),
    ("DECAY", 0.0, 20.0, 1.0),
    ("GATE", 0.0, 2.0, 1.0),
    ("TONE", 0.0, 10.0, 1.0),
    ("NOISE", 0.0, 10.0, 1.0),
    ("CLICK", 0.0, 10.0, 1.0),
    ("LEVEL", 0.0, 99.0, 1.0),
    ("AMP ENV", 0.0, 1.0, 1.0),
    ("A", 0.0, 50.0, 1.0),
    ("D", 0.0, 50.0, 1.0),
    ("S", 0.0, 99.0, 1.0),
    ("R", 0.0, 50.0, 1.0),
    ("A", 0.0, 50.0, 1.0),
    ("D", 0.0, 50.0, 1.0),
    ("S", 0.0, 99.0, 1.0),
    ("R", 0.0, 50.0, 1.0),
    ("AMOUNT", -10.0, 10.0, 1.0),
];

/// A knob's value on its own grid (whole numbers; the pitch in halves).
pub fn snap(k: usize, v: f32) -> f32 {
    let (_, lo, hi, per) = RANGES[k];
    ((v * per).round() / per).clamp(lo, hi)
}

macro_rules! channel_params {
    ($c:literal, $i:expr) => {{
        const D: Sound = bank::factory(0)[$i];
        const fn spec(name: &'static str, k: usize) -> ParamSpec {
            ParamSpec::lin(name, RANGES[k].1, RANGES[k].2, D[k], Unit::Integer)
        }
        [
            spec(concat!("CH", $c, " NOISE COLOR"), knob::N_COLOR).bipolar(),
            spec(concat!("CH", $c, " NOISE FILTER"), knob::N_FILTER),
            spec(concat!("CH", $c, " NOISE TYPE"), knob::N_TYPE),
            spec(concat!("CH", $c, " NOISE SWEEP"), knob::N_SWEEP),
            spec(concat!("CH", $c, " NOISE TIME"), knob::N_TIME),
            spec(concat!("CH", $c, " NOISE DECAY"), knob::N_DECAY),
            spec(concat!("CH", $c, " NOISE GATE"), knob::N_GATE),
            spec(concat!("CH", $c, " TONE WAVE"), knob::T_WAVE),
            spec(concat!("CH", $c, " TONE FILTER"), knob::T_FILTER),
            spec(concat!("CH", $c, " TONE PUNCH"), knob::T_PUNCH),
            spec(concat!("CH", $c, " TONE SWEEP"), knob::T_SWEEP),
            spec(concat!("CH", $c, " TONE TIME"), knob::T_TIME),
            spec(concat!("CH", $c, " TONE DECAY"), knob::T_DECAY),
            spec(concat!("CH", $c, " TONE GATE"), knob::T_GATE),
            ParamSpec::lin(concat!("CH", $c, " PITCH"), 11.0, 98.5, D[knob::PITCH], Unit::Note),
            spec(concat!("CH", $c, " BEND"), knob::BEND),
            spec(concat!("CH", $c, " BEND TIME"), knob::BEND_TIME),
            spec(concat!("CH", $c, " CLICK TYPE"), knob::C_TYPE),
            spec(concat!("CH", $c, " CLICK DECAY"), knob::C_DECAY),
            spec(concat!("CH", $c, " CLICK GATE"), knob::C_GATE),
            spec(concat!("CH", $c, " MIX TONE"), knob::TONE),
            spec(concat!("CH", $c, " MIX NOISE"), knob::NOISE),
            spec(concat!("CH", $c, " MIX CLICK"), knob::CLICK),
            spec(concat!("CH", $c, " LEVEL"), knob::LEVEL),
            spec(concat!("CH", $c, " AMP ENV"), knob::AMP_ENV),
            spec(concat!("CH", $c, " AMP A"), knob::AMP_A),
            spec(concat!("CH", $c, " AMP D"), knob::AMP_D),
            spec(concat!("CH", $c, " AMP S"), knob::AMP_S),
            spec(concat!("CH", $c, " AMP R"), knob::AMP_R),
            spec(concat!("CH", $c, " FILTER A"), knob::FLT_A),
            spec(concat!("CH", $c, " FILTER D"), knob::FLT_D),
            spec(concat!("CH", $c, " FILTER S"), knob::FLT_S),
            spec(concat!("CH", $c, " FILTER R"), knob::FLT_R),
            spec(concat!("CH", $c, " FILTER AMOUNT"), knob::FLT_AMT).bipolar(),
        ]
    }};
}

const PARAMS: [ParamSpec; PARAM_COUNT] = {
    const C: [[ParamSpec; knob::COUNT]; CHANNELS] = [channel_params!("1", 0), channel_params!("2", 1), channel_params!("3", 2), channel_params!("4", 3)];
    let mut out = [ParamSpec::lin("", 0.0, 1.0, 0.0, Unit::Percent); PARAM_COUNT];
    let mut c = 0;
    while c < CHANNELS {
        let mut k = 0;
        while k < knob::COUNT {
            out[ch(c, k)] = C[c][k];
            k += 1;
        }
        c += 1;
    }
    out[MASTER] = ParamSpec::lin("MASTER", 0.0, 1.0, 0.7, Unit::Percent);
    out[SWING] = ParamSpec::lin("SWING", 0.0, 1.0, 0.0, Unit::Percent);
    out
};

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// Sequencer steps at most.
pub const MAX_LEN: usize = 32;
/// A step's velocity: off, soft, medium, accent.
pub const STEP_VELOCITY: [f32; 4] = [0.0, 0.4, 0.72, 1.0];
/// Floats per program per channel: three settings packed in each.
const PACKED: usize = knob::COUNT.div_ceil(3);
/// Floats per program.
pub const PROGRAM_LEN: usize = CHANNELS * PACKED;

/// What `ModuleParams::data` holds besides the knobs.
pub mod data {
    use super::{CHANNELS, MAX_LEN, PROGRAM_LEN, PROGRAMS};
    /// The program loaded (0 = P1).
    pub const PROGRAM: usize = 0;
    /// The channel being edited (and soloed, and triggered by TRIG).
    pub const CHANNEL: usize = 1;
    /// The sequencer plays with the transport.
    pub const RUN: usize = 2;
    pub const LENGTH: usize = 3;
    /// Step length: an index into `DIVISIONS`.
    pub const RATE: usize = 4;
    /// 0 Off, 1…16.
    pub const MIDI_CH: usize = 5;
    pub const LOCAL: usize = 6;
    /// Program change: 0 Off, 1 Send, 2 Receive, 3 both.
    pub const PROG_CHANGE: usize = 7;
    pub const PROTECT: usize = 8;
    /// The panel's TRIG velocity (1…127).
    pub const TRIG_VEL: usize = 9;
    /// Solo Edit: 0 off, 1 tone, 2 noise, 3 click (the edited channel).
    pub const SOLO: usize = 10;
    /// Waiting to learn: 0 nothing, 1 the MIDI channel, 2…5 channel 1…4's
    /// note.
    pub const LEARN: usize = 11;
    /// MIDI notes and Trig input hits pick the edited channel (until one
    /// is picked on the panel), as pads do on the hardware.
    pub const FOLLOW: usize = 12;
    /// How long a step's note is held, in % of the step (the ADSRs'
    /// sustain, and the MIDI notes sent).
    pub const GATE: usize = 13;
    /// The panel's page: 0 SOUND, 1 SEQUENCER · MIDI.
    pub const TAB: usize = 14;
    /// How far an ALEA throw goes, in % (see `alea`).
    pub const CHANCE: usize = 15;
    /// Per channel: its MIDI note, Trig Type, Dyn curve, threshold,
    /// sensitivity, and its lane's mute.
    pub const NOTE: usize = 16;
    pub const TRIG_TYPE: usize = NOTE + CHANNELS;
    pub const DYN: usize = TRIG_TYPE + CHANNELS;
    pub const THRES: usize = DYN + CHANNELS;
    pub const SENS: usize = THRES + CHANNELS;
    pub const MUTE: usize = SENS + CHANNELS;
    /// The steps, lane by lane (0 off … 3 accent).
    pub const STEPS: usize = MUTE + CHANNELS;
    /// The programs, packed (see `pack`).
    pub const BANK: usize = STEPS + CHANNELS * MAX_LEN;
    pub const LEN: usize = BANK + PROGRAMS * PROGRAM_LEN;
}

/// MIDI notes the channels start on: General MIDI bass drum 1, snare 1,
/// high tom and low-mid tom.
pub const NOTES: [u8; CHANNELS] = [36, 38, 50, 47];

/// The pattern the sequencer starts with (0 off … 3 accent).
const PATTERN: [[u8; 16]; CHANNELS] = [
    [3, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 1, 2, 0, 0, 0],
    [0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 3, 0, 1, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2],
    [2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 1],
];

/// A setting as the byte it is stored as.
const fn encode(k: usize, v: f32) -> u32 {
    let x = match k {
        knob::N_COLOR | knob::FLT_AMT => v + 10.0,
        knob::BEND => v + 9.0,
        knob::PITCH => v * 2.0,
        _ => v,
    };
    let b = (x + 0.5) as i32;
    if b < 0 {
        0
    } else if b > 255 {
        255
    } else {
        b as u32
    }
}

const fn decode(k: usize, b: u32) -> f32 {
    let x = b as f32;
    match k {
        knob::N_COLOR | knob::FLT_AMT => x - 10.0,
        knob::BEND => x - 9.0,
        knob::PITCH => x * 0.5,
        _ => x,
    }
}

/// A channel's settings packed: three bytes in each float (exact up to
/// 2^24).
const fn pack(s: &Sound) -> [f32; PACKED] {
    let mut out = [0.0; PACKED];
    let mut i = 0;
    while i < PACKED {
        let mut b = 0;
        let mut j = 0;
        while j < 3 && 3 * i + j < knob::COUNT {
            b |= encode(3 * i + j, s[3 * i + j]) << (8 * j);
            j += 1;
        }
        out[i] = b as f32;
        i += 1;
    }
    out
}

fn unpack(packed: &[f32]) -> Sound {
    let mut s = [0.0; knob::COUNT];
    for (i, v) in packed.iter().take(PACKED).enumerate() {
        let b = v.max(0.0) as u32;
        for j in (0..3).filter(|j| 3 * i + j < knob::COUNT) {
            s[3 * i + j] = decode(3 * i + j, (b >> (8 * j)) & 0xFF);
        }
    }
    s
}

static DATA: [f32; data::LEN] = {
    let mut d = [0.0; data::LEN];
    d[data::RUN] = 1.0;
    d[data::LENGTH] = 16.0;
    d[data::RATE] = 3.0; // 1/16
    d[data::MIDI_CH] = 10.0;
    d[data::LOCAL] = 1.0;
    d[data::PROG_CHANGE] = 2.0;
    d[data::PROTECT] = 1.0;
    d[data::TRIG_VEL] = 100.0;
    d[data::GATE] = 50.0;
    d[data::CHANCE] = 60.0;
    let mut c = 0;
    while c < CHANNELS {
        d[data::NOTE + c] = NOTES[c] as f32;
        d[data::THRES + c] = 10.0;
        d[data::SENS + c] = 30.0;
        let mut s = 0;
        while s < 16 {
            d[data::STEPS + c * MAX_LEN + s] = PATTERN[c][s] as f32;
            s += 1;
        }
        c += 1;
    }
    let mut k = 0;
    while k < PROGRAMS {
        let sounds = bank::factory(k);
        let mut c = 0;
        while c < CHANNELS {
            let packed = pack(&sounds[c]);
            let mut i = 0;
            while i < PACKED {
                d[data::BANK + k * PROGRAM_LEN + c * PACKED + i] = packed[i];
                i += 1;
            }
            c += 1;
        }
        k += 1;
    }
    d
};

/// Layout of `ModuleParams::viz`.
pub mod viz {
    use super::CHANNELS;
    /// The step playing (−1: stopped).
    pub const STEP: usize = 0;
    /// Hits per channel so far (the panel flashes on a change).
    pub const HITS: usize = 1;
    /// The last hit's strength per channel (a Trig input's against its
    /// Sens: 1 or more is the hardest; others: their velocity).
    pub const STRENGTH: usize = HITS + CHANNELS;
    /// From the panel: bumped to strike a channel once (TRIG).
    pub const AUDITION: usize = STRENGTH + CHANNELS;
    /// From the panel: 1 while TRIG is held down (its note is held).
    pub const HOLD: usize = AUDITION + CHANNELS;
    pub const END: usize = HOLD + CHANNELS;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Fjord",
    prefix: "FJORD",
    hint: "4-channel drum synth · tone, noise, click · steps",
    inputs: &["TRIG 1", "TRIG 2", "TRIG 3", "TRIG 4"],
    outputs: &["OUT", "CH 1", "CH 2", "CH 3", "CH 4"],
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

// ---------------------------------------------------------------------------
// Settings, programs
// ---------------------------------------------------------------------------

/// A data value as a whole number in `lo..=hi`.
pub fn int(p: &ModuleParams, i: usize, lo: i32, hi: i32) -> i32 {
    (p.data[i].get().round() as i32).clamp(lo, hi)
}

pub fn on(p: &ModuleParams, i: usize) -> bool {
    p.data[i].get() > 0.5
}

/// The channel being edited.
pub fn selected(p: &ModuleParams) -> usize {
    int(p, data::CHANNEL, 0, CHANNELS as i32 - 1) as usize
}

pub fn program(p: &ModuleParams) -> usize {
    int(p, data::PROGRAM, 0, PROGRAMS as i32 - 1) as usize
}

/// Channel `c`'s settings as the knobs hold them.
pub fn sound(p: &ModuleParams, c: usize) -> Sound {
    std::array::from_fn(|k| p.get(ch(c, k)))
}

pub fn set_sound(p: &ModuleParams, c: usize, s: &Sound) {
    for (k, &v) in s.iter().enumerate() {
        p.set(ch(c, k), snap(k, v));
    }
}

fn slot_at(k: usize, c: usize) -> usize {
    data::BANK + k.min(PROGRAMS - 1) * PROGRAM_LEN + c * PACKED
}

/// Program `k` as stored.
pub fn stored(p: &ModuleParams, k: usize) -> [Sound; CHANNELS] {
    std::array::from_fn(|c| {
        let at = slot_at(k, c);
        let packed: [f32; PACKED] = std::array::from_fn(|i| p.data[at + i].get());
        unpack(&packed)
    })
}

/// Loads program `k` into the knobs.
pub fn load_program(p: &ModuleParams, k: usize) {
    let k = k.min(PROGRAMS - 1);
    for (c, s) in stored(p, k).iter().enumerate() {
        set_sound(p, c, s);
    }
    p.data[data::PROGRAM].set(k as f32);
}

/// Stores the knobs as program `k` (and makes it the current one). False
/// when Memory Protect is on.
pub fn store_program(p: &ModuleParams, k: usize) -> bool {
    if on(p, data::PROTECT) {
        return false;
    }
    let k = k.min(PROGRAMS - 1);
    for c in 0..CHANNELS {
        let packed = pack(&sound(p, c));
        for (i, v) in packed.iter().enumerate() {
            p.data[slot_at(k, c) + i].set(*v);
        }
    }
    p.data[data::PROGRAM].set(k as f32);
    true
}

/// The knobs differ from the stored program (the display's dot).
pub fn edited(p: &ModuleParams) -> bool {
    let k = program(p);
    (0..CHANNELS).any(|c| {
        let packed = pack(&sound(p, c));
        packed.iter().enumerate().any(|(i, v)| p.data[slot_at(k, c) + i].get() != *v)
    })
}

/// What program `k` holds: its factory name, "Empty" (Sound Init on every
/// channel) or "User".
pub fn program_name(p: &ModuleParams, k: usize) -> &'static str {
    let packed_now: [[f32; PACKED]; CHANNELS] = std::array::from_fn(|c| std::array::from_fn(|i| p.data[slot_at(k, c) + i].get()));
    let same = |sounds: &[Sound; CHANNELS]| (0..CHANNELS).all(|c| pack(&sounds[c]) == packed_now[c]);
    if k < FACTORY.len() && same(&FACTORY[k].channels) {
        FACTORY[k].name
    } else if same(&[INIT; CHANNELS]) {
        "Empty"
    } else {
        "User"
    }
}

/// Sound Init: channel `c` back to the plain starting sound.
pub fn init_channel(p: &ModuleParams, c: usize) {
    set_sound(p, c, &INIT);
}

/// Lane `c`'s step `s` (0 off … 3 accent).
pub fn step(p: &ModuleParams, c: usize, s: usize) -> usize {
    (p.data[data::STEPS + c.min(CHANNELS - 1) * MAX_LEN + s.min(MAX_LEN - 1)].get().round().max(0.0) as usize).min(3)
}

pub fn set_step(p: &ModuleParams, c: usize, s: usize, v: usize) {
    p.data[data::STEPS + c.min(CHANNELS - 1) * MAX_LEN + s.min(MAX_LEN - 1)].set(v.min(3) as f32);
}

pub fn length(p: &ModuleParams) -> usize {
    int(p, data::LENGTH, 1, MAX_LEN as i32) as usize
}

/// The receiving and sending channel (0-based), if not Off.
pub fn midi_channel(p: &ModuleParams) -> Option<u8> {
    let c = int(p, data::MIDI_CH, 0, 16);
    (c > 0).then(|| (c - 1) as u8)
}

/// Channel `c`'s MIDI note.
pub fn note(p: &ModuleParams, c: usize) -> u8 {
    int(p, data::NOTE + c, 0, 127) as u8
}

// ---------------------------------------------------------------------------
// The processor
// ---------------------------------------------------------------------------

/// A hit to play this block.
#[derive(Clone, Copy)]
struct Hit {
    frame: usize,
    channel: usize,
    velocity: f32,
    /// Played here (not from MIDI): it goes out on MIDI, and Local Off
    /// keeps it from the sound.
    local: bool,
    /// How long its note is held, in seconds, or who holds it.
    gate: Gate,
    /// A note let go rather than struck.
    release: bool,
}

/// How long a strike's note is held: for a time (a step, a Trig input) or
/// until a MIDI note-off or the panel's TRIG is let go.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Gate {
    For(f32),
    Key(u8),
    /// Samples since TRIG was pressed.
    Panel(u32),
}

impl Hit {
    fn strike(frame: usize, channel: usize, velocity: f32, local: bool, gate: Gate) -> Self {
        Hit { frame, channel, velocity, local, gate, release: false }
    }
}

const MAX_HITS: usize = 64;
/// MIDI notes sent out last this long (but a step's: its gate).
const NOTE_SECONDS: f32 = 0.03;
/// A Trig input's note is held this long.
const TRIG_IN_GATE: f32 = 0.08;
/// TRIG holds its note at least this long, however short the click.
const PANEL_GATE: f32 = 0.1;

pub struct Fjord {
    params: Arc<ModuleParams>,
    sr: f32,
    channels: [Channel; CHANNELS],
    detectors: [Detector; CHANNELS],
    clock: StepClock,
    was_playing: bool,
    was_running: bool,
    out: MidiBuf,
    /// The panel's TRIG counters as last seen.
    auditioned: [f32; CHANNELS],
    /// Frames until each sent note ends (−1: none pending), and its note.
    note_off: [i64; CHANNELS],
    sent: [u8; CHANNELS],
    hits: [u32; CHANNELS],
    /// Who holds each channel's note (None: nobody, or a timed gate).
    held: [Option<Gate>; CHANNELS],
    /// The program last seen (a change from the panel is sent out).
    program: usize,
    master: f32,
}

impl Fjord {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        params.viz[viz::STEP].set(-1.0);
        Self {
            channels: std::array::from_fn(|c| Channel::new(sample_rate, 0x00F1_0D00 + c as u32 * 7919, &sound(&params, c))),
            detectors: std::array::from_fn(|_| Detector::default()),
            auditioned: std::array::from_fn(|c| params.viz[viz::AUDITION + c].get()),
            program: program(&params),
            master: params.get(MASTER),
            params,
            sr: sample_rate,
            clock: StepClock::new(),
            was_playing: false,
            was_running: false,
            out: MidiBuf::default(),
            note_off: [-1; CHANNELS],
            sent: NOTES,
            hits: [0; CHANNELS],
            held: [None; CHANNELS],
        }
    }
}

impl Processor for Fjord {
    #[allow(clippy::needless_range_loop)] // channels and samples side by side
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        self.out.clear();
        let p = self.params.clone();
        let midi_ch = midi_channel(&p);
        let mut hits = [Hit::strike(0, 0, 0.0, false, Gate::For(0.0)); MAX_HITS];
        let mut count = 0;
        let mut push = |hits: &mut [Hit; MAX_HITS], hit: Hit| {
            if count < MAX_HITS {
                hits[count] = hit;
                count += 1;
            }
        };
        let curve = |c: usize| int(&p, data::DYN + c, 0, 1) as usize;

        // A program picked on the panel goes out as a program change.
        let prog = program(&p);
        if prog != self.program {
            self.program = prog;
            if let Some(mc) = midi_ch
                && matches!(int(&p, data::PROG_CHANGE, 0, 3), 1 | 3)
            {
                self.out.push(0, [0xC0 | mc, prog as u8, 0]);
            }
        }

        // MIDI in: notes strike the channels, program changes load programs.
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                let [status, d1, d2] = e.data;
                if !(0x80..0xF0).contains(&status) {
                    continue;
                }
                let channel = status & 0x0F;
                let learn = int(&p, data::LEARN, 0, 5);
                if learn == 1 {
                    p.data[data::MIDI_CH].set(channel as f32 + 1.0);
                    p.data[data::LEARN].set(0.0);
                }
                if midi_channel(&p) != Some(channel) {
                    continue;
                }
                match status & 0xF0 {
                    0x90 if d2 > 0 => {
                        if learn >= 2 {
                            p.data[data::NOTE + learn as usize - 2].set(d1 as f32);
                            p.data[data::LEARN].set(0.0);
                        }
                        for c in 0..CHANNELS {
                            if note(&p, c) == d1 {
                                let velocity = trigger::curve(d2 as f32 / 127.0, curve(c));
                                push(&mut hits, Hit::strike(e.frame as usize, c, velocity, false, Gate::Key(d1)));
                                if on(&p, data::FOLLOW) {
                                    p.data[data::CHANNEL].set(c as f32);
                                }
                            }
                        }
                    }
                    0x80 | 0x90 => {
                        for c in (0..CHANNELS).filter(|&c| note(&p, c) == d1) {
                            push(&mut hits, Hit { release: true, ..Hit::strike(e.frame as usize, c, 0.0, false, Gate::Key(d1)) });
                        }
                    }
                    0xC0 if matches!(int(&p, data::PROG_CHANGE, 0, 3), 2 | 3) && (d1 as usize) < PROGRAMS => {
                        load_program(&p, d1 as usize);
                        self.program = d1 as usize;
                    }
                    _ => {}
                }
            }
        }

        // The panel's TRIG.
        let trig_vel = int(&p, data::TRIG_VEL, 1, 127) as f32 / 127.0;
        for (c, seen) in self.auditioned.iter_mut().enumerate() {
            let v = p.viz[viz::AUDITION + c].get();
            if v != *seen {
                *seen = v;
                push(&mut hits, Hit::strike(0, c, trig_vel, true, Gate::Panel(0)));
            }
        }

        // The Trig inputs.
        let mut strength = [-1.0f32; CHANNELS];
        for c in 0..CHANNELS {
            let Some(buf) = inputs.get(c) else { continue };
            let s = Settings::new(
                int(&p, data::TRIG_TYPE + c, 0, 5) as usize,
                curve(c),
                p.data[data::THRES + c].get(),
                p.data[data::SENS + c].get(),
                self.sr,
            );
            for i in 0..n {
                let x = buf[0][i].abs().max(buf[1][i].abs());
                if let Some(h) = self.detectors[c].tick(x, &s) {
                    push(&mut hits, Hit::strike(i, c, h.velocity, true, Gate::For(TRIG_IN_GATE)));
                    strength[c] = h.strength;
                    if on(&p, data::FOLLOW) {
                        p.data[data::CHANNEL].set(c as f32);
                    }
                }
            }
        }

        // The sequencer, on the board's grid while the transport plays.
        let running = on(&p, data::RUN);
        let started = ctx.playing && !self.was_playing;
        if ctx.restart || started || (running && !self.was_running) {
            self.clock = StepClock::new();
        }
        self.was_playing = ctx.playing;
        self.was_running = running;
        if running && ctx.playing {
            let mut steps = [Step { frame: 0, beat: 0.0, index: 0 }; MAX_STEPS];
            let per_beat = steps_per_beat(p.data[data::RATE].get());
            let found = self.clock.steps(ctx, n, per_beat, p.get(SWING), &mut steps);
            let len = length(&p);
            let step_seconds = (1.0 / (per_beat * ctx.beats_per_sample * self.sr as f64)) as f32;
            let gate = step_seconds * p.data[data::GATE].get().clamp(5.0, 100.0) / 100.0;
            for st in &steps[..found] {
                let s = st.index.rem_euclid(len as i64) as usize;
                p.viz[viz::STEP].set(s as f32);
                for c in 0..CHANNELS {
                    let v = step(&p, c, s);
                    if v > 0 && !on(&p, data::MUTE + c) {
                        push(&mut hits, Hit::strike(st.frame, c, STEP_VELOCITY[v], true, Gate::For(gate)));
                    }
                }
            }
        } else {
            p.viz[viz::STEP].set(-1.0);
        }
        hits[..count].sort_by_key(|h| h.frame);

        // MIDI out: the local hits as short notes, as pads send them.
        // Pending note-offs count from the last block's start: from this
        // one's now.
        for (c, off) in self.note_off.iter_mut().enumerate() {
            if *off >= 0 {
                *off -= n as i64;
                if *off < n as i64 {
                    if let Some(mc) = midi_ch {
                        self.out.push((*off).max(0) as usize, [0x80 | mc, self.sent[c], 0]);
                    }
                    *off = -1;
                }
            }
        }
        if let Some(mc) = midi_ch {
            for h in hits[..count].iter().filter(|h| h.local && !h.release) {
                let c = h.channel;
                if self.note_off[c] >= 0 {
                    self.out.push(h.frame, [0x80 | mc, self.sent[c], 0]);
                }
                self.sent[c] = note(&p, c);
                self.out.push(h.frame, [0x90 | mc, self.sent[c], ((h.velocity * 127.0).round() as u8).clamp(1, 127)]);
                let length = if let Gate::For(g) = h.gate { g } else { NOTE_SECONDS };
                self.note_off[c] = h.frame as i64 + ((length * self.sr) as i64).max(1);
            }
            // Notes so short they end in this block.
            for (c, off) in self.note_off.iter_mut().enumerate() {
                if *off >= 0 && *off < n as i64 {
                    self.out.push(*off as usize, [0x80 | mc, self.sent[c], 0]);
                    *off = -1;
                }
            }
        }

        // Sound.
        let local = on(&p, data::LOCAL);
        let sounds: [Sound; CHANNELS] = std::array::from_fn(|c| sound(&p, c));
        let solo = int(&p, data::SOLO, 0, 3) as usize;
        let edited = selected(&p);
        let parts: [Parts; CHANNELS] = std::array::from_fn(|c| if solo > 0 && c == edited { std::array::from_fn(|i| i + 1 == solo) } else { ALL_PARTS });
        for (c, channel) in self.channels.iter_mut().enumerate() {
            channel.prepare(&sounds[c], parts[c]);
        }
        // TRIG let go (after its shortest hold): its note ends.
        for c in 0..CHANNELS {
            if let Some(Gate::Panel(t)) = &mut self.held[c] {
                *t = t.saturating_add(n as u32);
                if p.viz[viz::HOLD + c].get() < 0.5 && *t as f32 >= PANEL_GATE * self.sr {
                    self.channels[c].release();
                    self.held[c] = None;
                }
            }
        }
        let master_t = p.get(MASTER);
        let idle = count == 0 && self.channels.iter().all(|c| !c.active());
        let mut next = 0;
        let mut peak = 0.0f32;
        for i in 0..n {
            while next < count && hits[next].frame <= i {
                let h = hits[next];
                next += 1;
                if h.release {
                    if self.held[h.channel] == Some(h.gate) {
                        self.channels[h.channel].release();
                        self.held[h.channel] = None;
                    }
                    continue;
                }
                self.hits[h.channel] = self.hits[h.channel].wrapping_add(1);
                if strength[h.channel] < 0.0 || !h.local {
                    strength[h.channel] = h.velocity;
                }
                if h.local && !local {
                    continue;
                }
                let (gate, held) = match h.gate {
                    Gate::For(g) => (Some(g), None),
                    holder => (None, Some(holder)),
                };
                self.held[h.channel] = held;
                self.channels[h.channel].trigger(&sounds[h.channel], h.velocity, parts[h.channel], gate);
            }
            if idle {
                for out in outputs.iter_mut() {
                    out[0][i] = 0.0;
                    out[1][i] = 0.0;
                }
                continue;
            }
            self.master += (master_t - self.master) * ctx.smooth;
            let mut mix = 0.0;
            for c in 0..CHANNELS {
                let y = self.channels[c].tick(&sounds[c], ctx.smooth);
                mix += y;
                if let Some(out) = outputs.get_mut(1 + c) {
                    let y = soft_limit(y);
                    out[0][i] = y;
                    out[1][i] = y;
                }
            }
            let y = soft_limit(mix * self.master);
            outputs[0][0][i] = y;
            outputs[0][1][i] = y;
            peak = peak.max(y.abs());
        }
        for c in 0..CHANNELS {
            p.viz[viz::HITS + c].set(self.hits[c] as f32);
            if strength[c] >= 0.0 {
                p.viz[viz::STRENGTH + c].set(strength[c]);
            }
        }
        p.meter.fetch_max(peak);
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}

/// The step lengths the sequencer offers, as `DIVISIONS` indices.
pub const RATES: [usize; 4] = [1, 3, 4, 5];
