//! VST3 plugins as nodes. A plugin is shared between the UI (its editor
//! window, parameter display, saved state) and the audio thread, which
//! renders it. Knob moves reach it through a lock-free channel, and
//! instruments can take the external MIDI stream (keyboards) directly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use vst3_host::{AudioBuffers, MidiChannel, MidiEvent, Plugin, Vst3Host};

use super::engine::{BLOCK, Buf, Inputs, ProcessCtx, Processor};
use super::midi::MidiState;
use super::params::AtomicF32;

/// What the UI asks of a plugin, applied on the audio thread.
pub enum Control {
    /// Normalized (0..1) value for a parameter id.
    Param(u32, f64),
}

/// The part of a plugin node both threads see.
pub struct PluginShared {
    pub plugin: Arc<Mutex<Plugin>>,
    /// Output peak since the UI last looked.
    pub meter: AtomicF32,
    /// Feed the external MIDI stream into the plugin.
    pub midi_in: AtomicBool,
    /// Blocks left silent because the UI held the plugin too long.
    pub missed: AtomicU32,
}

/// What we know about a loaded plugin.
#[derive(Clone, Debug)]
pub struct PluginMeta {
    pub path: PathBuf,
    pub name: String,
    pub vendor: String,
    pub category: String,
    /// Plays notes rather than processing audio.
    pub instrument: bool,
    /// Stereo audio in (effects) or none (instruments).
    pub inputs: usize,
    pub outputs: usize,
    /// Has its own editor window.
    pub editor: bool,
}

/// A parameter the user can set.
#[derive(Clone, Debug)]
pub struct PluginParam {
    pub id: u32,
    pub name: String,
    pub default: f64,
}

pub struct Loaded {
    pub plugin: Plugin,
    pub meta: PluginMeta,
    pub params: Vec<PluginParam>,
}

/// One host for the whole app, set to the engine's rate and block size.
pub fn new_host(sample_rate: f32) -> Result<Vst3Host, String> {
    Vst3Host::builder()
        .sample_rate(sample_rate as f64)
        .block_size(BLOCK)
        .with_process_isolation(false)
        .build()
        .map_err(|e| e.to_string())
}

/// Loads and starts a plugin bundle. Runs plugin code: UI thread only.
pub fn load(host: &mut Vst3Host, path: &Path) -> Result<Loaded, String> {
    let mut plugin = host.load_plugin(path).map_err(|e| e.to_string())?;
    let info = plugin.info().clone();
    let params = plugin
        .get_parameters()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|p| !p.is_read_only && !p.is_bypass)
        .map(|p| PluginParam { id: p.id, name: p.name, default: p.default })
        .collect();
    plugin.start_processing().map_err(|e| e.to_string())?;
    let instrument = info.category.contains("Instrument") || (info.has_midi_input && info.audio_inputs == 0);
    let meta = PluginMeta {
        path: path.to_path_buf(),
        name: info.name.clone(),
        vendor: info.vendor.clone(),
        category: info.category.clone(),
        instrument,
        inputs: if info.audio_inputs > 0 { 2 } else { 0 },
        outputs: plugin.output_channel_count().max(1),
        editor: plugin.has_editor(),
    };
    Ok(Loaded { plugin, meta, params })
}

/// Folders where VST3 plugins are installed.
pub fn plugin_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        dirs.extend(home.map(|h| h.join("Library/Audio/Plug-Ins/VST3")));
    } else if cfg!(target_os = "windows") {
        dirs.push(PathBuf::from(r"C:\Program Files\Common Files\VST3"));
    } else {
        dirs.push(PathBuf::from("/usr/lib/vst3"));
        dirs.push(PathBuf::from("/usr/local/lib/vst3"));
        dirs.extend(home.map(|h| h.join(".vst3")));
    }
    dirs
}

/// Plugin bundles installed, sorted by name. Only lists files: no plugin
/// code runs.
pub fn scan() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = plugin_dirs()
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vst3")))
        .collect();
    found.sort_by_key(|p| bundle_name(p).to_lowercase());
    found
}

/// "ValhallaFreqEcho" for ".../ValhallaFreqEcho.vst3".
pub fn bundle_name(path: &Path) -> String {
    path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// A raw MIDI message as a plugin event.
fn midi_event([status, d1, d2]: [u8; 3]) -> Option<MidiEvent> {
    let channel = MidiChannel::from_index(status & 0x0F)?;
    Some(match status & 0xF0 {
        0x90 if d2 > 0 => MidiEvent::NoteOn { channel, note: d1, velocity: d2 },
        0x80 | 0x90 => MidiEvent::NoteOff { channel, note: d1, velocity: d2 },
        0xA0 => MidiEvent::PolyAftertouch { channel, note: d1, pressure: d2 },
        0xB0 => MidiEvent::ControlChange { channel, controller: d1, value: d2 },
        0xC0 => MidiEvent::ProgramChange { channel, program: d1 },
        0xD0 => MidiEvent::ChannelAftertouch { channel, pressure: d1 },
        0xE0 => MidiEvent::PitchBend { channel, value: (d2 as u16) << 7 | d1 as u16 },
        _ => return None,
    })
}

/// How long the audio thread waits for the UI to let go of a plugin.
const LOCK_WAIT: Duration = Duration::from_micros(200);

pub struct PluginProcessor {
    shared: Arc<PluginShared>,
    control: Receiver<Control>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    buffers: AudioBuffers,
}

impl PluginProcessor {
    pub fn new(shared: Arc<PluginShared>, control: Receiver<Control>, midi: Arc<MidiState>, meta: &PluginMeta, sample_rate: f32) -> Self {
        Self {
            midi_cursor: midi.stream_head(),
            buffers: AudioBuffers::new(meta.inputs, meta.outputs, BLOCK, sample_rate as f64),
            shared,
            control,
            midi,
        }
    }
}

impl Processor for PluginProcessor {
    fn process(&mut self, _ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let [out_l, out_r] = &mut outputs[0];
        // The UI holds the plugin only briefly; never wait long for it.
        let started = Instant::now();
        let guard = loop {
            match self.shared.plugin.try_lock() {
                Ok(g) => break Some(g),
                Err(TryLockError::Poisoned(p)) => break Some(p.into_inner()),
                Err(TryLockError::WouldBlock) if started.elapsed() < LOCK_WAIT => std::hint::spin_loop(),
                Err(TryLockError::WouldBlock) => break None,
            }
        };
        let Some(mut plugin) = guard else {
            out_l[..n].fill(0.0);
            out_r[..n].fill(0.0);
            self.shared.missed.fetch_add(1, Relaxed);
            return;
        };

        for command in self.control.try_iter() {
            match command {
                Control::Param(id, value) => {
                    let _ = plugin.set_parameter(id, value.clamp(0.0, 1.0));
                }
            }
        }
        if self.shared.midi_in.load(Relaxed) {
            self.midi.read_stream(&mut self.midi_cursor, |msg| {
                if let Some(event) = midi_event(msg) {
                    let _ = plugin.send_midi_event(event);
                }
            });
        } else {
            self.midi_cursor = self.midi.stream_head();
        }

        // Within the capacity reserved up front: no allocation.
        let b = &mut self.buffers;
        for ch in b.inputs.iter_mut().chain(b.outputs.iter_mut()) {
            ch.resize(n, 0.0);
            ch.fill(0.0);
        }
        b.block_size = n;
        if let Some(input) = inputs.get(0) {
            for (k, ch) in b.inputs.iter_mut().enumerate() {
                ch.copy_from_slice(&input[k.min(1)][..n]);
            }
        }
        if plugin.process_audio(b).is_err() {
            out_l[..n].fill(0.0);
            out_r[..n].fill(0.0);
            return;
        }
        drop(plugin);

        let left = &b.outputs[0];
        let right = b.outputs.get(1).unwrap_or(left);
        out_l[..n].copy_from_slice(&left[..n]);
        out_r[..n].copy_from_slice(&right[..n]);
        let peak = left[..n].iter().chain(&right[..n]).fold(0.0f32, |m, s| m.max(s.abs()));
        self.shared.meter.fetch_max(peak);
    }
}
