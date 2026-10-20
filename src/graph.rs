//! The patch as the UI sees it: nodes with positions, and cables.
//! Every structural change is mirrored to the audio engine.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::{Color32, Pos2, Vec2};

use crate::audio::AudioHandle;
use crate::audio::dsp::{Mixer, Oscillator};
use crate::audio::engine::{Command, MAX_INPUTS, NodeId, Slot, TopoEntry, Topology};
use crate::audio::midi::{MidiState, note_hz};
use crate::audio::modules::{ModuleKind, ModuleParams, midi_cc, midi_note, random};
use crate::audio::params::{MIXER_CHANNELS, MixerParams, OSC_PARAMS, OscParams, ParamSpec, Unit, osc};
use crate::ui::nodes;
use crate::ui::theme;
use crate::ui::widgets::MeterState;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PortRef {
    pub node: NodeId,
    pub port: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cable {
    /// Output port of the source node.
    pub from: PortRef,
    /// Input port of the destination node.
    pub to: PortRef,
}

/// Everything that can be added to a patch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeType {
    Osc,
    Mixer,
    Module(ModuleKind),
}

impl NodeType {
    /// The "add node" menu, in groups.
    pub const MENU: [(&'static str, &'static [NodeType]); 5] = [
        (
            "Sources",
            &[NodeType::Osc, NodeType::Module(ModuleKind::Drone), NodeType::Module(ModuleKind::Random)],
        ),
        (
            "Texture",
            &[NodeType::Module(ModuleKind::Granular), NodeType::Module(ModuleKind::Resonator)],
        ),
        (
            "Effects",
            &[
                NodeType::Module(ModuleKind::Filter),
                NodeType::Module(ModuleKind::Delay),
                NodeType::Module(ModuleKind::Reverb),
            ],
        ),
        (
            "MIDI",
            &[NodeType::Module(ModuleKind::MidiCc), NodeType::Module(ModuleKind::MidiNote)],
        ),
        ("Output", &[NodeType::Mixer]),
    ];

    pub fn name(self) -> &'static str {
        match self {
            NodeType::Osc => "Oscillator",
            NodeType::Mixer => "Mixer",
            NodeType::Module(k) => k.def().name,
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            NodeType::Osc => "sine · saw · pulse",
            NodeType::Mixer => "4 channels",
            NodeType::Module(k) => k.def().hint,
        }
    }

    pub fn size(self) -> Vec2 {
        nodes::type_size(self)
    }
}

pub struct OscNode {
    pub params: Arc<OscParams>,
    pub meter: MeterState,
}

pub struct MixerNode {
    pub params: Arc<MixerParams>,
    pub channels: [MeterState; MIXER_CHANNELS],
    pub master: [MeterState; 2],
}

/// A knob or fader that a MIDI node can drive.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ParamRef {
    Osc(usize),
    Module(usize),
    Fader(usize),
    Pan(usize),
    Master,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    pub node: NodeId,
    pub param: ParamRef,
}

pub struct ModuleNode {
    pub params: Arc<ModuleParams>,
    pub meter: MeterState,
    /// Recent output values, for modulation sources' trace display.
    pub trace: VecDeque<f32>,
    /// MIDI nodes: the control they drive directly, if mapped.
    pub target: Option<Target>,
    /// MIDI nodes: the last MIDI message seen (only newer ones move the target).
    midi_seen: Option<u32>,
    /// MIDI CC while learning: the controller count when LEARN was armed.
    learn_from: Option<u32>,
}

impl ModuleNode {
    fn new(params: Arc<ModuleParams>) -> Self {
        Self {
            params,
            meter: MeterState::default(),
            trace: VecDeque::with_capacity(TRACE_LEN),
            target: None,
            midi_seen: None,
            learn_from: None,
        }
    }
}

pub const TRACE_LEN: usize = 180;

pub enum NodeKind {
    Osc(OscNode),
    Mixer(MixerNode),
    Module(ModuleNode),
}

impl NodeKind {
    pub fn node_type(&self) -> NodeType {
        match self {
            NodeKind::Osc(_) => NodeType::Osc,
            NodeKind::Mixer(_) => NodeType::Mixer,
            NodeKind::Module(m) => NodeType::Module(m.params.kind),
        }
    }
}

pub struct Node {
    pub id: NodeId,
    pub pos: Pos2,
    pub title: String,
    pub accent: Color32,
    /// Shown as a compact strip: title, ports and the main value only.
    pub collapsed: bool,
    pub kind: NodeKind,
}

impl Node {
    pub fn size(&self) -> Vec2 {
        if self.collapsed {
            nodes::collapsed_size(self.inputs().len().max(self.outputs().len()))
        } else {
            nodes::type_size(self.kind.node_type())
        }
    }

    pub fn inputs(&self) -> &'static [&'static str] {
        match &self.kind {
            NodeKind::Osc(_) => &["FM", "AM"],
            NodeKind::Mixer(_) => &["1", "2", "3", "4"],
            NodeKind::Module(m) => m.params.kind.def().inputs,
        }
    }

    pub fn outputs(&self) -> &'static [&'static str] {
        match &self.kind {
            NodeKind::Osc(_) => &["OUT"],
            NodeKind::Mixer(_) => &[],
            NodeKind::Module(m) => m.params.kind.def().outputs,
        }
    }

    pub fn input_pos(&self, i: usize) -> Pos2 {
        self.pos
            + if self.collapsed {
                nodes::collapsed_port(i, false)
            } else {
                nodes::input_offset(self.kind.node_type(), i)
            }
    }

    pub fn output_pos(&self, i: usize) -> Pos2 {
        self.pos
            + if self.collapsed {
                nodes::collapsed_port(i, true)
            } else {
                nodes::output_offset(self.kind.node_type(), i)
            }
    }

    /// Name of one of this node's controls ("CUTOFF", "FADER 2"…).
    pub fn param_name(&self, p: ParamRef) -> Option<String> {
        match (&self.kind, p) {
            (NodeKind::Osc(_), ParamRef::Osc(i)) => OSC_PARAMS.get(i).map(|s| s.name.to_owned()),
            (NodeKind::Module(m), ParamRef::Module(i)) => m.params.kind.def().params.get(i).map(|s| s.name.to_owned()),
            (NodeKind::Mixer(_), ParamRef::Fader(ch)) if ch < MIXER_CHANNELS => Some(format!("FADER {}", ch + 1)),
            (NodeKind::Mixer(_), ParamRef::Pan(ch)) if ch < MIXER_CHANNELS => Some(format!("PAN {}", ch + 1)),
            (NodeKind::Mixer(_), ParamRef::Master) => Some("MASTER".to_owned()),
            _ => None,
        }
    }

    /// The control called `name` (see [`Self::param_name`]).
    pub fn param_by_name(&self, name: &str) -> Option<ParamRef> {
        let specs: &[ParamSpec] = match &self.kind {
            NodeKind::Osc(_) => &OSC_PARAMS,
            NodeKind::Module(m) => m.params.kind.def().params,
            NodeKind::Mixer(_) => &[],
        };
        let wrap = |i| match self.kind {
            NodeKind::Osc(_) => ParamRef::Osc(i),
            _ => ParamRef::Module(i),
        };
        if let Some(i) = specs.iter().position(|s| s.name == name) {
            return Some(wrap(i));
        }
        let channel = |prefix: &str| {
            let ch: usize = name.strip_prefix(prefix)?.trim().parse().ok()?;
            (1..=MIXER_CHANNELS).contains(&ch).then_some(ch - 1)
        };
        match &self.kind {
            NodeKind::Mixer(_) if name == "MASTER" => Some(ParamRef::Master),
            NodeKind::Mixer(_) => channel("FADER")
                .map(ParamRef::Fader)
                .or_else(|| channel("PAN").map(ParamRef::Pan)),
            _ => None,
        }
    }

    /// Smoothed output level, used to animate cables.
    pub fn signal_level(&self) -> f32 {
        match &self.kind {
            NodeKind::Osc(o) => o.meter.level,
            NodeKind::Mixer(m) => m.master[0].level.max(m.master[1].level),
            NodeKind::Module(m) => m.meter.level,
        }
    }

    /// Pulls peaks from the audio thread into the display meters.
    pub fn update_meters(&mut self, dt: f32) {
        match &mut self.kind {
            NodeKind::Osc(o) => o.meter.update(o.params.meter.take(), dt),
            NodeKind::Mixer(m) => {
                for (state, src) in m.channels.iter_mut().zip(&m.params.ch_meter) {
                    state.update(src.take(), dt);
                }
                for (state, src) in m.master.iter_mut().zip(&m.params.master_meter) {
                    state.update(src.take(), dt);
                }
            }
            NodeKind::Module(m) => {
                m.meter.update(m.params.meter.take(), dt);
                let traced = match m.params.kind {
                    ModuleKind::Random => Some(m.params.viz[random::VALUE].get()),
                    ModuleKind::MidiCc => Some(m.params.viz[midi_cc::VALUE].get()),
                    ModuleKind::MidiNote => Some(m.params.viz[midi_note::ENV].get() * 2.0 - 1.0),
                    _ => None,
                };
                if let Some(v) = traced {
                    if m.trace.len() == TRACE_LEN {
                        m.trace.pop_front();
                    }
                    m.trace.push_back(v);
                }
            }
        }
    }
}

pub type GroupId = u32;

/// Nodes gathered into a module, to be moved, collapsed and duplicated
/// together. Purely visual: the audio graph does not change.
pub struct Group {
    pub id: GroupId,
    pub title: String,
    pub members: Vec<NodeId>,
    /// Shown as a single card exposing only the module's outer ports.
    pub collapsed: bool,
}

/// A MIDI value on its way to a control.
#[derive(Clone, Copy)]
enum Write {
    /// Controller position, 0..1.
    Norm(f32),
    /// MIDI note number.
    Note(f32),
}

#[derive(Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub cables: Vec<Cable>,
    pub groups: Vec<Group>,
    next_id: NodeId,
    next_group: GroupId,
    /// Nodes given a colour so far; each new source/effect takes the next.
    accent_count: usize,
}

impl Graph {
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Removes every node, cable and module.
    pub fn clear(&mut self, audio: &AudioHandle) {
        let ids: Vec<NodeId> = self.nodes.iter().map(|n| n.id).collect();
        for id in ids {
            self.remove_node(id, audio);
        }
        self.groups.clear();
        self.accent_count = 0;
    }

    fn alloc_id(&mut self) -> NodeId {
        self.next_id += 1;
        self.next_id
    }

    fn next_accent(&mut self) -> Color32 {
        let c = theme::OSC_ACCENTS[self.accent_count % theme::OSC_ACCENTS.len()];
        self.accent_count += 1;
        c
    }

    /// "OSC 3" etc.: one past the highest number currently in use.
    fn next_title(&self, prefix: &str) -> String {
        let n = self
            .nodes
            .iter()
            .filter_map(|n| n.title.strip_prefix(prefix)?.trim().parse::<usize>().ok())
            .max()
            .unwrap_or(0);
        format!("{prefix} {}", n + 1)
    }

    #[cfg(test)]
    pub fn rename(&mut self, id: NodeId, title: &str) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
            n.title = title.to_owned();
        }
    }

    pub fn add(&mut self, ty: NodeType, pos: Pos2, audio: &AudioHandle) -> NodeId {
        match ty {
            NodeType::Osc => self.add_osc(pos, audio),
            NodeType::Mixer => self.add_mixer(pos, audio),
            NodeType::Module(kind) => self.add_module(kind, pos, audio),
        }
    }

    pub fn add_osc(&mut self, pos: Pos2, audio: &AudioHandle) -> NodeId {
        self.add_osc_with(pos, Arc::new(OscParams::new()), audio)
    }

    pub fn add_osc_with(&mut self, pos: Pos2, params: Arc<OscParams>, audio: &AudioHandle) -> NodeId {
        let id = self.alloc_id();
        let accent = self.next_accent();
        let title = self.next_title("OSC");
        audio.send(Command::AddNode(id, Slot::new(Box::new(Oscillator::new(params.clone())), 1)));
        self.nodes.push(Node {
            id,
            pos,
            title,
            accent,
            collapsed: false,
            kind: NodeKind::Osc(OscNode {
                params,
                meter: MeterState::default(),
            }),
        });
        self.sync(audio);
        id
    }

    pub fn add_mixer(&mut self, pos: Pos2, audio: &AudioHandle) -> NodeId {
        self.add_mixer_with(pos, Arc::new(MixerParams::new()), audio)
    }

    pub fn add_mixer_with(&mut self, pos: Pos2, params: Arc<MixerParams>, audio: &AudioHandle) -> NodeId {
        let id = self.alloc_id();
        let title = self.next_title("MIXER");
        audio.send(Command::AddNode(id, Slot::new(Box::new(Mixer::new(params.clone())), 1)));
        self.nodes.push(Node {
            id,
            pos,
            title,
            accent: theme::MIXER_ACCENT,
            collapsed: false,
            kind: NodeKind::Mixer(MixerNode {
                params,
                channels: Default::default(),
                master: Default::default(),
            }),
        });
        self.sync(audio);
        id
    }

    pub fn add_module(&mut self, kind: ModuleKind, pos: Pos2, audio: &AudioHandle) -> NodeId {
        self.add_module_with(pos, Arc::new(ModuleParams::new(kind)), audio)
    }

    pub fn add_module_with(&mut self, pos: Pos2, params: Arc<ModuleParams>, audio: &AudioHandle) -> NodeId {
        let id = self.alloc_id();
        let kind = params.kind;
        let accent = self.next_accent();
        let title = self.next_title(kind.def().prefix);
        let proc = kind.processor(params.clone(), audio);
        audio.send(Command::AddNode(id, Slot::new(proc, kind.def().outputs.len())));
        self.nodes.push(Node {
            id,
            pos,
            title,
            accent,
            collapsed: false,
            kind: NodeKind::Module(ModuleNode::new(params)),
        });
        self.sync(audio);
        id
    }

    /// Clones a node with its current settings (not its cables).
    pub fn duplicate(&mut self, id: NodeId, audio: &AudioHandle) -> Option<NodeId> {
        let node = self.node(id)?;
        let pos = node.pos + Vec2::new(32.0, 32.0);
        let collapsed = node.collapsed;
        let copy = match &node.kind {
            NodeKind::Osc(o) => {
                let p = OscParams::new();
                p.set_waveform(o.params.waveform());
                for i in 0..osc::COUNT {
                    p.set(i, o.params.get(i));
                }
                self.add_osc_with(pos, Arc::new(p), audio)
            }
            NodeKind::Mixer(m) => {
                let p = MixerParams::new();
                for ch in 0..MIXER_CHANNELS {
                    p.fader[ch].set(m.params.fader[ch].get());
                    p.pan[ch].set(m.params.pan[ch].get());
                    p.mute[ch].store(m.params.mute[ch].load(Ordering::Relaxed), Ordering::Relaxed);
                    p.solo[ch].store(m.params.solo[ch].load(Ordering::Relaxed), Ordering::Relaxed);
                }
                p.master.set(m.params.master.get());
                self.add_mixer_with(pos, Arc::new(p), audio)
            }
            NodeKind::Module(m) => {
                let p = Arc::new(m.params.duplicate());
                let target = m.target;
                let copy = self.add_module_with(pos, p, audio);
                self.set_target(copy, target);
                copy
            }
        };
        self.set_collapsed(copy, collapsed);
        Some(copy)
    }

    pub fn set_collapsed(&mut self, id: NodeId, collapsed: bool) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
            n.collapsed = collapsed;
        }
    }

    pub fn toggle_collapsed(&mut self, id: NodeId) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
            n.collapsed = !n.collapsed;
        }
    }

    /// Collapses or expands every node and module.
    pub fn set_all_collapsed(&mut self, collapsed: bool) {
        for n in &mut self.nodes {
            n.collapsed = collapsed;
        }
        for g in &mut self.groups {
            g.collapsed = collapsed;
        }
    }

    pub fn remove_node(&mut self, id: NodeId, audio: &AudioHandle) {
        self.nodes.retain(|n| n.id != id);
        self.cables.retain(|c| c.from.node != id && c.to.node != id);
        for g in &mut self.groups {
            g.members.retain(|m| *m != id);
        }
        self.groups.retain(|g| !g.members.is_empty());
        for n in &mut self.nodes {
            if let NodeKind::Module(m) = &mut n.kind
                && m.target.is_some_and(|t| t.node == id)
            {
                m.target = None;
            }
        }
        self.sync(audio);
        audio.send(Command::RemoveNode(id));
    }

    pub fn move_nodes(&mut self, ids: &HashSet<NodeId>, delta: Vec2) {
        for n in self.nodes.iter_mut().filter(|n| ids.contains(&n.id)) {
            n.pos += delta;
        }
    }

    /// Copies nodes with their settings and the cables between them. Modules
    /// whose members are all copied are copied too. Returns the new nodes.
    pub fn duplicate_nodes(&mut self, ids: &[NodeId], audio: &AudioHandle) -> Vec<NodeId> {
        let mut copies = HashMap::new();
        for &id in ids {
            if let Some(new) = self.duplicate(id, audio) {
                copies.insert(id, new);
            }
        }
        let inner: Vec<Cable> = self
            .cables
            .iter()
            .filter(|c| copies.contains_key(&c.from.node) && copies.contains_key(&c.to.node))
            .copied()
            .collect();
        for c in inner {
            let from = PortRef { node: copies[&c.from.node], port: c.from.port };
            let to = PortRef { node: copies[&c.to.node], port: c.to.port };
            self.connect(from, to, audio);
        }
        // MIDI mappings inside the copied set follow the copies.
        for &new in copies.values() {
            if let Some(t) = self.target_of(new)
                && let Some(&moved) = copies.get(&t.node)
            {
                self.set_target(new, Some(Target { node: moved, param: t.param }));
            }
        }
        let groups: Vec<(Vec<NodeId>, bool)> = self
            .groups
            .iter()
            .filter(|g| g.members.iter().all(|m| copies.contains_key(m)))
            .map(|g| (g.members.iter().map(|m| copies[m]).collect(), g.collapsed))
            .collect();
        for (members, collapsed) in groups {
            if let Some(gid) = self.create_group(&members) {
                self.set_group_collapsed(gid, collapsed);
            }
        }
        ids.iter().filter_map(|id| copies.get(id).copied()).collect()
    }

    // --- MIDI mapping ---------------------------------------------------------

    /// "FILTER 1 · CUTOFF".
    pub fn target_label(&self, t: Target) -> Option<String> {
        let n = self.node(t.node)?;
        Some(format!("{} · {}", n.title, n.param_name(t.param)?))
    }

    pub fn target_of(&self, id: NodeId) -> Option<Target> {
        match &self.node(id)?.kind {
            NodeKind::Module(m) => m.target,
            _ => None,
        }
    }

    /// Points a MIDI node at a control (or unmaps it with `None`).
    pub fn set_target(&mut self, midi_node: NodeId, target: Option<Target>) {
        if let Some(NodeKind::Module(m)) = self.node_mut(midi_node).map(|n| &mut n.kind) {
            m.target = target;
            m.midi_seen = None;
        }
    }

    /// Finishes MIDI learn and moves mapped controls. Only messages that
    /// arrive after a mapping is made move its control.
    pub fn update_midi(&mut self, midi: &MidiState) {
        let mut writes = Vec::new();
        for node in &mut self.nodes {
            let NodeKind::Module(m) = &mut node.kind else {
                continue;
            };
            let p = &m.params;
            let (seq, write) = match p.kind {
                ModuleKind::MidiCc => {
                    let (count, channel, number) = midi.last_cc();
                    if !p.toggled() {
                        m.learn_from = None;
                    } else if m.learn_from.is_none() {
                        m.learn_from = Some(count);
                    } else if m.learn_from != Some(count) {
                        p.set(midi_cc::CC, number as f32);
                        p.set(midi_cc::CHANNEL, channel as f32);
                        p.set_toggle(false);
                        m.learn_from = None;
                        m.midi_seen = None;
                    }
                    let (channel, number) = midi_cc::address(p);
                    let (value, seq) = midi.cc(channel, number);
                    (seq, Write::Norm(value))
                }
                ModuleKind::MidiNote => {
                    let key = midi.note(midi_note::channel(p));
                    (key.seq, Write::Note(midi_note::played_note(p, key.note)))
                }
                _ => continue,
            };
            if let Some(target) = m.target
                && m.midi_seen.is_some_and(|seen| seen != seq)
            {
                writes.push((target, write));
            }
            m.midi_seen = Some(seq);
        }
        for (target, write) in writes {
            self.write_param(target, write);
        }
    }

    fn write_param(&self, t: Target, write: Write) {
        let Some(node) = self.node(t.node) else {
            return;
        };
        let norm = match write {
            Write::Norm(n) => n,
            Write::Note(note) => note / 127.0,
        };
        // A note sets pitch knobs to its frequency and semitone knobs to its
        // distance from middle C; anything else follows the key's position.
        let value = |spec: &ParamSpec| match (write, spec.unit) {
            (Write::Note(note), Unit::Hz) => note_hz(note).clamp(spec.min, spec.max),
            (Write::Note(note), Unit::Semitones) if spec.bipolar => (note - 60.0).clamp(spec.min, spec.max),
            _ => spec.denorm(norm),
        };
        match (&node.kind, t.param) {
            (NodeKind::Osc(o), ParamRef::Osc(i)) if i < osc::COUNT => o.params.set(i, value(&OSC_PARAMS[i])),
            (NodeKind::Module(m), ParamRef::Module(i)) => {
                if let Some(spec) = m.params.kind.def().params.get(i) {
                    m.params.set(i, value(spec));
                }
            }
            (NodeKind::Mixer(m), ParamRef::Fader(ch)) if ch < MIXER_CHANNELS => m.params.fader[ch].set(norm),
            (NodeKind::Mixer(m), ParamRef::Pan(ch)) if ch < MIXER_CHANNELS => m.params.pan[ch].set(norm * 2.0 - 1.0),
            (NodeKind::Mixer(m), ParamRef::Master) => m.params.master.set(norm),
            _ => {}
        }
    }

    // --- Modules (groups of nodes) ------------------------------------------

    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    pub fn group_of(&self, node: NodeId) -> Option<&Group> {
        self.groups.iter().find(|g| g.members.contains(&node))
    }

    /// Gathers nodes into a new module; they leave any module they were in.
    pub fn create_group(&mut self, members: &[NodeId]) -> Option<GroupId> {
        let mut list: Vec<NodeId> = Vec::new();
        for &id in members {
            if self.node(id).is_some() && !list.contains(&id) {
                list.push(id);
            }
        }
        if list.is_empty() {
            return None;
        }
        for g in &mut self.groups {
            g.members.retain(|m| !list.contains(m));
        }
        self.groups.retain(|g| !g.members.is_empty());

        let n = self
            .groups
            .iter()
            .filter_map(|g| g.title.strip_prefix("MODULE")?.trim().parse::<usize>().ok())
            .max()
            .unwrap_or(0);
        self.next_group += 1;
        let id = self.next_group;
        self.groups.push(Group {
            id,
            title: format!("MODULE {}", n + 1),
            members: list,
            collapsed: false,
        });
        Some(id)
    }

    /// Dissolves a module, keeping its nodes.
    pub fn ungroup(&mut self, id: GroupId) {
        self.groups.retain(|g| g.id != id);
    }

    pub fn rename_group(&mut self, id: GroupId, title: &str) {
        if let Some(g) = self.group_mut(id) {
            g.title = title.to_owned();
        }
    }

    pub fn set_group_collapsed(&mut self, id: GroupId, collapsed: bool) {
        if let Some(g) = self.group_mut(id) {
            g.collapsed = collapsed;
        }
    }

    /// The ports linking a module to the rest of the patch: member inputs not
    /// fed by another member, and member outputs that are unused or feed
    /// something outside. A collapsed module shows exactly these.
    pub fn exposed_ports(&self, g: &Group) -> (Vec<PortRef>, Vec<PortRef>) {
        let inside = |n: NodeId| g.members.contains(&n);
        let (mut ins, mut outs) = (Vec::new(), Vec::new());
        for node in g.members.iter().filter_map(|m| self.node(*m)) {
            for port in 0..node.inputs().len() {
                let pr = PortRef { node: node.id, port };
                if !self.cables.iter().any(|c| c.to == pr && inside(c.from.node)) {
                    ins.push(pr);
                }
            }
            for port in 0..node.outputs().len() {
                let pr = PortRef { node: node.id, port };
                let mut uses = self.cables.iter().filter(|c| c.from == pr).peekable();
                let used = uses.peek().is_some();
                if !used || uses.any(|c| !inside(c.to.node)) {
                    outs.push(pr);
                }
            }
        }
        (ins, outs)
    }

    /// Connects an output to an input, replacing whatever fed that input.
    /// Refuses connections that would create a feedback loop.
    pub fn connect(&mut self, from: PortRef, to: PortRef, audio: &AudioHandle) -> bool {
        if from.node == to.node || self.reaches(to.node, from.node) {
            return false;
        }
        self.cables.retain(|c| c.to != to);
        self.cables.push(Cable { from, to });
        self.sync(audio);
        true
    }

    pub fn disconnect(&mut self, cable: Cable, audio: &AudioHandle) {
        self.cables.retain(|c| *c != cable);
        self.sync(audio);
    }

    pub fn would_cycle(&self, from: NodeId, to: NodeId) -> bool {
        from == to || self.reaches(to, from)
    }

    /// Is there a path of cables from `start` to `target`?
    fn reaches(&self, start: NodeId, target: NodeId) -> bool {
        let mut stack = vec![start];
        let mut seen = HashSet::new();
        while let Some(n) = stack.pop() {
            if n == target {
                return true;
            }
            if seen.insert(n) {
                stack.extend(self.cables.iter().filter(|c| c.from.node == n).map(|c| c.to.node));
            }
        }
        false
    }

    pub fn bring_to_front(&mut self, id: NodeId) {
        if let Some(i) = self.nodes.iter().position(|n| n.id == id) {
            let n = self.nodes.remove(i);
            self.nodes.push(n);
        }
    }

    /// Sends a topologically sorted processing order to the engine (Kahn).
    fn sync(&self, audio: &AudioHandle) {
        let mut indegree: HashMap<NodeId, usize> = self.nodes.iter().map(|n| (n.id, 0)).collect();
        for c in &self.cables {
            *indegree.entry(c.to.node).or_default() += 1;
        }
        let mut ready: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|n| indegree[&n.id] == 0)
            .map(|n| n.id)
            .rev()
            .collect();

        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(id) = ready.pop() {
            let mut inputs = [None; MAX_INPUTS];
            for c in self.cables.iter().filter(|c| c.to.node == id) {
                if c.to.port < MAX_INPUTS {
                    inputs[c.to.port] = Some((c.from.node, c.from.port));
                }
            }
            order.push(TopoEntry { id, inputs });
            for c in self.cables.iter().filter(|c| c.from.node == id) {
                let d = indegree.get_mut(&c.to.node).expect("cable to unknown node");
                *d -= 1;
                if *d == 0 {
                    ready.push(c.to.node);
                }
            }
        }
        audio.send(Command::SetTopology(Box::new(Topology { order })));
    }
}
