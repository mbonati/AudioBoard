//! VST3 plugins as nodes. A plugin is shared between the UI (its editor
//! window, parameter display, saved state) and the audio thread, which
//! renders it. Knob moves reach it through a lock-free channel. Plugins that
//! take MIDI play what arrives on their MIDI cable (a sequencer, an
//! arpeggiator), and can also take the MIDI keyboard directly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use vst3_host::{AudioBuffers, MidiChannel, MidiEvent, Plugin, TransportPosition, Vst3Host};

use super::engine::{BLOCK, Buf, Inputs, ProcessCtx, Processor};
use super::midi::MidiState;
use super::modules::steps::note_event;
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
        midi: info.has_midi_input,
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
}

impl PluginProcessor {
    pub fn new(shared: Arc<PluginShared>, control: Receiver<Control>, midi: Arc<MidiState>, meta: &PluginMeta, sample_rate: f32) -> Self {
        Self {
            midi_cursor: midi.stream_head(),
            cable_notes: [0; 128],
            tempo: 0.0,
            samples: 0,
            transport: None,
            buffers: AudioBuffers::new(meta.inputs, meta.outputs, BLOCK, sample_rate as f64),
            shared,
            control,
            midi,
        }
    }
}

impl Processor for PluginProcessor {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
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

        // The global clock, for tempo-synced LFOs, delays and arpeggiators.
        let tempo = ctx.beats_per_sample * 60.0 * ctx.sample_rate as f64;
        if (tempo - self.tempo).abs() > 1e-6 && tempo > 0.0 {
            let _ = plugin.set_tempo(tempo);
            self.tempo = tempo;
        }
        if self.transport != Some(ctx.playing) {
            let _ = plugin.set_playing(ctx.playing);
            self.transport = Some(ctx.playing);
        }
        if ctx.restart {
            self.samples = 0;
        }
        let _ = plugin.set_transport_position(TransportPosition { samples: self.samples, quarter_notes: ctx.beat });
        self.samples += n as i64;

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
        for e in inputs.midi.events() {
            let [status, note, _] = e.data;
            let bit = 1u16 << (status & 0x0F);
            match note_event(e.data) {
                Some(true) => self.cable_notes[note as usize & 0x7F] |= bit,
                Some(false) => self.cable_notes[note as usize & 0x7F] &= !bit,
                None => {}
            }
            if let Some(event) = midi_event(e.data) {
                let _ = plugin.send_midi_event_at(event, e.frame as i32);
            }
        }
        // Cable unplugged (or its source removed): end what it started.
        if !inputs.midi_connected {
            for (note, channels) in self.cable_notes.iter_mut().enumerate() {
                for ch in (0..16).filter(|ch| *channels & 1 << ch != 0) {
                    if let Some(event) = midi_event([0x80 | ch as u8, note as u8, 0]) {
                        let _ = plugin.send_midi_event(event);
                    }
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
