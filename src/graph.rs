//! The patch as the UI sees it: nodes with positions, and cables.
//! Every structural change is mirrored to the audio engine.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::{Color32, Pos2, Vec2};

use crate::audio::AudioHandle;
use crate::audio::dsp::{Mixer, Oscillator};
use crate::audio::engine::{Command, MAX_INPUTS, NodeId, Slot, TopoEntry, Topology};
use crate::audio::params::{MIXER_CHANNELS, MixerParams, OscParams, osc};
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

pub struct OscNode {
    pub params: Arc<OscParams>,
    pub meter: MeterState,
}

pub struct MixerNode {
    pub params: Arc<MixerParams>,
    pub channels: [MeterState; MIXER_CHANNELS],
    pub master: [MeterState; 2],
}

pub enum NodeKind {
    Osc(OscNode),
    Mixer(MixerNode),
}

pub struct Node {
    pub id: NodeId,
    pub pos: Pos2,
    pub title: String,
    pub accent: Color32,
    pub kind: NodeKind,
}

impl Node {
    pub fn size(&self) -> Vec2 {
        nodes::size(&self.kind)
    }

    pub fn inputs(&self) -> &'static [&'static str] {
        match self.kind {
            NodeKind::Osc(_) => &["FM", "AM"],
            NodeKind::Mixer(_) => &["1", "2", "3", "4"],
        }
    }

    pub fn outputs(&self) -> &'static [&'static str] {
        match self.kind {
            NodeKind::Osc(_) => &["OUT"],
            NodeKind::Mixer(_) => &[],
        }
    }

    pub fn input_pos(&self, i: usize) -> Pos2 {
        self.pos + nodes::input_offset(&self.kind, i)
    }

    pub fn output_pos(&self, i: usize) -> Pos2 {
        self.pos + nodes::output_offset(&self.kind, i)
    }

    /// Smoothed output level, used to animate cables.
    pub fn signal_level(&self) -> f32 {
        match &self.kind {
            NodeKind::Osc(o) => o.meter.level,
            NodeKind::Mixer(m) => m.master[0].level.max(m.master[1].level),
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
        }
    }
}

#[derive(Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub cables: Vec<Cable>,
    next_id: NodeId,
    osc_count: usize,
}

impl Graph {
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    fn alloc_id(&mut self) -> NodeId {
        self.next_id += 1;
        self.next_id
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

    pub fn rename(&mut self, id: NodeId, title: &str) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
            n.title = title.to_owned();
        }
    }

    pub fn add_osc(&mut self, pos: Pos2, audio: &AudioHandle) -> NodeId {
        self.add_osc_with(pos, Arc::new(OscParams::new()), audio)
    }

    fn add_osc_with(&mut self, pos: Pos2, params: Arc<OscParams>, audio: &AudioHandle) -> NodeId {
        let id = self.alloc_id();
        let accent = theme::OSC_ACCENTS[self.osc_count % theme::OSC_ACCENTS.len()];
        self.osc_count += 1;
        let title = self.next_title("OSC");
        audio.send(Command::AddNode(id, Slot::new(Box::new(Oscillator::new(params.clone())), 1)));
        self.nodes.push(Node {
            id,
            pos,
            title,
            accent,
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

    fn add_mixer_with(&mut self, pos: Pos2, params: Arc<MixerParams>, audio: &AudioHandle) -> NodeId {
        let id = self.alloc_id();
        let title = self.next_title("MIXER");
        audio.send(Command::AddNode(id, Slot::new(Box::new(Mixer::new(params.clone())), 2)));
        self.nodes.push(Node {
            id,
            pos,
            title,
            accent: theme::MIXER_ACCENT,
            kind: NodeKind::Mixer(MixerNode {
                params,
                channels: Default::default(),
                master: Default::default(),
            }),
        });
        self.sync(audio);
        id
    }

    /// Clones a node with its current settings (not its cables).
    pub fn duplicate(&mut self, id: NodeId, audio: &AudioHandle) -> Option<NodeId> {
        let node = self.node(id)?;
        let pos = node.pos + Vec2::new(32.0, 32.0);
        match &node.kind {
            NodeKind::Osc(o) => {
                let p = OscParams::new();
                p.set_waveform(o.params.waveform());
                for i in 0..osc::COUNT {
                    p.set(i, o.params.get(i));
                }
                Some(self.add_osc_with(pos, Arc::new(p), audio))
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
                Some(self.add_mixer_with(pos, Arc::new(p), audio))
            }
        }
    }

    pub fn remove_node(&mut self, id: NodeId, audio: &AudioHandle) {
        self.nodes.retain(|n| n.id != id);
        self.cables.retain(|c| c.from.node != id && c.to.node != id);
        self.sync(audio);
        audio.send(Command::RemoveNode(id));
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
