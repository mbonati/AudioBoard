//! Saving and loading: whole projects, and modules kept in a library for
//! reuse. Both are JSON. Parameters are stored by name, so files keep
//! working if knobs are added or reordered.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::audio::AudioHandle;
use crate::audio::engine::NodeId;
use crate::audio::modules::{ModuleKind, ModuleParams};
use crate::audio::params::{MIXER_CHANNELS, MixerParams, OSC_PARAMS, OscParams, Waveform};
use crate::graph::{Graph, GroupId, Node, NodeKind, PortRef, Target};
use crate::ui::groups;
use crate::ui::nodes::HEADER_H;

const FORMAT: &str = "audioboard";
const VERSION: u32 = 1;
pub const PROJECT_EXT: &str = "audioboard";
const PRESET_EXT: &str = "abmodule";

/// A whole patch.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct Project {
    format: String,
    version: u32,
    nodes: Vec<NodeData>,
    cables: Vec<CableData>,
    #[serde(default)]
    modules: Vec<ModuleData>,
}

/// A module saved in the library: its nodes (positioned relative to the
/// module's corner) and the cables between them.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Preset {
    format: String,
    version: u32,
    pub name: String,
    #[serde(default)]
    collapsed: bool,
    nodes: Vec<NodeData>,
    cables: Vec<CableData>,
}

impl Preset {
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct NodeData {
    id: u32,
    title: String,
    pos: [f32; 2],
    #[serde(default)]
    collapsed: bool,
    color: [u8; 3],
    #[serde(flatten)]
    kind: KindData,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "type")]
enum KindData {
    Oscillator {
        waveform: String,
        params: BTreeMap<String, f32>,
    },
    Mixer {
        channels: Vec<ChannelData>,
        master: f32,
    },
    Module {
        module: String,
        params: BTreeMap<String, f32>,
        #[serde(default)]
        mode: String,
        #[serde(default)]
        toggle: bool,
        /// MIDI nodes: the control they drive.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<TargetData>,
    },
}

/// A mapped control: node id in the file and control name ("CUTOFF").
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct TargetData {
    node: u32,
    param: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct ChannelData {
    fader: f32,
    pan: f32,
    mute: bool,
    solo: bool,
}

/// `[node, port]` → `[node, port]`.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
struct CableData {
    from: [u32; 2],
    to: [u32; 2],
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct ModuleData {
    title: String,
    members: Vec<u32>,
    #[serde(default)]
    collapsed: bool,
}

// ---------------------------------------------------------------------------
// Graph → data
// ---------------------------------------------------------------------------

fn node_data(graph: &Graph, n: &Node, numbers: &HashMap<NodeId, u32>, origin: Vec2) -> NodeData {
    let kind = match &n.kind {
        NodeKind::Osc(o) => KindData::Oscillator {
            waveform: o.params.waveform().name().to_owned(),
            params: OSC_PARAMS
                .iter()
                .enumerate()
                .map(|(i, spec)| (spec.name.to_owned(), o.params.get(i)))
                .collect(),
        },
        NodeKind::Mixer(m) => KindData::Mixer {
            channels: (0..MIXER_CHANNELS)
                .map(|ch| ChannelData {
                    fader: m.params.fader[ch].get(),
                    pan: m.params.pan[ch].get(),
                    mute: m.params.mute[ch].load(Ordering::Relaxed),
                    solo: m.params.solo[ch].load(Ordering::Relaxed),
                })
                .collect(),
            master: m.params.master.get(),
        },
        NodeKind::Module(m) => {
            let p = &m.params;
            let def = p.kind.def();
            KindData::Module {
                module: def.name.to_owned(),
                params: def
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, spec)| (spec.name.to_owned(), p.get(i)))
                    .collect(),
                mode: def.modes.get(p.mode()).copied().unwrap_or_default().to_owned(),
                toggle: p.toggled(),
                // Kept only when the mapped node is saved too.
                target: m.target.and_then(|t| {
                    Some(TargetData {
                        node: *numbers.get(&t.node)?,
                        param: graph.node(t.node)?.param_name(t.param)?,
                    })
                }),
            }
        }
    };
    NodeData {
        id: numbers[&n.id],
        title: n.title.clone(),
        pos: [n.pos.x - origin.x, n.pos.y - origin.y],
        collapsed: n.collapsed,
        color: [n.accent.r(), n.accent.g(), n.accent.b()],
        kind,
    }
}

/// Nodes (renumbered from 1, in the given order) and the cables between them.
fn capture(graph: &Graph, ids: &[NodeId], origin: Vec2) -> (Vec<NodeData>, Vec<CableData>, HashMap<NodeId, u32>) {
    let numbers: HashMap<NodeId, u32> = ids.iter().enumerate().map(|(i, id)| (*id, i as u32 + 1)).collect();
    let nodes = ids
        .iter()
        .filter_map(|id| graph.node(*id))
        .map(|n| node_data(graph, n, &numbers, origin))
        .collect();
    let cables = graph
        .cables
        .iter()
        .filter_map(|c| {
            Some(CableData {
                from: [*numbers.get(&c.from.node)?, c.from.port as u32],
                to: [*numbers.get(&c.to.node)?, c.to.port as u32],
            })
        })
        .collect();
    (nodes, cables, numbers)
}

pub fn snapshot(graph: &Graph) -> Project {
    let ids: Vec<NodeId> = graph.nodes.iter().map(|n| n.id).collect();
    let (nodes, cables, numbers) = capture(graph, &ids, Vec2::ZERO);
    let modules = graph
        .groups
        .iter()
        .map(|g| ModuleData {
            title: g.title.clone(),
            members: g.members.iter().filter_map(|m| numbers.get(m).copied()).collect(),
            collapsed: g.collapsed,
        })
        .collect();
    Project {
        format: FORMAT.to_owned(),
        version: VERSION,
        nodes,
        cables,
        modules,
    }
}

/// Library entry for a module, named after it.
pub fn preset_from_group(graph: &Graph, gid: GroupId) -> Option<Preset> {
    let g = graph.group(gid)?;
    let origin = g
        .members
        .iter()
        .filter_map(|id| graph.node(*id))
        .map(|n| Rect::from_min_size(n.pos, n.size()))
        .reduce(|a, b| a.union(b))?
        .min
        .to_vec2();
    let (nodes, cables, _) = capture(graph, &g.members, origin);
    Some(Preset {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: g.title.clone(),
        collapsed: g.collapsed,
        nodes,
        cables,
    })
}

// ---------------------------------------------------------------------------
// Data → graph
// ---------------------------------------------------------------------------

/// Adds the nodes at `origin` and wires them; returns file id → new node id.
/// Nodes of an unknown type are skipped (and so are their cables).
fn instantiate(
    graph: &mut Graph,
    nodes: &[NodeData],
    cables: &[CableData],
    origin: Vec2,
    audio: &AudioHandle,
) -> HashMap<u32, NodeId> {
    let mut ids = HashMap::new();
    for data in nodes {
        let pos = pos2(data.pos[0], data.pos[1]) + origin;
        let id = match &data.kind {
            KindData::Oscillator { waveform, params } => {
                let p = OscParams::new();
                if let Some(w) = Waveform::ALL.iter().find(|w| w.name() == waveform) {
                    p.set_waveform(*w);
                }
                for (i, spec) in OSC_PARAMS.iter().enumerate() {
                    if let Some(v) = params.get(spec.name) {
                        p.set(i, v.clamp(spec.min, spec.max));
                    }
                }
                graph.add_osc_with(pos, Arc::new(p), audio)
            }
            KindData::Mixer { channels, master } => {
                let p = MixerParams::new();
                for (ch, c) in channels.iter().take(MIXER_CHANNELS).enumerate() {
                    p.fader[ch].set(c.fader.clamp(0.0, 1.0));
                    p.pan[ch].set(c.pan.clamp(-1.0, 1.0));
                    p.mute[ch].store(c.mute, Ordering::Relaxed);
                    p.solo[ch].store(c.solo, Ordering::Relaxed);
                }
                p.master.set(master.clamp(0.0, 1.0));
                graph.add_mixer_with(pos, Arc::new(p), audio)
            }
            KindData::Module { module, params, mode, toggle, .. } => {
                let Some(kind) = ModuleKind::ALL.iter().copied().find(|k| k.def().name == module) else {
                    continue;
                };
                let def = kind.def();
                let p = ModuleParams::new(kind);
                for (i, spec) in def.params.iter().enumerate() {
                    if let Some(v) = params.get(spec.name) {
                        p.set(i, v.clamp(spec.min, spec.max));
                    }
                }
                p.set_mode(def.modes.iter().position(|m| m == mode).unwrap_or(0));
                p.set_toggle(*toggle);
                graph.add_module_with(pos, Arc::new(p), audio)
            }
        };
        if let Some(n) = graph.node_mut(id) {
            n.title = data.title.clone();
            n.collapsed = data.collapsed;
            n.accent = Color32::from_rgb(data.color[0], data.color[1], data.color[2]);
        }
        ids.insert(data.id, id);
    }
    for data in nodes {
        if let KindData::Module { target: Some(t), .. } = &data.kind
            && let (Some(&from), Some(&to)) = (ids.get(&data.id), ids.get(&t.node))
            && let Some(param) = graph.node(to).and_then(|n| n.param_by_name(&t.param))
        {
            graph.set_target(from, Some(Target { node: to, param }));
        }
    }
    for c in cables {
        if let (Some(&from), Some(&to)) = (ids.get(&c.from[0]), ids.get(&c.to[0])) {
            let from = PortRef { node: from, port: c.from[1] as usize };
            let to = PortRef { node: to, port: c.to[1] as usize };
            graph.connect(from, to, audio);
        }
    }
    ids
}

/// Replaces the patch with the project's. Returns how many nodes could not
/// be loaded (unknown types).
pub fn restore(graph: &mut Graph, project: &Project, audio: &AudioHandle) -> usize {
    graph.clear(audio);
    let ids = instantiate(graph, &project.nodes, &project.cables, Vec2::ZERO, audio);
    for m in &project.modules {
        let members: Vec<NodeId> = m.members.iter().filter_map(|id| ids.get(id).copied()).collect();
        if let Some(gid) = graph.create_group(&members) {
            graph.rename_group(gid, &m.title);
            graph.set_group_collapsed(gid, m.collapsed);
        }
    }
    project.nodes.len() - ids.len()
}

/// Adds a library module with its frame's top-left corner at `at`; returns
/// its nodes.
pub fn insert_preset(graph: &mut Graph, preset: &Preset, at: Pos2, audio: &AudioHandle) -> Vec<NodeId> {
    let origin = at.to_vec2() + vec2(groups::PAD, groups::PAD + HEADER_H);
    let ids = instantiate(graph, &preset.nodes, &preset.cables, origin, audio);
    let members: Vec<NodeId> = preset.nodes.iter().filter_map(|n| ids.get(&n.id).copied()).collect();
    // Arrives folded: a library module is used as one block.
    if let Some(gid) = graph.create_group(&members) {
        graph.rename_group(gid, &preset.name);
        graph.set_group_collapsed(gid, true);
    }
    members
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

pub fn save_project(path: &Path, project: &Project) -> Result<(), String> {
    let json = serde_json::to_string_pretty(project).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub fn load_project(path: &Path) -> Result<Project, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let project: Project = serde_json::from_str(&text).map_err(|e| format!("not a valid project ({e})"))?;
    if project.format != FORMAT {
        return Err("not an AudioBoard project".to_owned());
    }
    Ok(project)
}

/// Per-user application data folder.
pub fn data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/AudioBoard"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("AudioBoard"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local/share")))
            .map(|p| p.join("audioboard"))
    }
}

/// Saved modules, one file each, sorted by name.
pub struct Library {
    pub dir: PathBuf,
    pub presets: Vec<Preset>,
    /// File of each preset.
    paths: Vec<PathBuf>,
}

impl Library {
    pub fn open(dir: PathBuf) -> Self {
        let mut lib = Self {
            dir,
            presets: Vec::new(),
            paths: Vec::new(),
        };
        lib.reload();
        lib
    }

    pub fn reload(&mut self) {
        let entries = std::fs::read_dir(&self.dir).into_iter().flatten().flatten();
        let mut found: Vec<(PathBuf, Preset)> = entries
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == PRESET_EXT))
            .filter_map(|p| {
                let preset: Preset = serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
                (preset.format == FORMAT).then_some((p, preset))
            })
            .collect();
        found.sort_by_key(|(_, p)| p.name.to_lowercase());
        (self.paths, self.presets) = found.into_iter().unzip();
    }

    fn path_of(&self, name: &str) -> PathBuf {
        let safe: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' })
            .collect();
        self.dir.join(format!("{}.{PRESET_EXT}", safe.trim()))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.presets.iter().any(|p| p.name == name)
    }

    /// Writes the module, replacing any entry with the same name.
    pub fn save(&mut self, preset: &Preset) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(preset).map_err(|e| e.to_string())?;
        let path = match self.presets.iter().position(|p| p.name == preset.name) {
            Some(i) => self.paths[i].clone(),
            None => self.path_of(&preset.name),
        };
        std::fs::write(path, json).map_err(|e| e.to_string())?;
        self.reload();
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> Result<(), String> {
        let path = self.paths.get(index).ok_or("no such module")?;
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
        self.reload();
        Ok(())
    }
}
