//! MIDIcer: a stochastic MIDI sequencer after Vermona's meloDICER. Random
//! values turn into notes only through the settings: the rhythm section
//! (NOTE value, VARIATION, LEGATO, REST) decides when notes start and how
//! long they last, the melody section (LOW…HIGH and a probability for each
//! semitone) which notes they are.
//!
//! DICE keeps one set of random values per section, so the pattern repeats
//! (DICE again for new ones); LIVE draws new values all the time, and going
//! back to DICE brings the kept values back. LOCK freezes the settings in
//! use while new ones are prepared. FIRST…LAST sets the pattern's length in
//! sixteenths (up to four beats). Everything runs on the global tempo.
//!
//! Modes: SEQ (the sequencer), SEQ+G (the rhythm comes from notes on the
//! MIDI input), QUANT1 (CV2 quantized to the enabled notes on every beat)
//! and QUANT2 (CV2 quantized while a note is held on the MIDI input).
//! In SEQ mode, notes on the MIDI input are commands, like meloDICER's gate
//! inputs: C restarts, D and E dice rhythm and melody anew, F and G switch
//! rhythm and melody between DICE and LIVE, and A mutes while held.

use std::sync::Arc;

use super::steps::{Held, note_event};
use super::{ModuleDef, ModuleParams, Rng};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{NOTE_VALUES, ParamSpec, Unit};

pub const NOTE: usize = 0;
pub const VARIATION: usize = 1;
pub const LEGATO: usize = 2;
pub const REST: usize = 3;
pub const LOW: usize = 4;
pub const HIGH: usize = 5;
pub const FIRST: usize = 6;
pub const LAST: usize = 7;
pub const GATE: usize = 8;
pub const VELOCITY: usize = 9;
pub const DYNAMICS: usize = 10;
pub const CHANNEL: usize = 11;

const PARAMS: [ParamSpec; 12] = [
    ParamSpec::lin("NOTE", 0.0, 8.0, 6.0, Unit::NoteValue),
    ParamSpec::lin("VARIATION", -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
    ParamSpec::lin("LEGATO", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("REST", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("LOW", 24.0, 96.0, 48.0, Unit::Note),
    ParamSpec::lin("HIGH", 24.0, 96.0, 60.0, Unit::Note),
    ParamSpec::lin("FIRST", 1.0, 16.0, 1.0, Unit::Integer),
    ParamSpec::lin("LAST", 1.0, 16.0, 16.0, Unit::Integer),
    ParamSpec::lin("GATE", 0.05, 1.0, 0.8, Unit::Percent),
    ParamSpec::lin("VELOCITY", 1.0, 127.0, 100.0, Unit::Integer),
    ParamSpec::lin("DYNAMICS", 0.0, 1.0, 0.2, Unit::Percent),
    ParamSpec::lin("CHANNEL", 1.0, 16.0, 1.0, Unit::Channel),
];

pub const SEQ: usize = 0;
pub const SEQ_GATE: usize = 1;
pub const QUANT1: usize = 2;
pub const QUANT2: usize = 3;

pub const MODE_NAMES: [&str; 4] = [
    "Sequencer",
    "Sequencer + gate: the rhythm comes from the MIDI input",
    "Quantizer 1: CV2 quantized on every beat",
    "Quantizer 2: CV2 quantized while a MIDI note is held",
];

/// What `ModuleParams::data` holds besides the knobs.
pub mod data {
    /// Probability of each semitone, C to B (12 values).
    pub const WEIGHTS: usize = 0;
    /// 1: DICE (the pattern repeats); 0: LIVE (new values all the time).
    pub const DICE_R: usize = 12;
    pub const DICE_M: usize = 13;
    pub const LOCK: usize = 14;
    pub const MUTE: usize = 15;
    /// VARIATION may pick triplets / thirty-seconds.
    pub const TRIPLETS: usize = 16;
    pub const THIRTY_SECOND: usize = 17;
    /// Leaving mute restarts the pattern on the next beat.
    pub const UNMUTE_RESTART: usize = 18;
    /// The kept random values, as seeds.
    pub const SEED_R: usize = 19;
    pub const SEED_M: usize = 20;
    /// Bumped to restart the pattern on the next beat.
    pub const RESTART: usize = 21;
    /// CV1 on pitch: 0 off, 1 BEND (pitch bend, unquantized), 2 TRANSPOSE.
    pub const CV1_PITCH: usize = 22;
    /// CV1 also moves LOW / HIGH.
    pub const CV1_LOW: usize = 23;
    pub const CV1_HIGH: usize = 24;
    /// CV2 adds to NOTE / VARIATION / LEGATO / REST.
    pub const CV2_NOTE: usize = 25;
    pub const CV2_VAR: usize = 26;
    pub const CV2_LEGATO: usize = 27;
    pub const CV2_REST: usize = 28;
    pub const LEN: usize = 29;
}

const DATA: [f32; data::LEN] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, // only C, as on meloDICER
    1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, // dice, dice, unlocked, playing, no triplets/32nds
    1.0, 2.0, 0.0, // seeds, restarts
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, // CV routing off
];

/// `viz[CURRENT]`: the note sounding, or −1.
pub const CURRENT: usize = 0;
/// `viz[STEP]`: the sixteenth playing (0…15), or −1.
pub const STEP: usize = 1;

pub static DEF: ModuleDef = ModuleDef {
    name: "MIDIcer",
    prefix: "MIDICER",
    hint: "dice rhythm & melody → MIDI",
    inputs: &["CV1", "CV2"],
    outputs: &[],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["SEQ", "SEQ+G", "QUANT1", "QUANT2"],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: true,
    midi_out: true,
    data: &DATA,
    display_h: 96.0,
    controls_h: 46.0,
    viz: 2,
};

/// Random values kept per note of the pattern.
const SLOTS: usize = 64;
/// Semitones CV1 and CV2 move pitches and ranges by at ±1.
const CV_SEMITONES: f32 = 24.0;

/// The settings a pattern is made from, as read at one moment (LOCK keeps
/// one copy while the knobs move).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub mode: usize,
    pub note: usize,
    pub variation: f32,
    pub legato: f32,
    pub rest: f32,
    pub low: i32,
    pub high: i32,
    pub first: usize,
    pub last: usize,
    pub gate: f32,
    pub velocity: f32,
    pub dynamics: f32,
    pub channel: u8,
    pub weights: [f32; 12],
    pub triplets: bool,
    pub thirty_second: bool,
}

impl Settings {
    pub fn read(p: &ModuleParams) -> Self {
        let on = |i: usize| p.data[i].get() > 0.5;
        let first = (p.get(FIRST).round() as usize).clamp(1, 16);
        Self {
            mode: p.mode().min(QUANT2),
            note: (p.get(NOTE).round().max(0.0) as usize).min(NOTE_VALUES.len() - 1),
            variation: p.get(VARIATION).clamp(-1.0, 1.0),
            legato: p.get(LEGATO).clamp(0.0, 1.0),
            rest: p.get(REST).clamp(0.0, 1.0),
            low: p.get(LOW).round() as i32,
            high: p.get(HIGH).round() as i32,
            first,
            last: (p.get(LAST).round() as usize).clamp(first, 16),
            gate: p.get(GATE).clamp(0.01, 1.0),
            velocity: p.get(VELOCITY).clamp(1.0, 127.0),
            dynamics: p.get(DYNAMICS).clamp(0.0, 1.0),
            channel: (p.get(CHANNEL).round() as u8).clamp(1, 16) - 1,
            weights: std::array::from_fn(|i| p.data[data::WEIGHTS + i].get().clamp(0.0, 1.0)),
            triplets: on(data::TRIPLETS),
            thirty_second: on(data::THIRTY_SECOND),
        }
    }

    /// The pattern's length in beats (FIRST…LAST sixteenths).
    pub fn pattern_beats(&self) -> f64 {
        (self.last + 1 - self.first) as f64 * 0.25
    }

    /// The note range, low to high.
    fn range(&self) -> (i32, i32) {
        (self.low.min(self.high).clamp(0, 127), self.low.max(self.high).clamp(0, 127))
    }
}

/// A note's length in beats: NOTE, or with probability |VARIATION| a value
/// one to three steps longer (VARIATION < 0) or shorter (> 0) among those
/// VARIATION may use. `r_var` and `r_amount` are random values in [0, 1).
pub fn note_length(s: &Settings, r_var: f32, r_amount: f32) -> f64 {
    let base = s.note;
    let allowed = |i: usize| {
        let (_, _, triplet, thirty_second) = NOTE_VALUES[i];
        i == base || ((!triplet || s.triplets) && (!thirty_second || s.thirty_second))
    };
    // The values VARIATION may use, longest first (no allocation: this runs
    // on the audio thread).
    let mut values = [0usize; NOTE_VALUES.len()];
    let mut len = 0;
    for i in (0..NOTE_VALUES.len()).filter(|&i| allowed(i)) {
        values[len] = i;
        len += 1;
    }
    let values = &values[..len];
    let at = values.iter().position(|&i| i == base).unwrap_or(0);
    let amount = s.variation.abs();
    let index = if r_var < amount {
        let steps = 1 + ((r_amount * 3.0 * amount) as usize).min(2);
        if s.variation < 0.0 { at.saturating_sub(steps) } else { (at + steps).min(values.len() - 1) }
    } else {
        at
    };
    NOTE_VALUES[values[index]].1
}

/// The note for a random value: every note in LOW…HIGH whose semitone is
/// enabled, each as likely as its semitone's probability. None if none is.
pub fn pick_note(s: &Settings, r: f32) -> Option<i32> {
    let (lo, hi) = s.range();
    let weight = |n: i32| s.weights[n.rem_euclid(12) as usize];
    let total: f32 = (lo..=hi).map(weight).sum();
    if total <= 0.0 {
        return None;
    }
    let target = r.clamp(0.0, 1.0) * total;
    let mut sum = 0.0;
    let mut last = None;
    for n in lo..=hi {
        let w = weight(n);
        if w > 0.0 {
            sum += w;
            last = Some(n);
            if sum > target {
                return last;
            }
        }
    }
    last
}

/// Quantizes a pitch (semitones, fractional) to the enabled notes in
/// LOW…HIGH. A raised semitone pulls from further away: the distance to
/// each note is divided by its probability.
pub fn quantize(s: &Settings, x: f32) -> Option<i32> {
    let (lo, hi) = s.range();
    (lo..=hi)
        .filter_map(|n| {
            let w = s.weights[n.rem_euclid(12) as usize];
            (w > 0.0).then(|| (n, (x - n as f32).abs() / w))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(n, _)| n)
}

/// Random values for one pattern: per note, rhythm (variation, amount,
/// rest, legato) and melody (pitch, velocity).
fn table<const N: usize>(seed: f32, salt: u32) -> [[f32; N]; SLOTS] {
    let mut rng = Rng::new((seed as u32).wrapping_mul(2_654_435_761) ^ salt);
    std::array::from_fn(|_| std::array::from_fn(|_| rng.unit()))
}

/// A new seed for DICE (from the UI or a MIDI command).
pub fn new_seed(from: u32) -> f32 {
    let mut rng = Rng::new(from.wrapping_mul(747_796_405).wrapping_add(2_891_336_453));
    // Kept exact as an f32 (and so in project files).
    (1.0 + rng.unit() * 16_000_000.0).floor()
}

/// The note sounding, and the beat it ends on (None: it runs on into the
/// next note, legato).
#[derive(Clone, Copy)]
struct Sounding {
    note: u8,
    off: Option<f64>,
}

pub struct Midicer {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    cursor: u32,
    out: MidiBuf,
    rhythm: [[f32; 4]; SLOTS],
    melody: [[f32; 2]; SLOTS],
    seeds: (f32, f32),
    live: Rng,
    /// The settings held while LOCK is on.
    locked: Option<Settings>,
    mode: usize,
    /// Pattern clock: where it began, the next note's start and its index.
    pattern_start: f64,
    next_at: f64,
    index: usize,
    /// A restart waiting for the next beat.
    restart_at: Option<f64>,
    restarts_seen: f32,
    started: bool,
    sounding: Option<Sounding>,
    muted: bool,
    /// Muted from the MIDI input (A held).
    midi_mute: bool,
    /// Input notes held (SEQ+G and QUANT2), and the note each one plays.
    held: Held,
    plays: [Option<u8>; 128],
    /// The next beat QUANT1 samples on.
    quant_at: f64,
    bend: Option<i32>,
    cable: bool,
}

impl Midicer {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>) -> Self {
        let seeds = (params.data[data::SEED_R].get(), params.data[data::SEED_M].get());
        Self {
            restarts_seen: params.data[data::RESTART].get(),
            cursor: midi.stream_head(),
            midi,
            params,
            out: MidiBuf::default(),
            rhythm: table(seeds.0, 0x5157_0001),
            melody: table(seeds.1, 0x3E10_0D1E),
            seeds,
            live: Rng::new(0x11FE_CAFE),
            locked: None,
            mode: SEQ,
            pattern_start: 0.0,
            next_at: f64::INFINITY,
            index: 0,
            restart_at: None,
            started: false,
            sounding: None,
            muted: false,
            midi_mute: false,
            held: Held::new(),
            plays: [None; 128],
            quant_at: 0.0,
            bend: None,
            cable: false,
        }
    }

    fn on(&mut self, frame: usize, ch: u8, note: u8, velocity: u8) {
        self.out.push(frame, [0x90 | ch, note, velocity.max(1)]);
    }

    fn off(&mut self, frame: usize, ch: u8, note: u8) {
        self.out.push(frame, [0x80 | ch, note, 0]);
    }

    /// Ends the note sounding, if any.
    fn release(&mut self, frame: usize, ch: u8) {
        if let Some(s) = self.sounding.take() {
            self.off(frame, ch, s.note);
        }
    }

    /// Plays `note` (or a rest), tying or overlapping a legato note before it.
    fn play(&mut self, frame: usize, ch: u8, note: Option<u8>, velocity: u8, off: Option<f64>) {
        match (self.sounding.take(), note) {
            // Legato into the same pitch: a tie, no new note.
            (Some(cur), Some(n)) if cur.off.is_none() && cur.note == n => {}
            // Legato into another: the new note starts before the old ends.
            (Some(cur), Some(n)) => {
                self.on(frame, ch, n, velocity);
                self.off(frame, ch, cur.note);
            }
            (Some(cur), None) => self.off(frame, ch, cur.note),
            (None, Some(n)) => self.on(frame, ch, n, velocity),
            (None, None) => {}
        }
        self.sounding = note.map(|n| Sounding { note: n, off });
    }

    fn rhythm_values(&mut self, k: usize, dice: bool) -> [f32; 4] {
        if dice { self.rhythm[k % SLOTS] } else { std::array::from_fn(|_| self.live.unit()) }
    }

    fn melody_values(&mut self, k: usize, dice: bool) -> [f32; 2] {
        if dice { self.melody[k % SLOTS] } else { std::array::from_fn(|_| self.live.unit()) }
    }

    fn velocity(s: &Settings, r: f32) -> u8 {
        (s.velocity * (1.0 - s.dynamics * r)).round().clamp(1.0, 127.0) as u8
    }

    /// SEQ: the pattern, note by note, in this block.
    fn sequence(&mut self, ctx: &ProcessCtx, n: usize, s: &Settings, transpose: i32, muted: bool) {
        let p = self.params.clone();
        let dice = (p.data[data::DICE_R].get() > 0.5, p.data[data::DICE_M].get() > 0.5);
        let end = block_end(ctx, n);
        loop {
            let off_at = self.sounding.and_then(|s| s.off).unwrap_or(f64::INFINITY);
            let restart = self.restart_at.filter(|&r| r < self.next_at);
            let boundary = restart.unwrap_or(self.next_at);
            let t = off_at.min(boundary);
            if t >= end {
                break;
            }
            let frame = frame_of(ctx, n, t);
            if off_at <= boundary {
                self.release(frame, s.channel);
                continue;
            }
            // Never play catch-up: a pattern that fell behind starts over.
            if t < ctx.beat - 0.5 {
                self.next_at = f64::INFINITY;
                self.restart_at = Some(ctx.beat.ceil());
                continue;
            }
            if restart.is_some() {
                self.restart_at = None;
                self.pattern_start = t;
                self.index = 0;
            }
            // Wrap at the pattern's end (also when it was just shortened).
            let length = s.pattern_beats();
            if t >= self.pattern_start + length - 1e-9 {
                self.pattern_start = t;
                self.index = 0;
            }
            let k = self.index;
            self.index += 1;
            let r = self.rhythm_values(k, dice.0);
            let m = self.melody_values(k, dice.1);
            let len = note_length(s, r[0], r[1]).min(self.pattern_start + length - t);
            self.next_at = t + len;
            let rest = muted || r[2] < s.rest;
            let note = if rest { None } else { pick_note(s, m[0]).map(|n| (n + transpose).clamp(0, 127) as u8) };
            let legato = r[3] < s.legato;
            let off = (!legato).then(|| t + (len * s.gate as f64).max(0.01));
            self.play(frame, s.channel, note, Self::velocity(s, m[1]), off);
        }
    }

    /// Notes on the MIDI input: commands in SEQ mode, the gate otherwise.
    fn input(&mut self, frame: usize, msg: [u8; 3], s: &Settings, transpose: i32) {
        let key = msg[1] & 0x7F;
        let Some(down) = note_event(msg) else { return };
        let p = self.params.clone();
        match s.mode {
            SEQ => match (key % 12, down) {
                (0, true) => self.restart_at = Some(f64::NEG_INFINITY),
                (2, true) => p.data[data::SEED_R].set(new_seed(self.live.unit().to_bits())),
                (4, true) => p.data[data::SEED_M].set(new_seed(self.live.unit().to_bits())),
                (5, true) => p.data[data::DICE_R].set(if p.data[data::DICE_R].get() > 0.5 { 0.0 } else { 1.0 }),
                (7, true) => p.data[data::DICE_M].set(if p.data[data::DICE_M].get() > 0.5 { 0.0 } else { 1.0 }),
                (9, down) => self.midi_mute = down,
                _ => {}
            },
            SEQ_GATE => {
                if down {
                    self.held.press(key);
                    let dice = (p.data[data::DICE_R].get() > 0.5, p.data[data::DICE_M].get() > 0.5);
                    // The random values repeat every FIRST…LAST sixteenths' worth of notes.
                    let k = self.index % (s.last + 1 - s.first);
                    self.index += 1;
                    let r = self.rhythm_values(k, dice.0);
                    let m = self.melody_values(k, dice.1);
                    let note = if self.muted || r[2] < s.rest {
                        None
                    } else {
                        pick_note(s, m[0]).map(|n| (n + transpose).clamp(0, 127) as u8)
                    };
                    let legato = r[3] < s.legato && self.sounding.is_some();
                    if !legato {
                        self.release(frame, s.channel);
                    }
                    let velocity = if self.cable { msg[2] } else { Self::velocity(s, m[1]) };
                    self.play(frame, s.channel, note, velocity, None);
                    self.plays[key as usize] = note;
                } else {
                    self.held.release(key);
                    let played = self.plays[key as usize].take();
                    if played.is_some() && self.sounding.map(|s| s.note) == played {
                        self.release(frame, s.channel);
                    }
                }
            }
            _ => {
                if down {
                    self.held.press(key);
                } else {
                    self.held.release(key);
                }
            }
        }
    }

    /// QUANT1: CV2 quantized on every beat.
    fn quantize_beats(&mut self, ctx: &ProcessCtx, n: usize, s: &Settings, cv: Option<&[f32; 64]>, muted: bool) {
        let end = block_end(ctx, n);
        if self.quant_at < ctx.beat - 1.0 {
            self.quant_at = ctx.beat.ceil();
        }
        loop {
            let off_at = self.sounding.and_then(|s| s.off).unwrap_or(f64::INFINITY);
            let t = off_at.min(self.quant_at);
            if t >= end {
                break;
            }
            let frame = frame_of(ctx, n, t);
            if off_at <= self.quant_at {
                self.release(frame, s.channel);
                continue;
            }
            self.quant_at += 1.0;
            let note = match cv {
                Some(cv) if !muted => quantize(s, cv_pitch(s, cv[frame])).map(|n| n.clamp(0, 127) as u8),
                _ => None,
            };
            self.release(frame, s.channel);
            let velocity = Self::velocity(s, self.live.unit());
            self.play(frame, s.channel, note, velocity, Some(t + s.gate as f64));
        }
    }

    /// QUANT2: CV2 quantized while a note is held on the MIDI input.
    fn quantize_gate(&mut self, s: &Settings, cv: Option<&[f32; 64]>, muted: bool) {
        let gate = !self.held.is_empty() && !muted;
        let note = match cv {
            Some(cv) if gate => quantize(s, cv_pitch(s, cv[0])).map(|n| n.clamp(0, 127) as u8),
            _ => None,
        };
        if note.is_none() {
            self.release(0, s.channel);
        } else if note != self.sounding.map(|s| s.note) {
            // The held gate glides between quantized notes, legato.
            if let Some(cur) = &mut self.sounding {
                cur.off = None;
            }
            let velocity = Self::velocity(s, self.live.unit());
            self.play(0, s.channel, note, velocity, None);
        }
    }
}

/// A CV (−1…1) as a pitch across LOW…HIGH.
fn cv_pitch(s: &Settings, cv: f32) -> f32 {
    let (lo, hi) = s.range();
    lo as f32 + (cv.clamp(-1.0, 1.0) + 1.0) * 0.5 * (hi - lo) as f32
}

/// The beat where this block's last frame stops being nearest.
fn block_end(ctx: &ProcessCtx, n: usize) -> f64 {
    ctx.beat + ctx.beats_per_sample * (n as f64 - 0.5)
}

/// The nearest frame in this block for a beat.
fn frame_of(ctx: &ProcessCtx, n: usize, beat: f64) -> usize {
    ((((beat - ctx.beat) / ctx.beats_per_sample).round().max(0.0)) as usize).min(n.saturating_sub(1))
}

impl Processor for Midicer {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, _outputs: &mut [Buf], n: usize) {
        self.out.clear();
        let p = self.params.clone();

        // New seeds (DICE pressed): new kept values.
        let seeds = (p.data[data::SEED_R].get(), p.data[data::SEED_M].get());
        if seeds.0 != self.seeds.0 {
            self.rhythm = table(seeds.0, 0x5157_0001);
        }
        if seeds.1 != self.seeds.1 {
            self.melody = table(seeds.1, 0x3E10_0D1E);
        }
        self.seeds = seeds;

        // LOCK keeps the settings it found.
        let live = Settings::read(&p);
        let mut s = if p.data[data::LOCK].get() > 0.5 { *self.locked.get_or_insert(live) } else { live };
        if p.data[data::LOCK].get() <= 0.5 {
            self.locked = None;
        }
        s.mode = live.mode;
        s.channel = live.channel;
        if s.mode != self.mode {
            self.mode = s.mode;
            self.next_at = f64::INFINITY;
            self.release(0, s.channel);
            self.held.clear();
            self.plays = [None; 128];
            self.restart_at = Some(ctx.beat.ceil());
        }

        // CV1: pitch (bend or transpose) and the range; CV2: the rhythm.
        let on = |i: usize| p.data[i].get() > 0.5;
        let cv1 = inputs.cv(0).map(|c| c[0].clamp(-1.0, 1.0));
        let cv2 = inputs.cv(1);
        let mut transpose = 0;
        let mut bend = None;
        if let Some(v) = cv1 {
            let semis = (v * CV_SEMITONES).round() as i32;
            match p.data[data::CV1_PITCH].get().round() as i32 {
                1 => bend = Some((8192.0 + v * 8191.0).round() as i32),
                2 => transpose = semis,
                _ => {}
            }
            if on(data::CV1_LOW) {
                s.low += semis;
            }
            if on(data::CV1_HIGH) {
                s.high += semis;
            }
        }
        if let Some(v) = cv2.map(|c| c[0].clamp(-1.0, 1.0)).filter(|_| s.mode <= SEQ_GATE) {
            if on(data::CV2_NOTE) {
                s.note = (s.note as f32 + v * (NOTE_VALUES.len() - 1) as f32).round().clamp(0.0, 8.0) as usize;
            }
            if on(data::CV2_VAR) {
                s.variation = (s.variation + v).clamp(-1.0, 1.0);
            }
            if on(data::CV2_LEGATO) {
                s.legato = (s.legato + v).clamp(0.0, 1.0);
            }
            if on(data::CV2_REST) {
                s.rest = (s.rest + v).clamp(0.0, 1.0);
            }
        }
        // Pitch bend follows CV1 (and recentres when it stops).
        let bend = bend.unwrap_or(8192);
        if self.bend.is_none_or(|b| (b - bend).abs() >= 8 || (bend == 8192 && b != 8192)) {
            if self.bend.is_some() || bend != 8192 {
                self.out.push(0, [0xE0 | s.channel, (bend & 0x7F) as u8, (bend >> 7) as u8]);
            }
            self.bend = Some(bend);
        }

        // The MIDI input: its cable, or else the keyboard.
        if inputs.midi_connected != self.cable {
            self.cable = inputs.midi_connected;
            self.held.clear();
            self.midi_mute = false;
        }
        if self.cable {
            self.cursor = self.midi.stream_head();
            for e in inputs.midi.events() {
                self.input(e.frame as usize, e.data, &s, transpose);
            }
        } else {
            let midi = self.midi.clone();
            let mut cursor = self.cursor;
            midi.read_stream(&mut cursor, |msg| self.input(0, msg, &s, transpose));
            self.cursor = cursor;
        }

        // Rewound (Stop, then Start): the pattern starts over at beat 0.
        if ctx.restart {
            self.release(0, s.channel);
            self.next_at = f64::INFINITY;
            self.restart_at = Some(ctx.beat.ceil());
            self.quant_at = ctx.beat.ceil();
            self.index = 0;
        }
        // Restarts (RESTART, MIDI C) land on the next beat.
        let restarts = p.data[data::RESTART].get();
        if restarts != self.restarts_seen || !self.started || self.restart_at == Some(f64::NEG_INFINITY) {
            self.restarts_seen = restarts;
            self.started = true;
            self.restart_at = Some(ctx.beat.ceil());
        }
        // Mute stops new notes; leaving it may restart the pattern.
        let muted = p.data[data::MUTE].get() > 0.5 || self.midi_mute;
        if muted != self.muted {
            self.muted = muted;
            if muted {
                self.release(0, s.channel);
            } else if on(data::UNMUTE_RESTART) {
                self.restart_at = Some(ctx.beat.ceil());
            }
        }

        match s.mode {
            SEQ => self.sequence(ctx, n, &s, transpose, muted),
            QUANT1 => self.quantize_beats(ctx, n, &s, cv2, muted),
            QUANT2 => self.quantize_gate(&s, cv2, muted),
            _ => {}
        }

        p.viz[CURRENT].set(self.sounding.map_or(-1.0, |s| s.note as f32));
        let step = if s.mode == SEQ && self.restart_at.is_none_or(|r| r > ctx.beat + 1.0) {
            let pos = ((ctx.beat - self.pattern_start) / 0.25).floor();
            if pos >= 0.0 { (s.first - 1 + pos as usize).min(15) as f32 } else { -1.0 }
        } else {
            -1.0
        };
        p.viz[STEP].set(step);
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::read(&ModuleParams::new(super::super::ModuleKind::Midicer))
    }

    #[test]
    fn defaults_follow_the_manual() {
        let s = settings();
        assert_eq!(NOTE_VALUES[s.note].0, "1/16");
        assert_eq!((s.low, s.high), (48, 60), "two octaves of C");
        assert_eq!(s.pattern_beats(), 4.0);
        // Only C enabled: C3 or C4, whatever the dice say.
        for r in [0.0, 0.3, 0.7, 0.999] {
            assert!(matches!(pick_note(&s, r), Some(48 | 60)));
        }
    }

    #[test]
    fn variation_picks_longer_or_shorter_values() {
        let mut s = settings();
        s.note = 4; // 1/8
        assert_eq!(note_length(&s, 0.0, 0.0), 0.5, "off at 0");
        s.variation = 1.0;
        assert_eq!(note_length(&s, 0.5, 0.0), 0.25, "one step shorter: 1/16");
        assert_eq!(note_length(&s, 0.5, 0.99), 0.25, "1/32 and triplets are off");
        s.thirty_second = true;
        assert_eq!(note_length(&s, 0.5, 0.99), 0.125);
        s.variation = -1.0;
        assert_eq!(note_length(&s, 0.5, 0.0), 1.0, "one step longer: 1/4");
        assert_eq!(note_length(&s, 0.5, 0.99), 4.0, "three steps: 1/1");
        s.triplets = true;
        assert_eq!(note_length(&s, 0.5, 0.0), 2.0 / 3.0, "1/4T when triplets are on");
        s.variation = -0.3;
        assert_eq!(note_length(&s, 0.5, 0.0), 0.5, "above the probability: unchanged");
    }

    #[test]
    fn semitone_probabilities_and_quantizing() {
        let mut s = settings();
        s.low = 60;
        s.high = 71;
        s.weights = [0.0; 12];
        assert_eq!(pick_note(&s, 0.5), None);
        s.weights[0] = 1.0; // C
        s.weights[5] = 1.0; // F
        let picks: Vec<i32> = (0..10).map(|i| pick_note(&s, i as f32 / 10.0).unwrap()).collect();
        assert_eq!(picks.iter().filter(|&&n| n == 60).count(), 5, "equal chances");
        assert!(picks.iter().all(|&n| n == 60 || n == 65));
        s.weights[5] = 0.5;
        assert_eq!(quantize(&s, 62.0), Some(60), "C pulls harder than F");
        assert_eq!(quantize(&s, 64.5), Some(65));
        assert_eq!(quantize(&s, 69.0), Some(65), "within the range only: C5 is out");
    }
}
