//! The C ABI between AudioBoard and a node plugin (a `cdylib`).
//!
//! Rust has no stable ABI, so a plugin and the app, possibly built by
//! different compilers and SDK versions, only share `#[repr(C)]` data and
//! `extern "C"` functions. Nothing here is generic or owns Rust types.
//!
//! A plugin exports one symbol, [`ENTRY_SYMBOL`] (`audioboard_node_entry`),
//! an [`EntryFn`]. The host calls it once with its own [`ABI_VERSION`]; it
//! returns a pointer to a [`PluginInfo`] that stays valid until the process
//! ends (the host never unloads a plugin). **The first field of
//! `PluginInfo`, `abi_version: u32`, is frozen across every version**: the
//! host reads it first and refuses a plugin built for another version
//! before touching anything else.
//!
//! Each [`RawNode`] carries a description (copied by the host at load) and
//! a [`NodeVTable`]: `create` (UI thread) returns an opaque instance,
//! `process` (audio thread) renders one block of at most [`BLOCK`] frames,
//! `destroy` (UI thread) frees it. The host never calls `process` on one
//! instance from two threads at once, and never after `destroy`.
//!
//! Plugins don't write this by hand: [`export_nodes!`](crate::export_nodes)
//! generates the entry point from [`Node`] types.
//!
//! Version history:
//! - 1: first version.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32};

use crate::buffer::{BLOCK, Buf, MidiMsg, ProcessCtx};
use crate::def::{Descriptor, Display, Readout};
use crate::node::{Init, Io, Node, Shared};
use crate::param::AtomicF32;

/// The ABI this SDK speaks. Bumped on any change to the structs or
/// functions below; the host refuses plugins built for another one.
pub const ABI_VERSION: u32 = 1;

/// The symbol a plugin exports (an [`EntryFn`]).
pub const ENTRY_SYMBOL: &str = "audioboard_node_entry";

/// `audioboard_node_entry(host_abi_version) -> *const PluginInfo`. Null if
/// the plugin failed to describe itself.
pub type EntryFn = unsafe extern "C" fn(host_abi: u32) -> *const PluginInfo;

/// UTF-8 text owned by the plugin (not NUL-terminated).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl RawStr {
    pub const EMPTY: RawStr = RawStr { ptr: std::ptr::null(), len: 0 };

    pub const fn new(s: &'static str) -> Self {
        Self { ptr: s.as_ptr(), len: s.len() }
    }

    /// The text, copied; an error for invalid UTF-8.
    ///
    /// # Safety
    /// `ptr` must point to `len` readable bytes (or be null with `len` 0).
    pub unsafe fn to_string(&self) -> Result<String, String> {
        if self.ptr.is_null() || self.len == 0 {
            return Ok(String::new());
        }
        // SAFETY: see the function's contract.
        let bytes = unsafe { std::slice::from_raw_parts(self.ptr, self.len) };
        std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| "text that is not UTF-8".to_owned())
    }
}

/// An array owned by the plugin.
#[repr(C)]
pub struct RawSlice<T> {
    pub ptr: *const T,
    pub len: usize,
}

impl<T> Clone for RawSlice<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for RawSlice<T> {}

impl<T> RawSlice<T> {
    pub const EMPTY: RawSlice<T> = RawSlice { ptr: std::ptr::null(), len: 0 };

    fn of(items: &'static [T]) -> Self {
        Self { ptr: items.as_ptr(), len: items.len() }
    }

    /// # Safety
    /// `ptr` must point to `len` valid `T`s (or be null with `len` 0), alive
    /// for `'a`.
    pub unsafe fn as_slice<'a>(&self) -> &'a [T] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            // SAFETY: see the function's contract.
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

/// What a plugin library is and the nodes it offers.
#[repr(C)]
pub struct PluginInfo {
    /// Frozen at offset 0 in every version: the ABI this plugin speaks.
    pub abi_version: u32,
    /// Frames per block the plugin was built for (must equal [`BLOCK`]).
    pub block: u32,
    /// The `audioboard-node` version it was built with ("0.1.0").
    pub sdk_version: RawStr,
    /// The library's name, maker and version, for the UI.
    pub name: RawStr,
    pub maker: RawStr,
    pub version: RawStr,
    pub nodes: RawSlice<RawNode>,
}

/// One node type: its description and its functions.
#[repr(C)]
pub struct RawNode {
    pub id: RawStr,
    pub maker: RawStr,
    pub name: RawStr,
    pub prefix: RawStr,
    pub hint: RawStr,
    pub inputs: RawSlice<RawStr>,
    pub outputs: RawSlice<RawStr>,
    pub params: RawSlice<RawParam>,
    /// `[param, input]` pairs.
    pub cv_depth: RawSlice<[u32; 2]>,
    pub modes: RawSlice<RawStr>,
    /// Empty: no switch.
    pub toggle: RawStr,
    pub toggle_default: bool,
    pub midi_in: bool,
    pub midi_out: bool,
    pub data: RawSlice<f32>,
    pub display_h: f32,
    pub viz: u32,
    pub display: RawDisplay,
    pub readout: RawReadout,
    pub vtable: NodeVTable,
}

#[repr(C)]
pub struct RawParam {
    pub name: RawStr,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// [`crate::Taper::code`].
    pub taper: u32,
    /// [`crate::Unit::code`].
    pub unit: u32,
    pub bipolar: bool,
}

/// [`Display`] as plain numbers: `kind` 0 none, 1 level, 2 bars (`a` from,
/// `b` count), 3 curve (same), 4 trace (`a` index), 5 dot (`a` x, `b` y).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawDisplay {
    pub kind: u32,
    pub a: u32,
    pub b: u32,
}

/// [`Readout`] as plain numbers: `kind` 0 name, 1 param (`index`), 2 viz
/// (`index`, `unit` code), 3 mode.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawReadout {
    pub kind: u32,
    pub index: u32,
    pub unit: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NodeVTable {
    /// A new instance (UI thread), or null on failure.
    pub create: unsafe extern "C" fn(init: *const RawInit) -> *mut c_void,
    /// Renders one block (audio thread). Returns 0, or non-zero if the node
    /// failed (it panicked): the host then stops calling it.
    pub process: unsafe extern "C" fn(node: *mut c_void, io: *const RawProcess) -> u32,
    /// Frees an instance (UI thread).
    pub destroy: unsafe extern "C" fn(node: *mut c_void),
}

/// The node's settings: pointers into the host's [`crate::Params`], valid
/// until the instance is destroyed.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawParams {
    pub values: *const AtomicF32,
    pub n_values: u32,
    pub mode: *const AtomicU32,
    pub toggle: *const AtomicBool,
    pub data: *const AtomicF32,
    pub n_data: u32,
    pub viz: *const AtomicF32,
    pub n_viz: u32,
}

#[repr(C)]
pub struct RawInit {
    pub sample_rate: f32,
    pub params: RawParams,
}

/// [`ProcessCtx`], as the ABI fixes it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawCtx {
    pub sample_rate: f32,
    pub smooth: f32,
    pub beat: f64,
    pub beats_per_sample: f64,
    pub playing: bool,
    pub restart: bool,
}

impl From<&ProcessCtx> for RawCtx {
    fn from(c: &ProcessCtx) -> Self {
        Self {
            sample_rate: c.sample_rate,
            smooth: c.smooth,
            beat: c.beat,
            beats_per_sample: c.beats_per_sample,
            playing: c.playing,
            restart: c.restart,
        }
    }
}

impl From<RawCtx> for ProcessCtx {
    fn from(c: RawCtx) -> Self {
        Self {
            sample_rate: c.sample_rate,
            smooth: c.smooth,
            beat: c.beat,
            beats_per_sample: c.beats_per_sample,
            playing: c.playing,
            restart: c.restart,
        }
    }
}

/// One block's buffers, valid for the duration of the `process` call.
#[repr(C)]
pub struct RawProcess {
    pub ctx: RawCtx,
    /// Frames to render, at most [`BLOCK`].
    pub frames: u32,
    /// `n_inputs` blocks; `connected[i]` false: input `i` is unpatched.
    pub inputs: *const Buf,
    pub connected: *const bool,
    pub n_inputs: u32,
    pub outputs: *mut Buf,
    pub n_outputs: u32,
    pub midi_in: *const MidiMsg,
    pub n_midi_in: u32,
    pub midi_connected: bool,
    /// Room for `midi_out_cap` events; the plugin writes how many it sent
    /// to `n_midi_out`.
    pub midi_out: *mut MidiMsg,
    pub midi_out_cap: u32,
    pub n_midi_out: *mut u32,
}

// ---------------------------------------------------------------------------
// Plugin side: turning `Node` types into the tables above
// ---------------------------------------------------------------------------

/// A node type ready to export: its description and functions. Made by
/// [`entry`], gathered by [`export_nodes!`](crate::export_nodes).
pub struct NodeEntry {
    pub descriptor: Descriptor,
    pub vtable: NodeVTable,
}

/// The entry for node type `N`.
pub fn entry<N: Node>() -> NodeEntry {
    NodeEntry {
        descriptor: N::DESCRIPTOR,
        vtable: NodeVTable { create: create::<N>, process: process::<N>, destroy: destroy::<N> },
    }
}

unsafe extern "C" fn create<N: Node>(init: *const RawInit) -> *mut c_void {
    if init.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the host passes a valid RawInit whose params outlive the node.
    let (sample_rate, params) = unsafe { ((*init).sample_rate, Shared::from_raw((*init).params)) };
    catch_unwind(AssertUnwindSafe(|| Box::into_raw(Box::new(N::new(&Init { sample_rate, params }))) as *mut c_void))
        .unwrap_or(std::ptr::null_mut())
}

unsafe extern "C" fn process<N: Node>(node: *mut c_void, io: *const RawProcess) -> u32 {
    if node.is_null() || io.is_null() {
        return 1;
    }
    // SAFETY: `node` came from `create::<N>`, and only this call uses it
    // now; `io` is valid for the call.
    let (node, raw) = unsafe { (&mut *(node as *mut N), &*io) };
    let ctx = ProcessCtx::from(raw.ctx);
    // SAFETY: see above.
    let mut io = unsafe { Io::from_raw(raw) };
    let result = catch_unwind(AssertUnwindSafe(|| node.process(&ctx, &mut io)));
    if !raw.n_midi_out.is_null() {
        // SAFETY: valid for the call.
        unsafe { *raw.n_midi_out = io.midi_sent().len() as u32 };
    }
    u32::from(result.is_err())
}

unsafe extern "C" fn destroy<N: Node>(node: *mut c_void) {
    if !node.is_null() {
        // SAFETY: `node` came from `create::<N>` and is freed once.
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(node as *mut N) })));
    }
}

/// The tables for one library, built once and kept until the process ends.
pub struct Built {
    info: PluginInfo,
    _nodes: Box<[RawNode]>,
    _strs: Vec<Box<[RawStr]>>,
    _params: Vec<Box<[RawParam]>>,
    _cv: Vec<Box<[[u32; 2]]>>,
}

// SAFETY: everything is immutable after it is built, and every pointer is to
// 'static data or to boxes owned alongside.
unsafe impl Send for Built {}
unsafe impl Sync for Built {}

impl Built {
    pub fn info(&self) -> &PluginInfo {
        &self.info
    }
}

/// What [`export_nodes!`](crate::export_nodes) expands to: builds the
/// tables on the first call (kept in `cell`) and returns them.
pub fn build(
    cell: &'static OnceLock<Option<Built>>,
    name: &'static str,
    maker: &'static str,
    version: &'static str,
    nodes: fn() -> Vec<NodeEntry>,
) -> *const PluginInfo {
    let built = cell.get_or_init(|| catch_unwind(AssertUnwindSafe(|| build_tables(name, maker, version, nodes()))).ok());
    built.as_ref().map_or(std::ptr::null(), |b| &b.info as *const PluginInfo)
}

fn build_tables(name: &'static str, maker: &'static str, version: &'static str, entries: Vec<NodeEntry>) -> Built {
    let mut strs: Vec<Box<[RawStr]>> = Vec::new();
    let mut params: Vec<Box<[RawParam]>> = Vec::new();
    let mut cvs: Vec<Box<[[u32; 2]]>> = Vec::new();
    let mut list = |items: &'static [&'static str]| -> RawSlice<RawStr> {
        let b: Box<[RawStr]> = items.iter().map(|s| RawStr::new(s)).collect();
        let slice = RawSlice { ptr: b.as_ptr(), len: b.len() };
        strs.push(b);
        slice
    };
    let nodes: Box<[RawNode]> = entries
        .into_iter()
        .map(|e| {
            let d = e.descriptor.def;
            let ps: Box<[RawParam]> = d
                .params
                .iter()
                .map(|p| RawParam {
                    name: RawStr::new(p.name),
                    min: p.min,
                    max: p.max,
                    default: p.default,
                    taper: p.taper.code(),
                    unit: p.unit.code(),
                    bipolar: p.bipolar,
                })
                .collect();
            let cv: Box<[[u32; 2]]> = d.cv_depth.iter().map(|&(p, i)| [p as u32, i as u32]).collect();
            let node = RawNode {
                id: RawStr::new(e.descriptor.id),
                maker: RawStr::new(e.descriptor.maker),
                name: RawStr::new(d.name),
                prefix: RawStr::new(d.prefix),
                hint: RawStr::new(d.hint),
                inputs: list(d.inputs),
                outputs: list(d.outputs),
                params: RawSlice { ptr: ps.as_ptr(), len: ps.len() },
                cv_depth: RawSlice { ptr: cv.as_ptr(), len: cv.len() },
                modes: list(d.modes),
                toggle: d.toggle.map_or(RawStr::EMPTY, RawStr::new),
                toggle_default: d.toggle_default,
                midi_in: d.midi_in,
                midi_out: d.midi_out,
                data: RawSlice::of(d.data),
                display_h: d.display_h,
                viz: d.viz as u32,
                display: raw_display(e.descriptor.panel.display),
                readout: raw_readout(e.descriptor.panel.readout),
                vtable: e.vtable,
            };
            params.push(ps);
            cvs.push(cv);
            node
        })
        .collect();
    Built {
        info: PluginInfo {
            abi_version: ABI_VERSION,
            block: BLOCK as u32,
            sdk_version: RawStr::new(crate::SDK_VERSION),
            name: RawStr::new(name),
            maker: RawStr::new(maker),
            version: RawStr::new(version),
            nodes: RawSlice { ptr: nodes.as_ptr(), len: nodes.len() },
        },
        _nodes: nodes,
        _strs: strs,
        _params: params,
        _cv: cvs,
    }
}

pub fn raw_display(d: Display) -> RawDisplay {
    let (kind, a, b) = match d {
        Display::None => (0, 0, 0),
        Display::Level => (1, 0, 0),
        Display::Bars { from, count } => (2, from, count),
        Display::Curve { from, count } => (3, from, count),
        Display::Trace { index } => (4, index, 0),
        Display::Dot { x, y } => (5, x, y),
    };
    RawDisplay { kind, a: a as u32, b: b as u32 }
}

/// The display for its numbers; an unknown kind (from a newer SDK) shows
/// nothing.
pub fn display_from_raw(r: RawDisplay) -> Display {
    let (a, b) = (r.a as usize, r.b as usize);
    match r.kind {
        1 => Display::Level,
        2 => Display::Bars { from: a, count: b },
        3 => Display::Curve { from: a, count: b },
        4 => Display::Trace { index: a },
        5 => Display::Dot { x: a, y: b },
        _ => Display::None,
    }
}

pub fn raw_readout(r: Readout) -> RawReadout {
    match r {
        Readout::Name => RawReadout { kind: 0, index: 0, unit: 0 },
        Readout::Param(i) => RawReadout { kind: 1, index: i as u32, unit: 0 },
        Readout::Viz { index, unit } => RawReadout { kind: 2, index: index as u32, unit: unit.code() },
        Readout::Mode => RawReadout { kind: 3, index: 0, unit: 0 },
    }
}

pub fn readout_from_raw(r: RawReadout) -> Readout {
    match r.kind {
        1 => Readout::Param(r.index as usize),
        2 => Readout::Viz { index: r.index as usize, unit: crate::Unit::from_code(r.unit) },
        3 => Readout::Mode,
        _ => Readout::Name,
    }
}

/// Exports node types from a `cdylib` as an AudioBoard node plugin.
///
/// ```ignore
/// audioboard_node::export_nodes! {
///     name: "Example nodes",
///     maker: "Example",
///     version: env!("CARGO_PKG_VERSION"),
///     nodes: [Shimmer, Pulse],
/// }
/// ```
///
/// It defines the `audioboard_node_entry` symbol (see [`crate::abi`]).
#[macro_export]
macro_rules! export_nodes {
    (name: $name:expr, maker: $maker:expr, version: $version:expr, nodes: [$($node:ty),* $(,)?] $(,)?) => {
        /// AudioBoard node plugin entry point (see `audioboard_node::abi`).
        #[unsafe(no_mangle)]
        pub extern "C" fn audioboard_node_entry(_host_abi: u32) -> *const $crate::abi::PluginInfo {
            static BUILT: ::std::sync::OnceLock<::std::option::Option<$crate::abi::Built>> = ::std::sync::OnceLock::new();
            fn entries() -> ::std::vec::Vec<$crate::abi::NodeEntry> {
                ::std::vec![$($crate::abi::entry::<$node>()),*]
            }
            $crate::abi::build(&BUILT, $name, $maker, $version, entries)
        }
    };
}

/// Like [`export_nodes!`], but as a plain function `$fname` (no exported
/// symbol): for nodes linked into the host, or tests, that go through the
/// same ABI without a dynamic library.
#[macro_export]
macro_rules! static_nodes {
    ($vis:vis fn $fname:ident, name: $name:expr, maker: $maker:expr, version: $version:expr, nodes: [$($node:ty),* $(,)?] $(,)?) => {
        $vis extern "C" fn $fname(_host_abi: u32) -> *const $crate::abi::PluginInfo {
            static BUILT: ::std::sync::OnceLock<::std::option::Option<$crate::abi::Built>> = ::std::sync::OnceLock::new();
            fn entries() -> ::std::vec::Vec<$crate::abi::NodeEntry> {
                ::std::vec![$($crate::abi::entry::<$node>()),*]
            }
            $crate::abi::build(&BUILT, $name, $maker, $version, entries)
        }
    };
}
