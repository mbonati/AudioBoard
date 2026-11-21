//! The track view's reading of the patch (README, "Track view"): the same
//! [`Graph`] the canvas edits, seen as one row per track with its chain of
//! devices left to right, Ableton style; and the edits made there, done as
//! cable changes. Nothing here is a second model: a track is computed from
//! the cables every time it is needed.
//!
//! The rules:
//!
//! - A *chain cable* is an audio cable into a node's main input (port 0
//!   named IN, on a node with an audio output: effects, plugin effects,
//!   Iris) or into a Mixer/Console channel. Every other audio cable (CUT,
//!   SC, PITCH, TRIG…) is a *side* cable: modulation, sidechain. MIDI
//!   cables are their own kind.
//! - An input holds one cable, so a node has at most one chain
//!   predecessor and walking back from anywhere follows a single path.
//! - A track is that path, walked back from where a chain ends: a patched
//!   mixer channel (one track per channel), or a node whose audio goes on
//!   to no chain input (an unrouted source, the last of a row of effects).
//! - The walk stops at a node with nothing in its main input (the track's
//!   own source, or an effect fed by nothing), at a mixer's OUT or AUX (a
//!   bus or return track), or at a node an earlier track already shows: a
//!   split. The shared devices stay in the earlier track and the later
//!   one says where it branches off ("from FILTER 1").
//! - Nodes that make no chain audio (MIDI-only nodes, modulators whose
//!   outputs only reach side inputs, MIDI CC/Note nodes mapped to a
//!   control) are *attached*: drawn in the lane just before the device
//!   they feed or drive.
//! - Each Mixer/Console has a master row; whatever is left (a MIDI node
//!   patched to nothing) is shown in a last row, "Not in a track".
//! - Every cable is shown ([`Shown`]): as the link between neighbouring
//!   devices, as a track's input or destination, from an attached device
//!   into its device, or else as a tag on the devices at both ends.
//!
//! Track order, folded rows and open device panels are the track view's
//! own layout ([`Layout`]), saved with the project.

use std::collections::HashSet;

use egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::audio::AudioHandle;
use crate::audio::engine::NodeId;
use crate::graph::{Cable, Graph, MIDI_PORT, Node, NodeKind, NodeType, PortRef, is_midi};

/// How the patch is shown: the free canvas or the track rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    #[default]
    Nodes,
    Tracks,
}

/// Names a track across edits: the mixer channel it ends in, else the
/// node it ends at. Master rows go by their mixer.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TrackKey {
    Channel(PortRef),
    Node(NodeId),
    /// The "Not in a track" row.
    Loose,
}

impl TrackKey {
    fn mentions(self, id: NodeId) -> bool {
        match self {
            TrackKey::Channel(p) => p.node == id,
            TrackKey::Node(n) => n == id,
            TrackKey::Loose => false,
        }
    }
}

/// The track view's own layout, kept in the [`Graph`] and saved with the
/// project. The canvas ignores it.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Layout {
    pub view: View,
    /// Tracks in the order the user put them (tracks not listed come
    /// after, in their natural order: by mixer channel, then unrouted).
    pub order: Vec<TrackKey>,
    /// Tracks shown as a thin row.
    pub folded: Vec<TrackKey>,
    /// Devices whose panel is open in their track.
    pub open: Vec<NodeId>,
}

impl Layout {
    pub fn is_open(&self, id: NodeId) -> bool {
        self.open.contains(&id)
    }

    pub fn set_open(&mut self, id: NodeId, open: bool) {
        self.open.retain(|n| *n != id);
        if open {
            self.open.push(id);
        }
    }

    pub fn is_folded(&self, key: TrackKey) -> bool {
        self.folded.contains(&key)
    }

    pub fn set_folded(&mut self, key: TrackKey, folded: bool) {
        self.folded.retain(|k| *k != key);
        if folded {
            self.folded.push(key);
        }
    }

    /// A track now goes by another name (it was routed, or its last
    /// device changed): keeps its place and its folding.
    pub fn rekey(&mut self, old: TrackKey, new: TrackKey) {
        if old == new {
            return;
        }
        for list in [&mut self.order, &mut self.folded] {
            if list.contains(&new) {
                list.retain(|k| *k != old);
            } else {
                for k in list.iter_mut().filter(|k| **k == old) {
                    *k = new;
                }
            }
        }
    }

    /// Drops what refers to a removed node.
    pub fn forget(&mut self, id: NodeId) {
        self.open.retain(|n| *n != id);
        self.order.retain(|k| !k.mentions(id));
        self.folded.retain(|k| !k.mentions(id));
    }
}

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

/// Takes a chain on its main input: port 0 is IN and it has an audio output
/// (effects, plugin effects, Iris).
pub fn inline(n: &Node) -> bool {
    n.inputs().first() == Some(&"IN") && !n.outputs().is_empty()
}

/// A Mixer or Console: where tracks end.
pub fn is_sink(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Mixer(_))
}

/// Nodes of this type go into a chain (see [`inline`]). Plugins are only
/// known once loaded.
pub fn inline_type(ty: NodeType) -> bool {
    match ty {
        NodeType::Module(k) => {
            let def = k.def();
            def.inputs.first() == Some(&"IN") && !def.outputs.is_empty()
        }
        _ => false,
    }
}

/// Nodes of this type make MIDI and no audio (sequencers, arpeggiators…):
/// they go in front of an instrument.
pub fn midi_type(ty: NodeType) -> bool {
    match ty {
        NodeType::Module(k) => {
            let def = k.def();
            def.midi_out && def.outputs.is_empty()
        }
        _ => false,
    }
}

fn is_chain(g: &Graph, c: &Cable) -> bool {
    !is_midi(c.to.port)
        && g
            .node(c.to.node)
            .is_some_and(|n| is_sink(n) || (c.to.port == 0 && inline(n)))
}

/// The cable into a node's main input, if it is a chain cable.
fn chain_pred(g: &Graph, id: NodeId) -> Option<Cable> {
    let n = g.node(id)?;
    if is_sink(n) || !inline(n) {
        return None;
    }
    g.cables
        .iter()
        .find(|c| c.to == PortRef { node: id, port: 0 })
        .copied()
}

fn feeds_chain(g: &Graph, id: NodeId) -> bool {
    g.cables.iter().any(|c| c.from.node == id && is_chain(g, c))
}

/// Makes no chain audio of its own and so sits next to the device it
/// serves: MIDI-only nodes, modulators reaching side inputs only, and MIDI
/// nodes mapped to a control with their outputs unused.
fn attachable(g: &Graph, n: &Node) -> bool {
    if is_sink(n) {
        return false;
    }
    if n.outputs().is_empty() {
        return true;
    }
    if chain_pred(g, n.id).is_some() || feeds_chain(g, n.id) {
        return false;
    }
    let audio_out = g
        .cables
        .iter()
        .any(|c| c.from.node == n.id && !is_midi(c.from.port));
    audio_out || g.target_of(n.id).is_some()
}

/// What an attachable node serves, best first: what its cables feed, then
/// the control it is mapped to.
fn served(g: &Graph, id: NodeId) -> Vec<NodeId> {
    g.cables
        .iter()
        .filter(|c| c.from.node == id)
        .map(|c| c.to.node)
        .chain(g.target_of(id).map(|t| t.node))
        .collect()
}

// ---------------------------------------------------------------------------
// The map
// ---------------------------------------------------------------------------

/// Where a row's chain comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    /// Its first device makes the sound (an instrument, a generator).
    Source,
    /// Its first device is an effect fed by nothing.
    Empty,
    /// It branches off a device another track shows: that output.
    Branch(PortRef),
    /// A mixer's OUT (a bus) or AUX (a return).
    Bus(PortRef),
    /// The row of a Mixer or Console itself.
    Master,
    /// The "Not in a track" row.
    Loose,
}

/// One device in a row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    pub node: NodeId,
    /// Chain devices: the output going on to what follows.
    pub port: usize,
    /// Attached devices: the device they serve (just after them).
    pub serves: Option<NodeId>,
}

impl Slot {
    fn out(&self) -> PortRef {
        PortRef {
            node: self.node,
            port: self.port,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Track {
    pub key: TrackKey,
    pub input: Input,
    /// Devices left to right: attached ones just before what they serve.
    pub slots: Vec<Slot>,
    /// The mixer channel it ends in.
    pub dest: Option<PortRef>,
}

impl Track {
    /// A real track (not a master row or the loose row).
    pub fn is_track(&self) -> bool {
        !matches!(self.input, Input::Master | Input::Loose)
    }

    /// The chain: devices the signal goes through, in order.
    pub fn chain(&self) -> Vec<Slot> {
        self.slots
            .iter()
            .filter(|s| s.serves.is_none())
            .copied()
            .collect()
    }

    pub fn chain_nodes(&self) -> Vec<NodeId> {
        self.chain().iter().map(|s| s.node).collect()
    }

    /// The first device of the chain.
    pub fn head(&self) -> Option<NodeId> {
        self.slots.iter().find(|s| s.serves.is_none()).map(|s| s.node)
    }

    /// The first gap an effect can go in: gap `g` is before chain device
    /// `g` (the chain's length: at the end). Nothing goes before a source.
    pub fn first_gap(&self) -> usize {
        usize::from(self.input == Input::Source)
    }

    /// Gaps an effect can go in, if any.
    pub fn gaps(&self) -> Option<std::ops::RangeInclusive<usize>> {
        self.is_track()
            .then(|| self.first_gap()..=self.chain().len())
            .filter(|r| !r.is_empty())
    }

    /// What feeds gap `gap`, and what it feeds.
    fn gap_ends(&self, gap: usize) -> (Option<PortRef>, Option<PortRef>) {
        let chain = self.chain();
        let up = if gap == 0 {
            match self.input {
                Input::Branch(p) | Input::Bus(p) => Some(p),
                _ => None,
            }
        } else {
            chain.get(gap - 1).map(Slot::out)
        };
        let down = match chain.get(gap) {
            Some(s) => Some(PortRef {
                node: s.node,
                port: 0,
            }),
            None => self.dest,
        };
        (up, down)
    }

    /// The output that ends the chain: its last device's, else its input.
    pub fn tail(&self) -> Option<PortRef> {
        match self.chain().last() {
            Some(s) => Some(s.out()),
            None => match self.input {
                Input::Branch(p) | Input::Bus(p) => Some(p),
                _ => None,
            },
        }
    }
}

/// How a cable shows in the track view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shown {
    /// Between neighbouring devices of a chain.
    Link,
    /// A track's input (a split or a mixer output) into its first device,
    /// or straight into its channel.
    Input,
    /// A track's last device into its channel.
    Dest,
    /// From an attached device into the device it serves.
    Attach,
    /// Anywhere else: a tag on the devices at both ends.
    Tag,
}

/// A device's note about a cable the lane doesn't draw.
#[derive(Clone, PartialEq, Debug)]
pub struct Tag {
    pub text: String,
    /// Coming in (else going out).
    pub incoming: bool,
    pub midi: bool,
    /// The node at the other end.
    pub other: NodeId,
}

/// The rows of the track view: tracks (in the user's order), then a
/// master row per mixer, then the loose row if anything is left.
pub struct TrackMap {
    pub rows: Vec<Track>,
}

impl TrackMap {
    pub fn build(g: &Graph) -> Self {
        // Where chains end: patched channels, then unrouted chain nodes.
        let mut ends: Vec<(TrackKey, PortRef)> = Vec::new();
        for sink in g.nodes.iter().filter(|n| is_sink(n)) {
            let mut chans: Vec<&Cable> = g
                .cables
                .iter()
                .filter(|c| c.to.node == sink.id && !is_midi(c.to.port))
                .collect();
            chans.sort_by_key(|c| c.to.port);
            ends.extend(chans.iter().map(|c| (TrackKey::Channel(c.to), c.from)));
        }
        for n in &g.nodes {
            if !is_sink(n) && !n.outputs().is_empty() && !feeds_chain(g, n.id) && !attachable(g, n) {
                ends.push((TrackKey::Node(n.id), PortRef { node: n.id, port: 0 }));
            }
        }
        let order = &g.tracks.order;
        ends.sort_by_key(|(k, _)| order.iter().position(|o| o == k).unwrap_or(usize::MAX));

        let mut placed: HashSet<NodeId> = HashSet::new();
        let mut rows = Vec::new();
        for (key, start) in ends {
            let mut slots = Vec::new();
            let mut cur = start;
            let input = loop {
                let Some(n) = g.node(cur.node) else {
                    break Input::Empty;
                };
                if is_sink(n) {
                    break Input::Bus(cur);
                }
                if placed.contains(&cur.node) {
                    break Input::Branch(cur);
                }
                placed.insert(cur.node);
                slots.push(Slot {
                    node: cur.node,
                    port: cur.port,
                    serves: None,
                });
                match chain_pred(g, cur.node) {
                    Some(c) => cur = c.from,
                    None if inline(n) => break Input::Empty,
                    None => break Input::Source,
                }
            };
            slots.reverse();
            let dest = match key {
                TrackKey::Channel(p) => Some(p),
                _ => None,
            };
            rows.push(Track {
                key,
                input,
                slots,
                dest,
            });
        }
        for sink in g.nodes.iter().filter(|n| is_sink(n)) {
            placed.insert(sink.id);
            rows.push(Track {
                key: TrackKey::Node(sink.id),
                input: Input::Master,
                slots: vec![Slot {
                    node: sink.id,
                    port: 0,
                    serves: None,
                }],
                dest: None,
            });
        }

        // Attached devices, until none is left to place (one can serve
        // another: a sequencer into an arpeggiator into a synth).
        loop {
            let mut changed = false;
            for n in &g.nodes {
                if placed.contains(&n.id) || !attachable(g, n) {
                    continue;
                }
                let Some(target) = served(g, n.id).into_iter().find(|t| placed.contains(t)) else {
                    continue;
                };
                for row in &mut rows {
                    if let Some(i) = row.slots.iter().position(|s| s.node == target) {
                        row.slots.insert(
                            i,
                            Slot {
                                node: n.id,
                                port: 0,
                                serves: Some(target),
                            },
                        );
                        break;
                    }
                }
                placed.insert(n.id);
                changed = true;
            }
            if !changed {
                break;
            }
        }

        let loose: Vec<Slot> = g
            .nodes
            .iter()
            .filter(|n| !placed.contains(&n.id))
            .map(|n| Slot {
                node: n.id,
                port: 0,
                serves: None,
            })
            .collect();
        if !loose.is_empty() {
            rows.push(Track {
                key: TrackKey::Loose,
                input: Input::Loose,
                slots: loose,
                dest: None,
            });
        }
        Self { rows }
    }

    /// The real tracks, in order.
    pub fn tracks(&self) -> impl Iterator<Item = &Track> {
        self.rows.iter().filter(|t| t.is_track())
    }

    pub fn find(&self, key: TrackKey) -> Option<&Track> {
        self.rows.iter().find(|t| t.key == key)
    }

    /// The row holding a node, and where in it.
    pub fn locate(&self, id: NodeId) -> Option<(usize, usize)> {
        self.rows
            .iter()
            .enumerate()
            .find_map(|(r, t)| Some((r, t.slots.iter().position(|s| s.node == id)?)))
    }

    /// How a cable shows (see [`Shown`]).
    pub fn shown(&self, c: &Cable) -> Shown {
        for t in self.rows.iter().filter(|t| t.is_track()) {
            let chain = t.chain();
            let input = match t.input {
                Input::Branch(p) | Input::Bus(p) => Some(p),
                _ => None,
            };
            if t.dest == Some(c.to) {
                if chain.last().map(Slot::out) == Some(c.from) {
                    return Shown::Dest;
                }
                if chain.is_empty() && input == Some(c.from) {
                    return Shown::Input;
                }
            }
            for (i, s) in chain.iter().enumerate() {
                if c.to != (PortRef { node: s.node, port: 0 }) {
                    continue;
                }
                if i == 0 && input == Some(c.from) {
                    return Shown::Input;
                }
                if i > 0 && chain[i - 1].out() == c.from {
                    return Shown::Link;
                }
            }
        }
        let attached = self
            .rows
            .iter()
            .flat_map(|t| &t.slots)
            .any(|s| s.node == c.from.node && s.serves == Some(c.to.node));
        if attached {
            Shown::Attach
        } else {
            Shown::Tag
        }
    }

    /// Notes on a device about its cables that the lane doesn't draw:
    /// side inputs (always, saying which), MIDI from elsewhere than the
    /// device just before, splits and anything else going out, and the
    /// control a MIDI node is mapped to.
    pub fn tags(&self, g: &Graph, id: NodeId) -> Vec<Tag> {
        let Some(node) = g.node(id) else {
            return Vec::new();
        };
        let title = |n: NodeId| g.node(n).map_or("?", |n| n.title.as_str()).to_owned();
        let mut tags = Vec::new();
        for c in g.cables.iter().filter(|c| c.to.node == id) {
            let shown = self.shown(c);
            let midi = is_midi(c.to.port);
            let side = !midi && !is_sink(node) && !(c.to.port == 0 && inline(node));
            let text = if side {
                format!("{} ◂ {}", node.port_label(c.to.port, false), title(c.from.node))
            } else if shown == Shown::Tag && midi {
                format!("MIDI ◂ {}", title(c.from.node))
            } else if shown == Shown::Tag {
                format!("◂ {}", title(c.from.node))
            } else {
                continue;
            };
            tags.push(Tag {
                text,
                incoming: true,
                midi,
                other: c.from.node,
            });
        }
        for c in g.cables.iter().filter(|c| c.from.node == id) {
            let shown = self.shown(c);
            // A split, or a mixer output feeding a bus or return track.
            let feeds_track = shown == Shown::Input;
            if !(shown == Shown::Tag || shown == Shown::Attach || feeds_track) {
                continue;
            }
            let Some(to) = g.node(c.to.node) else {
                continue;
            };
            let midi = is_midi(c.to.port);
            let out = if c.from.port != 0 && !midi {
                format!("{} ", node.port_label(c.from.port, true))
            } else {
                String::new()
            };
            let what = if midi {
                format!("MIDI ▸ {}", to.title)
            } else if is_sink(to) || (c.to.port == 0 && inline(to)) {
                format!("{out}▸ {}", to.title)
            } else {
                format!("{out}▸ {} · {}", to.title, to.port_label(c.to.port, false))
            };
            tags.push(Tag {
                text: what,
                incoming: false,
                midi,
                other: c.to.node,
            });
        }
        if let Some(t) = g.target_of(id)
            && let Some(label) = g.target_label(t)
        {
            tags.push(Tag {
                text: format!("MAP ▸ {label}"),
                incoming: false,
                midi: true,
                other: t.node,
            });
        }
        tags
    }
}

// ---------------------------------------------------------------------------
// Edits: each one a few cable changes on the graph
// ---------------------------------------------------------------------------

/// Horizontal room between nodes placed on the canvas from the track view.
const ROOM: f32 = 48.0;

fn track_of(g: &Graph, key: TrackKey) -> Option<Track> {
    TrackMap::build(g).rows.into_iter().find(|t| t.key == key)
}

fn rect_of(n: &Node) -> Rect {
    Rect::from_min_size(n.pos, n.size())
}

/// Below everything on the canvas, at its left edge.
pub fn below_content(g: &Graph) -> Pos2 {
    g.nodes
        .iter()
        .map(rect_of)
        .reduce(|a, b| a.union(b))
        .map_or(pos2(0.0, 0.0), |r| pos2(r.left(), r.bottom() + 80.0))
}

/// A canvas spot for a node going between `up` and `down`: right of what
/// feeds it, else left of what it feeds.
fn spot_between(g: &Graph, up: Option<NodeId>, down: Option<NodeId>, size: Vec2) -> Pos2 {
    if let Some(u) = up.and_then(|u| g.node(u)).filter(|n| !is_sink(n)) {
        return u.pos + vec2(u.size().x + ROOM, 0.0);
    }
    if let Some(d) = down.and_then(|d| g.node(d)) {
        return d.pos - vec2(size.x + ROOM, 0.0);
    }
    below_content(g)
}

/// Moves what lies downstream of a node just inserted on the canvas to
/// the right, when the newcomer overlaps something.
fn make_room(g: &mut Graph, id: NodeId) {
    let Some(r) = g.node(id).map(rect_of) else {
        return;
    };
    if !g.nodes.iter().any(|n| n.id != id && rect_of(n).intersects(r)) {
        return;
    }
    let mut stack = vec![id];
    let mut seen = HashSet::new();
    while let Some(n) = stack.pop() {
        if seen.insert(n) {
            stack.extend(g.cables.iter().filter(|c| c.from.node == n).map(|c| c.to.node));
        }
    }
    let dx = r.width() + ROOM;
    for n in g.nodes.iter_mut() {
        if n.id != id && seen.contains(&n.id) && n.pos.x >= r.left() - 1.0 {
            n.pos.x += dx;
        }
    }
}

/// Puts an existing node (an effect) into gap `gap` of track `key`: what
/// fed the gap now feeds it, and it feeds what came after. Other cables of
/// the neighbours stay. False (and nothing changed) if it can't go there.
pub fn insert(g: &mut Graph, key: TrackKey, gap: usize, id: NodeId, audio: &AudioHandle) -> bool {
    let Some(t) = track_of(g, key) else {
        return false;
    };
    let fits = t.gaps().is_some_and(|r| r.contains(&gap));
    if !fits || !g.node(id).is_some_and(inline) || t.slots.iter().any(|s| s.node == id) {
        return false;
    }
    let (up, down) = t.gap_ends(gap);
    let saved = g.cables.clone();
    let this = PortRef { node: id, port: 0 };
    let ok = up.is_none_or(|u| g.connect(u, this, audio)) && down.is_none_or(|d| g.connect(this, d, audio));
    if !ok {
        g.set_cables(saved, audio);
        return false;
    }
    // An unrouted track now ends at the newcomer.
    if down.is_none() && matches!(key, TrackKey::Node(_)) {
        g.tracks.rekey(key, TrackKey::Node(id));
    }
    true
}

/// Adds a new node of type `ty` into gap `gap` of track `key`, placed on
/// the canvas between its neighbours. None if that type can't go there.
pub fn insert_new(g: &mut Graph, key: TrackKey, gap: usize, ty: NodeType, audio: &AudioHandle) -> Option<NodeId> {
    let t = track_of(g, key)?;
    if !inline_type(ty) || !t.gaps().is_some_and(|r| r.contains(&gap)) {
        return None;
    }
    let (up, down) = t.gap_ends(gap);
    let at = spot_between(g, up.map(|p| p.node), down.map(|p| p.node), ty.size());
    let id = g.add(ty, at, audio);
    if insert(g, key, gap, id, audio) {
        make_room(g, id);
        Some(id)
    } else {
        g.remove_node(id, audio);
        None
    }
}

/// Puts an existing node (just added, e.g. a plugin) at gap `gap`, placed
/// on the canvas between its neighbours. False if it doesn't fit there.
pub fn insert_placed(g: &mut Graph, key: TrackKey, gap: usize, id: NodeId, audio: &AudioHandle) -> bool {
    let Some(t) = track_of(g, key) else {
        return false;
    };
    let (up, down) = t.gap_ends(gap);
    let size = g.node(id).map_or(Vec2::ZERO, |n| n.size());
    let at = spot_between(g, up.map(|p| p.node), down.map(|p| p.node), size);
    if !insert(g, key, gap, id, audio) {
        return false;
    }
    if let Some(n) = g.node_mut(id) {
        n.pos = at;
    }
    make_room(g, id);
    true
}

/// Puts a MIDI node in front of an instrument: it takes what played the
/// instrument (if it can take MIDI) and plays the instrument. Refused when
/// that would unplug what played it.
pub fn insert_midi(g: &mut Graph, instrument: NodeId, id: NodeId, audio: &AudioHandle) -> bool {
    let (Some(inst), Some(n)) = (g.node(instrument), g.node(id)) else {
        return false;
    };
    if !inst.midi_in() || !n.midi_out() || instrument == id {
        return false;
    }
    let takes = n.midi_in();
    let target = PortRef {
        node: instrument,
        port: MIDI_PORT,
    };
    let feed = g.cables.iter().find(|c| c.to == target).map(|c| c.from);
    if feed.is_some() && !takes {
        return false;
    }
    let this = PortRef {
        node: id,
        port: MIDI_PORT,
    };
    let saved = g.cables.clone();
    let ok = feed.is_none_or(|f| g.connect(f, this, audio)) && g.connect(this, target, audio);
    if !ok {
        g.set_cables(saved, audio);
    }
    ok
}

/// Adds a MIDI node of type `ty` in front of an instrument (see
/// [`insert_midi`]), placed to its left on the canvas.
pub fn insert_midi_new(g: &mut Graph, instrument: NodeId, ty: NodeType, audio: &AudioHandle) -> Option<NodeId> {
    let at = spot_between(g, None, Some(instrument), ty.size());
    let id = g.add(ty, at, audio);
    if insert_midi(g, instrument, id, audio) {
        Some(id)
    } else {
        g.remove_node(id, audio);
        None
    }
}

/// Deletes a device, closing the gap: what fed its main input now feeds
/// everything it fed (every track it was in), and the same for MIDI
/// through a MIDI effect. Its side cables go with it.
pub fn remove(g: &mut Graph, id: NodeId, audio: &AudioHandle) {
    let Some(n) = g.node(id) else {
        return;
    };
    let main = PortRef { node: id, port: 0 };
    let midi = PortRef {
        node: id,
        port: MIDI_PORT,
    };
    let up = inline(n)
        .then(|| g.cables.iter().find(|c| c.to == main).map(|c| c.from))
        .flatten();
    let up_midi = (n.midi_in() && n.midi_out())
        .then(|| g.cables.iter().find(|c| c.to == midi).map(|c| c.from))
        .flatten();
    let outs: Vec<PortRef> = g
        .cables
        .iter()
        .filter(|c| c.from.node == id && !is_midi(c.from.port))
        .map(|c| c.to)
        .collect();
    let midi_outs: Vec<PortRef> = g
        .cables
        .iter()
        .filter(|c| c.from == midi)
        .map(|c| c.to)
        .collect();
    // An unrouted track ending here now ends at what fed it.
    if let Some(u) = up
        && g.node(u.node).is_some_and(|n| !is_sink(n))
    {
        g.tracks.rekey(TrackKey::Node(id), TrackKey::Node(u.node));
    }
    g.remove_node(id, audio);
    if let Some(u) = up {
        for to in outs {
            g.connect(u, to, audio);
        }
    }
    if let Some(u) = up_midi {
        for to in midi_outs {
            g.connect(u, to, audio);
        }
    }
}

/// Moves an effect to gap `gap` of track `key` (the same track or
/// another): its neighbours close up where it was, and it goes between
/// the new ones. Its other cables (splits, side inputs) go with it. False
/// (and nothing changed) if it can't go there.
pub fn move_device(g: &mut Graph, id: NodeId, key: TrackKey, gap: usize, audio: &AudioHandle) -> bool {
    let map = TrackMap::build(g);
    let (Some(target), Some(node)) = (map.find(key), g.node(id)) else {
        return false;
    };
    if !inline(node) || !target.gaps().is_some_and(|r| r.contains(&gap)) {
        return false;
    }
    let Some((row, _)) = map.locate(id) else {
        return false;
    };
    let from = &map.rows[row];
    let from_chain = from.chain();
    let Some(i) = from_chain.iter().position(|s| s.node == id) else {
        return false;
    };
    if !from.is_track() {
        return false;
    }
    let same = from.key == key;
    if same && (gap == i || gap == i + 1) {
        return true;
    }
    // The new neighbours, as if it were already out of its place.
    let mut chain = target.chain();
    let mut gap = gap;
    if same {
        chain.remove(i);
        if gap > i {
            gap -= 1;
        }
    }
    let up = if gap == 0 {
        match target.input {
            Input::Branch(p) | Input::Bus(p) => Some(p),
            _ => None,
        }
    } else {
        chain.get(gap - 1).map(Slot::out)
    };
    let down = chain
        .get(gap)
        .map(|s| PortRef {
            node: s.node,
            port: 0,
        })
        .or(target.dest);

    let main = PortRef { node: id, port: 0 };
    let fed_by = g.cables.iter().find(|c| c.to == main).map(|c| c.from);
    let next = from_chain
        .get(i + 1)
        .map(|s| PortRef {
            node: s.node,
            port: 0,
        })
        .or(if i + 1 == from_chain.len() { from.dest } else { None });
    let own_out = from_chain[i].out();
    let saved = g.cables.clone();
    // Out of its place: its neighbours close up.
    if let Some(f) = fed_by {
        g.disconnect(Cable { from: f, to: main }, audio);
    }
    let mut ok = true;
    if let Some(nx) = next {
        match fed_by {
            Some(f) => ok &= g.connect(f, nx, audio),
            None => g.disconnect(Cable { from: own_out, to: nx }, audio),
        }
    }
    // Into the new one.
    let out = PortRef { node: id, port: 0 };
    ok = ok && up.is_none_or(|u| g.connect(u, main, audio)) && down.is_none_or(|d| g.connect(out, d, audio));
    if !ok {
        g.set_cables(saved, audio);
        return false;
    }
    if from.key == TrackKey::Node(id)
        && let Some(f) = fed_by
        && g.node(f.node).is_some_and(|n| !is_sink(n))
    {
        g.tracks.rekey(from.key, TrackKey::Node(f.node));
    }
    // An unrouted track it went to the end of now ends at it.
    if down.is_none() && matches!(key, TrackKey::Node(_)) {
        g.tracks.rekey(key, TrackKey::Node(id));
    }
    true
}

/// The first mixer channel with nothing plugged in.
pub fn free_channel(g: &Graph) -> Option<PortRef> {
    free_channels(g).into_iter().next()
}

/// Every mixer channel with nothing plugged in, mixer by mixer.
pub fn free_channels(g: &Graph) -> Vec<PortRef> {
    g.nodes
        .iter()
        .filter(|n| is_sink(n))
        .flat_map(|n| (0..n.inputs().len()).map(move |port| PortRef { node: n.id, port }))
        .filter(|p| !g.cables.iter().any(|c| c.to == *p))
        .collect()
}

/// Sends a track to a free mixer channel, or (None) to nothing. Never
/// takes a channel another track uses. False if it can't.
pub fn route(g: &mut Graph, key: TrackKey, dest: Option<PortRef>, audio: &AudioHandle) -> bool {
    let Some(t) = track_of(g, key) else {
        return false;
    };
    let Some(tail) = t.tail() else {
        return false;
    };
    if t.dest == dest {
        return true;
    }
    if let Some(d) = dest
        && (g.cables.iter().any(|c| c.to == d) || !g.node(d.node).is_some_and(is_sink))
    {
        return false;
    }
    let saved = g.cables.clone();
    if let Some(old) = t.dest {
        g.disconnect(Cable { from: tail, to: old }, audio);
    }
    if let Some(d) = dest
        && !g.connect(tail, d, audio)
    {
        g.set_cables(saved, audio);
        return false;
    }
    let new = match dest {
        Some(d) => TrackKey::Channel(d),
        None => TrackKey::Node(tail.node),
    };
    g.tracks.rekey(key, new);
    true
}

/// A new track: adds a node of type `ty` below the patch on the canvas and
/// sends it to the first free mixer channel (if it makes audio and one is
/// free). Returns the node.
pub fn new_track(g: &mut Graph, ty: NodeType, audio: &AudioHandle) -> NodeId {
    let at = below_content(g);
    let id = g.add(ty, at, audio);
    route_new(g, id, audio);
    id
}

/// Makes an existing node (just added) a track of its own: below the
/// patch on the canvas, sent to the first free channel, last in order.
pub fn route_new(g: &mut Graph, id: NodeId, audio: &AudioHandle) {
    let at = below_content_except(g, id);
    if let Some(n) = g.node_mut(id) {
        n.pos = at;
    }
    let makes_audio = g.node(id).is_some_and(|n| !n.outputs().is_empty() && !is_sink(n));
    let key = match free_channel(g).filter(|_| makes_audio) {
        Some(ch) if g.connect(PortRef { node: id, port: 0 }, ch, audio) => TrackKey::Channel(ch),
        _ => TrackKey::Node(id),
    };
    if !g.tracks.order.is_empty() && !g.tracks.order.contains(&key) {
        g.tracks.order.push(key);
    }
}

fn below_content_except(g: &Graph, id: NodeId) -> Pos2 {
    g.nodes
        .iter()
        .filter(|n| n.id != id)
        .map(rect_of)
        .reduce(|a, b| a.union(b))
        .map_or(pos2(0.0, 0.0), |r| pos2(r.left(), r.bottom() + 80.0))
}

/// Moves track `from` (an index among the tracks) to index `to`.
pub fn reorder(g: &mut Graph, from: usize, to: usize) {
    let mut keys: Vec<TrackKey> = TrackMap::build(g).tracks().map(|t| t.key).collect();
    if from >= keys.len() {
        return;
    }
    let k = keys.remove(from);
    keys.insert(to.min(keys.len()), k);
    g.tracks.order = keys;
}
