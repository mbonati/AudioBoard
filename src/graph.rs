//! The patch as the UI sees it: nodes with positions, and cables.
//! Every structural change is mirrored to the audio engine.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Color32, Pos2, Vec2, vec2};

use crate::audio::AudioHandle;
use crate::audio::dsp::{Mixer, Oscillator};
use crate::audio::engine::{Command, MAX_INPUTS, NodeId, Slot, TopoEntry, Topology, audio_idle, wait_audio_idle};
use crate::audio::midi::{MidiState, note_hz};
use crate::audio::modules::{ModuleKind, ModuleParams, midi_cc, midi_note, random};
use crate::audio::params::{
    AtomicF32, DEFAULT_TEMPO, MIXER_CHANNELS, MixerParams, OSC_PARAMS, OscParams, ParamSpec, Unit, osc,
};
use crate::audio::plugin::{self, Control, Loaded, PluginMeta, PluginParam, PluginProcessor, PluginShared};
use crate::ui::nodes;
use crate::ui::theme;
use crate::ui::widgets::MeterState;

/// Port index of a node's MIDI input or output (audio ports count from 0).
pub const MIDI_PORT: usize = 64;

pub fn is_midi(port: usize) -> bool {
    port == MIDI_PORT
}

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
    /// A VST3 plugin (added from a path, not from the menu).
    Plugin,
}

impl NodeType {
    /// The "add node" menu, in groups.
    pub const MENU: [(&'static str, &'static [NodeType]); 5] = [
        (
            "Sources",
            &[
                NodeType::Osc,
                NodeType::Module(ModuleKind::Drone),
                NodeType::Module(ModuleKind::Random),
                NodeType::Module(ModuleKind::Pops),
            ],
        ),
        (
            "Texture",
            &[
                NodeType::Module(ModuleKind::Granular),
                NodeType::Module(ModuleKind::Frakture),
                NodeType::Module(ModuleKind::Resonator),
            ],
        ),
        (
            "Effects",
            &[
                NodeType::Module(ModuleKind::Filter),
                NodeType::Module(ModuleKind::Magma),
                NodeType::Module(ModuleKind::Equalizer),
                NodeType::Module(ModuleKind::Delay),
                NodeType::Module(ModuleKind::PingPong),
                NodeType::Module(ModuleKind::Reverb),
            ],
        ),
        (
            "MIDI",
            &[
                NodeType::Module(ModuleKind::Sequencer),
                NodeType::Module(ModuleKind::Midicer),
                NodeType::Module(ModuleKind::Arpeggiator),
                NodeType::Module(ModuleKind::MidiIn),
                NodeType::Module(ModuleKind::MidiNote),
                NodeType::Module(ModuleKind::MidiCc),
            ],
        ),
        ("Output", &[NodeType::Mixer]),
    ];

    pub fn name(self) -> &'static str {
        match self {
            NodeType::Osc => "Oscillator",
            NodeType::Mixer => "Mixer",
            NodeType::Module(k) => k.def().name,
            NodeType::Plugin => "VST3 plugin",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            NodeType::Osc => "sine · saw · pulse",
            NodeType::Mixer => "4 channels",
            NodeType::Module(k) => k.def().hint,
            NodeType::Plugin => "",
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
    /// A VST3 parameter id.
    Plugin(u32),
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

/// Knobs shown on a plugin node.
pub const PINNED_MAX: usize = 8;

pub struct PluginNode {
    pub shared: Arc<PluginShared>,
    pub meta: PluginMeta,
    pub params: Vec<PluginParam>,
    /// Last known normalized value of each parameter (UI side).
    pub values: Vec<f64>,
    /// The plugin's own text for each pinned parameter's value.
    pub texts: Vec<String>,
    /// Parameters shown as knobs (indices into `params`).
    pub pinned: Vec<usize>,
    pub control: Sender<Control>,
    pub meter: MeterState,
    /// The plugin's own editor, while open.
    pub window: Option<vst3_host::PluginWindow>,
    /// Filter for the parameter list.
    pub query: String,
    /// Until when a parameter the user just moved ignores the plugin's
    /// (not yet updated) value.
    held_until: Vec<f64>,
    /// When each parameter was last read from the plugin, and when the
    /// pinned ones were. Reading takes the plugin's lock, which the audio
    /// thread needs too, so it happens a few times a second at most.
    read_at: Vec<f64>,
    synced_at: f64,
    /// The plugin's saved state (base64) as last read, for telling whether
    /// the project changed without asking the plugin every time, and
    /// whether it may have changed since.
    state: RefCell<Option<(String, Instant)>>,
    state_stale: Cell<bool>,
}

/// How often the UI reads plugin parameters (seconds).
const PLUGIN_SYNC_EVERY: f64 = 0.1;

impl PluginNode {
    fn new(meta: PluginMeta, params: Vec<PluginParam>, shared: Arc<PluginShared>, control: Sender<Control>) -> Self {
        let n = params.len();
        Self {
            values: params.iter().map(|p| p.default).collect(),
            texts: vec![String::new(); n],
            pinned: (0..n.min(PINNED_MAX)).collect(),
            held_until: vec![0.0; n],
            read_at: vec![f64::NEG_INFINITY; n],
            synced_at: f64::NEG_INFINITY,
            state: RefCell::new(None),
            state_stale: Cell::new(true),
            shared,
            meta,
            params,
            control,
            meter: MeterState::default(),
            window: None,
            query: String::new(),
        }
    }

    /// Sets a parameter from the UI (applied on the audio thread).
    pub fn set_param(&mut self, index: usize, value: f64, now: f64) {
        let Some(p) = self.params.get(index) else { return };
        let value = value.clamp(0.0, 1.0);
        self.values[index] = value;
        self.held_until[index] = now + 0.3;
        self.state_stale.set(true);
        let _ = self.control.send(Control::Param(p.id, value));
    }

    /// Sends a parameter change from a MIDI mapping.
    pub fn send_param(&self, id: u32, value: f64) {
        self.state_stale.set(true);
        let _ = self.control.send(Control::Param(id, value));
    }

    /// Keeps the knobs in step with the plugin (including edits made in its
    /// own editor) and services the editor window. The plugin's lock is
    /// taken for one call at a time and never waited for, so the audio
    /// thread is held up as little as possible.
    pub fn sync(&mut self, now: f64) {
        if self.window.as_ref().is_some_and(|w| w.closed_by_user() || !w.is_open()) {
            self.window = None;
        }
        // Every call below takes the plugin's lock, which the audio thread
        // needs for each callback: only between callbacks.
        if !audio_idle() {
            return;
        }
        if let Some(w) = &self.window {
            let _ = w.service_platform_events();
            // Its editor can change anything, presets included.
            self.state_stale.set(true);
        }
        if now - self.synced_at < PLUGIN_SYNC_EVERY {
            return;
        }
        self.synced_at = now;
        match self.shared.plugin.try_lock() {
            Ok(mut plugin) => {
                let _ = plugin.service_host_requests();
            }
            Err(_) => return,
        }
        for k in 0..self.pinned.len() {
            let i = self.pinned[k];
            let id = self.params[i].id;
            if !audio_idle() {
                return;
            }
            let Ok(plugin) = self.shared.plugin.try_lock() else { return };
            if self.held_until[i] < now
                && let Ok(v) = plugin.get_parameter(id)
                && v != self.values[i]
            {
                self.values[i] = v;
            }
            if let Ok(text) = plugin.format_parameter(id, self.values[i]) {
                self.texts[i] = text;
            }
            self.read_at[i] = now;
        }
    }

    /// Reads the current value of any parameter (for the parameter list).
    pub fn refresh(&mut self, index: usize, now: f64) {
        if self.held_until[index] >= now || now - self.read_at[index] < PLUGIN_SYNC_EVERY || !audio_idle() {
            return;
        }
        self.read_at[index] = now;
        let id = self.params[index].id;
        if let Ok(plugin) = self.shared.plugin.try_lock()
            && let Ok(v) = plugin.get_parameter(id)
        {
            self.values[index] = v;
        }
    }

    /// The saved state as base64. `fresh` asks the plugin now (saving);
    /// otherwise the copy from the last save is reused and the plugin is not
    /// asked at all: some plugins take hundreds of milliseconds to save
    /// (a drum machine's kits), holding up the audio thread meanwhile.
    pub fn state_base64(&self, fresh: bool) -> Option<String> {
        let mut cache = self.state.borrow_mut();
        if fresh && let Some(bytes) = self.save_state() {
            *cache = Some((crate::project::base64_encode(&bytes), Instant::now()));
            self.state_stale.set(false);
        }
        cache.as_ref().map(|(s, _)| s.clone())
    }

    /// Changed since its state was last saved: a knob moved here or from
    /// MIDI, its editor opened, or never saved.
    pub fn edited(&self) -> bool {
        self.state_stale.get()
    }

    /// Opens or closes the plugin's own window.
    pub fn toggle_editor(&mut self) -> Result<(), String> {
        if self.window.take().is_some() {
            return Ok(());
        }
        let mut window = vst3_host::PluginWindow::new(self.shared.plugin.clone());
        wait_audio_idle(Duration::from_millis(50));
        window.open().map_err(|e| e.to_string())?;
        self.window = Some(window);
        Ok(())
    }

    /// The plugin's complete state (its own format). Waits for a gap
    /// between audio callbacks, so as not to hold the audio thread up.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        wait_audio_idle(Duration::from_millis(50));
        self.shared.plugin.lock().ok()?.save_state().ok()
    }

    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        wait_audio_idle(Duration::from_millis(50));
        let mut plugin = self.shared.plugin.lock().map_err(|e| e.to_string())?;
        plugin.load_state(data).map_err(|e| e.to_string())?;
        self.state_stale.set(true);
        for (i, p) in self.params.iter().enumerate() {
            if let Ok(v) = plugin.get_parameter(p.id) {
                self.values[i] = v;
            }
        }
        Ok(())
    }
}

/// Node title prefix for a plugin: "CINEMATIC PADS" for "Originals - Cinematic Pads".
fn plugin_prefix(name: &str) -> String {
    let short = name.rsplit(" - ").next().unwrap_or(name);
    short.to_uppercase().chars().take(24).collect::<String>().trim().to_owned()
}

pub const TRACE_LEN: usize = 180;

pub enum NodeKind {
    Osc(OscNode),
    Mixer(MixerNode),
    Module(ModuleNode),
    Plugin(Box<PluginNode>),
}

impl NodeKind {
    pub fn node_type(&self) -> NodeType {
        match self {
            NodeKind::Osc(_) => NodeType::Osc,
            NodeKind::Mixer(_) => NodeType::Mixer,
            NodeKind::Module(m) => NodeType::Module(m.params.kind),
            NodeKind::Plugin(_) => NodeType::Plugin,
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
            nodes::collapsed_size(self.input_ports().len().max(self.output_ports().len()))
        } else if let NodeKind::Module(m) = &self.kind {
            // Some panels grow and shrink with their settings.
            crate::ui::modules::size_of(&m.params)
        } else {
            nodes::type_size(self.kind.node_type())
        }
    }

    pub fn inputs(&self) -> &'static [&'static str] {
        match &self.kind {
            NodeKind::Osc(_) => &["FM", "AM"],
            NodeKind::Mixer(_) => &["1", "2", "3", "4"],
            NodeKind::Module(m) => m.params.kind.def().inputs,
            NodeKind::Plugin(p) if p.meta.inputs > 0 => &["IN"],
            NodeKind::Plugin(_) => &[],
        }
    }

    pub fn outputs(&self) -> &'static [&'static str] {
        match &self.kind {
            NodeKind::Osc(_) => &["OUT"],
            NodeKind::Mixer(_) => &[],
            NodeKind::Module(m) => m.params.kind.def().outputs,
            NodeKind::Plugin(_) => &["OUT"],
        }
    }

    /// Takes MIDI on a MIDI cable.
    pub fn midi_in(&self) -> bool {
        match &self.kind {
            NodeKind::Module(m) => m.params.kind.def().midi_in,
            NodeKind::Plugin(p) => p.meta.midi,
            _ => false,
        }
    }

    /// Sends MIDI on a MIDI cable.
    pub fn midi_out(&self) -> bool {
        match &self.kind {
            NodeKind::Module(m) => m.params.kind.def().midi_out,
            _ => false,
        }
    }

    /// Every input port: the audio ones, then MIDI ([`MIDI_PORT`]).
    pub fn input_ports(&self) -> Vec<usize> {
        (0..self.inputs().len()).chain(self.midi_in().then_some(MIDI_PORT)).collect()
    }

    pub fn output_ports(&self) -> Vec<usize> {
        (0..self.outputs().len()).chain(self.midi_out().then_some(MIDI_PORT)).collect()
    }

    pub fn port_label(&self, port: usize, output: bool) -> &'static str {
        let names = if output { self.outputs() } else { self.inputs() };
        if is_midi(port) { "MIDI" } else { names.get(port).copied().unwrap_or("?") }
    }

    /// Which row a port sits on: MIDI comes after the audio ports.
    fn row(&self, port: usize, output: bool) -> usize {
        match (is_midi(port), output) {
            (true, false) => self.inputs().len(),
            (true, true) => self.outputs().len(),
            (false, _) => port,
        }
    }

    pub fn input_pos(&self, port: usize) -> Pos2 {
        let row = self.row(port, false);
        self.pos
            + if self.collapsed {
                nodes::collapsed_port(row, false)
            } else {
                nodes::input_offset(self.kind.node_type(), row)
            }
    }

    pub fn output_pos(&self, port: usize) -> Pos2 {
        let row = self.row(port, true);
        self.pos
            + if self.collapsed {
                nodes::collapsed_port(row, true)
            } else {
                // On the right edge, wherever it is now.
                let offset = nodes::output_offset(self.kind.node_type(), row);
                vec2(self.size().x, offset.y)
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
            (NodeKind::Plugin(p), ParamRef::Plugin(id)) => p.params.iter().find(|q| q.id == id).map(|q| q.name.clone()),
            _ => None,
        }
    }

    /// The control called `name` (see [`Self::param_name`]).
    pub fn param_by_name(&self, name: &str) -> Option<ParamRef> {
        let specs: &[ParamSpec] = match &self.kind {
            NodeKind::Osc(_) => &OSC_PARAMS,
            NodeKind::Module(m) => m.params.kind.def().params,
            NodeKind::Plugin(p) => return p.params.iter().find(|q| q.name == name).map(|q| ParamRef::Plugin(q.id)),
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
            NodeKind::Plugin(p) => p.meter.level,
        }
    }

    /// Pulls peaks from the audio thread into the display meters.
    pub fn update_meters(&mut self, dt: f32) {
        match &mut self.kind {
            NodeKind::Plugin(p) => p.meter.update(p.shared.meter.take(), dt),
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

pub struct Graph {
    pub nodes: Vec<Node>,
    pub cables: Vec<Cable>,
    pub groups: Vec<Group>,
    /// Beats per minute of the global clock (sequencers, arpeggiators,
    /// tempo-synced plugins); the app passes it to the engine.
    pub tempo: f32,
    next_id: NodeId,
    next_group: GroupId,
    /// Nodes given a colour so far; each new source/effect takes the next.
    accent_count: usize,
    /// Loads VST3 plugins; made on first use.
    plugin_host: Option<vst3_host::Vst3Host>,
    /// Removed plugins, kept until the audio thread lets go of them so they
    /// are destroyed here, never on the audio thread.
    graveyard: Vec<Arc<PluginShared>>,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            cables: Vec::new(),
            groups: Vec::new(),
            tempo: DEFAULT_TEMPO,
            next_id: 0,
            next_group: 0,
            accent_count: 0,
            plugin_host: None,
            graveyard: Vec::new(),
        }
    }
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
        self.tempo = DEFAULT_TEMPO;
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
            // Plugins come from a file: see `add_plugin`. The menus never offer this.
            NodeType::Plugin => unreachable!("plugins are added with add_plugin"),
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

    /// Loads a VST3 plugin as a node. Runs plugin code (UI thread).
    pub fn add_plugin(&mut self, path: &Path, pos: Pos2, audio: &AudioHandle) -> Result<NodeId, String> {
        let host = match &mut self.plugin_host {
            Some(h) => h,
            None => self.plugin_host.insert(plugin::new_host(audio.info.sample_rate as f32)?),
        };
        let loaded = plugin::load(host, path)?;
        Ok(self.add_plugin_loaded(loaded, pos, audio))
    }

    fn add_plugin_loaded(&mut self, loaded: Loaded, pos: Pos2, audio: &AudioHandle) -> NodeId {
        let Loaded { plugin, meta, params } = loaded;
        let id = self.alloc_id();
        let accent = self.next_accent();
        let title = self.next_title(&plugin_prefix(&meta.name));
        let instrument = meta.instrument;
        let shared = Arc::new(PluginShared {
            plugin: Arc::new(Mutex::new(plugin)),
            meter: AtomicF32::new(0.0),
            midi_in: AtomicBool::new(instrument),
            missed: AtomicU32::new(0),
        });
        let (tx, rx) = mpsc::channel();
        let sample_rate = audio.info.sample_rate as f32;
        let proc = PluginProcessor::new(shared.clone(), rx, audio.midi.clone(), &meta, sample_rate);
        audio.send(Command::AddNode(id, Slot::new(Box::new(proc), 1)));
        self.nodes.push(Node {
            id,
            pos,
            title,
            accent,
            collapsed: false,
            kind: NodeKind::Plugin(Box::new(PluginNode::new(meta, params, shared, tx))),
        });
        self.sync(audio);
        id
    }

    /// Per frame, on the UI thread: keeps plugin nodes in step and destroys
    /// removed plugins once the audio thread has let go of them.
    /// Some plugin may have changed since the project was saved (its state
    /// is only read when saving).
    pub fn plugins_edited(&self) -> bool {
        self.nodes.iter().any(|n| matches!(&n.kind, NodeKind::Plugin(p) if p.edited()))
    }

    pub fn service_plugins(&mut self, now: f64) {
        self.graveyard.retain(|shared| Arc::strong_count(shared) > 1);
        for n in &mut self.nodes {
            if let NodeKind::Plugin(p) = &mut n.kind {
                p.sync(now);
            }
        }
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
            NodeKind::Plugin(p) => {
                let (path, state, pinned) = (p.meta.path.clone(), p.save_state(), p.pinned.clone());
                let midi = p.shared.midi_in.load(Ordering::Relaxed);
                let copy = self.add_plugin(&path, pos, audio).ok()?;
                if let Some(NodeKind::Plugin(q)) = self.node_mut(copy).map(|n| &mut n.kind) {
                    if let Some(state) = state {
                        let _ = q.load_state(&state);
                    }
                    q.pinned = pinned;
                    q.shared.midi_in.store(midi, Ordering::Relaxed);
                }
                copy
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
        if let Some(i) = self.nodes.iter().position(|n| n.id == id)
            && let NodeKind::Plugin(mut p) = self.nodes.remove(i).kind
        {
            p.window = None; // closes the editor first
            self.graveyard.push(p.shared.clone());
        }
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
                ModuleKind::MidiNote if p.viz[midi_note::CABLE].get() > 0.5 => {
                    // Played from a MIDI cable: the processor reports the notes.
                    let seq = p.viz[midi_note::SEQ].get() as u32 | 1 << 31;
                    (seq, Write::Note(p.viz[midi_note::NOTE].get().max(0.0)))
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
            (NodeKind::Plugin(p), ParamRef::Plugin(id)) => {
                p.send_param(id, norm as f64);
            }
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
            for port in node.input_ports() {
                let pr = PortRef { node: node.id, port };
                if !self.cables.iter().any(|c| c.to == pr && inside(c.from.node)) {
                    ins.push(pr);
                }
            }
            for port in node.output_ports() {
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
        // MIDI only into MIDI, audio only into audio, between ports that exist.
        let exists = self.node(from.node).is_some_and(|n| n.output_ports().contains(&from.port))
            && self.node(to.node).is_some_and(|n| n.input_ports().contains(&to.port));
        if !exists || is_midi(from.port) != is_midi(to.port) || from.node == to.node || self.reaches(to.node, from.node) {
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
            let mut midi_input = None;
            for c in self.cables.iter().filter(|c| c.to.node == id) {
                if is_midi(c.to.port) {
                    midi_input = Some(c.from.node);
                } else if c.to.port < MAX_INPUTS {
                    inputs[c.to.port] = Some((c.from.node, c.from.port));
                }
            }
            order.push(TopoEntry { id, inputs, midi_input });
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
