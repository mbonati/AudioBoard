//! Song Mode: a song made of the board's own states, played in order.
//!
//! - A **scene** is a snapshot of the board: for each node its knobs, mode,
//!   switch, bypass and the settings beyond the knobs that make a pattern
//!   (a sequencer's steps, Orrery's rings, MIDIcer's faders and dice,
//!   Automa's progression…), and each mixer's faders, pans, sends and mutes.
//!   What a module keeps that isn't a pattern (a MIDI Looper's recordings,
//!   Orrery's pattern buttons, counters the panels bump, panel sizes) and
//!   plugins' states are left out: see [`data_ranges`].
//! - A **step** of the song plays a scene for a number of bars, in a key
//!   (the song's home key transposed), optionally at a tempo, gliding the
//!   continuous knobs into it over MORPH bars, and then goes on to the next
//!   step, repeats, or stops.
//! - The key moves the notes of the nodes that follow it (on the engine, see
//!   `audio::cue::KeyShift`) and the pitch knobs of the sources that follow
//!   it (a Drone's PITCH, an oscillator's FREQ…), see [`key_knobs`].
//!
//! The [`Player`] runs on the UI thread and sends the engine one change at a
//! time, ahead of its beat; the engine makes it on the beat, to the sample.
//! A scene is recalled only when the step's scene differs from the one on
//! the board, so two steps of the same scene play on, tweaks and all.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};

use crate::audio::cue::{self, Cue, Field, Glide, NodeCue, Set, Target};
use crate::audio::engine::NodeId;
use crate::audio::modules::{ModuleKind, fjord, iris, midicer, orrery, pops, vesper};
use crate::audio::params::{NOTE_NAMES, OSC_PARAMS, ParamSpec, TEMPO_RANGE, Taper, Unit, Waveform, osc};
use crate::graph::{Graph, Node, NodeKind, is_midi};

/// The song counts in 4/4.
pub const BEATS_PER_BAR: f64 = 4.0;
pub const MAX_BARS: u32 = 64;
pub const MAX_STEPS: usize = 64;
pub const MAX_SCENES: usize = 32;
/// MORPH lengths offered, in bars.
pub const MORPHS: [u32; 5] = [0, 1, 2, 4, 8];
/// Scene colours (indices into the panel's palette).
pub const COLORS: u8 = 8;

/// What a step does after its last bar.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Then {
    /// The next step (after the last, the first).
    #[default]
    Next,
    /// The step again, until another is asked for.
    Loop,
    /// The transport stops.
    Stop,
}

impl Then {
    pub const ALL: [Then; 3] = [Then::Next, Then::Loop, Then::Stop];

    pub fn name(self) -> &'static str {
        match self {
            Then::Next => "NEXT",
            Then::Loop => "LOOP",
            Then::Stop => "STOP",
        }
    }
}

/// When a step asked for while playing comes in.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Quant {
    Bar,
    Two,
    Four,
    /// At the end of the step playing.
    #[default]
    End,
}

impl Quant {
    pub const ALL: [Quant; 4] = [Quant::Bar, Quant::Two, Quant::Four, Quant::End];

    pub fn name(self) -> &'static str {
        match self {
            Quant::Bar => "1 BAR",
            Quant::Two => "2 BARS",
            Quant::Four => "4 BARS",
            Quant::End => "END",
        }
    }
}

/// A node's settings in a scene, by index (by name in the file).
#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Module {
        kind: ModuleKind,
        values: Vec<f32>,
        mode: usize,
        toggle: bool,
        /// Ranges of the module's data (see [`data_ranges`]): where each
        /// starts, and its values.
        data: Vec<(usize, Vec<f32>)>,
        bypass: bool,
    },
    Osc {
        values: Vec<f32>,
        wave: u32,
    },
    Mixer {
        fader: Vec<f32>,
        pan: Vec<f32>,
        aux: Vec<f32>,
        mute: Vec<bool>,
        solo: Vec<bool>,
        master: f32,
        aux_master: f32,
        master_mute: bool,
    },
}

#[derive(Clone, PartialEq, Debug)]
pub struct Scene {
    /// Steps refer to scenes by id, so scenes can be reordered and renamed.
    pub id: u32,
    pub name: String,
    pub color: u8,
    pub nodes: BTreeMap<NodeId, State>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Step {
    pub scene: u32,
    pub bars: u32,
    /// The key, as a pitch class (0 = C); none: the song's home key.
    pub key: Option<u8>,
    /// Bars the continuous knobs glide over into the scene (0: a cut).
    pub morph: u32,
    /// The tempo from this step on (none: as it is).
    pub tempo: Option<f32>,
    pub then: Then,
}

impl Step {
    pub fn new(scene: u32) -> Self {
        Self {
            scene,
            bars: 4,
            key: None,
            morph: 0,
            tempo: None,
            then: Then::Next,
        }
    }

    pub fn beats(&self) -> f64 {
        self.bars.clamp(1, MAX_BARS) as f64 * BEATS_PER_BAR
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Song {
    pub scenes: Vec<Scene>,
    pub steps: Vec<Step>,
    /// The home key, as a pitch class.
    pub home: u8,
    /// The song plays with the transport.
    pub on: bool,
    /// Every step repeats until another is asked for (live).
    pub hold: bool,
    pub quant: Quant,
    /// The step Start plays from.
    pub start: usize,
    /// Nodes the song leaves alone (played by hand).
    pub free: BTreeSet<NodeId>,
    /// Nodes that follow the key, or don't, against their default (see
    /// [`follows_by_default`]).
    pub keys: BTreeMap<NodeId, bool>,
    /// MIDI channel whose notes and program changes pick steps (0: off).
    pub channel: u8,
}

impl Song {
    pub fn scene(&self, id: u32) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }

    pub fn scene_mut(&mut self, id: u32) -> Option<&mut Scene> {
        self.scenes.iter_mut().find(|s| s.id == id)
    }

    /// The transposition of `step`'s key from the home key, in semitones
    /// (the nearest: −5 to +6).
    pub fn shift(&self, step: &Step) -> i8 {
        let Some(key) = step.key else {
            return 0;
        };
        let d = (key as i32 - self.home as i32).rem_euclid(12);
        (if d > 6 { d - 12 } else { d }) as i8
    }

    /// The key `step` plays in, as a pitch class.
    pub fn key_of(&self, step: &Step) -> u8 {
        step.key.unwrap_or(self.home) % 12
    }

    /// Whether `node` follows the song's key.
    pub fn follows(&self, node: &Node) -> bool {
        keyable(node)
            && self
                .keys
                .get(&node.id)
                .copied()
                .unwrap_or_else(|| follows_by_default(node))
    }

    /// Whether the song sets `node` (not left alone, and something it can
    /// keep: plugins are left out).
    pub fn drives(&self, node: &Node) -> bool {
        !self.free.contains(&node.id) && !matches!(node.kind, NodeKind::Plugin(_))
    }

    fn next_scene_id(&self) -> u32 {
        self.scenes.iter().map(|s| s.id).max().unwrap_or(0) + 1
    }

    /// A new scene holding `nodes` (see [`capture`]). Returns its id.
    pub fn add_scene(&mut self, nodes: BTreeMap<NodeId, State>) -> u32 {
        let id = self.next_scene_id();
        let used: Vec<u8> = self.scenes.iter().map(|s| s.color).collect();
        let color = (0..COLORS)
            .find(|c| !used.contains(c))
            .unwrap_or(self.scenes.len() as u8 % COLORS);
        self.scenes.push(Scene {
            id,
            name: scene_name(self.scenes.len()),
            color,
            nodes,
        });
        id
    }

    /// Keeps `nodes` (see [`capture`]) in scene `id`.
    pub fn store(&mut self, id: u32, nodes: BTreeMap<NodeId, State>) {
        if let Some(s) = self.scene_mut(id) {
            s.nodes = nodes;
        }
    }

    /// Removes scene `id` and the steps that play it.
    pub fn remove_scene(&mut self, id: u32) {
        self.scenes.retain(|s| s.id != id);
        self.steps.retain(|s| s.scene != id);
        self.start = self.start.min(self.steps.len().saturating_sub(1));
    }

    /// A copy of scene `id` after it; returns the copy's id.
    pub fn duplicate_scene(&mut self, id: u32) -> Option<u32> {
        let at = self.scenes.iter().position(|s| s.id == id)?;
        let mut copy = self.scenes[at].clone();
        copy.id = self.next_scene_id();
        copy.name = format!("{} 2", copy.name);
        copy.color = (copy.color + 1) % COLORS;
        let new = copy.id;
        self.scenes.insert(at + 1, copy);
        Some(new)
    }

    /// Adds a step playing scene `scene` after step `after` (at the end
    /// with none); returns its index.
    pub fn add_step(&mut self, scene: u32, after: Option<usize>) -> Option<usize> {
        if self.steps.len() >= MAX_STEPS {
            return None;
        }
        let at = after.map_or(self.steps.len(), |a| (a + 1).min(self.steps.len()));
        let mut step = Step::new(scene);
        // A new step after another keeps its length and key.
        if let Some(prev) = after.and_then(|a| self.steps.get(a)) {
            step.bars = prev.bars;
            step.key = prev.key;
        }
        self.steps.insert(at, step);
        if self.start >= at && at > 0 && self.start + 1 < self.steps.len() {
            self.start += 1;
        }
        Some(at)
    }

    pub fn remove_step(&mut self, i: usize) {
        if i < self.steps.len() {
            self.steps.remove(i);
        }
        if self.start > i || self.start >= self.steps.len() {
            self.start = self.start.saturating_sub(1);
        }
    }

    /// Moves step `from` to index `to`.
    pub fn move_step(&mut self, from: usize, to: usize) {
        if from >= self.steps.len() {
            return;
        }
        let step = self.steps.remove(from);
        let to = to.min(self.steps.len());
        self.steps.insert(to, step);
    }

    /// The song's length in beats, once through.
    pub fn beats(&self) -> f64 {
        self.steps.iter().map(Step::beats).sum()
    }
}

/// "A", "B"… then "A2"…
pub fn scene_name(i: usize) -> String {
    let letter = (b'A' + (i % 26) as u8) as char;
    match i / 26 {
        0 => letter.to_string(),
        n => format!("{letter}{}", n + 1),
    }
}

pub fn key_name(pc: u8) -> &'static str {
    NOTE_NAMES[pc as usize % 12]
}

// ---------------------------------------------------------------------------
// What a scene holds
// ---------------------------------------------------------------------------

/// The parts of a module's data a scene keeps: its pattern, not what it
/// records or holds in banks, nor what its panel asks for or shows.
pub fn data_ranges(kind: ModuleKind, len: usize) -> Vec<Range<usize>> {
    let all = vec![0..len];
    let ranges = match kind {
        ModuleKind::Sequencer
        | ModuleKind::Automa
        | ModuleKind::Colloid
        | ModuleKind::Nullpunkt
        | ModuleKind::Aether
        | ModuleKind::Larsen => all,
        // Not RESTART, a counter the panel bumps.
        ModuleKind::Midicer => {
            vec![0..midicer::data::RESTART, midicer::data::RESTART + 1..len]
        }
        // The rings; not the pattern buttons, LAUNCH or what is pending.
        ModuleKind::Orrery => vec![orrery::data::RINGS..orrery::data::LAUNCH],
        // Which rhythm or pattern plays and the mutes; not the panel's
        // size, the clock, nor the user patterns themselves.
        ModuleKind::Pops => vec![
            pops::data::SOURCE..pops::data::DETAIL,
            pops::data::MUTE..pops::data::USER,
        ],
        // Length, rate, gate, the lanes' mutes and steps; not the bank.
        ModuleKind::Fjord => vec![
            fjord::data::LENGTH..fjord::data::RATE + 1,
            fjord::data::GATE..fjord::data::GATE + 1,
            fjord::data::MUTE..fjord::data::BANK,
        ],
        ModuleKind::Vesper => vec![
            0..vesper::data::SELECTED,
            vesper::data::OSTINATO..len,
        ],
        // The picture; not the panel's size or frame.
        ModuleKind::Iris => vec![
            iris::data::SOURCE..iris::data::FRAMELESS,
            iris::data::LAYER..len,
        ],
        _ => Vec::new(),
    };
    ranges
        .into_iter()
        .map(|r| r.start.min(len)..r.end.min(len))
        .filter(|r| !r.is_empty())
        .collect()
}

/// How a pitch knob follows the key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyHow {
    /// A frequency: times 2^(shift/12).
    Ratio,
    /// A MIDI note: plus the shift.
    Note,
    /// A pitch class (0–11): plus the shift, round the octave.
    Class,
}

/// The knobs that follow the key, by index: sources tuned by a knob. Nodes
/// that send notes follow it on their notes instead (see `KeyShift`).
pub fn key_knobs(node: &Node) -> Vec<(usize, KeyHow)> {
    match &node.kind {
        NodeKind::Osc(_) => vec![(osc::FREQ, KeyHow::Ratio)],
        NodeKind::Module(m) => {
            let named = |name: &str, how| {
                m.params
                    .kind
                    .def()
                    .params
                    .iter()
                    .position(|s| s.name == name)
                    .map(|i| vec![(i, how)])
                    .unwrap_or_default()
            };
            match m.params.kind {
                ModuleKind::Drone | ModuleKind::Resonator => named("PITCH", KeyHow::Ratio),
                ModuleKind::Frakture => named("KEY", KeyHow::Ratio),
                ModuleKind::Aether => named("ROOT", KeyHow::Note),
                ModuleKind::Larsen => named("FREQ", KeyHow::Note),
                ModuleKind::Vesper => named("ROOT", KeyHow::Class),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// Whether the key can move `node`: it sends notes, or has a pitch knob.
pub fn keyable(node: &Node) -> bool {
    node.midi_out() || !key_knobs(node).is_empty()
}

/// Nodes that follow the key unless told otherwise: those that make melodies
/// (not drums, arpeggiators, loopers or the keyboard, whose notes are
/// already in the key they are played in).
pub fn follows_by_default(node: &Node) -> bool {
    match &node.kind {
        NodeKind::Osc(_) => true,
        NodeKind::Module(m) => matches!(
            m.params.kind,
            ModuleKind::Sequencer
                | ModuleKind::Midicer
                | ModuleKind::Automa
                | ModuleKind::Colloid
                | ModuleKind::Drone
                | ModuleKind::Resonator
                | ModuleKind::Frakture
                | ModuleKind::Aether
                | ModuleKind::Larsen
                | ModuleKind::Vesper
        ),
        _ => false,
    }
}

/// `v` moved by `shift` semitones.
pub fn shifted(v: f32, how: KeyHow, shift: i8, spec: &ParamSpec) -> f32 {
    if shift == 0 {
        return v;
    }
    let s = shift as f32;
    let out = match how {
        KeyHow::Ratio => v * (s / 12.0).exp2(),
        KeyHow::Note => v + s,
        KeyHow::Class => (v.round() + s).rem_euclid(12.0),
    };
    out.clamp(spec.min, spec.max)
}

fn knob_spec(node: &Node, i: usize) -> Option<&'static ParamSpec> {
    match &node.kind {
        NodeKind::Osc(_) => OSC_PARAMS.get(i),
        NodeKind::Module(m) => m.params.kind.def().params.get(i),
        _ => None,
    }
}

/// How a knob goes to a new value over a MORPH.
fn glide_of(spec: &ParamSpec) -> Glide {
    let stepped = matches!(
        spec.unit,
        Unit::Integer
            | Unit::Channel
            | Unit::Division
            | Unit::NoteName
            | Unit::Note
            | Unit::NoteValue
            | Unit::Sync
            | Unit::Semitones
            | Unit::Octaves
    );
    if stepped {
        Glide::Step
    } else if spec.taper == Taper::Log {
        Glide::Ratio
    } else {
        Glide::Linear
    }
}

/// The settings of the board the song drives, as a scene keeps them, with
/// `shift` taken out of the pitch knobs of the nodes that follow the key.
pub fn capture(graph: &Graph, song: &Song, shift: i8) -> BTreeMap<NodeId, State> {
    graph
        .nodes
        .iter()
        .filter(|n| song.drives(n))
        .filter_map(|n| {
            let back = if song.follows(n) { -shift } else { 0 };
            state_of(n, back).map(|s| (n.id, s))
        })
        .collect()
}

/// A node's settings now, its pitch knobs moved by `shift`.
fn state_of(node: &Node, shift: i8) -> Option<State> {
    let r = Ordering::Relaxed;
    let keyed = key_knobs(node);
    let tune = |i: usize, v: f32| match keyed.iter().find(|k| k.0 == i) {
        Some(&(_, how)) => knob_spec(node, i).map_or(v, |s| shifted(v, how, shift, s)),
        None => v,
    };
    Some(match &node.kind {
        NodeKind::Module(m) => {
            let p = &m.params;
            let len = p.data.len();
            State::Module {
                kind: p.kind,
                values: (0..p.len()).map(|i| tune(i, p.get(i))).collect(),
                mode: p.mode(),
                toggle: p.toggled(),
                data: data_ranges(p.kind, len)
                    .into_iter()
                    .map(|r| (r.start, p.data[r].iter().map(|d| d.get()).collect()))
                    .collect(),
                bypass: node.is_effect() && node.bypass.load(r),
            }
        }
        NodeKind::Osc(o) => State::Osc {
            values: (0..osc::COUNT).map(|i| tune(i, o.params.get(i))).collect(),
            wave: o.params.wave.load(r),
        },
        NodeKind::Mixer(m) => {
            let p = &m.params;
            State::Mixer {
                fader: p.fader.iter().map(|v| v.get()).collect(),
                pan: p.pan.iter().map(|v| v.get()).collect(),
                aux: p.aux.iter().map(|v| v.get()).collect(),
                mute: p.mute.iter().map(|v| v.load(r)).collect(),
                solo: p.solo.iter().map(|v| v.load(r)).collect(),
                master: p.master.get(),
                aux_master: p.aux_master.get(),
                master_mute: p.master_mute.load(r),
            }
        }
        NodeKind::Plugin(_) => return None,
    })
}

/// Whether the board differs from scene `scene` (played at `shift`): it has
/// been tweaked since it was recalled or kept.
pub fn edited(graph: &Graph, song: &Song, scene: &Scene, shift: i8) -> bool {
    let close = |a: f32, b: f32| (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0);
    let same = |a: &[f32], b: &[f32]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| close(*x, *y));
    scene.nodes.iter().any(|(id, kept)| {
        let Some(node) = graph.node(*id).filter(|n| song.drives(n)) else {
            return false;
        };
        let back = if song.follows(node) { -shift } else { 0 };
        let Some(now) = state_of(node, back) else {
            return false;
        };
        match (kept, &now) {
            (
                State::Module {
                    kind,
                    values,
                    mode,
                    toggle,
                    data,
                    bypass,
                },
                State::Module {
                    kind: k2,
                    values: v2,
                    mode: m2,
                    toggle: t2,
                    data: d2,
                    bypass: b2,
                },
            ) => {
                kind != k2
                    || !same(values, v2)
                    || mode != m2
                    || toggle != t2
                    || bypass != b2
                    || data.len() != d2.len()
                    || data.iter().zip(d2).any(|(a, b)| a.0 != b.0 || !same(&a.1, &b.1))
            }
            (State::Osc { values, wave }, State::Osc { values: v2, wave: w2 }) => {
                !same(values, v2) || wave != w2
            }
            (
                State::Mixer {
                    fader,
                    pan,
                    aux,
                    mute,
                    solo,
                    master,
                    aux_master,
                    master_mute,
                },
                State::Mixer {
                    fader: f2,
                    pan: p2,
                    aux: a2,
                    mute: mu2,
                    solo: s2,
                    master: ma2,
                    aux_master: am2,
                    master_mute: mm2,
                },
            ) => {
                !same(fader, f2)
                    || !same(pan, p2)
                    || !same(aux, a2)
                    || mute != mu2
                    || solo != s2
                    || !close(*master, *ma2)
                    || !close(*aux_master, *am2)
                    || master_mute != mm2
            }
            _ => true,
        }
    })
}

// ---------------------------------------------------------------------------
// Scenes into cues
// ---------------------------------------------------------------------------

/// Whether a node's notes are moved by the key: it follows the key and what
/// plays into its MIDI input doesn't (its notes are moved already).
pub fn shifts_notes(graph: &Graph, song: &Song, node: &Node) -> bool {
    let mut seen = vec![node.id];
    let mut at = node;
    if !(at.midi_out() && song.follows(at)) {
        return false;
    }
    loop {
        let upstream = graph
            .cables
            .iter()
            .find(|c| c.to.node == at.id && is_midi(c.to.port))
            .and_then(|c| graph.node(c.from.node));
        match upstream {
            Some(up) if !seen.contains(&up.id) => {
                if up.midi_out() && song.follows(up) {
                    return false;
                }
                seen.push(up.id);
                at = up;
            }
            _ => return true,
        }
    }
}

/// The transposition of every node that sends notes, for a step at `shift`.
pub fn note_shifts(graph: &Graph, song: &Song, shift: i8) -> Vec<(NodeId, i8)> {
    graph
        .nodes
        .iter()
        .filter(|n| n.midi_out())
        .map(|n| {
            let s = if shift != 0 && shifts_notes(graph, song, n) {
                shift
            } else {
                0
            };
            (n.id, s)
        })
        .collect()
}

/// A cue that sets the board to `scene` at `shift`: all its settings when
/// `full`, else only the pitch knobs that follow the key (the same scene in
/// another key). The notes' transposition comes with it.
pub fn scene_cue(
    graph: &Graph,
    song: &Song,
    scene: Option<&Scene>,
    shift: i8,
    full: bool,
    id: u32,
    at: f64,
) -> Cue {
    let mut cue = Cue::new(id, at);
    cue.shifts = note_shifts(graph, song, shift);
    let Some(scene) = scene else {
        return cue;
    };
    for (nid, state) in &scene.nodes {
        let Some(node) = graph.node(*nid).filter(|n| song.drives(n)) else {
            continue;
        };
        let follows = song.follows(node);
        let keyed = if follows { key_knobs(node) } else { Vec::new() };
        if !full && keyed.is_empty() {
            continue;
        }
        if let Some(nc) = node_cue(node, state, shift, &keyed, full) {
            cue.nodes.push(nc);
        }
    }
    cue
}

fn node_cue(
    node: &Node,
    state: &State,
    shift: i8,
    keyed: &[(usize, KeyHow)],
    full: bool,
) -> Option<NodeCue> {
    let mut sets = Vec::new();
    let mut knobs = |values: &[f32], specs: &'static [ParamSpec]| {
        for (i, (&v, spec)) in values.iter().zip(specs).enumerate() {
            let key = keyed.iter().find(|k| k.0 == i);
            if !full && key.is_none() {
                continue;
            }
            let v = match key {
                Some(&(_, how)) => shifted(v, how, shift, spec),
                None => v,
            }
            .clamp(spec.min, spec.max);
            sets.push(Set::new(Field::Knob(i as u16), v, glide_of(spec)));
        }
    };
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    let target = match (&node.kind, state) {
        (
            NodeKind::Module(m),
            State::Module {
                kind,
                values,
                mode,
                toggle,
                data,
                bypass,
            },
        ) if m.params.kind == *kind => {
            knobs(values, kind.def().params);
            if full {
                sets.push(Set::new(Field::Mode, *mode as f32, Glide::Step));
                if kind.def().toggle.is_some() {
                    sets.push(Set::new(Field::Toggle, flag(*toggle), Glide::Step));
                }
                if node.is_effect() {
                    sets.push(Set::new(Field::Bypass, flag(*bypass), Glide::Step));
                }
                let len = m.params.data.len();
                for (start, values) in data {
                    for (j, &v) in values.iter().enumerate() {
                        if start + j < len {
                            sets.push(Set::new(Field::Data((start + j) as u32), v, Glide::Step));
                        }
                    }
                }
            }
            Target::Module(m.params.clone())
        }
        (NodeKind::Osc(o), State::Osc { values, wave }) => {
            knobs(values, &OSC_PARAMS);
            if full {
                sets.push(Set::new(Field::Wave, *wave as f32, Glide::Step));
            }
            Target::Osc(o.params.clone())
        }
        (
            NodeKind::Mixer(m),
            State::Mixer {
                fader,
                pan,
                aux,
                mute,
                solo,
                master,
                aux_master,
                master_mute,
            },
        ) => {
            if !full {
                return None;
            }
            let n = m.params.channels();
            for c in 0..n.min(fader.len()) {
                let ch = c as u16;
                sets.push(Set::new(Field::Fader(ch), fader[c], Glide::Linear));
                if let Some(&v) = pan.get(c) {
                    sets.push(Set::new(Field::Pan(ch), v, Glide::Linear));
                }
                if let Some(&v) = aux.get(c) {
                    sets.push(Set::new(Field::Aux(ch), v, Glide::Linear));
                }
                if let Some(&v) = mute.get(c) {
                    sets.push(Set::new(Field::Mute(ch), flag(v), Glide::Step));
                }
                if let Some(&v) = solo.get(c) {
                    sets.push(Set::new(Field::Solo(ch), flag(v), Glide::Step));
                }
            }
            sets.push(Set::new(Field::Master, *master, Glide::Linear));
            sets.push(Set::new(Field::AuxMaster, *aux_master, Glide::Linear));
            sets.push(Set::new(Field::MasterMute, flag(*master_mute), Glide::Step));
            Target::Mixer(m.params.clone())
        }
        _ => return None,
    };
    if sets.is_empty() {
        return None;
    }
    Some(NodeCue {
        target,
        bypass: node.bypass.clone(),
        sets: sets.into_boxed_slice(),
    })
}

// ---------------------------------------------------------------------------
// Playing the song
// ---------------------------------------------------------------------------

/// What the transport is doing, as the engine says.
#[derive(Clone, Copy, Debug)]
pub struct Transport {
    pub beat: f64,
    pub running: bool,
    /// The id of the last cue the engine applied.
    pub applied: u32,
}

/// What to tell the engine.
pub enum Send {
    Cue(Box<Cue>),
    /// The cue waiting is no longer wanted.
    Cancel,
}

#[derive(Default)]
pub struct Update {
    pub sends: Vec<Send>,
    /// A cue with a tempo was applied: the board's tempo is now this.
    pub tempo: Option<f32>,
}

/// A cue sent and not yet applied.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pending {
    id: u32,
    at: f64,
    /// The step it starts (none: a scene recalled by hand, or a stop).
    step: Option<usize>,
    scene: Option<u32>,
    shift: i8,
    tempo: Option<f32>,
    stop: bool,
    /// Asked for (live), rather than what follows by itself.
    asked: bool,
    /// What it was made from: the song's edits and the scene on the board.
    rev: u32,
    loaded: Option<(u32, i8)>,
}

/// Plays a song: decides each change and sends it to the engine ahead of its
/// beat. Driven by [`Player::update`] every frame.
#[derive(Default)]
pub struct Player {
    /// The step playing, and the beat its current pass started on.
    pub current: Option<usize>,
    pub started: f64,
    /// The step asked for next (live), not yet sent.
    queued: Option<usize>,
    pending: Option<Pending>,
    /// The scene on the board (as last recalled or kept) and the
    /// transposition it plays at.
    pub loaded: Option<(u32, i8)>,
    next_id: u32,
    /// Bumped by every edit of the song (see [`Player::touch`]).
    rev: u32,
    was_running: bool,
    was_on: bool,
    last_beat: f64,
}

impl Player {
    /// The song changed: a change waiting for Start is made again.
    pub fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    fn id(&mut self) -> u32 {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.next_id
    }

    /// The transposition the board plays at now.
    pub fn shift(&self) -> i8 {
        self.loaded.map_or(0, |l| l.1)
    }

    /// The step coming next, sent or asked for: it blinks.
    pub fn coming(&self) -> Option<usize> {
        self.queued.or(self.pending.and_then(|p| p.step))
    }

    /// The scene coming next, recalled by hand.
    pub fn coming_scene(&self) -> Option<u32> {
        self.pending.filter(|p| p.step.is_none()).and_then(|p| p.scene)
    }

    /// Asks for step `i` next, while playing: it comes in as QUANT says.
    pub fn queue(&mut self, i: usize) {
        self.queued = Some(i);
    }

    /// Forgets everything (a project opened).
    pub fn reset(&mut self) -> Send {
        *self = Self {
            next_id: self.next_id,
            ..Self::default()
        };
        Send::Cancel
    }

    /// Recalls scene `id` by hand: at once when stopped, on the next bar
    /// while playing.
    pub fn recall(&mut self, graph: &Graph, song: &Song, id: u32, t: Transport) -> Send {
        let at = if t.running {
            (t.beat / BEATS_PER_BAR).floor() * BEATS_PER_BAR + BEATS_PER_BAR
        } else {
            cue::NOW
        };
        let shift = self.shift();
        let cue_id = self.id();
        let cue = scene_cue(graph, song, song.scene(id), shift, true, cue_id, at);
        self.pending = Some(Pending {
            id: cue_id,
            at,
            step: None,
            scene: Some(id),
            shift,
            tempo: None,
            stop: false,
            asked: true,
            rev: self.rev,
            loaded: self.loaded,
        });
        Send::Cue(Box::new(cue))
    }

    /// The board was kept into scene `id` (at the transposition it plays at).
    pub fn stored(&mut self, id: u32) {
        self.loaded = Some((id, self.shift()));
    }

    /// The cue that starts step `i` on beat `at` (`asked`: picked live).
    fn step_cue(&mut self, graph: &Graph, song: &Song, i: usize, at: f64, asked: bool) -> Send {
        let step = song.steps[i];
        let shift = song.shift(&step);
        let scene = song.scene(step.scene);
        let full = self.loaded.map(|l| l.0) != Some(step.scene);
        let cue_id = self.id();
        let mut cue = scene_cue(graph, song, scene, shift, full, cue_id, at);
        cue.morph = step.morph as f64 * BEATS_PER_BAR;
        cue.tempo = step
            .tempo
            .map(|t| t.clamp(TEMPO_RANGE.0, TEMPO_RANGE.1));
        self.pending = Some(Pending {
            id: cue_id,
            at,
            step: Some(i),
            scene: scene.map(|s| s.id),
            shift,
            tempo: cue.tempo,
            stop: false,
            asked,
            rev: self.rev,
            loaded: self.loaded,
        });
        Send::Cue(Box::new(cue))
    }

    /// Where a step asked for now comes in: on the next bar, two or four
    /// bars of the step playing, or at its end.
    fn boundary(&self, song: &Song, beat: f64) -> f64 {
        let (start, len) = match self.current.and_then(|i| song.steps.get(i)) {
            Some(step) => (self.started, step.beats()),
            None => (0.0, BEATS_PER_BAR),
        };
        let grid = match song.quant {
            Quant::Bar => BEATS_PER_BAR,
            Quant::Two => 2.0 * BEATS_PER_BAR,
            Quant::Four => 4.0 * BEATS_PER_BAR,
            Quant::End => len,
        };
        let k = ((beat - start) / grid).floor().max(-1.0) + 1.0;
        start + k * grid
    }

    /// Follows the transport: notes what the engine applied, and sends the
    /// next change when one is due.
    pub fn update(&mut self, graph: &Graph, song: &Song, t: Transport) -> Update {
        let mut out = Update::default();
        if let Some(p) = self.pending.filter(|p| p.id == t.applied) {
            self.pending = None;
            if let Some(scene) = p.scene {
                self.loaded = Some((scene, p.shift));
            } else if let Some(l) = self.loaded.as_mut() {
                l.1 = p.shift;
            }
            if let Some(i) = p.step {
                self.current = Some(i);
                self.started = if p.at == cue::NOW { t.beat } else { p.at };
            }
            if p.stop {
                self.current = None;
            }
            out.tempo = p.tempo;
        }

        // Stopped, or sent somewhere else by a clock: start over.
        let stopped = self.was_running && !t.running;
        let jumped = t.running && self.was_running && t.beat + 0.5 < self.last_beat;
        self.was_running = t.running;
        self.last_beat = t.beat;
        if stopped || jumped {
            self.current = None;
            self.queued = None;
            if self.pending.take().is_some() {
                out.sends.push(Send::Cancel);
            }
        }

        if !song.on || song.steps.is_empty() {
            if self.was_on {
                // Back to playing by hand: the notes and pitch knobs go
                // back to the home key.
                self.was_on = false;
                self.current = None;
                self.queued = None;
                let scene = self.loaded.and_then(|l| song.scene(l.0));
                let id = self.id();
                let cue = scene_cue(graph, song, scene, 0, false, id, cue::NOW);
                self.pending = Some(Pending {
                    id,
                    at: cue::NOW,
                    step: None,
                    scene: None,
                    shift: 0,
                    tempo: None,
                    stop: false,
                    asked: true,
                    rev: self.rev,
                    loaded: self.loaded,
                });
                out.sends.push(Send::Cue(Box::new(cue)));
            }
            return out;
        }
        self.was_on = true;
        let len = song.steps.len();

        if !t.running {
            // The start step waits for Start, made again when the song or
            // the scene on the board change.
            self.queued = None;
            let start = song.start.min(len - 1);
            let fresh = self.pending.is_some_and(|p| {
                p.step == Some(start) && p.at == 0.0 && p.rev == self.rev && p.loaded == self.loaded
            });
            let waiting_by_hand = self.pending.is_some_and(|p| p.step.is_none());
            if !fresh && !waiting_by_hand {
                out.sends.push(self.step_cue(graph, song, start, 0.0, false));
            }
            return out;
        }

        if let Some(i) = self.queued.take() {
            let i = i.min(len - 1);
            let at = self.boundary(song, t.beat);
            out.sends.push(self.step_cue(graph, song, i, at, true));
            return out;
        }
        // The song was edited while a change waits: made again (what follows
        // by itself is worked out again, a step picked keeps its beat).
        if let Some(p) = self.pending.filter(|p| p.rev != self.rev && p.at > t.beat + 0.05) {
            match p.step {
                Some(i) if p.asked => {
                    out.sends.push(self.step_cue(graph, song, i.min(len - 1), p.at, true));
                    return out;
                }
                Some(_) => self.pending = None,
                None if p.stop => self.pending = None,
                None => {}
            }
        }
        if self.pending.is_some() {
            return out;
        }
        match self.current {
            None => {
                // Switched on while playing: from the start step, on the
                // next bar.
                let at = (t.beat / BEATS_PER_BAR).floor() * BEATS_PER_BAR + BEATS_PER_BAR;
                out.sends.push(self.step_cue(graph, song, song.start.min(len - 1), at, false));
            }
            Some(i) => {
                let i = i.min(len - 1);
                let step = song.steps[i];
                let end = self.started + step.beats();
                let then = if song.hold { Then::Loop } else { step.then };
                match then {
                    Then::Loop => {
                        while t.beat >= self.started + step.beats() {
                            self.started += step.beats();
                        }
                    }
                    Then::Next => out.sends.push(self.step_cue(graph, song, (i + 1) % len, end, false)),
                    Then::Stop => {
                        let id = self.id();
                        let mut cue = Cue::new(id, end);
                        cue.stop = true;
                        self.pending = Some(Pending {
                            id,
                            at: end,
                            step: None,
                            scene: None,
                            shift: self.shift(),
                            tempo: None,
                            stop: true,
                            asked: false,
                            rev: self.rev,
                            loaded: self.loaded,
                        });
                        out.sends.push(Send::Cue(Box::new(cue)));
                    }
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// MIDI control
// ---------------------------------------------------------------------------

/// The lowest note that picks a step (C3: step 1, C#3 step 2…).
pub const FIRST_STEP_NOTE: u8 = 48;
/// B2 asks for the next step, A#2 the one before.
pub const NEXT_NOTE: u8 = 47;
pub const PREVIOUS_NOTE: u8 = 46;

/// What a MIDI message on the song's channel asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MidiAsk {
    Step(usize),
    Next,
    Previous,
}

pub fn midi_ask(song: &Song, msg: [u8; 3]) -> Option<MidiAsk> {
    if song.channel == 0 || msg[0] & 0x0F != song.channel - 1 {
        return None;
    }
    match msg[0] & 0xF0 {
        0x90 if msg[2] > 0 => match msg[1] {
            NEXT_NOTE => Some(MidiAsk::Next),
            PREVIOUS_NOTE => Some(MidiAsk::Previous),
            n if n >= FIRST_STEP_NOTE => Some(MidiAsk::Step((n - FIRST_STEP_NOTE) as usize)),
            _ => None,
        },
        0xC0 => Some(MidiAsk::Step(msg[1] as usize)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// In the project file
// ---------------------------------------------------------------------------

fn is_false(b: &bool) -> bool {
    !b
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}

fn is_next(t: &Then) -> bool {
    *t == Then::Next
}

/// The song in a project: nodes by their number in the file, knobs by name.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct SongData {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scenes: Vec<SceneData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    steps: Vec<StepData>,
    /// The home key's name ("C", "F#").
    #[serde(default = "home_c")]
    home: String,
    #[serde(default, skip_serializing_if = "is_false")]
    on: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    hold: bool,
    #[serde(default)]
    quant: Quant,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    start: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    free: Vec<u32>,
    /// Nodes following the key against their default: `[node, follows]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    keys: Vec<(u32, bool)>,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    channel: u8,
}

fn home_c() -> String {
    "C".to_owned()
}

impl SongData {
    pub fn is_default(&self) -> bool {
        self.scenes.is_empty() && self.steps.is_empty()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct SceneData {
    id: u32,
    name: String,
    #[serde(default)]
    color: u8,
    nodes: Vec<NodeStateData>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct NodeStateData {
    node: u32,
    #[serde(flatten)]
    state: StateData,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "type")]
enum StateData {
    Module {
        module: String,
        params: BTreeMap<String, f32>,
        #[serde(default)]
        mode: String,
        #[serde(default)]
        toggle: bool,
        /// Each kept range of data: where it starts, and its values.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        data: Vec<(u32, Vec<f32>)>,
        #[serde(default, skip_serializing_if = "is_false")]
        bypass: bool,
    },
    Oscillator {
        waveform: String,
        params: BTreeMap<String, f32>,
    },
    Mixer {
        channels: Vec<ChannelState>,
        master: f32,
        #[serde(default = "unity")]
        aux: f32,
        #[serde(default, skip_serializing_if = "is_false")]
        master_mute: bool,
    },
}

fn unity() -> f32 {
    crate::audio::params::FADER_UNITY
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct ChannelState {
    fader: f32,
    pan: f32,
    #[serde(default)]
    aux: f32,
    #[serde(default, skip_serializing_if = "is_false")]
    mute: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    solo: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct StepData {
    scene: u32,
    bars: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    morph: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tempo: Option<f32>,
    #[serde(default, skip_serializing_if = "is_next")]
    then: Then,
}

fn pitch_class(name: &str) -> Option<u8> {
    NOTE_NAMES.iter().position(|n| *n == name).map(|i| i as u8)
}

fn state_data(state: &State) -> StateData {
    match state {
        State::Module {
            kind,
            values,
            mode,
            toggle,
            data,
            bypass,
        } => {
            let def = kind.def();
            StateData::Module {
                module: kind.key().to_owned(),
                params: def
                    .params
                    .iter()
                    .zip(values)
                    .map(|(s, v)| (s.name.to_owned(), *v))
                    .collect(),
                mode: def.modes.get(*mode).map(|m| (*m).to_owned()).unwrap_or_default(),
                toggle: *toggle,
                data: data.iter().map(|(s, v)| (*s as u32, v.clone())).collect(),
                bypass: *bypass,
            }
        }
        State::Osc { values, wave } => StateData::Oscillator {
            waveform: Waveform::from_u32(*wave).name().to_owned(),
            params: OSC_PARAMS
                .iter()
                .zip(values)
                .map(|(s, v)| (s.name.to_owned(), *v))
                .collect(),
        },
        State::Mixer {
            fader,
            pan,
            aux,
            mute,
            solo,
            master,
            aux_master,
            master_mute,
        } => StateData::Mixer {
            channels: (0..fader.len())
                .map(|c| ChannelState {
                    fader: fader[c],
                    pan: pan.get(c).copied().unwrap_or(0.0),
                    aux: aux.get(c).copied().unwrap_or(0.0),
                    mute: mute.get(c).copied().unwrap_or(false),
                    solo: solo.get(c).copied().unwrap_or(false),
                })
                .collect(),
            master: *master,
            aux: *aux_master,
            master_mute: *master_mute,
        },
    }
}

fn state_from(data: &StateData) -> Option<State> {
    let by_name = |specs: &[ParamSpec], params: &BTreeMap<String, f32>| {
        specs
            .iter()
            .map(|s| params.get(s.name).copied().unwrap_or(s.default).clamp(s.min, s.max))
            .collect::<Vec<f32>>()
    };
    Some(match data {
        StateData::Module {
            module,
            params,
            mode,
            toggle,
            data,
            bypass,
        } => {
            let kind = ModuleKind::from_key(module)?;
            let def = kind.def();
            State::Module {
                kind,
                values: by_name(def.params, params),
                mode: def.modes.iter().position(|m| m == mode).unwrap_or(0),
                toggle: *toggle,
                data: data.iter().map(|(s, v)| (*s as usize, v.clone())).collect(),
                bypass: *bypass,
            }
        }
        StateData::Oscillator { waveform, params } => State::Osc {
            values: by_name(&OSC_PARAMS, params),
            wave: Waveform::ALL
                .iter()
                .position(|w| w.name() == waveform)
                .unwrap_or(0) as u32,
        },
        StateData::Mixer {
            channels,
            master,
            aux,
            master_mute,
        } => State::Mixer {
            fader: channels.iter().map(|c| c.fader.clamp(0.0, 1.0)).collect(),
            pan: channels.iter().map(|c| c.pan.clamp(-1.0, 1.0)).collect(),
            aux: channels.iter().map(|c| c.aux.clamp(0.0, 1.0)).collect(),
            mute: channels.iter().map(|c| c.mute).collect(),
            solo: channels.iter().map(|c| c.solo).collect(),
            master: master.clamp(0.0, 1.0),
            aux_master: aux.clamp(0.0, 1.0),
            master_mute: *master_mute,
        },
    })
}

/// The song for saving, nodes numbered as in the file (`numbers`; nodes
/// that are gone are left out).
pub fn to_data(song: &Song, numbers: &HashMap<NodeId, u32>) -> SongData {
    // No song yet: nothing in the file (and the same as reading none back).
    if song.scenes.is_empty() && song.steps.is_empty() {
        return SongData::default();
    }
    let num = |id: &NodeId| numbers.get(id).copied();
    SongData {
        scenes: song
            .scenes
            .iter()
            .map(|s| SceneData {
                id: s.id,
                name: s.name.clone(),
                color: s.color,
                nodes: s
                    .nodes
                    .iter()
                    .filter_map(|(id, state)| {
                        Some(NodeStateData {
                            node: num(id)?,
                            state: state_data(state),
                        })
                    })
                    .collect(),
            })
            .collect(),
        steps: song
            .steps
            .iter()
            .map(|s| StepData {
                scene: s.scene,
                bars: s.bars,
                key: s.key.map(|k| key_name(k).to_owned()),
                morph: s.morph,
                tempo: s.tempo,
                then: s.then,
            })
            .collect(),
        home: key_name(song.home).to_owned(),
        on: song.on,
        hold: song.hold,
        quant: song.quant,
        start: song.start,
        free: song.free.iter().filter_map(num).collect(),
        keys: song
            .keys
            .iter()
            .filter_map(|(id, on)| Some((num(id)?, *on)))
            .collect(),
        channel: song.channel,
    }
}

/// The song from a project, nodes by their ids in the patch (`ids`).
pub fn from_data(data: &SongData, ids: &HashMap<u32, NodeId>) -> Song {
    let id = |n: &u32| ids.get(n).copied();
    let scenes: Vec<Scene> = data
        .scenes
        .iter()
        .map(|s| Scene {
            id: s.id,
            name: s.name.clone(),
            color: s.color % COLORS,
            nodes: s
                .nodes
                .iter()
                .filter_map(|n| Some((id(&n.node)?, state_from(&n.state)?)))
                .collect(),
        })
        .collect();
    let steps: Vec<Step> = data
        .steps
        .iter()
        .filter(|s| scenes.iter().any(|sc| sc.id == s.scene))
        .take(MAX_STEPS)
        .map(|s| Step {
            scene: s.scene,
            bars: s.bars.clamp(1, MAX_BARS),
            key: s.key.as_deref().and_then(pitch_class),
            morph: s.morph.min(*MORPHS.last().unwrap_or(&8)),
            tempo: s.tempo.map(|t| t.clamp(TEMPO_RANGE.0, TEMPO_RANGE.1)),
            then: s.then,
        })
        .collect();
    Song {
        start: data.start.min(steps.len().saturating_sub(1)),
        scenes,
        steps,
        home: pitch_class(&data.home).unwrap_or(0),
        on: data.on,
        hold: data.hold,
        quant: data.quant,
        free: data.free.iter().filter_map(id).collect(),
        keys: data
            .keys
            .iter()
            .filter_map(|(n, on)| Some((id(n)?, *on)))
            .collect(),
        channel: data.channel.min(16),
    }
}
