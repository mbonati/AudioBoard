//! Automa: a generative MIDI node. Four generators offer notes on every
//! step of the global grid — LIFE (a Game of Life read column by column),
//! CASCADE (a one-dimensional automaton pouring down like a waterfall),
//! TREE (Nørgård's self-similar infinity series) and JULIA (an orbit in the
//! Julia set of z² + c) — and a harmony layer fits each note to the chord
//! under it: a key, a scale and a progression of chords changing with the
//! bar. Whatever the cells do, the line stays consonant: chord tones on the
//! strong steps, tensions on the weaker ones as far as TENSION allows.
//!
//! LEAP sets how far the line may jump, VOICES harmonises it in up to four
//! parts (with the other cells that offered, then chord tones below), PAD
//! plays the chords as well, under the melody's range, and WANDER lets a
//! chord give way to another of the same function after the first time
//! round. MUTATE keeps the worlds from dying out, PHRASE starts the
//! generator again every so many steps, FREEZE loops what is playing.
//!
//! Everything follows the transport: Start plays from the start state
//! (LIFE's pattern, the seed), so the same settings play the same music.

pub mod engines;
pub mod harmony;

use std::sync::Arc;

use self::engines::{Cascade, H, Julia, Life, Offer, Tree, W, first_row, julia_c, tree_start};
use self::harmony::{CHANGES, Chord, Harmony, Progression, SCALES, SLOTS};
use super::steps::{MAX_STEPS, Step, StepClock, block_end, frame_of, steps_per_beat};
use super::{ModuleDef, ModuleParams, Rng};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::params::{ParamSpec, Unit};

pub const RATE: usize = 0;
pub const DENSITY: usize = 1;
pub const SWING: usize = 2;
pub const GATE: usize = 3;
pub const LEAP: usize = 4;
pub const TENSION: usize = 5;
pub const VOICES: usize = 6;
pub const OCTAVE: usize = 7;
pub const KEY: usize = 8;
pub const SCALE: usize = 9;
pub const CHANGE: usize = 10;
pub const VELOCITY: usize = 11;
pub const DYNAMICS: usize = 12;
pub const CHANNEL: usize = 13;
pub const LIFE_RULE: usize = 14;
pub const LIFE_GEN: usize = 15;
pub const CASCADE_RULE: usize = 16;
pub const CASCADE_GEN: usize = 17;
pub const MUTATE: usize = 18;
pub const PHRASE: usize = 19;
pub const STRIDE: usize = 20;
pub const INTERVAL: usize = 21;
pub const SHAPE: usize = 22;
pub const CHAOS: usize = 23;
pub const MORPH: usize = 24;

const PARAMS: [ParamSpec; 25] = [
    ParamSpec::lin("RATE", 0.0, 5.0, 3.0, Unit::Division),
    ParamSpec::lin("DENSITY", 0.0, 1.0, 0.8, Unit::Percent),
    ParamSpec::lin("SWING", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("GATE", 0.05, 1.0, 0.6, Unit::Percent),
    ParamSpec::lin("LEAP", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("TENSION", 0.0, 1.0, 0.4, Unit::Percent),
    ParamSpec::lin("VOICES", 1.0, 4.0, 1.0, Unit::Integer),
    ParamSpec::lin("OCTAVE", 1.0, 6.0, 4.0, Unit::Integer),
    ParamSpec::lin("KEY", 0.0, 11.0, 0.0, Unit::NoteName),
    ParamSpec::lin("SCALE", 0.0, 7.0, 0.0, Unit::Integer),
    ParamSpec::lin("CHANGE", 0.0, 3.0, 1.0, Unit::Integer),
    ParamSpec::lin("VELOCITY", 1.0, 127.0, 100.0, Unit::Integer),
    ParamSpec::lin("DYNAMICS", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("CHANNEL", 1.0, 16.0, 1.0, Unit::Channel),
    ParamSpec::lin("LIFE RULE", 0.0, 5.0, 0.0, Unit::Integer),
    ParamSpec::lin("LIFE GEN", 1.0, 16.0, 16.0, Unit::Integer),
    ParamSpec::lin("CASCADE RULE", 0.0, 7.0, 0.0, Unit::Integer),
    ParamSpec::lin("CASCADE GEN", 1.0, 8.0, 1.0, Unit::Integer),
    ParamSpec::lin("MUTATE", 0.0, 1.0, 0.25, Unit::Percent),
    ParamSpec::lin("PHRASE", 0.0, 64.0, 0.0, Unit::Integer),
    ParamSpec::lin("STRIDE", 1.0, 12.0, 1.0, Unit::Integer),
    ParamSpec::lin("INTERVAL", 1.0, 4.0, 1.0, Unit::Integer),
    ParamSpec::lin("SHAPE", 0.0, 1.0, 0.31, Unit::Percent),
    ParamSpec::lin("CHAOS", 0.0, 1.0, 0.45, Unit::Percent),
    ParamSpec::lin("MORPH", 0.0, 1.0, 0.1, Unit::Percent),
];

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// The generators (the mode).
pub const LIFE: usize = 0;
pub const CASCADE: usize = 1;
pub const TREE: usize = 2;
pub const JULIA: usize = 3;

pub const ENGINES: [(&str, &str); 4] = [
    (
        "LIFE",
        "Game of Life read column by column: each row is a scale degree",
    ),
    (
        "CASCADE",
        "A one-dimensional automaton pouring down: each generation a row of degrees",
    ),
    (
        "TREE",
        "Nørgård's infinity series: a melody that holds itself at every scale",
    ),
    (
        "JULIA",
        "An orbit of z² + c round its fixed point: spirals, figures, escapes",
    ),
];

/// What `ModuleParams::data` holds besides the knobs.
pub mod data {
    /// The generators' start (with LIFE's pattern): RESEED picks a new one.
    pub const SEED: usize = 0;
    pub const FREEZE: usize = 1;
    /// Chords with their seventh.
    pub const SEVENTHS: usize = 2;
    /// The chords are played too.
    pub const PAD: usize = 3;
    pub const WANDER: usize = 4;
    /// Chords in the progression (1…8), and each one's degree.
    pub const CHORDS: usize = 5;
    pub const PROG: usize = 6;
    /// LIFE's start: one row of bits per degree, the lowest first.
    pub const PATTERN: usize = PROG + super::SLOTS;
    pub const LEN: usize = PATTERN + super::H;
}

const DATA: [f32; data::LEN] = [
    1.0, 0.0, 0.0, 0.0, 0.0, // seed, free, triads, no pad, as written
    4.0, 0.0, 4.0, 5.0, 3.0, 0.0, 0.0, 0.0, 0.0, // I–V–vi–IV
    // A glider low down, another higher up, an R-pentomino in the middle
    // and a blinker near the top.
    0.0, 0.0, 14.0, 8.0, 4.0, 14336.0, 8192.0, 4096.0, 128.0, 192.0, 384.0, 0.0, 0.0, 14336.0, 0.0,
    0.0,
];

/// What the processor publishes for the panel (and what the panel hands it).
pub mod viz {
    /// The step playing (steps since beat 0), or −1 when stopped.
    pub const STEP: usize = 0;
    /// The transport's beat.
    pub const BEAT: usize = 1;
    /// Generations (LIFE, CASCADE), the term (TREE), the phrase (JULIA).
    pub const GENERATION: usize = 2;
    /// The cells that offered on the last step, and those that sounded
    /// (LIFE's rows, CASCADE's columns), as bits.
    pub const OFFERED: usize = 3;
    pub const PLAYED: usize = 4;
    /// TREE's term, the last played.
    pub const TERM: usize = 5;
    /// JULIA's c and fixed point, and its orbit (a ring of points).
    pub const C: usize = 6;
    pub const FIX: usize = 8;
    pub const ORBIT_POS: usize = 10;
    pub const ORBIT: usize = 11;
    pub const ORBIT_LEN: usize = super::engines::TRAIL;
    /// Notes played so far, and the latest ones: per note its number (from
    /// 1), pitch, velocity, step, length in steps and voice (4: the pad).
    pub const EVENT_SEQ: usize = ORBIT + 2 * ORBIT_LEN;
    pub const EVENTS: usize = EVENT_SEQ + 1;
    pub const EVENT_CAP: usize = 64;
    pub const EVENT_LEN: usize = 6;
    /// LIFE: each cell's age (0: dead), row by row from the lowest degree.
    /// CASCADE: the generations, newest first, 1 for a living cell.
    pub const CELLS: usize = EVENTS + EVENT_CAP * EVENT_LEN;
    /// Written by the panel: +1 a cell is born, −1 it dies, and the flag
    /// saying there are some.
    pub const EDITS: usize = CELLS + super::W * super::H;
    pub const EDITED: usize = EDITS + super::W * super::H;
    pub const LEN: usize = EDITED + 1;
}

/// The voice the pad's notes are published as.
pub const PAD_VOICE: usize = 4;

pub static DEF: ModuleDef = ModuleDef {
    name: "Automa",
    prefix: "AUTOMA",
    hint: "cells & fractals → melodies in key",
    inputs: &[],
    outputs: &[],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["LIFE", "CASCADE", "TREE", "JULIA"],
    toggle: Some("PLAY"),
    toggle_default: true,
    map: false,
    midi_in: false,
    midi_out: true,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

pub const MAX_VOICES: usize = 4;
/// Offers kept for FREEZE (its loop is PHRASE steps long, up to this, or 16).
const RING: usize = 32;
/// How strong each kind of step is (see [`harmony::strength`]).
const WEIGHTS: [f32; 4] = [1.0, 0.85, 0.65, 0.5];

/// A random value in [0, 1) that depends only on its arguments: what
/// Automa decides on a step depends on where it is, so it plays the same
/// way every time from Start.
pub fn hash01(seed: u32, n: u64, salt: u32) -> f32 {
    let mut x = ((seed as u64) << 32 | salt as u64) ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

fn on(p: &ModuleParams, i: usize) -> bool {
    p.data[i].get() > 0.5
}

/// A knob's value as a whole number within `lo..=hi`.
pub fn int(p: &ModuleParams, i: usize, lo: i64, hi: i64) -> i64 {
    (p.get(i).round() as i64).clamp(lo, hi)
}

pub fn seed(p: &ModuleParams) -> u32 {
    p.data[data::SEED].get().max(0.0) as u32
}

pub fn harmony(p: &ModuleParams) -> Harmony {
    Harmony {
        key: int(p, KEY, 0, 11) as u8,
        scale: int(p, SCALE, 0, SCALES.len() as i64 - 1) as usize,
        sevenths: on(p, data::SEVENTHS),
    }
}

pub fn chords(p: &ModuleParams) -> usize {
    (p.data[data::CHORDS].get().round().max(1.0) as usize).min(SLOTS)
}

pub fn progression(p: &ModuleParams) -> Progression {
    Progression {
        degrees: std::array::from_fn(|i| (p.data[data::PROG + i].get().round().max(0.0) as u8) % 7),
        len: chords(p),
        beats: CHANGES[int(p, CHANGE, 0, CHANGES.len() as i64 - 1) as usize].1,
        wander: on(p, data::WANDER),
        seed: seed(p),
    }
}

/// Writes a progression (at most [`SLOTS`] chords).
pub fn set_progression(p: &ModuleParams, degrees: &[u8]) {
    let len = degrees.len().clamp(1, SLOTS);
    p.data[data::CHORDS].set(len as f32);
    for (i, &d) in degrees.iter().take(SLOTS).enumerate() {
        p.data[data::PROG + i].set((d % 7) as f32);
    }
}

pub fn pattern(p: &ModuleParams) -> [u16; H] {
    std::array::from_fn(|y| p.data[data::PATTERN + y].get().clamp(0.0, 65535.0) as u16)
}

pub fn set_pattern(p: &ModuleParams, rows: &[u16; H]) {
    for (y, &row) in rows.iter().enumerate() {
        p.data[data::PATTERN + y].set(row as f32);
    }
}

/// A new start: a new seed, and a random pattern for LIFE.
pub fn reseed(p: &ModuleParams, seed: u32) {
    let seed = (seed % (1 << 24)).max(1);
    set_pattern(p, &engines::soup(seed));
    p.data[data::SEED].set(seed as f32);
}

/// The pitch of degree 0, the bottom row (OCTAVE 4 in C: middle C).
pub fn base(p: &ModuleParams) -> i32 {
    12 * (int(p, OCTAVE, 1, 6) as i32 + 1) + int(p, KEY, 0, 11) as i32
}

/// How wide a range (in degrees) TREE and JULIA play in: LEAP.
pub fn span(p: &ModuleParams) -> i32 {
    4 + (p.get(LEAP).clamp(0.0, 1.0) * 12.0).round() as i32
}

pub fn channel(p: &ModuleParams) -> u8 {
    int(p, CHANNEL, 1, 16) as u8 - 1
}

/// Steps between restarts (0: never).
pub fn phrase(p: &ModuleParams) -> i64 {
    int(p, PHRASE, 0, 64)
}

/// A note sounding, and the beat it ends on.
#[derive(Clone, Copy)]
struct Sounding {
    note: u8,
    channel: u8,
    end: f64,
}

pub struct Automa {
    params: Arc<ModuleParams>,
    clock: StepClock,
    rng: Rng,
    out: MidiBuf,
    melody: [Option<Sounding>; MAX_VOICES],
    pad: [Option<Sounding>; 4],
    /// The chord playing, and where: (slot, time round).
    chord: Option<((usize, i64), Chord)>,
    life: Life,
    cascade: Cascade,
    tree: Tree,
    julia: Julia,
    /// LIFE and CASCADE play their start state before evolving.
    fresh: [bool; 2],
    seed: u32,
    /// The melody's last pitch, for the next to be near it.
    last: Option<i32>,
    /// The latest offers, by step: what FREEZE loops.
    ring: [Offer; RING],
    /// Steps recorded in `ring` since the start.
    recorded: i64,
    /// The step FREEZE took hold on, and the generator it froze.
    frozen: Option<(i64, usize)>,
    playing: bool,
    events: u32,
    /// The generator whose state the display shows (None: to be published).
    shown: Option<usize>,
}

impl Automa {
    pub fn new(params: Arc<ModuleParams>) -> Self {
        let seed = seed(&params);
        let mut automa = Self {
            clock: StepClock::new(),
            rng: Rng::new(1),
            out: MidiBuf::default(),
            melody: [None; MAX_VOICES],
            pad: [None; 4],
            chord: None,
            life: Life::new(pattern(&params)),
            cascade: Cascade::new(first_row(seed)),
            tree: Tree::new(tree_start(seed)),
            julia: Julia::new(seed),
            fresh: [true; 2],
            seed,
            last: None,
            ring: [Offer::default(); RING],
            recorded: 0,
            frozen: None,
            playing: false,
            events: 0,
            shown: None,
            params,
        };
        let p = automa.params.clone();
        automa.reset(&p);
        automa
    }

    /// Every generator back to its start, and the melody's memory cleared.
    fn reset(&mut self, p: &ModuleParams) {
        self.rng = Rng::new(self.seed.wrapping_mul(0x9E37_79B9) ^ 0x0A70_3A00);
        for engine in [LIFE, CASCADE, TREE, JULIA] {
            self.reset_engine(p, engine);
        }
        self.ring = [Offer::default(); RING];
        self.recorded = 0;
        self.frozen = None;
        self.last = None;
        self.chord = None;
    }

    fn reset_engine(&mut self, p: &ModuleParams, engine: usize) {
        match engine {
            LIFE => {
                self.life.load(pattern(p));
                self.fresh[0] = true;
            }
            CASCADE => {
                self.cascade.reset(first_row(self.seed));
                self.fresh[1] = true;
            }
            TREE => self.tree.reset(tree_start(self.seed)),
            _ => self.julia.reset(self.seed),
        }
        self.shown = None;
    }

    /// Ends the notes that end before `beat`.
    fn release_before(&mut self, ctx: &ProcessCtx, n: usize, beat: f64) {
        for voice in &mut self.melody {
            if let Some(s) = *voice
                && s.end < beat
            {
                self.out
                    .push(frame_of(ctx, n, s.end), [0x80 | s.channel, s.note, 0]);
                *voice = None;
            }
        }
    }

    fn end(out: &mut MidiBuf, notes: &mut [Option<Sounding>], frame: usize) {
        for s in notes.iter_mut().filter_map(Option::take) {
            out.push(frame, [0x80 | s.channel, s.note, 0]);
        }
    }

    /// Ends every note, the pad's too.
    fn silence(&mut self, frame: usize) {
        Self::end(&mut self.out, &mut self.melody, frame);
        Self::end(&mut self.out, &mut self.pad, frame);
    }

    fn event(
        &mut self,
        p: &ModuleParams,
        note: u8,
        velocity: u8,
        step: i64,
        length: f64,
        voice: usize,
    ) {
        let at = viz::EVENTS + (self.events as usize % viz::EVENT_CAP) * viz::EVENT_LEN;
        self.events += 1;
        p.viz[at].set(-1.0);
        for (k, v) in [
            note as f32,
            velocity as f32,
            step as f32,
            length as f32,
            voice as f32,
        ]
        .into_iter()
        .enumerate()
        {
            p.viz[at + 1 + k].set(v);
        }
        p.viz[at].set(self.events as f32);
        p.viz[viz::EVENT_SEQ].set(self.events as f32);
    }

    /// What the generator offers on step `k`, moving it on.
    fn offer(&mut self, p: &ModuleParams, engine: usize, k: i64) -> Offer {
        let mutate = p.get(MUTATE).clamp(0.0, 1.0);
        match engine {
            LIFE => {
                let generation = int(p, LIFE_GEN, 1, 16);
                if !self.fresh[0] && k.rem_euclid(generation) == 0 {
                    self.life.step(int(p, LIFE_RULE, 0, 5) as usize);
                    self.life.mutate(&mut self.rng, mutate);
                }
                self.fresh[0] = false;
                self.life.column(k.rem_euclid(W as i64) as usize)
            }
            CASCADE => {
                let generation = int(p, CASCADE_GEN, 1, 8);
                if !self.fresh[1] && k.rem_euclid(generation) == 0 {
                    self.cascade
                        .step(int(p, CASCADE_RULE, 0, 7) as usize, &mut self.rng, mutate);
                }
                self.fresh[1] = false;
                self.cascade.offer()
            }
            TREE => {
                let offer = self.tree.offer(int(p, INTERVAL, 1, 4) as i32, span(p));
                self.tree
                    .advance(int(p, STRIDE, 1, 12) as u64, &mut self.rng, mutate);
                offer
            }
            _ => self.julia.step(span(p), &mut self.rng, mutate),
        }
    }

    /// The degrees to play from an offer: the melody (an offered degree
    /// fitted to the chord, the nearer the last note the likelier unless
    /// LEAP lets it jump), then up to VOICES − 1 harmony parts. Also the
    /// offered cells that sounded. The choices depend on step `kd`, so a
    /// frozen loop chooses again as it did.
    #[allow(clippy::too_many_arguments)]
    fn choose(
        &self,
        p: &ModuleParams,
        offer: &Offer,
        h: &Harmony,
        chord: &Chord,
        strength: usize,
        kd: i64,
    ) -> ([i32; MAX_VOICES], usize, u16) {
        let range = (0, H as i32 - 1);
        let tension = p.get(TENSION).clamp(0.0, 1.0);
        let reach = 1.0 + p.get(LEAP).clamp(0.0, 1.0) * 11.0;
        let pitch = |d: i32| h.semis(d);
        let mut fitted = [(0i32, 0f32); H];
        let mut total = 0.0;
        for (slot, (d, w)) in fitted.iter_mut().zip(offer.degrees()) {
            let f = harmony::snap(h, chord, d, range, strength, tension);
            let distance = self.last.map_or(0.0, |l| (pitch(f) - l).abs() as f32);
            let mut weight = w * (-distance / reach).exp();
            if distance == 0.0 && offer.len > 1 {
                weight *= 0.4;
            }
            *slot = (f, weight.max(1e-6));
            total += slot.1;
        }
        let mut pick = hash01(self.seed, kd as u64, 0x91C) * total;
        let mut melody = 0;
        for (i, &(_, w)) in fitted[..offer.len].iter().enumerate() {
            melody = i;
            if pick < w {
                break;
            }
            pick -= w;
        }
        let mut chosen = [fitted[melody].0; MAX_VOICES];
        let mut count = 1;
        let mut cells = 1u16 << (offer.degrees[melody].clamp(0, 15));

        let voices = int(p, VOICES, 1, MAX_VOICES as i64) as usize;
        let apart =
            |d: i32, chosen: &[i32]| chosen.iter().all(|&c| (pitch(c) - pitch(d)).abs() >= 3);
        // The other cells that offered, as chord tones…
        let start = (hash01(self.seed, kd as u64, 0x40C) * offer.len as f32) as usize;
        for i in (0..offer.len).map(|i| (start + i) % offer.len) {
            if count == voices {
                break;
            }
            let d = harmony::snap(h, chord, offer.degrees[i] as i32, range, 0, 0.0);
            if apart(d, &chosen[..count]) {
                chosen[count] = d;
                count += 1;
                cells |= 1 << offer.degrees[i].clamp(0, 15);
            }
        }
        // …then chord tones below the lowest part (above, if no room).
        let lowest = chosen[..count].iter().copied().min().unwrap_or(0);
        let highest = chosen[..count].iter().copied().max().unwrap_or(0);
        for d in (range.0..lowest).rev().chain(highest + 1..=range.1) {
            if count == voices {
                break;
            }
            if chord.contains(h.pc(d)) && apart(d, &chosen[..count]) {
                chosen[count] = d;
                count += 1;
            }
        }
        (chosen, count, cells)
    }

    /// The chord's tones held under the melody's range, each the highest of
    /// its pitch class below it: the voicing follows the chords by inversion.
    #[allow(clippy::too_many_arguments)]
    fn play_pad(
        &mut self,
        p: &ModuleParams,
        step: &Step,
        chord: &Chord,
        base: i32,
        channel: u8,
        velocity: u8,
        length: f64,
    ) {
        Self::end(&mut self.out, &mut self.pad, step.frame);
        for (k, &pc) in chord.tones().iter().enumerate() {
            let note = (base - 1 - (base - 1 - pc as i32).rem_euclid(12)).clamp(0, 127) as u8;
            self.out.push(step.frame, [0x90 | channel, note, velocity]);
            self.pad[k] = Some(Sounding {
                note,
                channel,
                end: f64::INFINITY,
            });
            self.event(p, note, velocity, step.index, length, PAD_VOICE);
        }
    }

    fn step(&mut self, p: &ModuleParams, engine: usize, step: &Step, per_beat: f64) {
        let k = step.index;
        // Where on the grid (swing moves the note, not the harmony).
        let grid = k as f64 / per_beat;
        let h = harmony(p);
        let prog = progression(p);
        let (slot, cycle) = prog.slot_at(grid);
        let chord = h.chord(prog.degree(slot, cycle));
        let base = base(p);
        let channel = channel(p);
        let velocity = p.get(VELOCITY).clamp(1.0, 127.0);

        let changed = self
            .chord
            .is_none_or(|(at, c)| at != (slot, cycle) || c != chord);
        let pad = on(p, data::PAD);
        if changed || (pad && self.pad.iter().all(Option::is_none)) {
            self.chord = Some(((slot, cycle), chord));
            if pad {
                let length = (prog.beats - grid.rem_euclid(prog.beats)) * per_beat;
                self.play_pad(
                    p,
                    step,
                    &chord,
                    base,
                    channel,
                    (velocity * 0.7).round() as u8,
                    length,
                );
            }
        }

        // FREEZE loops the last bar (PHRASE steps, up to 32, for all but
        // LIFE, whose frozen grid loops by itself): the offers recorded then,
        // chosen from as they were, over whatever chord is playing now. It
        // takes hold once there is a whole loop to play.
        let length = if engine == LIFE {
            W as i64
        } else {
            match phrase(p) {
                l @ 1..=32 => l,
                _ => 16,
            }
        };
        if !on(p, data::FREEZE) || self.frozen.is_some_and(|(_, e)| e != engine) {
            self.frozen = None;
        } else if self.frozen.is_none() && (engine == LIFE || self.recorded >= length) {
            self.frozen = Some((k, engine));
        }
        let (offer, kd) = match self.frozen {
            Some((from, _)) => {
                let kd = from - length + (k - from).rem_euclid(length);
                let offer = if engine == LIFE {
                    self.life.column(k.rem_euclid(W as i64) as usize)
                } else {
                    self.ring[kd.rem_euclid(RING as i64) as usize]
                };
                (offer, kd)
            }
            None => {
                let phrase = phrase(p);
                if phrase > 0 && k.rem_euclid(phrase) == 0 {
                    self.reset_engine(p, engine);
                }
                let offer = self.offer(p, engine, k);
                self.ring[k.rem_euclid(RING as i64) as usize] = offer;
                self.recorded += 1;
                (offer, k)
            }
        };

        let strength = harmony::strength(grid, prog.beats);
        let weight = WEIGHTS[strength];
        let density = p.get(DENSITY).clamp(0.0, 1.0);
        let plays = !offer.is_empty()
            && hash01(self.seed, kd as u64, 0xD3) < density.powf(1.0 / (0.5 + weight));
        let mut played = 0;
        if plays {
            let (chosen, count, cells) = self.choose(p, &offer, &h, &chord, strength, kd);
            played = cells;
            let dynamics = p.get(DYNAMICS).clamp(0.0, 1.0);
            let accent = weight * (0.6 + 0.4 * offer.energy);
            let humanize = (hash01(self.seed, kd as u64, 0x7E1) - 0.5) * 0.12 * dynamics;
            let level = velocity * (1.0 - dynamics * (1.0 - accent) + humanize);
            let gate = p.get(GATE).clamp(0.05, 1.0) as f64;
            Self::end(&mut self.out, &mut self.melody, step.frame);
            for (voice, &d) in chosen[..count].iter().enumerate() {
                let note = (base + h.semis(d)).clamp(0, 127) as u8;
                let vel = (level * if voice == 0 { 1.0 } else { 0.8 })
                    .round()
                    .clamp(1.0, 127.0) as u8;
                self.out.push(step.frame, [0x90 | channel, note, vel]);
                self.melody[voice] = Some(Sounding {
                    note,
                    channel,
                    end: step.beat + gate / per_beat,
                });
                self.event(p, note, vel, k, gate, voice);
            }
            self.last = Some(h.semis(chosen[0]));
        }
        // STEP last: the panel takes a new step to mean the others are new.
        p.viz[viz::OFFERED].set(offer.cells as f32);
        p.viz[viz::PLAYED].set(played as f32);
        p.viz[viz::STEP].set(k as f32);
        self.shown = None;
    }

    /// The generator's state, for the display.
    fn publish(&mut self, p: &ModuleParams, engine: usize) {
        match engine {
            LIFE => {
                for y in 0..H {
                    for x in 0..W {
                        p.viz[viz::CELLS + y * W + x].set(self.life.age[y][x] as f32);
                    }
                }
                p.viz[viz::GENERATION].set(self.life.generation as f32);
            }
            CASCADE => {
                for age in 0..engines::HISTORY {
                    let row = self.cascade.row(age);
                    for x in 0..W {
                        p.viz[viz::CELLS + age * W + x].set((row >> x & 1) as f32);
                    }
                }
                p.viz[viz::GENERATION].set(self.cascade.generation as f32);
            }
            TREE => {
                p.viz[viz::TERM].set(self.tree.played as f32);
                p.viz[viz::GENERATION].set(self.tree.played as f32);
            }
            _ => {
                for (k, z) in self.julia.trail.iter().enumerate() {
                    p.viz[viz::ORBIT + 2 * k].set(z.0);
                    p.viz[viz::ORBIT + 2 * k + 1].set(z.1);
                }
                p.viz[viz::ORBIT_POS].set(self.julia.trail_pos as f32);
                p.viz[viz::GENERATION].set(self.julia.phrase as f32);
            }
        }
        self.shown = Some(engine);
    }
}

impl Processor for Automa {
    fn process(&mut self, ctx: &ProcessCtx, _inputs: &Inputs, _outputs: &mut [Buf], n: usize) {
        self.out.clear();
        let p = self.params.clone();
        let engine = p.mode().min(JULIA);

        // Cells drawn on LIFE's grid.
        if p.viz[viz::EDITED].take() > 0.5 {
            for i in 0..W * H {
                let edit = p.viz[viz::EDITS + i].take();
                if edit != 0.0 {
                    self.life.set(i % W, i / W, edit > 0.0);
                }
            }
            self.shown = None;
        }
        // RESEED: every generator from its new start.
        let seed = seed(&p);
        if seed != self.seed {
            self.seed = seed;
            self.reset(&p);
        }
        // Stopped or rewound: back to the start, so Start plays it again.
        if ctx.restart {
            self.clock = StepClock::new();
        }
        if ctx.restart || (self.playing && !ctx.playing) {
            self.silence(0);
            self.reset(&p);
        }
        self.playing = ctx.playing;

        // JULIA's c follows its knobs (and the beat), even when stopped.
        let c = julia_c(p.get(SHAPE), p.get(CHAOS), p.get(MORPH), ctx.beat);
        self.julia.set_c(c);
        for (k, v) in [c.0, c.1, self.julia.fix.0, self.julia.fix.1]
            .into_iter()
            .enumerate()
        {
            p.viz[viz::C + k].set(v);
        }
        p.viz[viz::BEAT].set(ctx.beat as f32);

        if !p.toggled() || !ctx.playing {
            self.silence(0);
            p.viz[viz::STEP].set(-1.0);
        } else {
            if !on(&p, data::PAD) {
                Self::end(&mut self.out, &mut self.pad, 0);
            }
            let per_beat = steps_per_beat(p.get(RATE));
            let mut steps = [Step {
                frame: 0,
                beat: 0.0,
                index: 0,
            }; MAX_STEPS];
            let count = self.clock.steps(ctx, n, per_beat, p.get(SWING), &mut steps);
            for step in &steps[..count] {
                self.release_before(ctx, n, step.beat);
                self.step(&p, engine, step, per_beat);
            }
            self.release_before(ctx, n, block_end(ctx, n));
        }
        if self.shown != Some(engine) {
            self.publish(&p, engine);
        }
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}
