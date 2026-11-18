//! Orrery: a Euclidean rhythm generator drawn as concentric rings, like the
//! orbits of an orrery. Each of the eight rings is a rhythm of its own: a
//! number of STEPS round it, PULSES spread over them as evenly as they go
//! (Bjorklund's Euclidean rhythms: 3 over 8 is the tresillo, 5 over 8 the
//! cinquillo), and a note on a MIDI channel of its own. Every step can be
//! switched on, off or accented by hand afterwards.
//!
//! Rings turn: ROTATE moves a ring's rhythm round by steps (or by any part
//! of a step), so the rings fall out of phase with each other, and TWIST
//! turns each ring a little further than the one outside it.
//!
//! Two ways to run the rings:
//! - **GRID**: every ring steps on the board's grid (RATE): a ring of 5
//!   steps comes round sooner than one of 16, so they drift (polymeter);
//! - **CYCLE**: every ring goes round once in CYCLE beats, its steps
//!   dividing the cycle evenly: 3 against 4 against 5 (polyrhythm).

use std::sync::Arc;

use super::automa::hash01;
use super::steps::{MAX_STEPS as MAX_BLOCK_STEPS, Step, StepClock, block_end, roll, steps_per_beat};
use super::{ModuleDef, ModuleParams, Rng};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::params::{DIVISIONS, ParamSpec, Unit};

/// Rings, from the outside in.
pub const RINGS: usize = 8;
/// Most steps a ring can have.
pub const MAX_STEPS: usize = 32;

pub const RATE: usize = 0;
pub const CYCLE: usize = 1;
pub const SWING: usize = 2;
pub const ACCENT: usize = 3;
pub const TWIST: usize = 4;

const PARAMS: [ParamSpec; 5] = [
    ParamSpec::lin("RATE", 0.0, 5.0, 3.0, Unit::Division),
    ParamSpec::lin("CYCLE", 1.0, 32.0, 4.0, Unit::Integer),
    ParamSpec::lin("SWING", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("ACCENT", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("TWIST", -2.0, 2.0, 0.0, Unit::Plain).bipolar(),
];

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// The modes: steps on the board's grid, or each ring once round a cycle.
pub const GRID: usize = 0;
pub const CYCLE_MODE: usize = 1;

/// A step: off, on, or on and accented.
pub const OFF: u8 = 0;
pub const ON: u8 = 1;
pub const ACCENTED: u8 = 2;

/// What `ModuleParams::data` holds.
pub mod data {
    /// The ring selected on the panel.
    pub const SELECTED: usize = 0;
    /// The rings, [`super::ring::LEN`] values each.
    pub const RINGS: usize = 1;
    pub const LEN: usize = RINGS + super::RINGS * super::ring::LEN;
}

/// A ring's values in `data`.
pub mod ring {
    /// 1 while the ring plays (0: muted).
    pub const ON: usize = 0;
    pub const STEPS: usize = 1;
    pub const PULSES: usize = 2;
    /// How far the ring is turned, in steps (any part of a step).
    pub const ROTATE: usize = 3;
    pub const NOTE: usize = 4;
    /// MIDI channel, 1–16.
    pub const CHANNEL: usize = 5;
    pub const VELOCITY: usize = 6;
    /// Note length, as a part of the step.
    pub const GATE: usize = 7;
    /// The chance a hit plays.
    pub const CHANCE: usize = 8;
    /// Each step: [`super::OFF`], [`super::ON`] or [`super::ACCENTED`].
    pub const PATTERN: usize = 9;
    pub const LEN: usize = PATTERN + super::MAX_STEPS;
}

/// What the processor publishes for the panel.
pub mod viz {
    use super::RINGS;

    /// 1 while the rings play.
    pub const LIVE: usize = 0;
    /// Per ring: where its planet is, in steps from the top of the ring
    /// (0…steps): a step's hit plays as the planet passes it.
    pub const PHASE: usize = 1;
    /// Per ring: the pattern step reached last, or −1.
    pub const STEP: usize = PHASE + RINGS;
    /// Per ring: notes played so far (the panel flashes on a change).
    pub const HITS: usize = STEP + RINGS;
    /// Per ring: the step the last note played on.
    pub const HIT_STEP: usize = HITS + RINGS;
    pub const LEN: usize = HIT_STEP + RINGS;
}

/// The Euclidean rhythm of `pulses` over `steps`: whether step `i` plays.
/// The pulses fall as evenly as the steps allow, the first on step 0.
pub const fn euclid(pulses: usize, steps: usize, i: usize) -> bool {
    let n = if steps == 0 { 1 } else { steps };
    let k = if pulses > n { n } else { pulses };
    k > 0 && (i % n) * k % n < k
}

/// The rings to start with: a General MIDI drum groove on channel 10, the
/// four inner rings muted, ready to bring in. (on, steps, pulses, rotate,
/// note, velocity, gate, accents every…)
type Start = (bool, usize, usize, usize, u8, u8, f32, usize);
const START: [Start; RINGS] = [
    (true, 16, 5, 0, 36, 112, 0.5, 0),  // bass drum
    (true, 16, 2, 4, 38, 104, 0.5, 0),  // snare: the backbeat
    (true, 16, 11, 0, 42, 76, 0.25, 4), // closed hi-hat, accents on the beats
    (true, 16, 2, 2, 46, 84, 0.4, 0),   // open hi-hat, on the offbeats
    (false, 12, 5, 0, 37, 84, 0.3, 0),  // side stick
    (false, 7, 3, 0, 45, 92, 0.5, 0),   // low tom
    (false, 9, 4, 1, 39, 92, 0.4, 0),   // hand clap
    (false, 5, 2, 0, 56, 80, 0.3, 0),   // cowbell
];

const fn defaults() -> [f32; data::LEN] {
    let mut d = [0.0; data::LEN];
    let mut r = 0;
    while r < RINGS {
        let (on, steps, pulses, rotate, note, velocity, gate, accents) = START[r];
        let at = data::RINGS + r * ring::LEN;
        d[at + ring::ON] = if on { 1.0 } else { 0.0 };
        d[at + ring::STEPS] = steps as f32;
        d[at + ring::PULSES] = pulses as f32;
        d[at + ring::ROTATE] = rotate as f32;
        d[at + ring::NOTE] = note as f32;
        d[at + ring::CHANNEL] = 10.0;
        d[at + ring::VELOCITY] = velocity as f32;
        d[at + ring::GATE] = gate;
        d[at + ring::CHANCE] = 1.0;
        let mut i = 0;
        while i < steps {
            if euclid(pulses, steps, i) {
                d[at + ring::PATTERN + i] = if accents > 0 && i % accents == 0 { ACCENTED as f32 } else { ON as f32 };
            }
            i += 1;
        }
        r += 1;
    }
    d
}

const DATA: [f32; data::LEN] = defaults();

pub static DEF: ModuleDef = ModuleDef {
    name: "Orrery",
    prefix: "ORRERY",
    hint: "Euclidean rings → MIDI",
    inputs: &[],
    outputs: &[],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["GRID", "CYCLE"],
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

// ---------------------------------------------------------------------------
// The rings' settings
// ---------------------------------------------------------------------------

/// Where ring `r`'s value `i` is in `data`.
pub fn at(r: usize, i: usize) -> usize {
    data::RINGS + r.min(RINGS - 1) * ring::LEN + i
}

pub fn get(p: &ModuleParams, r: usize, i: usize) -> f32 {
    p.data[at(r, i)].get()
}

pub fn set(p: &ModuleParams, r: usize, i: usize, value: f32) {
    p.data[at(r, i)].set(value);
}

/// A ring's value as a whole number within `lo..=hi`.
pub fn int(p: &ModuleParams, r: usize, i: usize, lo: i32, hi: i32) -> i32 {
    (get(p, r, i).round() as i32).clamp(lo, hi)
}

pub fn selected(p: &ModuleParams) -> usize {
    (p.data[data::SELECTED].get().round().max(0.0) as usize).min(RINGS - 1)
}

pub fn select(p: &ModuleParams, r: usize) {
    p.data[data::SELECTED].set(r.min(RINGS - 1) as f32);
}

pub fn is_on(p: &ModuleParams, r: usize) -> bool {
    get(p, r, ring::ON) > 0.5
}

pub fn steps(p: &ModuleParams, r: usize) -> usize {
    int(p, r, ring::STEPS, 1, MAX_STEPS as i32) as usize
}

pub fn pulses(p: &ModuleParams, r: usize) -> usize {
    int(p, r, ring::PULSES, 0, steps(p, r) as i32) as usize
}

pub fn note(p: &ModuleParams, r: usize) -> u8 {
    int(p, r, ring::NOTE, 0, 127) as u8
}

/// The ring's channel, 1–16.
pub fn channel(p: &ModuleParams, r: usize) -> u8 {
    int(p, r, ring::CHANNEL, 1, 16) as u8
}

pub fn velocity(p: &ModuleParams, r: usize) -> u8 {
    int(p, r, ring::VELOCITY, 1, 127) as u8
}

pub fn gate(p: &ModuleParams, r: usize) -> f32 {
    get(p, r, ring::GATE).clamp(0.05, 1.0)
}

pub fn chance(p: &ModuleParams, r: usize) -> f32 {
    get(p, r, ring::CHANCE).clamp(0.0, 1.0)
}

/// How far the ring itself is turned, in steps (0…steps).
pub fn rotation(p: &ModuleParams, r: usize) -> f32 {
    get(p, r, ring::ROTATE).rem_euclid(steps(p, r) as f32)
}

/// Turns the ring to `steps_turned` (taken round its steps).
pub fn set_rotation(p: &ModuleParams, r: usize, steps_turned: f32) {
    let n = steps(p, r) as f32;
    let mut value = steps_turned.rem_euclid(n);
    // Whole steps stay whole (no 2.9999 from the wrap).
    if (value - value.round()).abs() < 1e-4 {
        value = value.round() % n;
    }
    set(p, r, ring::ROTATE, value);
}

/// How far the ring plays turned: its own rotation and TWIST's share (each
/// ring TWIST steps further than the one outside it), in steps (0…steps).
pub fn turn(p: &ModuleParams, r: usize) -> f32 {
    (get(p, r, ring::ROTATE) + r as f32 * p.get(TWIST)).rem_euclid(steps(p, r) as f32)
}

/// Step `i` of the ring's pattern.
pub fn step(p: &ModuleParams, r: usize, i: usize) -> u8 {
    if i >= MAX_STEPS {
        return OFF;
    }
    (get(p, r, ring::PATTERN + i).round().clamp(0.0, 2.0)) as u8
}

pub fn set_step(p: &ModuleParams, r: usize, i: usize, value: u8) {
    if i < MAX_STEPS {
        set(p, r, ring::PATTERN + i, value.min(ACCENTED) as f32);
    }
}

/// Steps of the ring that play.
pub fn hits(p: &ModuleParams, r: usize) -> usize {
    (0..steps(p, r)).filter(|&i| step(p, r, i) != OFF).count()
}

/// Lays the ring's Euclidean rhythm down again (over hand edits); a step
/// still on keeps its accent.
pub fn lay(p: &ModuleParams, r: usize) {
    let (n, k) = (steps(p, r), pulses(p, r));
    for i in 0..MAX_STEPS {
        let was = step(p, r, i);
        let now = if i < n && euclid(k, n, i) {
            if was == ACCENTED { ACCENTED } else { ON }
        } else {
            OFF
        };
        set_step(p, r, i, now);
    }
}

/// New STEPS: the pulses fit within them and the rhythm is laid again.
pub fn set_steps(p: &ModuleParams, r: usize, n: usize) {
    let n = n.clamp(1, MAX_STEPS);
    let turned = rotation(p, r);
    set(p, r, ring::STEPS, n as f32);
    set(p, r, ring::PULSES, pulses(p, r).min(n) as f32);
    set_rotation(p, r, turned.min(n as f32 - 1.0));
    lay(p, r);
}

/// New PULSES: the rhythm is laid again.
pub fn set_pulses(p: &ModuleParams, r: usize, k: usize) {
    set(p, r, ring::PULSES, k.min(steps(p, r)) as f32);
    lay(p, r);
}

/// Every step the other way round: hits become rests, rests hits.
pub fn invert(p: &ModuleParams, r: usize) {
    for i in 0..steps(p, r) {
        set_step(p, r, i, if step(p, r, i) == OFF { ON } else { OFF });
    }
}

pub fn clear(p: &ModuleParams, r: usize) {
    for i in 0..MAX_STEPS {
        set_step(p, r, i, OFF);
    }
}

/// New rhythms from `seed`: the pulses and the turn of each ring that
/// plays (or of ring `only`), within what suits its length. Steps, notes and
/// channels stay.
pub fn dice(p: &ModuleParams, seed: u32, only: Option<usize>) {
    for r in 0..RINGS {
        if only.is_some_and(|o| o != r) || (only.is_none() && !is_on(p, r)) {
            continue;
        }
        let n = steps(p, r);
        let roll = |salt: u32| hash01(seed, r as u64, salt);
        // Sparse to busy, most often in the middle.
        let density = 0.12 + 0.5 * (roll(1) + roll(2)) * 0.5 + 0.25 * roll(3).powi(3);
        let k = ((n as f32 * density).round() as usize).clamp(1, n);
        set(p, r, ring::PULSES, k as f32);
        set(p, r, ring::ROTATE, (roll(4) * n as f32).floor().min(n as f32 - 1.0));
        lay(p, r);
    }
}

/// "E(5,16)": the ring's Euclidean name.
pub fn euclid_name(p: &ModuleParams, r: usize) -> String {
    format!("E({},{})", pulses(p, r), steps(p, r))
}

/// Beats per step of ring `r`: the grid's in GRID, the cycle shared out
/// among its steps in CYCLE.
pub fn step_beats(p: &ModuleParams, r: usize) -> f64 {
    if p.mode() == CYCLE_MODE {
        cycle_beats(p) / steps(p, r) as f64
    } else {
        1.0 / steps_per_beat(p.get(RATE))
    }
}

pub fn cycle_beats(p: &ModuleParams) -> f64 {
    p.get(CYCLE).round().clamp(1.0, 32.0) as f64
}

/// "1/16" (GRID) or "1 bar" (CYCLE).
pub fn clock_label(p: &ModuleParams) -> String {
    if p.mode() == CYCLE_MODE {
        beats_label(cycle_beats(p) as u32)
    } else {
        DIVISIONS[(p.get(RATE).round().max(0.0) as usize).min(DIVISIONS.len() - 1)].0.to_owned()
    }
}

/// "3 beats", "1 bar", "2 bars", "6 beats".
pub fn beats_label(beats: u32) -> String {
    match beats {
        1 => "1 beat".to_owned(),
        b if b % 4 == 0 => format!("{} bar{}", b / 4, if b == 4 { "" } else { "s" }),
        b => format!("{b} beats"),
    }
}

/// The General MIDI drum on `note` (channel 10), if there is one.
pub fn drum_name(note: u8) -> Option<&'static str> {
    const DRUMS: [&str; 47] = [
        "Acoustic Bass Drum", "Bass Drum", "Side Stick", "Acoustic Snare", "Hand Clap", "Electric Snare", "Low Floor Tom",
        "Closed Hi-Hat", "High Floor Tom", "Pedal Hi-Hat", "Low Tom", "Open Hi-Hat", "Low-Mid Tom", "Hi-Mid Tom",
        "Crash Cymbal 1", "High Tom", "Ride Cymbal 1", "Chinese Cymbal", "Ride Bell", "Tambourine", "Splash Cymbal",
        "Cowbell", "Crash Cymbal 2", "Vibraslap", "Ride Cymbal 2", "Hi Bongo", "Low Bongo", "Mute Hi Conga",
        "Open Hi Conga", "Low Conga", "High Timbale", "Low Timbale", "High Agogo", "Low Agogo", "Cabasa", "Maracas",
        "Short Whistle", "Long Whistle", "Short Guiro", "Long Guiro", "Claves", "Hi Wood Block", "Low Wood Block",
        "Mute Cuica", "Open Cuica", "Mute Triangle", "Open Triangle",
    ];
    (35..=81).contains(&note).then(|| DRUMS[(note - 35) as usize])
}

// ---------------------------------------------------------------------------
// The processor
// ---------------------------------------------------------------------------

/// A note a ring is sounding: its channel (1–16), note and the beat it ends.
#[derive(Clone, Copy)]
struct Held {
    channel: u8,
    note: u8,
    end: f64,
}

/// An event of this block, sorted before it goes out: by frame, note-offs
/// before note-ons, then in the order made.
#[derive(Clone, Copy, Default)]
struct Event {
    frame: usize,
    on: bool,
    order: u16,
    data: [u8; 3],
}

const EVENT_CAP: usize = 64;

pub struct Orrery {
    params: Arc<ModuleParams>,
    clocks: [StepClock; RINGS],
    held: [Option<Held>; RINGS],
    rng: Rng,
    hits: [u32; RINGS],
    out: MidiBuf,
    events: [Event; EVENT_CAP],
    count: usize,
}

impl Orrery {
    pub fn new(params: Arc<ModuleParams>) -> Self {
        Self {
            params,
            clocks: std::array::from_fn(|_| StepClock::new()),
            held: [None; RINGS],
            rng: Rng::new(0x0E7E_7E11),
            hits: [0; RINGS],
            out: MidiBuf::default(),
            events: [Event::default(); EVENT_CAP],
            count: 0,
        }
    }

    fn push(&mut self, frame: usize, data: [u8; 3]) {
        if self.count < EVENT_CAP {
            let on = data[0] & 0xF0 == 0x90;
            self.events[self.count] = Event { frame, on, order: self.count as u16, data };
            self.count += 1;
        }
    }

    /// Ends ring `r`'s note at `frame`.
    fn release(&mut self, r: usize, frame: usize) {
        if let Some(h) = self.held[r].take() {
            self.push(frame, [0x80 | (h.channel - 1), h.note, 0]);
        }
    }

    /// Ends ring `r`'s note if it ends before `beat`, where it ends.
    fn release_before(&mut self, ctx: &ProcessCtx, n: usize, r: usize, beat: f64) {
        if let Some(h) = self.held[r]
            && h.end < beat
        {
            self.release(r, super::steps::frame_of(ctx, n, h.end));
        }
    }

    fn release_all(&mut self) {
        for r in 0..RINGS {
            self.release(r, 0);
        }
    }

    /// The block's events, in order, onto the MIDI output.
    fn flush(&mut self) {
        let events = &mut self.events[..self.count];
        events.sort_unstable_by_key(|e| (e.frame, e.on, e.order));
        for e in events.iter() {
            self.out.push(e.frame, e.data);
        }
        self.count = 0;
    }

    /// Publishes where each ring's planet is at `beat`.
    fn publish_phases(&self, p: &ModuleParams, beat: f64) {
        for r in 0..RINGS {
            let n = steps(p, r) as f64;
            let phase = (beat / step_beats(p, r)).rem_euclid(n);
            p.viz[viz::PHASE + r].set(phase as f32);
        }
    }
}

impl Processor for Orrery {
    fn process(&mut self, ctx: &ProcessCtx, _inputs: &Inputs, _outputs: &mut [Buf], n: usize) {
        self.out.clear();
        self.count = 0;
        let p = self.params.clone();
        if ctx.restart {
            // Rewound: every ring from its first step, the notes ended.
            self.clocks = std::array::from_fn(|_| StepClock::new());
            self.release_all();
        }
        // Off, or the transport stopped: end the notes.
        if !p.toggled() || !ctx.playing {
            self.release_all();
            self.flush();
            p.viz[viz::LIVE].set(0.0);
            for r in 0..RINGS {
                p.viz[viz::STEP + r].set(-1.0);
            }
            self.publish_phases(&p, ctx.beat);
            return;
        }
        p.viz[viz::LIVE].set(1.0);
        let cycle = p.mode() == CYCLE_MODE;
        let swing = if cycle { 0.0 } else { p.get(SWING) };
        let accent = p.get(ACCENT).clamp(0.0, 1.0);
        // The shortest note: a few milliseconds, so its end never shares
        // a frame with its start.
        let shortest = 0.004 * ctx.beats_per_sample * ctx.sample_rate as f64;
        let end = block_end(ctx, n);
        for r in 0..RINGS {
            let count = steps(&p, r);
            let len = step_beats(&p, r);
            let turned = turn(&p, r) as f64;
            let (whole, part) = (turned.floor(), turned - turned.floor());
            // A part of a step turned delays every hit by as much: the steps
            // are found on a clock that runs that much behind.
            let shifted = ProcessCtx { beat: ctx.beat - part * len, ..*ctx };
            let mut found = [Step { frame: 0, beat: 0.0, index: 0 }; MAX_BLOCK_STEPS];
            let k = self.clocks[r].steps(&shifted, n, 1.0 / len, swing, &mut found);
            for s in &found[..k] {
                let beat = s.beat + part * len;
                self.release_before(ctx, n, r, beat);
                let i = (s.index - whole as i64).rem_euclid(count as i64) as usize;
                p.viz[viz::STEP + r].set(i as f32);
                let value = step(&p, r, i);
                if !is_on(&p, r) || value == OFF || !roll(&mut self.rng, chance(&p, r)) {
                    continue;
                }
                let base = velocity(&p, r) as f32;
                let vel = if value == ACCENTED { base + (127.0 - base) * accent } else { base };
                let (channel, note) = (channel(&p, r), note(&p, r));
                self.release(r, s.frame);
                self.push(s.frame, [0x90 | (channel - 1), note, vel.round().clamp(1.0, 127.0) as u8]);
                self.held[r] = Some(Held { channel, note, end: beat + (gate(&p, r) as f64 * len).max(shortest) });
                self.hits[r] = self.hits[r].wrapping_add(1);
                p.viz[viz::HITS + r].set((self.hits[r] % (1 << 20)) as f32);
                p.viz[viz::HIT_STEP + r].set(i as f32);
            }
            self.release_before(ctx, n, r, end);
        }
        self.flush();
        self.publish_phases(&p, end);
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}
