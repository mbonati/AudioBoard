//! Audio Units as plugins, on macOS: v3 app extensions (loaded out of
//! process, as Logic does, so a crash there can't take AudioBoard down) and
//! classic v2 components (in process), both through `AUAudioUnit`. The
//! Objective-C side is `au_host.m` (built by `build.rs`); this is its Rust
//! face, used by `plugin.rs` next to VST3.
//!
//! An Audio Unit has no file to point at: it is known by its component
//! description (type, subtype, maker), kept as a pseudo-path
//! (`audiounit:61756678-64656c79-6170706c`) wherever a plugin's path goes:
//! the palette, projects, library modules.

use std::path::{Path, PathBuf};

/// Pseudo-paths of Audio Units start with this.
pub const PREFIX: &str = "audiounit:";

/// `kAudioUnitType_MusicDevice`: instruments.
pub const INSTRUMENT: u32 = u32::from_be_bytes(*b"aumu");
/// `kAudioUnitType_Effect`.
pub const EFFECT: u32 = u32::from_be_bytes(*b"aufx");
/// `kAudioUnitType_MusicEffect`: effects that also take MIDI.
pub const MUSIC_EFFECT: u32 = u32::from_be_bytes(*b"aumf");

/// Which Audio Unit: its component description.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Component {
    pub kind: u32,
    pub subtype: u32,
    pub manufacturer: u32,
}

impl Component {
    /// Its pseudo-path.
    pub fn path(&self) -> PathBuf {
        PathBuf::from(format!("{PREFIX}{:08x}-{:08x}-{:08x}", self.kind, self.subtype, self.manufacturer))
    }

    /// The component a pseudo-path names.
    pub fn from_path(path: &Path) -> Option<Self> {
        let text = path.to_str()?.strip_prefix(PREFIX)?;
        let mut parts = text.split('-').map(|p| u32::from_str_radix(p, 16).ok());
        let c = Self { kind: parts.next()??, subtype: parts.next()??, manufacturer: parts.next()?? };
        parts.next().is_none().then_some(c)
    }

    /// The category, as VST3 plugins name theirs: "Instrument", "Fx".
    pub fn category(&self) -> &'static str {
        match self.kind {
            INSTRUMENT => "Instrument",
            MUSIC_EFFECT => "Fx|MIDI",
            EFFECT => "Fx",
            _ => "",
        }
    }
}

/// A plugin path that is an Audio Unit's pseudo-path.
pub fn is_au_path(path: &Path) -> bool {
    path.to_str().is_some_and(|p| p.starts_with(PREFIX))
}

/// An installed Audio Unit, as the system lists it (none of its code runs).
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub component: Component,
    pub name: String,
    pub vendor: String,
    /// An app extension (v3), not a component bundle (v2).
    pub v3: bool,
    pub midi: bool,
    /// Has a view of its own.
    pub view: bool,
}

/// What `au_host.m` lists, one object per Audio Unit.
#[derive(serde::Deserialize)]
struct ListedJson {
    name: String,
    vendor: String,
    #[serde(rename = "type")]
    kind: u32,
    subtype: u32,
    manufacturer: u32,
    v3: bool,
    midi: bool,
    view: bool,
}

/// The installed Audio Units' listing (`ab_au_list`) read.
pub fn parse_list(json: &str) -> Vec<Listed> {
    let items: Vec<ListedJson> = serde_json::from_str(json).unwrap_or_default();
    let mut list: Vec<Listed> = items
        .into_iter()
        .map(|j| Listed {
            component: Component { kind: j.kind, subtype: j.subtype, manufacturer: j.manufacturer },
            name: j.name,
            vendor: j.vendor,
            v3: j.v3,
            midi: j.midi,
            view: j.view,
        })
        .collect();
    list.sort_by_key(|l| l.name.to_lowercase());
    list.dedup_by_key(|l| l.component);
    list
}

/// A parameter as normalized 0..1 values map onto its range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f32,
    pub max: f32,
    /// Shown logarithmically (frequencies): knobs move evenly in octaves.
    pub log: bool,
}

impl Range {
    pub fn new(min: f32, max: f32, flags: u32) -> Self {
        const DISPLAY_LOGARITHMIC: u32 = 1 << 22;
        Self { min, max, log: flags & DISPLAY_LOGARITHMIC != 0 && min > 0.0 && max > min }
    }

    pub fn to_norm(&self, value: f32) -> f64 {
        if self.max <= self.min {
            return 0.0;
        }
        let value = value.clamp(self.min, self.max);
        let t = if self.log { (value / self.min).ln() / (self.max / self.min).ln() } else { (value - self.min) / (self.max - self.min) };
        t.clamp(0.0, 1.0) as f64
    }

    pub fn from_norm(&self, norm: f64) -> f32 {
        let t = norm.clamp(0.0, 1.0) as f32;
        if self.log { self.min * (self.max / self.min).powf(t) } else { self.min + (self.max - self.min) * t }
    }
}

/// Ids for parameters: their addresses, when those fit and stay distinct
/// in 32 bits (they almost always do), else their order.
pub fn param_ids(addresses: &[u64]) -> Vec<u32> {
    let mut seen = std::collections::HashSet::new();
    let fits = addresses.iter().all(|&a| a <= u32::MAX as u64 && seen.insert(a));
    if fits { addresses.iter().map(|&a| a as u32).collect() } else { (0..addresses.len() as u32).collect() }
}

#[cfg(target_os = "macos")]
pub use host::*;

#[cfg(target_os = "macos")]
mod host {
    use std::collections::HashMap;
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::ptr::NonNull;

    use super::{Component, Listed, Range, parse_list};

    #[repr(C)]
    struct Raw {
        _private: [u8; 0],
    }

    unsafe extern "C" {
        fn ab_free(p: *mut c_void);
        fn ab_au_list() -> *mut c_char;
        fn ab_au_open(kind: u32, subtype: u32, manufacturer: u32, sample_rate: f64, max_frames: u32, error: *mut *mut c_char) -> *mut Raw;
        fn ab_au_close(au: *mut Raw);
        fn ab_au_info(
            au: *mut Raw,
            in_channels: *mut u32,
            out_channels: *mut u32,
            midi: *mut c_int,
            v3: *mut c_int,
            name: *mut *mut c_char,
            vendor: *mut *mut c_char,
        );
        fn ab_au_set_transport(au: *mut Raw, tempo: f64, beat: f64, beats_per_sample: f64, sample_pos: f64, playing: c_int, changed: c_int);
        fn ab_au_midi(au: *mut Raw, offset: i64, bytes: *const u8, len: u32);
        fn ab_au_schedule_param(au: *mut Raw, index: u32, count: u32, value: f32);
        fn ab_au_render(
            au: *mut Raw,
            frames: u32,
            inputs: *const *const f32,
            in_count: u32,
            outputs: *const *mut f32,
            out_count: u32,
            sample_time: i64,
        ) -> c_int;
        fn ab_au_param_count(au: *mut Raw) -> u32;
        fn ab_au_param_handle(au: *mut Raw) -> *mut c_void;
        fn ab_au_handle_set(handle: *mut c_void, index: u32, value: f32);
        fn ab_au_handle_release(handle: *mut c_void);
        fn ab_au_param_info(
            au: *mut Raw,
            index: u32,
            address: *mut u64,
            min: *mut f32,
            max: *mut f32,
            value: *mut f32,
            flags: *mut u32,
            name: *mut *mut c_char,
        );
        fn ab_au_get_param(au: *mut Raw, index: u32) -> f32;
        fn ab_au_format_param(au: *mut Raw, index: u32, value: f32) -> *mut c_char;
        fn ab_au_save_state(au: *mut Raw, len: *mut usize) -> *mut u8;
        fn ab_au_load_state(au: *mut Raw, bytes: *const u8, len: usize) -> c_int;
        fn ab_au_open_editor(au: *mut Raw, title: *const c_char);
        fn ab_au_close_editor(au: *mut Raw);
        fn ab_au_editor_state(au: *mut Raw) -> c_int;
    }

    /// A string from the host, freed.
    fn take_string(p: *mut c_char) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: the host returns NUL-terminated UTF-8 it malloc'd.
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        unsafe { ab_free(p.cast()) };
        s
    }

    /// The installed effects and instruments (Apple's own included).
    pub fn list() -> Vec<Listed> {
        // SAFETY: returns a malloc'd JSON string.
        parse_list(&take_string(unsafe { ab_au_list() }))
    }

    /// A parameter of a loaded unit.
    #[derive(Clone, Debug)]
    pub struct AuParam {
        pub id: u32,
        pub name: String,
        pub range: Range,
        /// Normalized, as it was at load.
        pub default: f64,
    }

    /// Sets a unit's parameters from the UI thread, without its lock: the
    /// unit takes each value at once, rendering or not. Drop it before the
    /// unit (the plugin node does: the unit outlives its node).
    pub struct ParamHandle {
        raw: NonNull<c_void>,
        /// Index and range by id.
        params: HashMap<u32, (usize, Range)>,
    }

    // SAFETY: AUParameter may be set from any thread.
    unsafe impl Send for ParamHandle {}
    unsafe impl Sync for ParamHandle {}

    impl ParamHandle {
        /// Sets parameter `id` to `norm` (0..1).
        pub fn set(&self, id: u32, norm: f64) {
            if let Some(&(i, range)) = self.params.get(&id) {
                // SAFETY: a live, retained parameter array; `i` within it.
                unsafe { ab_au_handle_set(self.raw.as_ptr(), i as u32, range.from_norm(norm)) };
            }
        }
    }

    impl Drop for ParamHandle {
        fn drop(&mut self) {
            // SAFETY: releases the array retained for this handle.
            unsafe { ab_au_handle_release(self.raw.as_ptr()) };
        }
    }

    /// The editor window's state.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum EditorState {
        Closed,
        Opening,
        Open,
        /// The unit has no view (and the system no generic one).
        NoView,
    }

    /// A loaded Audio Unit. Not thread-safe by itself: `plugin.rs` keeps it
    /// in a mutex, so the UI never calls it during a render.
    pub struct AudioUnit {
        raw: NonNull<Raw>,
        pub name: String,
        pub vendor: String,
        pub v3: bool,
        pub midi: bool,
        pub inputs: usize,
        pub outputs: usize,
        pub params: Vec<AuParam>,
        /// Parameter index by id.
        index: HashMap<u32, usize>,
        /// Samples rendered.
        sample_time: i64,
    }

    // SAFETY: the unit is used from one thread at a time (behind a mutex);
    // AUAudioUnit may be called from any thread that way.
    unsafe impl Send for AudioUnit {}

    impl AudioUnit {
        /// Loads and prepares a unit for `sample_rate`, rendering up to
        /// `max_frames` per call. Runs plugin code: the UI thread only.
        pub fn open(component: Component, sample_rate: f32, max_frames: usize) -> Result<Self, String> {
            let mut error = std::ptr::null_mut();
            // SAFETY: plain values in, an owned handle or a malloc'd error out.
            let raw = unsafe {
                ab_au_open(component.kind, component.subtype, component.manufacturer, sample_rate as f64, max_frames as u32, &mut error)
            };
            let Some(raw) = NonNull::new(raw) else {
                let message = take_string(error);
                return Err(if message.is_empty() { "could not load the Audio Unit".to_owned() } else { message });
            };
            let (mut ins, mut outs, mut midi, mut v3) = (0u32, 0u32, 0, 0);
            let (mut name, mut vendor) = (std::ptr::null_mut(), std::ptr::null_mut());
            // SAFETY: `raw` is live; out-pointers are valid.
            unsafe { ab_au_info(raw.as_ptr(), &mut ins, &mut outs, &mut midi, &mut v3, &mut name, &mut vendor) };
            let count = unsafe { ab_au_param_count(raw.as_ptr()) };
            let mut addresses = Vec::with_capacity(count as usize);
            let mut infos = Vec::with_capacity(count as usize);
            for i in 0..count {
                let (mut address, mut min, mut max, mut value, mut flags) = (0u64, 0f32, 0f32, 0f32, 0u32);
                let mut pname = std::ptr::null_mut();
                // SAFETY: `i` < count; out-pointers are valid.
                unsafe { ab_au_param_info(raw.as_ptr(), i, &mut address, &mut min, &mut max, &mut value, &mut flags, &mut pname) };
                addresses.push(address);
                infos.push((take_string(pname), Range::new(min, max, flags), value));
            }
            let ids = super::param_ids(&addresses);
            let params: Vec<AuParam> = infos
                .into_iter()
                .zip(&ids)
                .map(|((name, range, value), &id)| AuParam { id, default: range.to_norm(value), name, range })
                .collect();
            let index = params.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
            Ok(Self {
                raw,
                name: take_string(name),
                vendor: take_string(vendor),
                v3: v3 != 0,
                midi: midi != 0,
                inputs: ins as usize,
                outputs: outs as usize,
                params,
                index,
                sample_time: 0,
            })
        }

        fn ptr(&self) -> *mut Raw {
            self.raw.as_ptr()
        }

        /// The host's clock for the next render (audio thread).
        pub fn set_transport(&mut self, tempo: f64, beat: f64, beats_per_sample: f64, playing: bool, changed: bool, restart: bool) {
            if restart {
                self.sample_time = 0;
            }
            // SAFETY: live handle; plain values.
            unsafe {
                ab_au_set_transport(self.ptr(), tempo, beat, beats_per_sample, self.sample_time as f64, playing as c_int, changed as c_int)
            };
        }

        /// A MIDI message `offset` frames into the next render (audio thread).
        pub fn send_midi(&mut self, msg: [u8; 3], offset: i64) {
            // Channel messages only: the keyboard's stream also carries the
            // clock and transport.
            if !self.midi || !(0x80..0xF0).contains(&msg[0]) {
                return;
            }
            let len = match msg[0] & 0xF0 {
                0xC0 | 0xD0 => 2,
                _ => 3,
            };
            // SAFETY: `msg` outlives the call; the unit copies the bytes.
            unsafe { ab_au_midi(self.ptr(), offset.max(0), msg.as_ptr(), len) };
        }

        /// A handle that sets parameters from the UI (see [`ParamHandle`]).
        pub fn param_handle(&self) -> Option<ParamHandle> {
            // SAFETY: returns the retained parameter array.
            let raw = NonNull::new(unsafe { ab_au_param_handle(self.ptr()) })?;
            let params = self.params.iter().enumerate().map(|(i, p)| (p.id, (i, p.range))).collect();
            Some(ParamHandle { raw, params })
        }

        /// Sets parameter `id` (normalized) for the next render (audio thread).
        pub fn set_parameter(&mut self, id: u32, norm: f64) {
            if let Some(&i) = self.index.get(&id) {
                let value = self.params[i].range.from_norm(norm);
                // SAFETY: `i` < the unit's parameter count.
                unsafe { ab_au_schedule_param(self.ptr(), i as u32, self.params.len() as u32, value) };
            }
        }

        /// Renders `buffers.block_size` frames (audio thread).
        pub fn process(&mut self, buffers: &mut vst3_host::AudioBuffers) -> bool {
            let n = buffers.block_size;
            let mut ins = [std::ptr::null::<f32>(); 8];
            let mut outs = [std::ptr::null_mut::<f32>(); 8];
            let in_count = buffers.inputs.len().min(8);
            let out_count = buffers.outputs.len().min(8);
            for (slot, ch) in ins.iter_mut().zip(&buffers.inputs) {
                *slot = ch.as_ptr();
            }
            for (slot, ch) in outs.iter_mut().zip(buffers.outputs.iter_mut()) {
                *slot = ch.as_mut_ptr();
            }
            if buffers.inputs.iter().chain(&buffers.outputs).any(|ch| ch.len() < n) {
                return false;
            }
            // SAFETY: every pointer is to a buffer of at least `n` floats
            // that lives through the call.
            let status = unsafe {
                ab_au_render(self.ptr(), n as u32, ins.as_ptr(), in_count as u32, outs.as_ptr(), out_count as u32, self.sample_time)
            };
            self.sample_time += n as i64;
            if status != 0 && cfg!(test) {
                eprintln!("{}: render error {status}", self.name);
            }
            status == 0
        }

        /// Parameter `id`'s value now, normalized (UI thread).
        pub fn get_parameter(&self, id: u32) -> Option<f64> {
            let &i = self.index.get(&id)?;
            // SAFETY: `i` < the unit's parameter count.
            Some(self.params[i].range.to_norm(unsafe { ab_au_get_param(self.ptr(), i as u32) }))
        }

        /// The unit's text for parameter `id` at `norm` (UI thread).
        pub fn format_parameter(&self, id: u32, norm: f64) -> Option<String> {
            let &i = self.index.get(&id)?;
            let value = self.params[i].range.from_norm(norm);
            // SAFETY: returns a malloc'd string.
            Some(take_string(unsafe { ab_au_format_param(self.ptr(), i as u32, value) }))
        }

        /// The complete state (a binary property list), for projects.
        pub fn save_state(&self) -> Option<Vec<u8>> {
            let mut len = 0usize;
            // SAFETY: returns `len` malloc'd bytes, or null.
            let p = unsafe { ab_au_save_state(self.ptr(), &mut len) };
            if p.is_null() {
                return None;
            }
            let bytes = unsafe { std::slice::from_raw_parts(p, len) }.to_vec();
            unsafe { ab_free(p.cast()) };
            Some(bytes)
        }

        pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
            // SAFETY: `data` lives through the call.
            match unsafe { ab_au_load_state(self.ptr(), data.as_ptr(), data.len()) } {
                0 => Ok(()),
                _ => Err("the Audio Unit did not take the saved state".to_owned()),
            }
        }

        /// Asks for the editor; its window opens when the view comes
        /// (see [`Self::editor_state`]). Main thread.
        pub fn open_editor(&mut self, title: &str) {
            let title = CString::new(title.replace('\0', "")).unwrap_or_default();
            // SAFETY: main thread; `title` lives through the call (copied).
            unsafe { ab_au_open_editor(self.ptr(), title.as_ptr()) };
        }

        pub fn close_editor(&mut self) {
            // SAFETY: main thread.
            unsafe { ab_au_close_editor(self.ptr()) };
        }

        pub fn editor_state(&self) -> EditorState {
            // SAFETY: reads an atomic.
            match unsafe { ab_au_editor_state(self.ptr()) } {
                1 => EditorState::Opening,
                2 => EditorState::Open,
                3 => EditorState::NoView,
                _ => EditorState::Closed,
            }
        }
    }

    impl Drop for AudioUnit {
        fn drop(&mut self) {
            // SAFETY: the handle is ours and used no more.
            unsafe { ab_au_close(self.ptr()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudo_paths_round_trip() {
        let c = Component { kind: EFFECT, subtype: u32::from_be_bytes(*b"dely"), manufacturer: u32::from_be_bytes(*b"appl") };
        let path = c.path();
        assert_eq!(path.to_str(), Some("audiounit:61756678-64656c79-6170706c"));
        assert!(is_au_path(&path));
        assert_eq!(Component::from_path(&path), Some(c));
        assert!(!is_au_path(Path::new("/Library/Audio/Plug-Ins/VST3/Dexed.vst3")));
        assert_eq!(Component::from_path(Path::new("audiounit:zz-1-2")), None);
        assert_eq!(Component::from_path(Path::new("audiounit:1-2-3-4")), None);
    }

    #[test]
    fn ranges_and_ids() {
        let lin = Range::new(-24.0, 24.0, 0);
        assert_eq!(lin.to_norm(0.0), 0.5);
        assert_eq!(lin.from_norm(0.25), -12.0);
        let log = Range::new(20.0, 20_000.0, 1 << 22);
        assert!(log.log);
        assert!((log.from_norm(0.5) - 632.46).abs() < 0.1, "{}", log.from_norm(0.5));
        assert!((log.to_norm(2_000.0) - 0.6667).abs() < 1e-3);
        assert!(!Range::new(0.0, 1.0, 1 << 22).log, "log needs a positive range");
        assert_eq!(param_ids(&[0, 7, 3]), vec![0, 7, 3]);
        assert_eq!(param_ids(&[1 << 40, 7]), vec![0, 1]);
        assert_eq!(param_ids(&[(1 << 32) + 7, 7]), vec![0, 1]);
    }

    /// Loads real Audio Units (Apple's, always installed) and plays them:
    /// `cargo test -- --ignored audio_units_load_and_play`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn audio_units_load_and_play() {
        let started = std::time::Instant::now();
        let list = list();
        eprintln!("{} Audio Units listed in {:?}", list.len(), started.elapsed());
        assert!(list.iter().any(|l| l.name == "AUDelay"), "Apple's AUDelay is listed");
        let code = |s: &[u8; 4]| u32::from_be_bytes(*s);
        let sr = 48_000.0;
        let frames = 512;

        // An effect: audio in, audio out.
        let delay = Component { kind: EFFECT, subtype: code(b"dely"), manufacturer: code(b"appl") };
        let mut fx = AudioUnit::open(delay, sr, 2048).expect("AUDelay loads");
        assert_eq!((fx.inputs, fx.outputs), (2, 2));
        assert!(!fx.params.is_empty());
        eprintln!("AUDelay params: {:?}", fx.params.iter().map(|p| &p.name).collect::<Vec<_>>());
        let mut b = vst3_host::AudioBuffers::new(2, 2, frames, sr as f64);
        let mut energy = 0.0f32;
        for k in 0..40 {
            for ch in b.inputs.iter_mut() {
                for (i, s) in ch.iter_mut().enumerate() {
                    *s = if k == 0 && i < 64 { 0.8 } else { 0.0 };
                }
            }
            fx.set_transport(120.0, 0.0, 120.0 / 60.0 / sr as f64, true, k == 0, false);
            assert!(fx.process(&mut b));
            energy += b.outputs.iter().flatten().map(|s| s * s).sum::<f32>();
        }
        assert!(energy > 0.01, "the delay passes sound ({energy})");
        // A parameter set on the render path reads back.
        let p = fx.params[0].clone();
        let handle = fx.param_handle().unwrap();
        handle.set(p.id, 0.25);
        assert!(fx.process(&mut b));
        let back = fx.get_parameter(p.id).unwrap();
        assert!((back - 0.25).abs() < 0.02, "{} reads {back}", p.name);
        eprintln!("{} at 25%: {}", p.name, fx.format_parameter(p.id, 0.25).unwrap());
        // State round trip.
        let state = fx.save_state().expect("a state");
        handle.set(p.id, 0.9);
        assert!((fx.get_parameter(p.id).unwrap() - 0.9).abs() < 0.02);
        assert!(fx.process(&mut b));
        fx.load_state(&state).unwrap();
        assert!((fx.get_parameter(p.id).unwrap() - 0.25).abs() < 0.02, "the state brings the value back");

        // An instrument: MIDI in, audio out.
        let dls = Component { kind: INSTRUMENT, subtype: code(b"dls "), manufacturer: code(b"appl") };
        let mut synth = AudioUnit::open(dls, sr, 2048).expect("DLSMusicDevice loads");
        assert!(synth.midi && synth.inputs == 0);
        let mut b = vst3_host::AudioBuffers::new(0, 2, frames, sr as f64);
        let mut energy = 0.0f32;
        for k in 0..60 {
            if k == 2 {
                synth.send_midi([0x90, 60, 110], 10);
            }
            if k == 40 {
                synth.send_midi([0x80, 60, 0], 0);
            }
            assert!(synth.process(&mut b));
            energy += b.outputs.iter().flatten().map(|s| s * s).sum::<f32>();
        }
        assert!(energy > 0.01, "a note sounds ({energy})");

        // Third-party ones that happen to be installed.
        for l in list.iter().filter(|l| l.component.manufacturer != code(b"appl")).take(3) {
            match AudioUnit::open(l.component, sr, 2048) {
                Ok(u) => eprintln!("{} ({}): {} in, {} out, {} params, midi {}", u.name, u.vendor, u.inputs, u.outputs, u.params.len(), u.midi),
                Err(e) => eprintln!("{}: {e}", l.name),
            }
        }
    }

    /// Audio Units as nodes, like VST3 plugins: an effect patched between a
    /// source and a mixer, a knob, the project saving and restoring it, and
    /// an instrument played from the MIDI keyboard. Apple's units are on
    /// every Mac.
    #[cfg(target_os = "macos")]
    #[test]
    fn audio_units_work_as_nodes() {
        use std::sync::Arc;
        use crate::audio::params::{EngineShared, Waveform, osc};
        use crate::audio::plugin::Format;
        use crate::graph::{Graph, NodeKind, PortRef};
        use egui::pos2;
        let (a, mut e) = crate::audio::offline(Arc::new(EngineShared::new()), 48_000);
        let mut g = Graph::default();
        let render = |e: &mut crate::audio::engine::Engine, frames: usize| {
            let mut buf = vec![0.0f32; frames * 2];
            for chunk in buf.chunks_mut(2 * 300) {
                e.render(chunk, 2);
            }
            (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
        };
        let code = |s: &[u8; 4]| u32::from_be_bytes(*s);
        let src = g.add_osc(pos2(0.0, 0.0), &a);
        if let Some(NodeKind::Osc(o)) = g.node(src).map(|n| &n.kind) {
            o.params.set_waveform(Waveform::Saw);
            o.params.set(osc::FREQ, 220.0);
            o.params.set(osc::LEVEL, 0.5);
        }
        let delay = Component { kind: EFFECT, subtype: code(b"dely"), manufacturer: code(b"appl") }.path();
        let fx = g.add_plugin(&delay, pos2(300.0, 0.0), &a).expect("AUDelay loads as a node");
        let mix = g.add_mixer(pos2(600.0, 0.0), &a);
        g.connect(PortRef { node: src, port: 0 }, PortRef { node: fx, port: 0 }, &a);
        g.connect(PortRef { node: fx, port: 0 }, PortRef { node: mix, port: 0 }, &a);
        let node = g.node(fx).unwrap();
        assert_eq!(node.title, "AUDELAY 1");
        assert_eq!(node.inputs(), &["IN"]);
        let NodeKind::Plugin(p) = &node.kind else { unreachable!() };
        assert_eq!(p.meta.format, Format::AudioUnit { v3: false });
        assert!(!p.meta.instrument && p.meta.editor);
        render(&mut e, 4800);
        assert!(render(&mut e, 24_000) > 0.01, "sound comes through the Audio Unit");

        // A knob reaches the unit at once (no audio thread involved).
        let mix_index = p.params.iter().position(|q| q.name == "Dry/Wet Mix").expect("a mix parameter");
        let id = p.params[mix_index].id;
        if let Some(NodeKind::Plugin(p)) = g.node_mut(fx).map(|n| &mut n.kind) {
            p.set_param(mix_index, 0.25, 0.0);
        }
        let value = |g: &Graph| match &g.node(fx).unwrap().kind {
            NodeKind::Plugin(p) => p.shared.plugin.lock().unwrap().get_parameter(id).unwrap(),
            _ => unreachable!(),
        };
        assert!((value(&g) - 0.25).abs() < 0.01, "{}", value(&g));

        // Saved by its component code with its state, and restored.
        let saved = crate::project::snapshot(&g);
        let json = serde_json::to_string(&saved).unwrap();
        assert!(json.contains("audiounit:61756678-64656c79-6170706c"), "saved by component");
        g.clear(&a);
        assert_eq!(crate::project::restore(&mut g, &saved, &a), 0);
        let fx = g.nodes.iter().find(|n| matches!(n.kind, NodeKind::Plugin(_))).expect("restored").id;
        let restored = match &g.node(fx).unwrap().kind {
            NodeKind::Plugin(p) => p.shared.plugin.lock().unwrap().get_parameter(id).unwrap(),
            _ => unreachable!(),
        };
        assert!((restored - 0.25).abs() < 0.01, "the state came back: {restored}");

        // An instrument plays from the MIDI keyboard.
        let dls = Component { kind: INSTRUMENT, subtype: code(b"dls "), manufacturer: code(b"appl") }.path();
        let synth = g.add_plugin(&dls, pos2(300.0, 300.0), &a).expect("DLSMusicDevice loads as a node");
        let mix2 = g.add_mixer(pos2(600.0, 300.0), &a);
        g.connect(PortRef { node: synth, port: 0 }, PortRef { node: mix2, port: 0 }, &a);
        if let Some(NodeKind::Plugin(p)) = g.node(synth).map(|n| &n.kind) {
            assert!(p.meta.instrument && p.meta.midi && p.shared.midi_in.load(std::sync::atomic::Ordering::Relaxed));
        }
        g.remove_node(fx, &a);
        render(&mut e, 4800);
        a.midi.handle(&[0x90, 64, 110]);
        assert!(render(&mut e, 12_000) > 0.005, "the note sounds");
        a.midi.handle(&[0x80, 64, 0]);
    }

    #[test]
    fn listing_is_read() {
        let json = r#"[{"name":"AUDelay","vendor":"Apple","type":1635083896,"subtype":1684368505,"manufacturer":1634758764,"v3":false,"midi":false,"view":true},
                       {"name":"Dexed","vendor":"Digital Suburban","type":1635085685,"subtype":1,"manufacturer":2,"v3":true,"midi":true,"view":true}]"#;
        let list = parse_list(json);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "AUDelay");
        assert_eq!(list[0].component.category(), "Fx");
        assert_eq!(list[1].component.category(), "Instrument");
        assert!(list[1].v3 && list[1].midi);
        assert!(parse_list("not json").is_empty());
    }
}
