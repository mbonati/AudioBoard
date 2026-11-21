//! Node plugins: precompiled nodes loaded at run time from dynamic
//! libraries (`.dylib` on macOS, `.so` on Linux, `.dll` on Windows) built
//! with the node SDK (`audioboard_node`, see its `abi` module).
//!
//! A plugin node becomes a module like any built-in one,
//! `ModuleKind::Ext(ExtId)`: the same `ModuleParams`, generic panel, MIDI
//! mapping, presets and project format (saved by its stable id instead of a
//! name). Its description is copied out of the library at load and kept for
//! the life of the process; libraries are never unloaded, since live nodes
//! run their code. A library that is refused (another ABI version, a broken
//! description) is reported and let go.
//!
//! Plugins are looked for in `<data dir>/Nodes`
//! (`~/Library/Application Support/AudioBoard/Nodes` on macOS) and in the
//! folders listed in `AUDIOBOARD_NODE_PATH`.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use audioboard_node::abi::{
    ABI_VERSION, ENTRY_SYMBOL, EntryFn, NodeVTable, PluginInfo, RawCtx, RawInit, RawNode,
    RawProcess, RawStr, display_from_raw, readout_from_raw,
};
use audioboard_node::def::{MAX_DATA, MAX_MODES, MAX_PARAMS, MAX_PORTS, MAX_VIZ};
use audioboard_node::{Descriptor, Panel, ParamSpec, SDK_VERSION, Taper, Unit};

use super::engine::{BLOCK, Buf, Inputs, MIDI_CAP, MidiBuf, ProcessCtx, Processor};
use super::modules::{ModuleDef, ModuleParams, silence};

/// A plugin node type: an index into the registry, stable while the app runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct ExtId(pub u16);

/// Most plugin node types loaded at once.
const MAX_EXT: usize = 512;

/// A plugin node type, as copied out of its library.
pub struct ExtNode {
    /// Stable id, saved in projects ("com.example.shimmer").
    pub id: &'static str,
    pub maker: &'static str,
    pub def: &'static ModuleDef,
    pub panel: Panel,
    vtable: NodeVTable,
    /// 0 fine; [`PANICKED`] or [`BAD_OUTPUT`] once a node of this type did.
    trouble: AtomicU8,
}

/// A node panicked: it stopped, and stays silent.
pub const PANICKED: u8 = 1;
/// A node output samples that were not finite (or absurdly loud): those
/// blocks were silenced.
pub const BAD_OUTPUT: u8 = 2;

impl ExtNode {
    /// What went wrong with nodes of this type, if anything (see
    /// [`PANICKED`], [`BAD_OUTPUT`]).
    pub fn trouble(&self) -> Option<&'static str> {
        match self.trouble.load(Ordering::Relaxed) {
            PANICKED => Some("Stopped: the plugin crashed"),
            BAD_OUTPUT => Some("Muted bad samples from the plugin"),
            _ => None,
        }
    }
}

/// Registered types, readable from any thread without a lock (the slots are
/// only ever filled, never changed).
static SLOTS: [OnceLock<ExtNode>; MAX_EXT] = [const { OnceLock::new() }; MAX_EXT];

/// A library that was loaded.
pub struct Library {
    pub path: PathBuf,
    pub name: String,
    pub maker: String,
    pub version: String,
    /// The SDK version it was built with.
    pub sdk: String,
    pub nodes: Vec<ExtId>,
    modified: Option<SystemTime>,
    /// Kept loaded for good (None: nodes linked into the app).
    _lib: Option<libloading::Library>,
}

#[derive(Default)]
struct Registry {
    libraries: Vec<Library>,
    by_id: HashMap<String, ExtId>,
    count: usize,
}

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The plugin node type `e`.
pub fn ext(e: ExtId) -> &'static ExtNode {
    SLOTS[e.0 as usize]
        .get()
        .expect("ExtIds are only made for registered types")
}

/// The type saved as `id`, if loaded.
pub fn find(id: &str) -> Option<ExtId> {
    registry().by_id.get(id).copied()
}

/// Every loaded plugin node type, in load order.
pub fn all() -> Vec<ExtId> {
    let n = registry().count;
    (0..n as u16).map(ExtId).collect()
}

/// Loaded types by maker (makers and names sorted), for menus.
pub fn by_maker() -> Vec<(&'static str, Vec<ExtId>)> {
    let mut groups: Vec<(&'static str, Vec<ExtId>)> = Vec::new();
    for e in all() {
        let maker = ext(e).maker;
        match groups.iter_mut().find(|(m, _)| *m == maker) {
            Some((_, list)) => list.push(e),
            None => groups.push((maker, vec![e])),
        }
    }
    groups.sort_by_key(|(m, _)| m.to_lowercase());
    for (_, list) in &mut groups {
        list.sort_by_key(|e| ext(*e).def.name.to_lowercase());
    }
    groups
}

/// (file name, library name and maker, version, node count) of each loaded
/// library.
pub fn libraries() -> Vec<(String, String, String, usize)> {
    registry()
        .libraries
        .iter()
        .map(|l| {
            let file = l
                .path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| l.name.clone());
            let name = if l.maker.is_empty() {
                l.name.clone()
            } else {
                format!("{} ({})", l.name, l.maker)
            };
            (
                file,
                name,
                format!("{} · SDK {}", l.version, l.sdk),
                l.nodes.len(),
            )
        })
        .collect()
}

/// The folder users install node plugins into.
pub fn folder() -> Option<PathBuf> {
    crate::project::data_dir().map(|d| d.join("Nodes"))
}

/// Where plugins are looked for: [`folder`], then `AUDIOBOARD_NODE_PATH`.
pub fn search_paths() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = folder().into_iter().collect();
    if let Some(extra) = std::env::var_os("AUDIOBOARD_NODE_PATH") {
        dirs.extend(std::env::split_paths(&extra).filter(|p| !p.as_os_str().is_empty()));
    }
    dirs
}

/// What a scan found.
#[derive(Default, Debug)]
pub struct Scan {
    /// Libraries loaded by this scan: (library name, node count).
    pub loaded: Vec<(String, usize)>,
    /// Files that were refused, and why.
    pub refused: Vec<String>,
    /// Loaded libraries whose file changed since: a restart loads them.
    pub changed: Vec<String>,
    /// Plugin node types available now.
    pub total: usize,
}

impl Scan {
    /// One line for a notice, and whether it reports a problem.
    pub fn summary(&self) -> (String, bool) {
        let mut parts = Vec::new();
        if !self.loaded.is_empty() {
            let nodes: usize = self.loaded.iter().map(|l| l.1).sum();
            parts.push(format!(
                "{} node plugin(s) loaded ({nodes} nodes)",
                self.loaded.len()
            ));
        }
        if !self.refused.is_empty() {
            parts.push(format!("refused: {}", self.refused.join("; ")));
        }
        if !self.changed.is_empty() {
            parts.push(format!(
                "restart to load the new {}",
                self.changed.join(", ")
            ));
        }
        (parts.join(" · "), !self.refused.is_empty())
    }
}

/// Loads the libraries in the search paths that are not loaded yet.
pub fn scan() -> Scan {
    scan_dirs(&search_paths())
}

pub fn scan_dirs(dirs: &[PathBuf]) -> Scan {
    let mut scan = Scan::default();
    let ext = std::env::consts::DLL_EXTENSION;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == ext))
            .collect();
        files.sort();
        for path in files {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            let modified = std::fs::metadata(&canonical)
                .and_then(|m| m.modified())
                .ok();
            let known = registry()
                .libraries
                .iter()
                .find(|l| l.path == canonical)
                .map(|l| l.modified);
            match known {
                Some(m) if m != modified => scan.changed.push(file_name(&path)),
                Some(_) => {}
                None => match load(&canonical) {
                    Ok((name, n)) => scan.loaded.push((name, n)),
                    Err(e) => scan.refused.push(format!("{}: {e}", file_name(&path))),
                },
            }
        }
    }
    scan.total = registry().count;
    scan
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Loads one library; returns its name and node count.
pub fn load(path: &Path) -> Result<(String, usize), String> {
    // SAFETY: loading a library runs its initializers; node plugins are
    // trusted code the user installed, like VST3s and Audio Units.
    let lib = unsafe { libloading::Library::new(path) }
        .map_err(|e| format!("could not be loaded ({e})"))?;
    // SAFETY: the symbol has this type in every ABI version.
    let entry: EntryFn = unsafe { lib.get::<EntryFn>(ENTRY_SYMBOL.as_bytes()) }
        .map(|s| *s)
        .map_err(|_| format!("not an AudioBoard node plugin (no {ENTRY_SYMBOL})"))?;
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    register(entry, path.to_path_buf(), modified, Some(lib))
}

/// Registers the nodes behind an entry point linked into the app (tests,
/// or built-in nodes written with the SDK): the same ABI, no library file.
#[cfg_attr(not(test), allow(dead_code))]
pub fn register_static(entry: EntryFn, name: &str) -> Result<Vec<ExtId>, String> {
    let path = PathBuf::from(format!("<linked>/{name}"));
    if let Some(lib) = registry().libraries.iter().find(|l| l.path == path) {
        return Ok(lib.nodes.clone());
    }
    register(entry, path.clone(), None, None)?;
    Ok(registry()
        .libraries
        .iter()
        .find(|l| l.path == path)
        .map(|l| l.nodes.clone())
        .unwrap_or_default())
}

fn register(
    entry: EntryFn,
    path: PathBuf,
    modified: Option<SystemTime>,
    lib: Option<libloading::Library>,
) -> Result<(String, usize), String> {
    // SAFETY: the entry point takes the host's ABI version and returns a
    // pointer to static data (or null).
    let info = unsafe { entry(ABI_VERSION) };
    if info.is_null() {
        return Err("it failed to describe its nodes".to_owned());
    }
    // SAFETY: the first field is a u32 ABI version in every version of the
    // ABI; nothing else is read before it matches.
    let abi = unsafe { *(info as *const u32) };
    if abi != ABI_VERSION {
        return Err(format!(
            "built for node ABI v{abi}, this AudioBoard loads v{ABI_VERSION}: rebuild it with audioboard-node {SDK_VERSION}"
        ));
    }
    // SAFETY: same ABI version: the layout is ours.
    let info: &PluginInfo = unsafe { &*info };
    if info.block as usize != BLOCK {
        return Err(format!(
            "built for {}-frame blocks, AudioBoard uses {BLOCK}",
            info.block
        ));
    }
    // SAFETY (all reads below): the plugin's tables, valid for good.
    let text = |s: &RawStr, what: &str| -> Result<String, String> {
        if s.len > 4096 {
            return Err(format!("its {what} is too long"));
        }
        unsafe { s.to_string() }.map_err(|e| format!("its {what} is {e}"))
    };
    let name = text(&info.name, "name")?;
    let maker = text(&info.maker, "maker")?;
    let version = text(&info.version, "version")?;
    let sdk = text(&info.sdk_version, "SDK version")?;
    if info.nodes.len > 256 {
        return Err("it offers too many nodes".to_owned());
    }
    let raws = unsafe { info.nodes.as_slice() };
    let mut nodes = Vec::new();
    for raw in raws {
        nodes.push(describe(raw).map_err(|e| format!("a node is not usable: {e}"))?);
    }
    // Ids are unique, within the library and among those loaded.
    let mut reg = registry();
    for (i, (d, _)) in nodes.iter().enumerate() {
        if reg.by_id.contains_key(d.id) || nodes[..i].iter().any(|(o, _)| o.id == d.id) {
            return Err(format!("its node id {} is already taken", d.id));
        }
    }
    if reg.count + nodes.len() > MAX_EXT {
        return Err("too many plugin nodes are loaded".to_owned());
    }
    let mut ids = Vec::new();
    for (d, vtable) in nodes {
        let e = ExtId(reg.count as u16);
        let node = ExtNode {
            id: d.id,
            maker: if d.maker.is_empty() {
                leak(maker.clone())
            } else {
                d.maker
            },
            def: Box::leak(Box::new(d.def)),
            panel: d.panel,
            vtable,
            trouble: AtomicU8::new(0),
        };
        if SLOTS[e.0 as usize].set(node).is_err() {
            return Err("plugin registry slot already taken".to_owned());
        }
        reg.by_id.insert(d.id.to_owned(), e);
        reg.count += 1;
        ids.push(e);
    }
    let count = ids.len();
    let label = if name.is_empty() {
        file_name(&path)
    } else {
        name.clone()
    };
    reg.libraries.push(Library {
        path,
        name,
        maker,
        version,
        sdk,
        nodes: ids,
        modified,
        _lib: lib,
    });
    Ok((label, count))
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// A node's description, copied out of the plugin and checked.
fn describe(raw: &RawNode) -> Result<(Descriptor, NodeVTable), String> {
    // SAFETY (all reads): the plugin's tables, valid for good; lengths are
    // checked before any slice is made.
    let s = |r: &RawStr| -> Result<&'static str, String> {
        if r.len > 1024 {
            return Err("a text is too long".to_owned());
        }
        unsafe { r.to_string() }.map(leak)
    };
    let id = s(&raw.id)?;
    let limit = |len: usize, max: usize, what: &str| {
        if len > max {
            Err(format!("{id}: more than {max} {what}"))
        } else {
            Ok(())
        }
    };
    limit(raw.inputs.len, MAX_PORTS, "inputs")?;
    limit(raw.outputs.len, MAX_PORTS, "outputs")?;
    limit(raw.params.len, MAX_PARAMS, "parameters")?;
    limit(raw.modes.len, MAX_MODES, "modes")?;
    limit(raw.data.len, MAX_DATA, "data values")?;
    limit(raw.cv_depth.len, MAX_PARAMS, "CV depths")?;
    limit(raw.viz as usize, MAX_VIZ, "display values")?;
    let list =
        |r: &audioboard_node::abi::RawSlice<RawStr>| -> Result<&'static [&'static str], String> {
            let v = unsafe { r.as_slice() }
                .iter()
                .map(&s)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Box::leak(v.into_boxed_slice()))
        };
    let mut params = Vec::new();
    for p in unsafe { raw.params.as_slice() } {
        let taper =
            Taper::from_code(p.taper).ok_or_else(|| format!("{id}: unknown taper {}", p.taper))?;
        params.push(ParamSpec {
            name: s(&p.name)?,
            min: p.min,
            max: p.max,
            default: p.default,
            taper,
            unit: Unit::from_code(p.unit),
            bipolar: p.bipolar,
        });
    }
    let cv: Vec<(usize, usize)> = unsafe { raw.cv_depth.as_slice() }
        .iter()
        .map(|&[p, i]| (p as usize, i as usize))
        .collect();
    let toggle = s(&raw.toggle)?;
    let name = s(&raw.name)?;
    let prefix = s(&raw.prefix)?;
    let def = ModuleDef {
        name,
        prefix: if prefix.is_empty() {
            leak(name.to_uppercase())
        } else {
            prefix
        },
        hint: s(&raw.hint)?,
        inputs: list(&raw.inputs)?,
        outputs: list(&raw.outputs)?,
        params: Box::leak(params.into_boxed_slice()),
        cv_depth: Box::leak(cv.into_boxed_slice()),
        modes: list(&raw.modes)?,
        toggle: (!toggle.is_empty()).then_some(toggle),
        toggle_default: raw.toggle_default,
        map: false,
        midi_in: raw.midi_in,
        midi_out: raw.midi_out,
        data: Box::leak(unsafe { raw.data.as_slice() }.to_vec().into_boxed_slice()),
        display_h: raw.display_h,
        controls_h: 0.0,
        viz: raw.viz as usize,
    };
    let d = Descriptor {
        id,
        maker: s(&raw.maker)?,
        def,
        panel: Panel {
            display: display_from_raw(raw.display),
            readout: readout_from_raw(raw.readout),
        },
    };
    if let Some(problem) = d.problem() {
        return Err(problem);
    }
    Ok((d, raw.vtable))
}

// ---------------------------------------------------------------------------
// Processing
// ---------------------------------------------------------------------------

/// Builds a plugin node's processor (UI thread). If the plugin fails to make
/// a node, the processor is silent and the type reports it.
pub fn processor(e: ExtId, params: Arc<ModuleParams>, sample_rate: f32) -> Box<dyn Processor> {
    let ext = ext(e);
    let init = RawInit {
        sample_rate,
        params: params.raw(),
    };
    // SAFETY: `init` is valid for the call; the params it points into live
    // in `params`, which the processor keeps until after `destroy`.
    let instance = unsafe { (ext.vtable.create)(&init) };
    if instance.is_null() {
        ext.trouble.store(PANICKED, Ordering::Relaxed);
    }
    Box::new(DynNode {
        ext,
        instance,
        params,
        midi: MidiBuf::default(),
        stopped: false,
    })
}

/// A plugin node on the audio thread: forwards each block through the
/// plugin's `process`, without allocating or locking.
struct DynNode {
    ext: &'static ExtNode,
    /// The plugin's node (null: it failed and stays silent).
    instance: *mut c_void,
    /// Kept alive until after the plugin's node is destroyed: it points
    /// into them.
    params: Arc<ModuleParams>,
    midi: MidiBuf,
    /// It panicked: silent from then on.
    stopped: bool,
}

// SAFETY: the instance is only used by the thread that owns the DynNode
// (the UI thread builds and frees it, the audio thread runs it in between),
// as the ABI requires.
unsafe impl Send for DynNode {}

impl Drop for DynNode {
    fn drop(&mut self) {
        if !self.instance.is_null() {
            // SAFETY: made by this type's `create`, freed once.
            unsafe { (self.ext.vtable.destroy)(self.instance) };
        }
    }
}

impl Processor for DynNode {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        self.midi.clear();
        if self.instance.is_null() || self.stopped {
            silence(outputs, n);
            return;
        }
        let def = self.ext.def;
        let n_out = def.outputs.len().min(outputs.len());
        let midi_in = inputs.midi.events();
        let midi_out_cap = if def.midi_out { MIDI_CAP as u32 } else { 0 };
        let mut sent: u32 = 0;
        let raw = RawProcess {
            ctx: RawCtx::from(ctx),
            frames: n.min(BLOCK) as u32,
            inputs: inputs.bufs.as_ptr(),
            connected: inputs.connected.as_ptr(),
            n_inputs: def.inputs.len().min(inputs.bufs.len()) as u32,
            outputs: outputs.as_mut_ptr(),
            n_outputs: n_out as u32,
            midi_in: midi_in.as_ptr(),
            n_midi_in: midi_in.len() as u32,
            midi_connected: inputs.midi_connected,
            midi_out: self.midi.room().as_mut_ptr(),
            midi_out_cap,
            n_midi_out: &mut sent,
        };
        // SAFETY: the instance is ours and every pointer in `raw` is valid
        // for the call.
        let status = unsafe { (self.ext.vtable.process)(self.instance, &raw) };
        if status != 0 {
            // It panicked: it is never run again, and freed with the node
            // (on the UI thread).
            self.ext.trouble.store(PANICKED, Ordering::Relaxed);
            self.stopped = true;
            silence(outputs, n);
            return;
        }
        self.midi.set_len(sent as usize);
        // Keep the mix safe from what a plugin outputs: NaN, infinities and
        // absurd levels silence the block.
        let mut peak = 0.0f32;
        let mut bad = false;
        for out in &outputs[..n_out] {
            for s in out[0][..n].iter().chain(&out[1][..n]) {
                let a = s.abs();
                bad |= !(a < 1.0e4);
                peak = peak.max(a);
            }
        }
        if bad {
            self.ext.trouble.store(BAD_OUTPUT, Ordering::Relaxed);
            silence(outputs, n);
            return;
        }
        self.params.meter.fetch_max(peak);
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        self.ext.def.midi_out.then_some(&self.midi)
    }
}

#[cfg(test)]
#[path = "node_plugin_tests.rs"]
mod tests;
