//! MIDI Looper: records the notes on its MIDI input into loops of a given
//! number of bars, in time with the board or with a MIDI clock, and plays
//! them back on its MIDI output.
//!
//! Eight scenes, each a loop of its own: its BARS, how many times it
//! repeats (or endlessly), which way it plays (forward, reversed, or a
//! RANDOM pick of its slices, a GRID step each) and what follows its last
//! repeat (the next scene, a random one, or a stop). Scenes switch on the
//! next beat, bar or loop end (LAUNCH), from the panel, from Program
//! Change 1–8, or from notes on the CONTROL channel.
//!
//! REC arms the scene: recording starts on the next bar, on the first note
//! played, or on a Start (the board's or a MIDI clock's), and runs for its
//! bars; then it plays. DUB layers notes over it while it plays. UNDO takes
//! back the last recording, overdub or clear (and again, redoes it). What
//! is recorded joins the loop at the end of its pass, snapped to the GRID
//! as far as QUANTIZE says. With STEP, notes played go in one GRID step at
//! a time, a step sequencer on the same loop that the panel edits.

pub mod clock;

use std::sync::Arc;

use self::clock::{Clock, Timeline};
use super::steps::{note_event, steps_per_beat};
use super::{ModuleDef, ModuleParams, Rng};
use crate::audio::engine::{Buf, Inputs, MIDI_CAP, MidiBuf, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit};

pub const GRID: usize = 0;
pub const QUANTIZE: usize = 1;
pub const LAUNCH: usize = 2;
pub const TRANSPOSE: usize = 3;
pub const CHANNEL: usize = 4;
pub const CONTROL: usize = 5;

const PARAMS: [ParamSpec; 6] = [
    ParamSpec::lin("GRID", 0.0, 5.0, 3.0, Unit::Division),
    ParamSpec::lin("QUANTIZE", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("LAUNCH", 0.0, 2.0, 1.0, Unit::Integer),
    ParamSpec::lin("TRANSPOSE", -24.0, 24.0, 0.0, Unit::Semitones).bipolar(),
    ParamSpec::lin("CHANNEL", 0.0, 16.0, 0.0, Unit::Channel),
    ParamSpec::lin("CONTROL", 0.0, 16.0, 16.0, Unit::Channel),
];

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// When a scene starts: on the next beat, bar, or at the end of the loop
/// playing.
pub const LAUNCHES: [&str; 3] = ["BEAT", "BAR", "LOOP"];
/// Which way a scene plays.
pub const DIRECTIONS: [&str; 3] = ["FWD", "REV", "RANDOM"];
pub const FORWARD: usize = 0;
pub const REVERSE: usize = 1;
pub const RANDOM: usize = 2;
/// What follows a scene's last repeat.
pub const FOLLOWS: [&str; 3] = ["NEXT", "RANDOM", "STOP"];
pub const NEXT: usize = 0;
pub const ANY: usize = 1;
pub const HALT: usize = 2;
/// What starts a recording.
pub const TRIGGERS: [&str; 3] = ["BAR", "NOTE", "START"];
pub const ON_BAR: usize = 0;
pub const ON_NOTE: usize = 1;
pub const ON_START: usize = 2;
/// The repeats a scene can have (0: endless).
pub const REPEATS: [u32; 9] = [0, 1, 2, 3, 4, 6, 8, 12, 16];
pub const MAX_BARS: usize = 16;
/// Beats a bar (the board counts in 4/4) and ticks a beat.
pub const BAR: f64 = 4.0;
pub const PPQ: u32 = 96;

/// The CONTROL channel's notes: from C2, REC, DUB, PLAY, UNDO, CLEAR, then
/// REV and RANDOM on and off; from C3, scenes 1 to 8.
pub const COMMAND_NOTES: u8 = 36;
pub const SCENE_NOTES: u8 = 48;

/// Settings beyond the knobs, then the scenes.
pub mod data {
    /// Follow the board (0) or a MIDI clock (1).
    pub const SYNC: usize = 0;
    /// What starts a recording: `TRIGGERS`.
    pub const TRIGGER: usize = 1;
    /// Pass what comes in through to the output.
    pub const THRU: usize = 2;
    /// Step recording.
    pub const STEP: usize = 3;
    /// The scene selected (playing, or to play).
    pub const SCENE: usize = 4;
    pub const GLOBAL: usize = 5;

    /// In a scene: its bars, repeats (0: endless), direction, what follows,
    /// and how many notes it holds, then the notes (three values each).
    pub const BARS: usize = 0;
    pub const REPEATS: usize = 1;
    pub const DIRECTION: usize = 2;
    pub const FOLLOW: usize = 3;
    pub const COUNT: usize = 4;
    pub const HEADER: usize = 5;
    /// Notes a scene holds, at most.
    pub const CAP: usize = 256;
    pub const NOTE: usize = 3;
    pub const SCENE_LEN: usize = HEADER + CAP * NOTE;
    pub const SCENES: usize = 8;
    pub const LEN: usize = GLOBAL + SCENES * SCENE_LEN;

    /// Where scene `s` starts.
    pub const fn scene(s: usize) -> usize {
        GLOBAL + s * SCENE_LEN
    }
}

const fn defaults() -> [f32; data::LEN] {
    let mut d = [0.0; data::LEN];
    d[data::THRU] = 1.0;
    let mut s = 0;
    while s < data::SCENES {
        d[data::scene(s) + data::BARS] = 2.0;
        s += 1;
    }
    d
}

static DATA: [f32; data::LEN] = defaults();

/// What the panel asks for: a counter each, bumped, and its argument.
pub mod ask {
    pub const REC: usize = 0;
    pub const DUB: usize = 1;
    pub const PLAY: usize = 2;
    pub const UNDO: usize = 3;
    pub const CLEAR: usize = 4;
    pub const SCENE: usize = 5;
    pub const CURSOR: usize = 6;
    pub const LEN: usize = 7;
}

/// What the processor shows the panel.
pub mod viz {
    use super::ask;
    /// `state::*`, and 1 while overdubbing.
    pub const STATE: usize = 0;
    pub const DUB: usize = STATE + 1;
    /// The scene playing or recording (−1: none), and the one to come.
    pub const PLAYING: usize = DUB + 1;
    pub const QUEUED: usize = PLAYING + 1;
    /// Where the loop is, in beats from its start (−1: not running), and
    /// where in the scene that is (reversed, or the random slice).
    pub const POS: usize = QUEUED + 1;
    pub const CONTENT: usize = POS + 1;
    /// Passes played since the scene started.
    pub const PASS: usize = CONTENT + 1;
    /// The step recording's position, in ticks.
    pub const CURSOR: usize = PASS + 1;
    /// The MIDI clock's tempo (0: none), and whether time runs.
    pub const TEMPO: usize = CURSOR + 1;
    pub const RUNNING: usize = TEMPO + 1;
    /// The panel's requests (`ask`), and their arguments.
    pub const ASK: usize = RUNNING + 1;
    pub const ARG: usize = ASK + ask::LEN;
    pub const END: usize = ARG + ask::LEN;
}

pub mod state {
    pub const STOPPED: usize = 0;
    pub const ARMED: usize = 1;
    pub const RECORDING: usize = 2;
    pub const PLAYING: usize = 3;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "MIDI Looper",
    prefix: "LOOPER",
    hint: "loops, scenes, steps → MIDI",
    inputs: &[],
    outputs: &[],
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

/// A note of a loop, in ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Note {
    pub at: u32,
    pub len: u32,
    pub key: u8,
    pub velocity: u8,
    /// 0–15.
    pub channel: u8,
}

impl Note {
    fn pack(self) -> [f32; 3] {
        let key = self.key as u32 & 0x7F | (self.velocity as u32 & 0x7F) << 7 | (self.channel as u32 & 0x0F) << 14;
        [self.at as f32, self.len as f32, key as f32]
    }

    fn unpack([at, len, key]: [f32; 3]) -> Self {
        let k = key.max(0.0) as u32;
        Self {
            at: at.max(0.0) as u32,
            len: len.max(1.0) as u32,
            key: (k & 0x7F) as u8,
            velocity: ((k >> 7) & 0x7F).max(1) as u8,
            channel: ((k >> 14) & 0x0F) as u8,
        }
    }
}

pub fn flag(p: &ModuleParams, d: usize) -> bool {
    p.data[d].get() > 0.5
}

pub fn set_flag(p: &ModuleParams, d: usize, on: bool) {
    p.data[d].set(if on { 1.0 } else { 0.0 });
}

/// A small choice kept in `data[d]` (below `count`).
pub fn choice(p: &ModuleParams, d: usize, count: usize) -> usize {
    (p.data[d].get().round().max(0.0) as usize).min(count - 1)
}

/// The scene selected.
pub fn scene(p: &ModuleParams) -> usize {
    choice(p, data::SCENE, data::SCENES)
}

fn field(s: usize, f: usize) -> usize {
    data::scene(s.min(data::SCENES - 1)) + f
}

pub fn bars(p: &ModuleParams, s: usize) -> usize {
    (p.data[field(s, data::BARS)].get().round().max(1.0) as usize).min(MAX_BARS)
}

pub fn set_bars(p: &ModuleParams, s: usize, bars: usize) {
    p.data[field(s, data::BARS)].set(bars.clamp(1, MAX_BARS) as f32);
}

/// A scene's length, in ticks and in beats.
pub fn length(p: &ModuleParams, s: usize) -> u32 {
    bars(p, s) as u32 * BAR as u32 * PPQ
}

fn beats(p: &ModuleParams, s: usize) -> f64 {
    bars(p, s) as f64 * BAR
}

/// A scene's repeats (0: endless).
pub fn repeats(p: &ModuleParams, s: usize) -> u32 {
    p.data[field(s, data::REPEATS)].get().round().clamp(0.0, 16.0) as u32
}

pub fn set_repeats(p: &ModuleParams, s: usize, n: u32) {
    p.data[field(s, data::REPEATS)].set(n.min(16) as f32);
}

pub fn direction(p: &ModuleParams, s: usize) -> usize {
    choice(p, field(s, data::DIRECTION), DIRECTIONS.len())
}

pub fn set_direction(p: &ModuleParams, s: usize, d: usize) {
    p.data[field(s, data::DIRECTION)].set(d.min(DIRECTIONS.len() - 1) as f32);
}

pub fn follow(p: &ModuleParams, s: usize) -> usize {
    choice(p, field(s, data::FOLLOW), FOLLOWS.len())
}

pub fn set_follow(p: &ModuleParams, s: usize, f: usize) {
    p.data[field(s, data::FOLLOW)].set(f.min(FOLLOWS.len() - 1) as f32);
}

pub fn count(p: &ModuleParams, s: usize) -> usize {
    (p.data[field(s, data::COUNT)].get().max(0.0) as usize).min(data::CAP)
}

pub fn note(p: &ModuleParams, s: usize, i: usize) -> Note {
    let at = field(s, data::HEADER + i.min(data::CAP - 1) * data::NOTE);
    Note::unpack([p.data[at].get(), p.data[at + 1].get(), p.data[at + 2].get()])
}

pub fn set_note(p: &ModuleParams, s: usize, i: usize, n: Note) {
    let at = field(s, data::HEADER + i.min(data::CAP - 1) * data::NOTE);
    for (k, v) in n.pack().into_iter().enumerate() {
        p.data[at + k].set(v);
    }
}

/// Adds a note (false when the scene is full).
pub fn add_note(p: &ModuleParams, s: usize, n: Note) -> bool {
    let c = count(p, s);
    if c >= data::CAP {
        return false;
    }
    set_note(p, s, c, n);
    p.data[field(s, data::COUNT)].set((c + 1) as f32);
    true
}

/// Removes note `i` (the last one takes its place).
pub fn remove_note(p: &ModuleParams, s: usize, i: usize) {
    let c = count(p, s);
    if i < c {
        set_note(p, s, i, note(p, s, c - 1));
        p.data[field(s, data::COUNT)].set((c - 1) as f32);
    }
}

pub fn clear(p: &ModuleParams, s: usize) {
    p.data[field(s, data::COUNT)].set(0.0);
}

/// Scene `from` (settings and notes) into scene `to`.
pub fn copy_scene(p: &ModuleParams, from: usize, to: usize) {
    if from == to {
        return;
    }
    for k in 0..data::SCENE_LEN {
        p.data[field(to, k)].set(p.data[field(from, k)].get());
    }
}

/// A GRID step, in ticks.
pub fn step(p: &ModuleParams) -> u32 {
    (PPQ as f64 / steps_per_beat(p.get(GRID))).round().max(1.0) as u32
}

/// Asks the processor for `what` (`ask::*`), with `arg`.
pub fn request(p: &ModuleParams, what: usize, arg: f32) {
    p.viz[viz::ARG + what].set(arg);
    p.viz[viz::ASK + what].set(p.viz[viz::ASK + what].get() + 1.0);
}

pub fn format(p: &ModuleParams, i: usize) -> String {
    let v = p.get(i);
    match i {
        LAUNCH => LAUNCHES[(v.round().max(0.0) as usize).min(LAUNCHES.len() - 1)].to_owned(),
        CHANNEL if v.round() < 1.0 => "as played".to_owned(),
        CONTROL if v.round() < 1.0 => "off".to_owned(),
        _ => PARAMS[i].format(v),
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Stopped,
    Armed,
    Recording,
    Playing,
}

/// A key down while recording: when, where in the scene, how hard.
#[derive(Clone, Copy)]
struct Down {
    t: f64,
    content: f64,
    velocity: u8,
}

/// A note playing, and when it ends.
#[derive(Clone, Copy)]
struct Sounding {
    key: u8,
    channel: u8,
    off: f64,
}

/// Points in time where something happens, in the order they happen when
/// they meet.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Boundary {
    RecEnd,
    Switch,
    Wrap,
    Slice,
    Arm,
}

/// Notes sounding at once, at most.
const VOICES: usize = 64;

pub struct Looper {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    stream: u32,
    sr: f32,
    out: MidiBuf,
    clock: Clock,
    state: State,
    /// What an armed REC goes back to if cancelled.
    armed_from: State,
    dub: bool,
    scene: usize,
    queued: Option<usize>,
    /// Where the scene started on the timeline (beats), the pass it is in
    /// and the passes finished.
    origin: f64,
    cycle: i64,
    pass: u32,
    /// RANDOM: the timeline's slice now, and the scene's slice it plays.
    slice: i64,
    source: i64,
    rec_end: f64,
    down: Box<[Option<Down>]>,
    /// Notes recorded this pass, joining the loop at its end.
    pending: Vec<Note>,
    sounding: Vec<Sounding>,
    undo: Box<[f32]>,
    undo_scene: Option<usize>,
    seen: [f32; ask::LEN],
    cursor: u32,
    step_keys: usize,
    step_was: bool,
    was_running: bool,
    rng: Rng,
    /// This block's messages out: frame, order (note-offs first), message.
    msgs: Vec<(usize, u8, [u8; 3])>,
    /// Where the last block ended on the timeline.
    last_end: f64,
    /// The block being worked on: its first beat, beats a sample, frames.
    b0: f64,
    bps: f64,
    n: usize,
}

impl Looper {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        let scene = scene(&params);
        let state = if count(&params, scene) > 0 { State::Playing } else { State::Stopped };
        let seen = std::array::from_fn(|k| params.viz[viz::ASK + k].get());
        Self {
            stream: midi.stream_head(),
            midi,
            sr: sample_rate,
            out: MidiBuf::default(),
            clock: Clock::new(sample_rate),
            state,
            armed_from: State::Stopped,
            dub: false,
            scene,
            queued: None,
            origin: 0.0,
            cycle: 0,
            pass: 0,
            slice: 0,
            source: 0,
            rec_end: 0.0,
            down: vec![None; 16 * 128].into_boxed_slice(),
            pending: Vec::with_capacity(data::CAP),
            sounding: Vec::with_capacity(VOICES),
            undo: vec![0.0; data::SCENE_LEN].into_boxed_slice(),
            undo_scene: None,
            seen,
            cursor: 0,
            step_keys: 0,
            step_was: false,
            was_running: false,
            rng: Rng::new(0x0100_BE12),
            msgs: Vec::with_capacity(2 * MIDI_CAP),
            last_end: 0.0,
            b0: 0.0,
            bps: 1e-5,
            n: 64,
            params,
        }
    }

    fn frame(&self, t: f64) -> usize {
        (((t - self.b0) / self.bps).round().max(0.0) as usize).min(self.n.saturating_sub(1))
    }

    fn emit(&mut self, frame: usize, msg: [u8; 3]) {
        if self.msgs.len() < self.msgs.capacity() {
            let order = if note_event(msg) == Some(false) { 0 } else { 1 };
            self.msgs.push((frame, order, msg));
        }
    }

    /// The scene's length now, in beats.
    fn beats(&self) -> f64 {
        beats(&self.params, self.scene)
    }

    /// A RANDOM slice, in beats (a GRID step).
    fn slice_beats(&self) -> f64 {
        step(&self.params) as f64 / PPQ as f64
    }

    fn pick_source(&mut self) {
        let slices = (self.beats() / self.slice_beats()).round().max(1.0) as u32;
        self.source = (self.rng.unit() * slices as f32) as i64 % slices as i64;
    }

    /// Ends every note playing.
    fn end_all(&mut self, frame: usize) {
        while let Some(s) = self.sounding.pop() {
            self.emit(frame, [0x80 | s.channel, s.key, 0]);
        }
    }

    fn snapshot(&mut self, s: usize) {
        for (k, v) in self.undo.iter_mut().enumerate() {
            *v = self.params.data[field(s, k)].get();
        }
        self.undo_scene = Some(s);
    }

    /// The last recording, overdub or clear undone (and again, redone).
    fn undo(&mut self, frame: usize) {
        let Some(s) = self.undo_scene else { return };
        for (k, v) in self.undo.iter_mut().enumerate() {
            let now = self.params.data[field(s, k)].get();
            self.params.data[field(s, k)].set(*v);
            *v = now;
        }
        self.pending.clear();
        self.end_all(frame);
    }

    /// The notes recorded this pass join the loop, snapped to the GRID as
    /// far as QUANTIZE says.
    fn commit(&mut self) {
        let p = self.params.clone();
        let (grid, amount, len) = (step(&p) as f64, p.get(QUANTIZE) as f64, length(&p, self.scene));
        for mut n in self.pending.drain(..) {
            let at = n.at as f64;
            let snapped = (at / grid).round() * grid;
            n.at = (at + (snapped - at) * amount).round() as u32 % len.max(1);
            add_note(&p, self.scene, n);
        }
    }

    fn begin_recording(&mut self, at: f64) {
        self.snapshot(self.scene);
        clear(&self.params, self.scene);
        self.pending.clear();
        let frame = self.frame(at);
        self.end_all(frame);
        self.state = State::Recording;
        self.origin = at;
        self.cycle = 0;
        self.pass = 0;
        self.rec_end = at + self.beats();
    }

    fn finish_recording(&mut self) {
        self.commit();
        self.state = State::Playing;
    }

    fn stop(&mut self, frame: usize) {
        if self.state == State::Recording {
            self.finish_recording();
        }
        self.commit();
        self.end_all(frame);
        self.state = State::Stopped;
        self.dub = false;
        self.queued = None;
    }

    /// Scene `s` asked for: chosen now if nothing plays, else at LAUNCH.
    fn ask_scene(&mut self, s: usize) {
        let s = s.min(data::SCENES - 1);
        match self.state {
            State::Stopped if self.queued.is_none() => self.select(s),
            State::Stopped => {
                self.select(s);
                self.queued = Some(s);
            }
            State::Armed => self.select(s),
            State::Recording | State::Playing => self.queued = Some(s),
        }
    }

    fn select(&mut self, s: usize) {
        self.scene = s;
        self.params.data[data::SCENE].set(s as f32);
    }

    /// Starts playing at the next LAUNCH point.
    fn start(&mut self) {
        self.queued = Some(self.scene);
    }

    /// Scene `s` from its start, at `at`.
    fn switch(&mut self, s: usize, at: f64) {
        self.commit();
        let frame = self.frame(at);
        self.end_all(frame);
        self.select(s);
        self.queued = None;
        self.dub = false;
        self.state = State::Playing;
        self.origin = at;
        self.cycle = 0;
        self.pass = 0;
        self.slice = 0;
        self.pick_source();
    }

    fn command(&mut self, what: usize, arg: f32) {
        let p = self.params.clone();
        match what {
            ask::REC => match self.state {
                State::Armed => self.state = self.armed_from,
                State::Recording => self.finish_recording(),
                state => {
                    self.armed_from = state;
                    self.state = State::Armed;
                    self.dub = false;
                }
            },
            ask::DUB => match self.state {
                State::Playing => {
                    self.dub = !self.dub;
                    if self.dub {
                        self.snapshot(self.scene);
                    }
                }
                State::Recording => self.dub = !self.dub,
                State::Stopped => {
                    self.dub = true;
                    self.snapshot(self.scene);
                    self.start();
                }
                State::Armed => {}
            },
            ask::PLAY => match self.state {
                State::Stopped if self.queued.is_none() => self.start(),
                _ => self.stop(0),
            },
            ask::UNDO => self.undo(0),
            ask::CLEAR => {
                self.snapshot(self.scene);
                clear(&p, self.scene);
                self.pending.clear();
                self.dub = false;
                self.end_all(0);
            }
            ask::SCENE => self.ask_scene(arg.max(0.0) as usize),
            ask::CURSOR => self.cursor = arg.max(0.0) as u32 % length(&p, self.scene).max(1),
            _ => {}
        }
    }

    /// A note on the CONTROL channel.
    fn control(&mut self, key: u8) {
        let p = self.params.clone();
        match key.wrapping_sub(COMMAND_NOTES) {
            k @ 0..=4 => self.command(k as usize, 0.0),
            k @ 5..=6 => {
                let to = if k == 5 { REVERSE } else { RANDOM };
                let d = if direction(&p, self.scene) == to { FORWARD } else { to };
                set_direction(&p, self.scene, d);
            }
            _ if (SCENE_NOTES..SCENE_NOTES + data::SCENES as u8).contains(&key) => self.ask_scene((key - SCENE_NOTES) as usize),
            _ => {}
        }
    }

    /// Where in the scene the timeline's beat `t` falls, as it plays.
    fn content_at(&self, t: f64) -> f64 {
        let len = self.beats();
        let pos = (t - self.origin).max(0.0).rem_euclid(len);
        if self.state == State::Recording {
            return pos;
        }
        match direction(&self.params, self.scene) {
            REVERSE => (len - pos).rem_euclid(len),
            RANDOM => {
                let sl = self.slice_beats();
                self.source as f64 * sl + pos.rem_euclid(sl)
            }
            _ => pos,
        }
    }

    fn recording(&self) -> bool {
        self.state == State::Recording || (self.state == State::Playing && self.dub)
    }

    fn key_down(&mut self, channel: u8, key: u8, velocity: u8, t: f64, running: bool) {
        let p = self.params.clone();
        if self.state == State::Armed && running && choice(&p, data::TRIGGER, TRIGGERS.len()) == ON_NOTE {
            // The bar it falls in; or, played up to an eighth early, the
            // next one.
            let bar = (t / BAR).floor() * BAR;
            self.begin_recording(if t - bar > BAR - 0.5 { bar + BAR } else { bar });
        }
        if self.recording() && running {
            let content = self.content_at(t);
            self.down[channel as usize * 128 + key as usize] = Some(Down { t, content, velocity });
        } else if flag(&p, data::STEP) {
            let len = length(&p, self.scene);
            add_note(&p, self.scene, Note { at: self.cursor % len.max(1), len: step(&p), key, velocity, channel });
            self.step_keys += 1;
        }
    }

    fn key_up(&mut self, channel: u8, key: u8, t: f64) {
        let p = self.params.clone();
        if let Some(d) = self.down[channel as usize * 128 + key as usize].take() {
            let len = self.beats();
            let held = (t - d.t).clamp(1.0 / PPQ as f64, len);
            // Reversed, a note starts in the scene where it was let go.
            let reversed = self.state == State::Playing && direction(&p, self.scene) == REVERSE;
            let start = if reversed { self.content_at(t) } else { d.content };
            let ticks = |b: f64| (b * PPQ as f64).round() as u32;
            if self.pending.len() < self.pending.capacity() {
                self.pending.push(Note { at: ticks(start) % ticks(len).max(1), len: ticks(held).max(1), key, velocity: d.velocity, channel });
            }
        } else if self.step_keys > 0 {
            self.step_keys -= 1;
            if self.step_keys == 0 {
                self.cursor = (self.cursor + step(&p)) % length(&p, self.scene).max(1);
            }
        }
    }

    /// This block's messages from the MIDI input.
    fn input(&mut self, msgs: &[(usize, [u8; 3])], running: bool) {
        let p = self.params.clone();
        let control = p.get(CONTROL).round() as u8;
        let thru = flag(&p, data::THRU);
        for &(frame, msg) in msgs {
            let t = self.b0 + frame as f64 * self.bps;
            let channel = msg[0] & 0x0F;
            if msg[0] & 0xF0 == 0xC0 {
                if (msg[1] as usize) < data::SCENES {
                    self.ask_scene(msg[1] as usize);
                }
                continue;
            }
            if control > 0 && channel + 1 == control {
                if note_event(msg) == Some(true) {
                    self.control(msg[1]);
                }
                continue;
            }
            if thru {
                self.emit(frame, msg);
            }
            match note_event(msg) {
                Some(true) => self.key_down(channel, msg[1], msg[2], t, running),
                Some(false) => self.key_up(channel, msg[1], t),
                None => {}
            }
        }
    }

    /// When a scene asked for starts, from `t`.
    fn launch(&self, t: f64) -> f64 {
        let ceil = |x: f64, unit: f64| ((x - 1e-9) / unit).ceil() * unit;
        match self.params.get(LAUNCH).round() as usize {
            0 => ceil(t, 1.0),
            2 if self.state == State::Playing => self.origin + (self.cycle + 1) as f64 * self.beats(),
            _ => ceil(t, BAR),
        }
    }

    /// The first thing to happen at or after `t`.
    fn next(&self, t: f64) -> (f64, Option<Boundary>) {
        let p = &self.params;
        let mut best = (f64::INFINITY, None);
        let mut offer = |at: f64, what: Boundary| {
            if at >= t - 1e-9 && at < best.0 - 1e-9 {
                best = (at, Some(what));
            }
        };
        if self.state == State::Recording {
            offer(self.rec_end, Boundary::RecEnd);
        } else if self.queued.is_some() {
            offer(self.launch(t), Boundary::Switch);
        }
        if self.state == State::Playing {
            offer(self.origin + (self.cycle + 1) as f64 * self.beats(), Boundary::Wrap);
            if direction(p, self.scene) == RANDOM {
                offer(self.origin + (self.slice + 1) as f64 * self.slice_beats(), Boundary::Slice);
            }
        }
        if self.state == State::Armed && choice(p, data::TRIGGER, TRIGGERS.len()) == ON_BAR {
            offer(((t - 1e-9) / BAR).ceil() * BAR, Boundary::Arm);
        }
        best
    }

    fn boundary(&mut self, what: Boundary, at: f64) {
        let p = self.params.clone();
        match what {
            Boundary::RecEnd => {
                self.finish_recording();
            }
            Boundary::Switch => {
                if let Some(s) = self.queued {
                    self.switch(s, at);
                }
            }
            Boundary::Wrap => {
                self.commit();
                self.cycle += 1;
                self.pass += 1;
                let repeats = repeats(&p, self.scene);
                if repeats > 0 && self.pass >= repeats && !self.dub && self.queued.is_none() {
                    self.follow(at);
                }
            }
            Boundary::Slice => {
                self.slice += 1;
                self.pick_source();
            }
            Boundary::Arm => self.begin_recording(at),
        }
    }

    /// After a scene's last repeat.
    fn follow(&mut self, at: f64) {
        let p = self.params.clone();
        let others = |s: usize| s != self.scene && count(&p, s) > 0;
        match follow(&p, self.scene) {
            NEXT => {
                if let Some(s) = (1..data::SCENES).map(|k| (self.scene + k) % data::SCENES).find(|&s| others(s)) {
                    self.switch(s, at);
                }
            }
            ANY => {
                let choices = (0..data::SCENES).filter(|&s| others(s)).count();
                if choices > 0 {
                    let pick = (self.rng.unit() * choices as f32) as usize % choices;
                    if let Some(s) = (0..data::SCENES).filter(|&s| others(s)).nth(pick) {
                        self.switch(s, at);
                    }
                }
            }
            HALT => {
                let frame = self.frame(at);
                self.stop(frame);
            }
            _ => {}
        }
    }

    /// The scene's notes that start in `[t0, t1)` of the timeline.
    fn play(&mut self, t0: f64, t1: f64) {
        let playing = self.state == State::Playing || (self.state == State::Armed && self.armed_from == State::Playing);
        if !playing || t1 <= t0 {
            return;
        }
        let p = self.params.clone();
        let len = self.beats();
        let mut pos0 = t0 - self.origin - self.cycle as f64 * len;
        if !(-1e-9..len).contains(&pos0) {
            // Time jumped (or BARS changed): back in step, quietly.
            self.cycle = ((t0 - self.origin) / len).floor() as i64;
            pos0 = t0 - self.origin - self.cycle as f64 * len;
        }
        let dir = direction(&p, self.scene);
        // Forward and reversed play the scene from pos0; RANDOM plays the
        // slice it picked, from the same place within a slice.
        let from = if dir == RANDOM {
            let sl = self.slice_beats();
            let slice = ((t0 - self.origin) / sl + 1e-9).floor() as i64;
            if slice != self.slice {
                self.slice = slice;
                self.pick_source();
            }
            self.source as f64 * sl + (t0 - self.origin - slice as f64 * sl)
        } else {
            pos0
        };
        let to = from + (t1 - t0);
        let transpose = p.get(TRANSPOSE).round() as i32;
        let out_channel = p.get(CHANNEL).round() as i32;
        for i in 0..count(&p, self.scene) {
            let n = note(&p, self.scene, i);
            let (start, dur) = (n.at as f64 / PPQ as f64, n.len as f64 / PPQ as f64);
            if start >= len {
                continue;
            }
            let start = if dir == REVERSE { (len - start - dur).rem_euclid(len) } else { start };
            if start < from - 1e-9 || start >= to - 1e-9 {
                continue;
            }
            let on = t0 + (start - from);
            let key = (n.key as i32 + transpose).clamp(0, 127) as u8;
            let channel = if out_channel >= 1 { (out_channel - 1) as u8 } else { n.channel };
            let frame = self.frame(on);
            if let Some(k) = self.sounding.iter().position(|s| s.key == key && s.channel == channel) {
                self.sounding.swap_remove(k);
                self.emit(frame, [0x80 | channel, key, 0]);
            }
            if self.sounding.len() == VOICES
                && let Some(k) = (0..VOICES).min_by(|&a, &b| self.sounding[a].off.total_cmp(&self.sounding[b].off))
            {
                let s = self.sounding.swap_remove(k);
                self.emit(frame, [0x80 | s.channel, s.key, 0]);
            }
            self.emit(frame, [0x90 | channel, key, n.velocity]);
            self.sounding.push(Sounding { key, channel, off: on + dur });
        }
    }

    /// The block from `b0` to `b1`, boundary by boundary.
    fn run(&mut self, b1: f64) {
        let mut t = self.b0;
        if self.state == State::Playing {
            // After a jump in time (or new BARS), back in step before the
            // boundaries are worked out.
            let len = self.beats();
            let pos = t - self.origin - self.cycle as f64 * len;
            if !(-1e-9..len).contains(&pos) {
                self.cycle = ((t - self.origin) / len).floor() as i64;
            }
        }
        for _ in 0..32 {
            let (at, what) = self.next(t);
            self.play(t, at.min(b1));
            let Some(what) = what.filter(|_| at < b1) else { break };
            self.boundary(what, at);
            t = at;
        }
    }

    fn publish(&self, running: bool, end: f64) {
        let p = &self.params;
        let state = match self.state {
            State::Stopped => state::STOPPED,
            State::Armed => state::ARMED,
            State::Recording => state::RECORDING,
            State::Playing => state::PLAYING,
        };
        p.viz[viz::STATE].set(state as f32);
        p.viz[viz::DUB].set(if self.dub { 1.0 } else { 0.0 });
        let active = matches!(self.state, State::Playing | State::Recording) || (self.state == State::Armed && self.armed_from == State::Playing);
        p.viz[viz::PLAYING].set(if active { self.scene as f32 } else { -1.0 });
        p.viz[viz::QUEUED].set(self.queued.map_or(-1.0, |s| s as f32));
        let len = self.beats();
        let pos = (end - self.origin).rem_euclid(len);
        let shown = running && active;
        p.viz[viz::POS].set(if shown { pos as f32 } else { -1.0 });
        p.viz[viz::CONTENT].set(if shown { self.content_at(end) as f32 } else { -1.0 });
        p.viz[viz::PASS].set(self.pass as f32);
        p.viz[viz::CURSOR].set(self.cursor as f32);
        p.viz[viz::TEMPO].set(self.clock.bpm(self.sr));
        p.viz[viz::RUNNING].set(if running { 1.0 } else { 0.0 });
    }
}

impl Processor for Looper {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, _outputs: &mut [Buf], n: usize) {
        self.out.clear();
        self.msgs.clear();
        let p = self.params.clone();

        // The input: clock and transport to the clock, the rest kept.
        let mut incoming = [(0usize, [0u8; 3]); MIDI_CAP];
        let mut count_in = 0;
        let mut take = |frame: usize, msg: [u8; 3], clock: &mut Clock| {
            if msg[0] >= 0xF0 {
                clock.handle(msg, frame);
            } else if count_in < MIDI_CAP {
                incoming[count_in] = (frame, msg);
                count_in += 1;
            }
        };
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                take(e.frame as usize, e.data, &mut self.clock);
            }
            self.stream = self.midi.stream_head();
        } else {
            let clock = &mut self.clock;
            self.midi.read_stream(&mut self.stream, |msg| take(0, msg, clock));
        }

        // The timeline: the board's, or the MIDI clock's.
        let midi_clock = self.clock.block(n);
        let tl = if flag(&p, data::SYNC) {
            midi_clock
        } else {
            Timeline { beat: ctx.beat, beats_per_sample: ctx.beats_per_sample, running: ctx.playing, restart: ctx.restart }
        };
        // A clock's ticks can put time a hair back: it never goes back
        // (a note on the line would play twice), short of a real jump.
        let back = self.last_end - tl.beat;
        self.b0 = if !tl.restart && back > 0.0 && back < 0.25 { self.last_end } else { tl.beat };
        self.bps = tl.beats_per_sample.max(1e-12);
        self.n = n.max(1);
        let b1 = self.b0 + self.bps * n as f64;

        // The panel's requests.
        for k in 0..ask::LEN {
            let c = p.viz[viz::ASK + k].get();
            if c != self.seen[k] {
                self.seen[k] = c;
                self.command(k, p.viz[viz::ARG + k].get());
            }
        }
        let step_on = flag(&p, data::STEP);
        if step_on && !self.step_was {
            self.snapshot(self.scene);
            self.step_keys = 0;
        }
        self.step_was = step_on;

        if tl.restart {
            self.origin = self.b0;
            self.cycle = 0;
            self.pass = 0;
            self.slice = 0;
            if self.state == State::Armed && choice(&p, data::TRIGGER, TRIGGERS.len()) == ON_START {
                let at = self.b0;
                self.begin_recording(at);
            }
        }

        self.input(&incoming[..count_in], tl.running);
        if tl.running {
            self.was_running = true;
            self.run(b1);
            // The notes that end in this block.
            let mut k = 0;
            while k < self.sounding.len() {
                if self.sounding[k].off < b1 {
                    let s = self.sounding.swap_remove(k);
                    let frame = self.frame(s.off);
                    self.emit(frame, [0x80 | s.channel, s.key, 0]);
                } else {
                    k += 1;
                }
            }
        } else if self.was_running {
            // Stopped: the notes end, and a recording with them.
            self.was_running = false;
            if self.state == State::Recording {
                self.finish_recording();
            }
            self.commit();
            self.end_all(0);
        }

        self.last_end = b1;
        self.msgs.sort_unstable_by_key(|&(frame, order, _)| (frame, order));
        for &(frame, _, msg) in &self.msgs {
            self.out.push(frame, msg);
        }
        self.publish(tl.running, b1);
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}
