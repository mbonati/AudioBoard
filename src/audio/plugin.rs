//! Plugins as nodes: VST3 everywhere, and Audio Units (v3 and v2) on macOS
//! (see `au`). A plugin is shared between the UI (its editor window,
//! parameter display, saved state) and the audio thread, which renders it.
//! Knob moves reach it through a lock-free channel. Plugins that take MIDI
//! play what arrives on their MIDI cable (a sequencer, an arpeggiator), and
//! can also take the MIDI keyboard directly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use vst3_host::{AudioBuffers, MidiChannel, MidiEvent, Plugin, TransportPosition, Vst3Host};

#[cfg(target_os = "macos")]
use super::au::{self, AudioUnit};
use super::engine::{BLOCK, Buf, Inputs, PASS, PassInputs, ProcessCtx, Processor};
use super::midi::MidiState;
use super::modules::steps::note_event;
use super::params::AtomicF32;

/// What the UI asks of a plugin, applied on the audio thread.
pub enum Control {
    /// Normalized (0..1) value for a parameter id.
    Param(u32, f64),
}

/// A loaded plugin, of either format, behind the lock the UI and the audio
/// thread share.
pub enum Instance {
    Vst3(Arc<Mutex<Plugin>>),
    #[cfg(target_os = "macos")]
    Au(Arc<Mutex<AudioUnit>>),
}

/// A locked plugin: the calls both formats answer.
pub enum Guard<'a> {
    Vst3(MutexGuard<'a, Plugin>),
    #[cfg(target_os = "macos")]
    Au(MutexGuard<'a, AudioUnit>),
}

impl Instance {
    /// The plugin if nobody holds it (a poisoned lock is taken anyway).
    pub fn try_lock(&self) -> Result<Guard<'_>, ()> {
        fn take<'a, T>(r: Result<MutexGuard<'a, T>, TryLockError<MutexGuard<'a, T>>>) -> Result<MutexGuard<'a, T>, ()> {
            match r {
                Ok(g) => Ok(g),
                Err(TryLockError::Poisoned(p)) => Ok(p.into_inner()),
                Err(TryLockError::WouldBlock) => Err(()),
            }
        }
        match self {
            Instance::Vst3(p) => take(p.try_lock()).map(Guard::Vst3),
            #[cfg(target_os = "macos")]
            Instance::Au(p) => take(p.try_lock()).map(Guard::Au),
        }
    }

    /// The plugin, waiting for it.
    pub fn lock(&self) -> Result<Guard<'_>, String> {
        Ok(match self {
            Instance::Vst3(p) => Guard::Vst3(p.lock().unwrap_or_else(|e| e.into_inner())),
            #[cfg(target_os = "macos")]
            Instance::Au(p) => Guard::Au(p.lock().unwrap_or_else(|e| e.into_inner())),
        })
    }
}

/// The clock a plugin follows, for one pass.
pub struct HostClock {
    pub tempo: f64,
    pub beat: f64,
    pub beats_per_sample: f64,
    pub playing: bool,
    /// Samples rendered since the transport last started over.
    pub samples: i64,
    pub tempo_changed: bool,
    pub playing_changed: bool,
    pub restart: bool,
}

impl Guard<'_> {
    /// Tempo, transport and position, before a render.
    pub fn clock(&mut self, c: &HostClock) {
        match self {
            Guard::Vst3(p) => {
                if c.tempo_changed {
                    let _ = p.set_tempo(c.tempo);
                }
                if c.playing_changed {
                    let _ = p.set_playing(c.playing);
                }
                let _ = p.set_transport_position(TransportPosition { samples: c.samples, quarter_notes: c.beat });
            }
            #[cfg(target_os = "macos")]
            Guard::Au(u) => u.set_transport(c.tempo, c.beat, c.beats_per_sample, c.playing, c.playing_changed || c.restart, c.restart),
        }
    }

    /// Sets a parameter (normalized) for the next render.
    pub fn set_parameter(&mut self, id: u32, value: f64) {
        match self {
            Guard::Vst3(p) => {
                let _ = p.set_parameter(id, value);
            }
            #[cfg(target_os = "macos")]
            Guard::Au(u) => u.set_parameter(id, value),
        }
    }

    /// A raw MIDI message, at `frame` into the next render (None: at once).
    pub fn send_midi(&mut self, msg: [u8; 3], frame: Option<i32>) {
        match self {
            Guard::Vst3(p) => {
                if let Some(event) = midi_event(msg) {
                    let _ = match frame {
                        Some(f) => p.send_midi_event_at(event, f),
                        None => p.send_midi_event(event),
                    };
                }
            }
            #[cfg(target_os = "macos")]
            Guard::Au(u) => u.send_midi(msg, frame.unwrap_or(0) as i64),
        }
    }

    /// Renders `buffers.block_size` frames; false on a plugin error.
    pub fn process(&mut self, buffers: &mut AudioBuffers) -> bool {
        match self {
            Guard::Vst3(p) => p.process_audio(buffers).is_ok(),
            #[cfg(target_os = "macos")]
            Guard::Au(u) => u.process(buffers),
        }
    }

    /// Lets a VST3 plugin do what it asked the host for (restart, resize).
    pub fn service_host_requests(&mut self) {
        match self {
            Guard::Vst3(p) => {
                let _ = p.service_host_requests();
            }
            #[cfg(target_os = "macos")]
            Guard::Au(_) => {}
        }
    }

    /// A parameter's value now, normalized.
    pub fn get_parameter(&self, id: u32) -> Result<f64, String> {
        match self {
            Guard::Vst3(p) => p.get_parameter(id).map_err(|e| e.to_string()),
            #[cfg(target_os = "macos")]
            Guard::Au(u) => u.get_parameter(id).ok_or_else(|| "no such parameter".to_owned()),
        }
    }

    /// The plugin's own text for a value of a parameter.
    pub fn format_parameter(&self, id: u32, value: f64) -> Result<String, String> {
        match self {
            Guard::Vst3(p) => p.format_parameter(id, value).map_err(|e| e.to_string()),
            #[cfg(target_os = "macos")]
            Guard::Au(u) => u.format_parameter(id, value).ok_or_else(|| "no such parameter".to_owned()),
        }
    }

    /// The complete state, in the plugin's own format.
    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        match self {
            Guard::Vst3(p) => p.save_state().map_err(|e| e.to_string()),
            #[cfg(target_os = "macos")]
            Guard::Au(u) => u.save_state().ok_or_else(|| "no state".to_owned()),
        }
    }

    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        match self {
            Guard::Vst3(p) => p.load_state(data).map_err(|e| e.to_string()),
            #[cfg(target_os = "macos")]
            Guard::Au(u) => u.load_state(data),
        }
    }
}

/// The part of a plugin node both threads see.
pub struct PluginShared {
    pub plugin: Instance,
    /// Output peak since the UI last looked.
    pub meter: AtomicF32,
    /// Feed the external MIDI stream into the plugin.
    pub midi_in: AtomicBool,
    /// Its own editor is open, where it can sound by itself (Kontakt's
    /// preset previews and on-screen keyboard): it never rests meanwhile.
    pub editor_open: AtomicBool,
    /// Blocks left silent because the UI held the plugin too long.
    pub missed: AtomicU32,
}

/// A plugin's format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Vst3,
    /// An Audio Unit: v3 (an app extension) or v2 (a component).
    AudioUnit { v3: bool },
}

impl Format {
    /// "VST3", "AUv3", "AU".
    pub fn label(&self) -> &'static str {
        match self {
            Format::Vst3 => "VST3",
            Format::AudioUnit { v3: true } => "AUv3",
            Format::AudioUnit { v3: false } => "AU",
        }
    }

    /// The format of a plugin path: Audio Units have pseudo-paths.
    pub fn of(path: &Path) -> Self {
        if super::au::is_au_path(path) { Format::AudioUnit { v3: false } } else { Format::Vst3 }
    }
}

/// What we know about a loaded plugin.
#[derive(Clone, Debug)]
pub struct PluginMeta {
    /// Its bundle, or an Audio Unit's pseudo-path (see `au`).
    pub path: PathBuf,
    pub format: Format,
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
    /// Accepts MIDI (instruments, MIDI effects).
    pub midi: bool,
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
        // Plugins get a whole device buffer per call (see `engine::PASS`).
        .block_size(PASS)
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
    // Only the main buses are used: switch off the others (a drum machine's
    // sixteen stereo outs, sidechains) so the plugin doesn't render them.
    if let Ok(layout) = plugin.audio_bus_layout() {
        use vst3_host::{BusDirection, MediaType};
        for (direction, count) in [(BusDirection::Output, layout.outputs.len()), (BusDirection::Input, layout.inputs.len())] {
            for bus in 1..count {
                let _ = plugin.set_bus_active(MediaType::Audio, direction, bus as i32, false);
            }
        }
    }
    plugin.start_processing().map_err(|e| e.to_string())?;
    let instrument = info.category.contains("Instrument") || (info.has_midi_input && info.audio_inputs == 0);
    let meta = PluginMeta {
        path: path.to_path_buf(),
        format: Format::Vst3,
        name: info.name.clone(),
        vendor: info.vendor.clone(),
        category: info.category.clone(),
        instrument,
        inputs: if info.audio_inputs > 0 { 2 } else { 0 },
        outputs: plugin.output_channel_count().max(1),
        editor: plugin.has_editor(),
        midi: info.has_midi_input,
    };
    Ok(Loaded { plugin, meta, params })
}

/// A loaded plugin of either format, ready for a node.
pub struct LoadedAny {
    pub instance: Instance,
    pub meta: PluginMeta,
    pub params: Vec<PluginParam>,
}

impl From<Loaded> for LoadedAny {
    fn from(l: Loaded) -> Self {
        Self { instance: Instance::Vst3(Arc::new(Mutex::new(l.plugin))), meta: l.meta, params: l.params }
    }
}

/// Loads an Audio Unit by its pseudo-path. Runs plugin code: UI thread only.
#[cfg(target_os = "macos")]
pub fn load_au(path: &Path, sample_rate: f32) -> Result<LoadedAny, String> {
    let component = au::Component::from_path(path).ok_or("not an Audio Unit")?;
    // Plugins get a whole device buffer per call (see `engine::PASS`).
    let unit = AudioUnit::open(component, sample_rate, PASS)?;
    let params = unit.params.iter().map(|p| PluginParam { id: p.id, name: p.name.clone(), default: p.default }).collect();
    let meta = PluginMeta {
        path: path.to_path_buf(),
        format: Format::AudioUnit { v3: unit.v3 },
        name: unit.name.clone(),
        vendor: clean_vendor(&unit.vendor),
        category: component.category().to_owned(),
        instrument: component.kind == au::INSTRUMENT,
        inputs: if unit.inputs > 0 { 2 } else { 0 },
        outputs: unit.outputs.max(1),
        // Its own view, else the system's generic one.
        editor: true,
        midi: unit.midi,
    };
    Ok(LoadedAny { instance: Instance::Au(Arc::new(Mutex::new(unit))), meta, params })
}

#[cfg(not(target_os = "macos"))]
pub fn load_au(_path: &Path, _sample_rate: f32) -> Result<LoadedAny, String> {
    Err("Audio Units are macOS only".to_owned())
}

/// Installed Audio Units (none outside macOS). Only the system's list: no
/// plugin code runs.
pub fn scan_au() -> Vec<super::au::Listed> {
    #[cfg(target_os = "macos")]
    {
        au::list()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
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

/// The maker of a plugin bundle, from its metadata files: no plugin code
/// runs. Tries the VST3 module info, then the bundle's copyright notice,
/// then its identifier ("com.maker.Plugin").
pub fn bundle_vendor(path: &Path) -> Option<String> {
    let contents = path.join("Contents");
    if let Ok(info) = std::fs::read_to_string(contents.join("Resources").join("moduleinfo.json"))
        && let Some(vendor) = json_string(&info, "Vendor").filter(|v| !v.trim().is_empty())
    {
        return Some(clean_vendor(&vendor));
    }
    // Binary property lists aren't text, and are skipped.
    let plist = std::fs::read_to_string(contents.join("Info.plist")).ok()?;
    plist_string(&plist, "NSHumanReadableCopyright")
        .and_then(|c| vendor_from_copyright(&c))
        .or_else(|| plist_string(&plist, "CFBundleIdentifier").and_then(|id| vendor_from_bundle_id(&id)))
}

/// A maker's name without its legal form: "Valhalla DSP" for
/// "Valhalla DSP, LLC".
pub fn clean_vendor(name: &str) -> String {
    const FORMS: [&str; 14] = [
        "llc", "l.l.c.", "inc.", "inc", "ltd.", "ltd", "limited", "gmbh", "s.r.l.", "srl", "corp.", "corp", "co.", "ab",
    ];
    let mut name = name.trim().trim_end_matches(['.', ',', ' ']).to_owned();
    loop {
        let lower = name.to_lowercase();
        let Some(form) = FORMS.iter().find(|f| lower.ends_with(&format!(" {f}")) || lower.ends_with(&format!(",{f}"))) else {
            break;
        };
        name.truncate(name.len() - form.len());
        name = name.trim_end_matches(['.', ',', ' ']).to_owned();
    }
    name
}

/// Groups spellings of one maker: "Valhalla DSP" and "ValhallaDSP" match.
pub fn vendor_key(name: &str) -> String {
    clean_vendor(name).chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// "Valhalla DSP" from "Copyright (c) 2022, Valhalla DSP, LLC. All Rights Reserved."
fn vendor_from_copyright(notice: &str) -> Option<String> {
    let mut text = notice.to_owned();
    for noise in ["all rights reserved", "copyright", "(c)", "©"] {
        while let Some(i) = text.to_lowercase().find(noise) {
            text.replace_range(i..i + noise.len(), " ");
        }
    }
    // Drop years ("2022", "2019-2024") and the punctuation around them.
    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| !w.trim_matches([',', '.', '-', '–']).chars().all(|c| c.is_ascii_digit() || c == '-' || c == '–'))
        .collect();
    let vendor = clean_vendor(words.join(" ").trim_matches([',', '.', ' ', '-']));
    (!vendor.is_empty()).then_some(vendor)
}

/// "ValhallaDSP" from "com.ValhallaDSP.ValhallaFreqEcho".
fn vendor_from_bundle_id(id: &str) -> Option<String> {
    const TOP: [&str; 10] = ["com", "net", "org", "io", "co", "de", "uk", "fr", "it", "audio"];
    let mut parts = id.split('.').filter(|p| !p.is_empty());
    let first = parts.next()?;
    let maker = if TOP.contains(&first.to_lowercase().as_str()) { parts.next()? } else { first };
    let mut chars = maker.chars();
    let head = chars.next()?;
    Some(head.to_uppercase().chain(chars).collect())
}

/// The first string value of `"key"` in JSON-like text. Module info files
/// are JSON5 (trailing commas), which strict parsers refuse.
fn json_string(text: &str, key: &str) -> Option<String> {
    let after = &text[text.find(&format!("\"{key}\""))? + key.len() + 2..];
    let after = after.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut value = String::new();
    let mut chars = after.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(value),
            '\\' => value.extend(chars.next()),
            c => value.push(c),
        }
    }
    None
}

/// The `<string>` after `<key>key</key>` in an XML property list.
fn plist_string(xml: &str, key: &str) -> Option<String> {
    let after = &xml[xml.find(&format!("<key>{key}</key>"))? + key.len() + 11..];
    let value = after.trim_start().strip_prefix("<string>")?;
    let value = &value[..value.find("</string>")?];
    Some(
        value
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&#169;", "©")
            .replace("&amp;", "&"),
    )
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

/// How long the audio thread waits for the UI to let go of a plugin. The UI
/// calls plugins only between callbacks (see `engine::audio_idle`), so a
/// wait is rare and short; past this the block is skipped.
const LOCK_WAIT: Duration = Duration::from_micros(800);

pub struct PluginProcessor {
    shared: Arc<PluginShared>,
    control: Receiver<Control>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    /// Notes started from the MIDI cable: one bit per channel for each
    /// note, so unplugging the cable can end them.
    cable_notes: [u16; 128],
    /// Tempo last told to the plugin, and samples it has rendered.
    tempo: f64,
    samples: i64,
    /// Whether the plugin was last told the transport is playing.
    transport: Option<bool>,
    buffers: AudioBuffers,
    /// Frames of silent output in a row, and how many let it rest (see
    /// `process_pass`).
    silent: usize,
    rest_after: usize,
}

/// A stopped plugin rests after this long silent with nothing coming in.
const PLUGIN_REST: f32 = 1.0;

impl PluginProcessor {
    pub fn new(shared: Arc<PluginShared>, control: Receiver<Control>, midi: Arc<MidiState>, meta: &PluginMeta, sample_rate: f32) -> Self {
        Self {
            midi_cursor: midi.stream_head(),
            cable_notes: [0; 128],
            tempo: 0.0,
            samples: 0,
            transport: None,
            buffers: AudioBuffers::new(meta.inputs, meta.outputs, PASS, sample_rate as f64),
            silent: 0,
            rest_after: (PLUGIN_REST * sample_rate) as usize,
            shared,
            control,
            midi,
        }
    }
}

impl Processor for PluginProcessor {
    /// Unused: the engine gives plugins whole passes ([`Self::process_pass`]).
    fn process(&mut self, _ctx: &ProcessCtx, _inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        for ch in outputs[0].iter_mut() {
            ch[..n].fill(0.0);
        }
    }

    fn wants_pass(&self) -> bool {
        true
    }

    /// One call to the plugin for the whole pass: plugins can cost a lot per
    /// call, however short. While the transport is stopped, a plugin that
    /// has been silent for a second with nothing coming in (no audio, no
    /// MIDI) rests, and wakes at the first input or on Start. Not while its
    /// editor is open: sound started there never reaches us as input.
    fn process_pass(&mut self, ctx: &ProcessCtx, inputs: &PassInputs, outputs: &mut [Box<[Buf]>], n: usize) {
        let blocks = n.div_ceil(BLOCK);
        let len = |k: usize| (n - k * BLOCK).min(BLOCK);
        let silence = |outputs: &mut [Box<[Buf]>]| {
            for out in outputs[..blocks].iter_mut() {
                out[0] = [[0.0; BLOCK]; 2];
            }
        };

        let keyboard = self.shared.midi_in.load(Relaxed);
        let incoming = (keyboard && self.midi.stream_head() != self.midi_cursor)
            || inputs.midi.iter().any(|m| !m.events().is_empty())
            || (!inputs.midi_connected && self.cable_notes.iter().any(|&c| c != 0))
            || inputs.audio.is_some_and(|a| a.iter().flatten().flatten().any(|s| s.abs() > 1e-6));
        let resting = !ctx.playing && !incoming && !self.shared.editor_open.load(Relaxed);
        if resting && self.silent >= self.rest_after {
            silence(outputs);
            if !keyboard {
                self.midi_cursor = self.midi.stream_head();
            }
            return;
        }

        // The UI holds the plugin only between callbacks; never wait long.
        let started = Instant::now();
        let guard = loop {
            match self.shared.plugin.try_lock() {
                Ok(g) => break Some(g),
                Err(()) if started.elapsed() < LOCK_WAIT => std::hint::spin_loop(),
                Err(()) => break None,
            }
        };
        let Some(mut plugin) = guard else {
            silence(outputs);
            self.shared.missed.fetch_add(1, Relaxed);
            return;
        };

        // The global clock, for tempo-synced LFOs, delays and arpeggiators.
        let tempo = ctx.beats_per_sample * 60.0 * ctx.sample_rate as f64;
        let tempo_changed = (tempo - self.tempo).abs() > 1e-6 && tempo > 0.0;
        if tempo_changed {
            self.tempo = tempo;
        }
        let playing_changed = self.transport != Some(ctx.playing);
        self.transport = Some(ctx.playing);
        if ctx.restart {
            self.samples = 0;
        }
        plugin.clock(&HostClock {
            tempo: self.tempo,
            beat: ctx.beat,
            beats_per_sample: ctx.beats_per_sample,
            playing: ctx.playing,
            samples: self.samples,
            tempo_changed,
            playing_changed,
            restart: ctx.restart,
        });
        self.samples += n as i64;

        for command in self.control.try_iter() {
            match command {
                Control::Param(id, value) => plugin.set_parameter(id, value.clamp(0.0, 1.0)),
            }
        }
        if keyboard {
            self.midi.read_stream(&mut self.midi_cursor, |msg| plugin.send_midi(msg, None));
        } else {
            self.midi_cursor = self.midi.stream_head();
        }
        for (k, buf) in inputs.midi.iter().enumerate() {
            for e in buf.events() {
                let [status, note, _] = e.data;
                let bit = 1u16 << (status & 0x0F);
                match note_event(e.data) {
                    Some(true) => self.cable_notes[note as usize & 0x7F] |= bit,
                    Some(false) => self.cable_notes[note as usize & 0x7F] &= !bit,
                    None => {}
                }
                if midi_event(e.data).is_some() {
                    plugin.send_midi(e.data, Some((k * BLOCK + e.frame as usize) as i32));
                }
            }
        }
        // Cable unplugged (or its source removed): end what it started.
        if !inputs.midi_connected {
            for (note, channels) in self.cable_notes.iter_mut().enumerate() {
                for ch in (0..16).filter(|ch| *channels & 1 << ch != 0) {
                    plugin.send_midi([0x80 | ch as u8, note as u8, 0], None);
                }
                *channels = 0;
            }
        }

        // Within the capacity reserved up front: no allocation.
        let b = &mut self.buffers;
        for ch in b.inputs.iter_mut().chain(b.outputs.iter_mut()) {
            ch.resize(n, 0.0);
            ch.fill(0.0);
        }
        b.block_size = n;
        if let Some(audio) = inputs.audio {
            for (c, ch) in b.inputs.iter_mut().enumerate() {
                for (k, block) in audio[..blocks].iter().enumerate() {
                    ch[k * BLOCK..k * BLOCK + len(k)].copy_from_slice(&block[c.min(1)][..len(k)]);
                }
            }
        }
        if !plugin.process(b) {
            silence(outputs);
            return;
        }
        drop(plugin);

        let left = &b.outputs[0];
        let right = b.outputs.get(1).unwrap_or(left);
        for (k, out) in outputs[..blocks].iter_mut().enumerate() {
            let range = k * BLOCK..k * BLOCK + len(k);
            out[0][0][..len(k)].copy_from_slice(&left[range.clone()]);
            out[0][1][..len(k)].copy_from_slice(&right[range]);
        }
        let peak = left[..n].iter().chain(&right[..n]).fold(0.0f32, |m, s| m.max(s.abs()));
        self.shared.meter.fetch_max(peak);
        self.silent = if peak < 1e-5 { self.silent.saturating_add(n) } else { 0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_names() {
        let c = |s| vendor_from_copyright(s);
        assert_eq!(c("Copyright (c) 2022, Valhalla DSP, LLC. All Rights Reserved.").as_deref(), Some("Valhalla DSP"));
        assert_eq!(c("© 2019-2024 Acme Audio Ltd.").as_deref(), Some("Acme Audio"));
        assert_eq!(c("Vital Audio").as_deref(), Some("Vital Audio"));
        assert_eq!(c(""), None);
        assert_eq!(vendor_from_bundle_id("com.ValhallaDSP.ValhallaFreqEcho").as_deref(), Some("ValhallaDSP"));
        assert_eq!(vendor_from_bundle_id("audio.vital.synth").as_deref(), Some("Vital"));
        assert_eq!(vendor_key("Valhalla DSP, LLC"), vendor_key("ValhallaDSP"));
        assert_eq!(vendor_key("Spitfire Audio"), vendor_key("spitfireaudio"));
        assert_eq!(clean_vendor("Native Instruments GmbH"), "Native Instruments");
        assert_eq!(clean_vendor("FabFilter"), "FabFilter");
    }

    #[test]
    fn vendor_from_bundle_files() {
        let dir = std::env::temp_dir().join(format!("audioboard-vendor-{}", std::process::id()));
        let bundle = |name: &str, files: &[(&str, &str)]| {
            let path = dir.join(format!("{name}.vst3"));
            for (file, text) in files {
                let f = path.join("Contents").join(file);
                std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                std::fs::write(f, text).unwrap();
            }
            path
        };
        let plist = |key: &str, value: &str| {
            format!("<?xml version=\"1.0\"?><plist><dict><key>{key}</key>\n\t<string>{value}</string></dict></plist>")
        };
        let module = bundle(
            "A",
            &[("Resources/moduleinfo.json", "{ \"Name\": \"A\", \"Factory Info\": { \"Vendor\": \"Spitfire Audio\", }, }")],
        );
        let copyright = bundle("B", &[("Info.plist", &plist("NSHumanReadableCopyright", "Copyright &#169; 2024 Big &amp; Small, Inc."))]);
        let id = bundle("C", &[("Info.plist", &plist("CFBundleIdentifier", "com.u-he.Diva"))]);
        let nothing = bundle("D", &[]);
        assert_eq!(bundle_vendor(&module).as_deref(), Some("Spitfire Audio"));
        assert_eq!(bundle_vendor(&copyright).as_deref(), Some("Big & Small"));
        assert_eq!(bundle_vendor(&id).as_deref(), Some("U-he"));
        assert_eq!(bundle_vendor(&nothing), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
