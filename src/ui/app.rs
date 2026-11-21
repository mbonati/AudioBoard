use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::{
    Align, Align2, CentralPanel, Color32, CursorIcon, FontId, Frame, Id, Key, KeyboardShortcut,
    Layout, Margin, Modifiers, Panel, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind,
    Ui, ViewportCommand, pos2, vec2,
};

use super::canvas::{Editor, Request, menu_item};
use super::monitor::Monitor;
use super::palette::{self, Palette, PluginEntry, Spawn};
use super::plugin_node;
use super::recent;
use super::settings::{
    PluginCounts, PluginSettings, SettingsChange, SettingsContext, SettingsWindow, SyncSettings,
};
use super::tempo::TempoControl;
use super::theme::{self, alpha};
use super::tracks::TrackView;
use crate::audio::device::{AudioConfig, BUFFER_SIZES};
use crate::audio::engine::NodeId;
use crate::audio::export::{self, FORMATS};
use crate::audio::midi::MidiInputs;
use crate::audio::modules::{
    ModuleKind, ModuleParams, delay, drone, filter, granular, random, reverb,
};
use crate::audio::params::{EngineShared, Waveform};
use crate::audio::plugin::Format;
use crate::audio::record::Recorder;
use crate::audio::sync::ClockOut;
use crate::audio::{self, AudioHandle};
use crate::graph::{Graph, NodeKind, PortRef};
use crate::project::{self, Library, PROJECT_EXT, Project};
use crate::tracks::View;

const SHORTCUTS: &[(&str, &str)] = &[
    ("Cmd+N / Cmd+O", "New / open project"),
    ("V", "Switch between the node view and the track view"),
    ("Cmd+S / Shift+Cmd+S", "Save / save as"),
    ("Drag from palette", "Add a node or library module"),
    ("Right-click", "Menu for what is under the pointer"),
    ("Drag port", "Patch a cable"),
    ("Drag input", "Re-patch / unplug"),
    ("Double-click cable", "Remove cable"),
    ("Drag knob", "Adjust (Shift: fine)"),
    ("Double-click knob", "Reset to default"),
    ("Drag empty space", "Select with a rectangle"),
    ("Shift+click", "Add to / remove from selection"),
    ("Cmd+A / Esc", "Select all / none"),
    ("Scroll", "Pan canvas"),
    ("Right / middle drag", "Pan canvas (also Alt+drag)"),
    ("Cmd+Scroll", "Zoom (or pinch)"),
    ("Del", "Delete selection"),
    ("Cmd+D", "Duplicate selection"),
    ("Cmd+G", "Make a module from the selection"),
    ("Cmd+Shift+G", "Ungroup"),
    ("Enter / F2", "Rename the selected module"),
    ("C", "Collapse / expand node or module"),
    ("Double-click title", "Collapse / expand"),
    ("Shift+C", "Collapse / expand all"),
    ("L", "Auto layout (selection or all) · Cmd+Z undoes"),
    ("F / Shift+F", "Fit all / fit selection"),
    (
        "Z",
        "Zoom to the node under the mouse (or the selection) · Z / Esc back",
    ),
    (
        "B",
        "Bypass the effect under the mouse (or the selected ones)",
    ),
    ("A", "Auto-fit: keep the patch in view"),
    ("Cmd+= / Cmd+− / Cmd+0", "Zoom in / out / 100%"),
    ("Space", "Start / stop (stop goes back to the start)"),
    ("R", "Record the board · R again stops and opens the export"),
    ("T", "Tap tempo (on each beat)"),
    ("Click BPM", "Type the tempo (drag to change it)"),
];

/// The open project: where it lives and what was last saved.
struct Document {
    path: Option<PathBuf>,
    saved: Project,
    dirty: bool,
    checked_at: f64,
}

impl Document {
    fn new(path: Option<PathBuf>, saved: Project) -> Self {
        Self {
            path,
            saved,
            dirty: false,
            checked_at: 0.0,
        }
    }

    fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Untitled".to_owned(), |s| s.to_string_lossy().into_owned())
    }
}

/// A short message in the status bar.
struct Toast {
    text: String,
    error: bool,
    until: f64,
}

enum FileCommand {
    New,
    Open,
    /// A project from Open Recent.
    OpenRecent(PathBuf),
    ClearRecent,
    Save,
    SaveAs,
}

enum ViewCommand {
    ZoomIn,
    ZoomOut,
    Actual,
    FitAll,
    FitSelection,
    AutoFit,
    AutoLayout,
}

pub struct AudioBoardApp {
    shared: Arc<EngineShared>,
    audio: AudioHandle,
    graph: Graph,
    editor: Editor,
    /// The same patch as rows of tracks (see `tracks`).
    tracks: TrackView,
    monitor: Monitor,
    show_monitor: bool,
    palette: Palette,
    show_palette: bool,
    doc: Document,
    library: Library,
    /// Category and maker of plugins loaded before, by bundle path.
    plugin_catalog: BTreeMap<String, PluginInfo>,
    /// Audio Units installed, as last listed, and the first listing while
    /// it runs in the background (it takes a while).
    audio_units: Vec<crate::audio::au::Listed>,
    au_scan: Option<std::sync::mpsc::Receiver<Vec<crate::audio::au::Listed>>>,
    toast: Option<Toast>,
    window_title: String,
    midi: MidiInputs,
    midi_scanned: f64,
    /// Sends the board's clock to the MIDI outputs chosen in Settings → Sync.
    clock_out: ClockOut,
    /// MIDI messages seen so far, and when the count last moved.
    midi_events: (u32, f64),
    tempo: TempoControl,
    settings: AppSettings,
    /// The highest DSP load shown, and since when.
    peak_load: (f32, f64),
    /// Dropouts seen so far, and when the last one happened.
    overloads: (u32, f64),
    /// The output limiter's deepest reduction shown (dB), and since when.
    limiting: (f32, f64),
    /// Recording the board, and exporting the take.
    recorder: Recorder,
    export: ExportWindow,
    settings_window: SettingsWindow,
    /// The audio engine the nodes were built for (see `AudioHandle::generation`).
    generation: u32,
    #[cfg(target_os = "macos")]
    window: Option<super::macos::NativeWindow>,
}

impl AudioBoardApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let shared = Arc::new(EngineShared::new());
        // The patch waits for Start.
        shared.running.store(false, Ordering::Relaxed);
        let settings = load_settings();
        shared.output.set(db_to_gain(settings.output_db));
        let audio = audio::start(shared.clone(), settings.audio.clone());
        let mut graph = Graph::default();
        demo_patch(&mut graph, &audio);
        let library = Library::open(
            project::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Library"),
        );
        let mut editor = Editor::new();
        editor.library = library.presets.clone();
        let plugin_catalog = load_catalog();
        editor.plugins = plugin_entries(&plugin_catalog, &settings.plugins, &[]);
        let (au_tx, au_scan) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("au-scan".into())
            .spawn(move || {
                let _ = au_tx.send(crate::audio::plugin::scan_au());
            });
        // Node plugins (dylibs built with the node SDK), before any project
        // that uses them is opened.
        let node_scan = crate::audio::node_plugin::scan();
        let (scan_text, scan_error) = node_scan.summary();
        if !scan_text.is_empty() {
            eprintln!("Node plugins: {scan_text}");
        }
        let mut midi = MidiInputs::new();
        midi.set_sync_source(settings.sync.source.clone(), &shared.sync);
        let clock_out = ClockOut::start(shared.clone());
        clock_out.set_outputs(clock_outputs(&settings.sync));
        Self {
            recorder: Recorder::new(shared.clone()),
            export: ExportWindow::default(),
            settings_window: SettingsWindow::default(),
            generation: audio.generation,
            shared,
            audio,
            doc: Document::new(None, project::snapshot(&graph)),
            graph,
            editor,
            tracks: TrackView::new(),
            monitor: Monitor::new(),
            show_monitor: true,
            palette: Palette::new(),
            show_palette: true,
            library,
            plugin_catalog,
            audio_units: Vec::new(),
            au_scan: Some(au_scan),
            toast: (!scan_text.is_empty()).then(|| Toast {
                text: scan_text,
                error: scan_error,
                until: 8.0,
            }),
            window_title: String::new(),
            midi,
            clock_out,
            midi_scanned: f64::NEG_INFINITY,
            midi_events: (0, f64::NEG_INFINITY),
            tempo: TempoControl::default(),
            settings,
            peak_load: (0.0, f64::NEG_INFINITY),
            overloads: (0, f64::NEG_INFINITY),
            limiting: (0.0, f64::NEG_INFINITY),
            #[cfg(target_os = "macos")]
            window: super::macos::NativeWindow::new(cc),
        }
    }

    fn running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    /// The MIDI input whose clock the transport follows, if any.
    fn following(&self) -> Option<&str> {
        self.settings.sync.source.as_deref()
    }

    /// Start / Stop from the toolbar or Space: the board's own transport,
    /// unless it follows a MIDI clock (then that clock's Start and Stop).
    fn transport_pressed(&mut self, now: f64) {
        match self.following().map(str::to_owned) {
            Some(source) => self.notify(
                now,
                format!("The transport follows the MIDI clock from {source} (Settings → Sync)"),
                false,
            ),
            None => self.toggle_running(),
        }
    }

    /// Start plays from the top; Stop silences everything and rewinds the
    /// transport, so sequencers start over on the next Start.
    fn toggle_running(&self) {
        if self.running() {
            self.shared.rewinds.fetch_add(1, Ordering::Relaxed);
        }
        self.shared.running.fetch_xor(true, Ordering::Relaxed);
    }

    /// The board's output level: drag it, or click to type. Returns whether
    /// the pointer is on it.
    fn output_control(&mut self, ui: &mut Ui) -> bool {
        let mut db = self.settings.output_db;
        // A pill like TAP's, the value inside it.
        let (rect, _) = ui.allocate_exact_size(vec2(92.0, 24.0), Sense::hover());
        ui.painter().rect(
            rect,
            12.0,
            Color32::TRANSPARENT,
            Stroke::new(1.0, theme::NODE_BORDER),
            StrokeKind::Inside,
        );
        let resp = ui
            .scope(|ui| {
                ui.style_mut().override_font_id = Some(FontId::proportional(11.5));
                ui.put(
                    rect.shrink2(vec2(8.0, 3.0)),
                    egui::DragValue::new(&mut db).range(-24.0..=12.0).speed(0.1).fixed_decimals(1).prefix("OUT ").suffix(" dB"),
                )
            })
            .inner
            .on_hover_text("The board's output level: drag, or click to type (the limiter keeps it under 0 dBFS)");
        if db != self.settings.output_db {
            self.settings.output_db = db;
            self.shared.output.set(db_to_gain(db));
        }
        if resp.drag_stopped() || resp.lost_focus() {
            save_settings(&self.settings);
        }
        resp.hovered() || resp.dragged() || resp.has_focus()
    }

    /// R: starts recording what the board plays, or stops and offers the
    /// take for export.
    fn toggle_recording(&mut self, now: f64) {
        if self.recorder.recording() {
            match self.recorder.stop() {
                Ok(take) => {
                    let lost = if take.dropped > 0 {
                        format!(" ({} frames lost)", take.dropped)
                    } else {
                        String::new()
                    };
                    self.notify(
                        now,
                        format!("Recorded {}{lost}", clock(take.seconds())),
                        take.dropped > 0,
                    );
                    self.export.open = true;
                }
                Err(e) => self.notify(now, format!("Recording failed: {e}"), true),
            }
        } else if let Err(e) = self.recorder.start(self.audio.info.sample_rate) {
            self.notify(now, format!("Could not record: {e}"), true);
        }
    }

    /// The export window: format, options, Export… (a save dialog), and
    /// the export running in the background.
    fn export_window(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(job) = self.export.job.take_if(|j| j.is_finished()) {
            match job.join() {
                Ok(Ok(path)) => {
                    let name = path
                        .file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    self.notify(now, format!("Exported {name}"), false);
                    self.export.open = false;
                }
                Ok(Err(e)) => self.notify(now, format!("Export failed: {e}"), true),
                Err(_) => self.notify(now, "Export failed", true),
            }
        }
        if !self.export.open {
            return;
        }
        let Some(take) = self.recorder.take.clone() else {
            self.export.open = false;
            return;
        };
        let mut open = true;
        let mut start = false;
        let mut discard = false;
        let busy = self.export.job.is_some();
        egui::Window::new("Export recording")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                let peak = 20.0 * take.peak.max(1e-6).log10();
                ui.label(
                    RichText::new(format!(
                        "{} · {} Hz · peak {:.1} dBFS",
                        clock(take.seconds()),
                        take.sample_rate,
                        peak
                    ))
                    .color(theme::TEXT_DIM),
                );
                ui.add_space(6.0);
                ui.label("Format");
                for format in FORMATS {
                    let missing = format.missing();
                    let pick = ui.add_enabled(
                        missing.is_none(),
                        egui::RadioButton::new(
                            self.export.options.format == format,
                            format.label(),
                        ),
                    );
                    let pick = match missing {
                        Some(why) => pick.on_disabled_hover_text(why),
                        None => pick,
                    };
                    if pick.clicked() {
                        self.export.options.format = format;
                    }
                }
                ui.add_space(6.0);
                ui.checkbox(
                    &mut self.export.options.normalize,
                    "Normalize the peak to −1 dBFS",
                );
                ui.checkbox(
                    &mut self.export.options.trim,
                    "Trim the silence at the start and end",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if busy {
                        ui.spinner();
                        ui.label("Exporting…");
                    } else {
                        start = ui.button("Export…").clicked();
                        discard = ui.button("Discard take").clicked();
                    }
                });
            });
        if discard {
            self.recorder.take = None;
            self.export.open = false;
            return;
        }
        self.export.open = open;
        if start {
            let format = self.export.options.format;
            let name = format!(
                "{} take {}.{}",
                self.doc.name(),
                self.recorder.count,
                format.extension()
            );
            let picked = rfd::FileDialog::new()
                .set_title("Export recording")
                .set_file_name(&name)
                .add_filter(format.label(), &[format.extension()])
                .save_file();
            if let Some(path) = picked {
                let options = self.export.options;
                self.export.job = Some(std::thread::spawn(move || {
                    export::export(&take, options, &path).map(|()| path)
                }));
            }
        }
    }

    fn notify(&mut self, now: f64, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast {
            text: text.into(),
            error,
            until: now + if error { 8.0 } else { 3.0 },
        });
    }

    // --- Projects -----------------------------------------------------------

    /// `fresh`: ask plugins for their state now (before closing the
    /// document), rather than reuse a recent copy (the periodic check).
    fn refresh_dirty(&mut self, fresh: bool) {
        let now = if fresh {
            project::snapshot(&self.graph)
        } else {
            project::snapshot_cached(&self.graph)
        };
        self.doc.dirty = now != self.doc.saved || self.graph.plugins_edited();
    }

    /// Where file dialogs start: next to the open project, else Documents.
    fn dialog_dir(&self) -> PathBuf {
        self.doc
            .path
            .as_ref()
            .and_then(|p| p.parent().map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents")))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Offers to save unsaved changes. False if the user cancelled.
    fn confirm_discard(&mut self, now: f64) -> bool {
        self.refresh_dirty(true);
        if !self.doc.dirty {
            return true;
        }
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Unsaved changes")
            .set_description(format!(
                "Do you want to save the changes to “{}”?",
                self.doc.name()
            ))
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                "Save".into(),
                "Don't Save".into(),
                "Cancel".into(),
            ))
            .show();
        match answer {
            rfd::MessageDialogResult::Yes => self.save(now),
            rfd::MessageDialogResult::Custom(label) if label == "Save" => self.save(now),
            rfd::MessageDialogResult::No => true,
            rfd::MessageDialogResult::Custom(label) if label == "Don't Save" => true,
            _ => false,
        }
    }

    fn new_project(&mut self, now: f64) {
        if !self.confirm_discard(now) {
            return;
        }
        // A new project opens in the view in use.
        let view = self.graph.tracks.view;
        self.graph.clear(&self.audio);
        self.graph.tracks.view = view;
        self.editor.reset();
        self.tracks.reset();
        self.doc = Document::new(None, project::snapshot(&self.graph));
    }

    fn open_project(&mut self, now: f64) {
        if !self.confirm_discard(now) {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("AudioBoard project", &[PROJECT_EXT])
            .set_directory(self.dialog_dir())
            .pick_file()
        else {
            return;
        };
        self.open_path(path, now);
    }

    /// Open Recent: the project at `path`, after offering to save changes.
    fn open_recent(&mut self, path: PathBuf, now: f64) {
        if !path.exists() {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            self.notify(now, format!("{name} is no longer there"), true);
            recent::forget(&mut self.settings.recent, &path);
            save_settings(&self.settings);
            return;
        }
        if self.confirm_discard(now) {
            self.open_path(path, now);
        }
    }

    /// Replaces the board with the project at `path`.
    fn open_path(&mut self, path: PathBuf, now: f64) {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        match project::load_project(&path) {
            Ok(p) => {
                let skipped = project::restore(&mut self.graph, &p, &self.audio);
                self.editor.reset();
                self.tracks.reset();
                self.remember_recent(&path);
                self.doc = Document::new(Some(path), project::snapshot(&self.graph));
                let missing = project::missing_modules(&p);
                if skipped > 0 && !missing.is_empty() {
                    // Plugin nodes are saved by id: name what to install.
                    self.notify(
                        now,
                        format!(
                            "Opened {name}: {skipped} node(s) skipped, not installed: {}",
                            missing.join(", ")
                        ),
                        true,
                    );
                } else if skipped > 0 {
                    self.notify(
                        now,
                        format!("Opened {name}: {skipped} unknown node(s) skipped"),
                        true,
                    );
                } else {
                    self.notify(now, format!("Opened {name}"), false);
                }
            }
            Err(e) => self.notify(now, format!("Could not open {name}: {e}"), true),
        }
    }

    /// Puts a project opened or saved at the top of Open Recent.
    fn remember_recent(&mut self, path: &std::path::Path) {
        recent::remember(&mut self.settings.recent, path);
        save_settings(&self.settings);
    }

    /// Saves to the current file (asking for one the first time).
    fn save(&mut self, now: f64) -> bool {
        match self.doc.path.clone() {
            Some(path) => self.write(path, now),
            None => self.save_as(now),
        }
    }

    fn save_as(&mut self, now: f64) -> bool {
        let Some(mut path) = rfd::FileDialog::new()
            .add_filter("AudioBoard project", &[PROJECT_EXT])
            .set_directory(self.dialog_dir())
            .set_file_name(format!("{}.{PROJECT_EXT}", self.doc.name()))
            .save_file()
        else {
            return false;
        };
        if path.extension().is_none_or(|x| x != PROJECT_EXT) {
            path.set_extension(PROJECT_EXT);
        }
        self.write(path, now)
    }

    fn write(&mut self, path: PathBuf, now: f64) -> bool {
        let snapshot = project::snapshot(&self.graph);
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        match project::save_project(&path, &snapshot) {
            Ok(()) => {
                self.remember_recent(&path);
                self.doc = Document::new(Some(path), snapshot);
                self.notify(now, format!("Saved {name}"), false);
                true
            }
            Err(e) => {
                self.notify(now, format!("Could not save {name}: {e}"), true);
                false
            }
        }
    }

    fn run_file_command(&mut self, cmd: FileCommand, now: f64) {
        match cmd {
            FileCommand::New => self.new_project(now),
            FileCommand::Open => self.open_project(now),
            FileCommand::OpenRecent(path) => self.open_recent(path, now),
            FileCommand::ClearRecent => {
                self.settings.recent.clear();
                save_settings(&self.settings);
            }
            FileCommand::Save => {
                self.save(now);
            }
            FileCommand::SaveAs => {
                self.save_as(now);
            }
        }
    }

    // --- Module library -----------------------------------------------------

    fn handle_request(&mut self, request: Request, now: f64) {
        match request {
            Request::SaveToLibrary(gid) => {
                let Some(preset) = project::preset_from_group(&self.graph, gid) else {
                    return;
                };
                if self.library.contains(&preset.name)
                    && !ask(&format!("Replace “{}” in the library?", preset.name))
                {
                    return;
                }
                match self.library.save(&preset) {
                    Ok(()) => self.notify(
                        now,
                        format!("“{}” saved to the library", preset.name),
                        false,
                    ),
                    Err(e) => self.notify(now, format!("Could not save to the library: {e}"), true),
                }
            }
            Request::DeletePreset(i) => {
                let Some(name) = self.library.presets.get(i).map(|p| p.name.clone()) else {
                    return;
                };
                if !ask(&format!("Remove “{name}” from the library?")) {
                    return;
                }
                if let Err(e) = self.library.remove(i) {
                    self.notify(now, format!("Could not remove “{name}”: {e}"), true);
                }
            }
            Request::RescanPlugins => {
                self.audio_units = crate::audio::plugin::scan_au();
                self.au_scan = None;
                self.refresh_plugins();
                let counts = plugin_counts(&self.editor.plugins);
                let au = if counts.au > 0 {
                    format!(", {} Audio Unit(s)", counts.au)
                } else {
                    String::new()
                };
                // Node plugins too: new libraries in the Nodes folder load now.
                let nodes = crate::audio::node_plugin::scan();
                let (detail, refused) = nodes.summary();
                let detail = if detail.is_empty() {
                    String::new()
                } else {
                    format!(" · {detail}")
                };
                self.notify(
                    now,
                    format!(
                        "{} VST3 plugin(s){au}, {} plugin node(s) found{detail}",
                        counts.vst3, nodes.total
                    ),
                    refused,
                );
            }
            Request::RevealNodePlugins => {
                if let Some(dir) = crate::audio::node_plugin::folder() {
                    if let Err(e) = reveal(&dir) {
                        self.notify(now, format!("Could not open {}: {e}", dir.display()), true);
                    }
                }
            }
            Request::RevealLibrary => {
                let dir = &self.library.dir;
                let opened = std::fs::create_dir_all(dir).and_then(|()| {
                    let opener = if cfg!(target_os = "macos") {
                        "open"
                    } else if cfg!(target_os = "windows") {
                        "explorer"
                    } else {
                        "xdg-open"
                    };
                    std::process::Command::new(opener)
                        .arg(dir)
                        .spawn()
                        .map(|_| ())
                });
                if let Err(e) = opened {
                    self.notify(now, format!("Could not open {}: {e}", dir.display()), true);
                }
            }
        }
        self.editor.library = self.library.presets.clone();
    }

    /// Keeps MIDI devices connected (they can come and go) and applies MIDI
    /// to mapped controls and to MIDI learn.
    fn poll_midi(&mut self, ui: &Ui, now: f64) {
        if now - self.midi_scanned > 3.0 {
            self.midi_scanned = now;
            let ctx = ui.ctx().clone();
            self.midi.rescan(&self.audio.midi, &self.shared, move || {
                ctx.request_repaint()
            });
        }
        self.graph.update_midi(&self.audio.midi);
        let events = self.audio.midi.events();
        if events != self.midi_events.0 {
            self.midi_events = (events, now);
        }
    }

    fn file_shortcuts(&mut self, ui: &Ui, now: f64) {
        if ui.memory(|m| m.focused().is_some()) {
            return;
        }
        let cmd = ui.input_mut(|i| {
            let mut pressed = |mods, key| i.consume_shortcut(&KeyboardShortcut::new(mods, key));
            if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
                Some(FileCommand::SaveAs)
            } else if pressed(Modifiers::COMMAND, Key::S) {
                Some(FileCommand::Save)
            } else if pressed(Modifiers::COMMAND, Key::O) {
                Some(FileCommand::Open)
            } else if pressed(Modifiers::COMMAND, Key::N) {
                Some(FileCommand::New)
            } else {
                None
            }
        });
        if ui.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma))
        }) {
            self.settings_window.open = !self.settings_window.open;
        }
        if let Some(cmd) = cmd {
            self.run_file_command(cmd, now);
        }
    }

    /// Keeps the unsaved-changes marker and the window title current, and
    /// offers to save before the window closes.
    fn track_document(&mut self, ui: &Ui, now: f64) {
        if now - self.doc.checked_at > 0.3 {
            self.doc.checked_at = now;
            self.refresh_dirty(false);
        }
        let edited = if self.doc.dirty { " (edited)" } else { "" };
        let title = format!("{}{edited} — AudioBoard", self.doc.name());
        if title != self.window_title {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
        if ui.input(|i| i.viewport().close_requested()) && !self.confirm_discard(now) {
            ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
        }
    }

    /// Adds a palette entry in the middle of the view (in the track view:
    /// after the selected device, or as a new track).
    fn add_spawn(&mut self, spawn: Spawn) {
        if self.graph.tracks.view == View::Tracks {
            let e = &self.editor;
            self.tracks
                .add_spawn(spawn, &mut self.graph, &self.audio, &e.library, &e.plugins);
            return;
        }
        match spawn {
            Spawn::Node(ty) => {
                let p = self.editor.spawn_point(&self.graph, ty.size());
                let id = self.graph.add(ty, p, &self.audio);
                self.editor.select_only(id);
            }
            Spawn::Plugin(i) => {
                let Some(entry) = self.editor.plugins.get(i).cloned() else {
                    return;
                };
                let p = self.editor.spawn_point(&self.graph, plugin_node::SIZE);
                match self.graph.add_plugin(&entry.path, p, &self.audio) {
                    Ok(id) => self.editor.select_only(id),
                    Err(e) => self
                        .editor
                        .notices
                        .push(format!("Could not load {}: {e}", entry.name)),
                }
            }
            Spawn::Preset(i) => {
                if let Some(preset) = self.library.presets.get(i) {
                    let p = self.editor.spawn_point(&self.graph, vec2(240.0, 80.0));
                    let nodes = project::insert_preset(&mut self.graph, preset, p, &self.audio);
                    self.editor.selected = nodes.into_iter().collect();
                }
            }
        }
    }

    /// The palette's plugin list, from what is known now.
    fn refresh_plugins(&mut self) {
        self.editor.plugins = plugin_entries(
            &self.plugin_catalog,
            &self.settings.plugins,
            &self.audio_units,
        );
    }

    /// The Audio Units listed in the background at launch, once there.
    fn take_au_scan(&mut self) {
        if let Some(units) = self.au_scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.au_scan = None;
            self.audio_units = units;
            self.refresh_plugins();
        }
    }

    /// Notes the category and maker of every plugin that has been loaded,
    /// so the palette can show them without loading plugins to list them.
    fn remember_plugins(&mut self) {
        let mut changed = false;
        for n in &self.graph.nodes {
            if let NodeKind::Plugin(p) = &n.kind {
                let key = p.meta.path.to_string_lossy().into_owned();
                if let std::collections::btree_map::Entry::Vacant(slot) =
                    self.plugin_catalog.entry(key)
                {
                    slot.insert(PluginInfo {
                        category: plugin_node::category_label(&p.meta.category),
                        vendor: p.meta.vendor.clone(),
                    });
                    changed = true;
                }
            }
        }
        if changed {
            save_catalog(&self.plugin_catalog);
            self.refresh_plugins();
        }
    }

    /// Switches between the node view and the track view (V): the same
    /// patch, the selected node carried across.
    fn toggle_view(&mut self) {
        match self.graph.tracks.view {
            View::Nodes => {
                self.graph.tracks.view = View::Tracks;
                if self.editor.selected.len() == 1 {
                    self.tracks.selected = self.editor.selected.iter().next().copied();
                }
            }
            View::Tracks => {
                self.graph.tracks.view = View::Nodes;
                if let Some(id) = self.tracks.selected {
                    self.editor.select_only(id);
                }
            }
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, now: f64) {
        // The bar doubles as the window title bar on macOS.
        let bar = ui.max_rect();
        let drag = ui.interact(bar, Id::unique("titlebar"), Sense::CLICK | Sense::DRAG);
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            let max = ui.input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
        }

        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if cfg!(target_os = "macos") {
                ui.add_space(66.0);
            }
            logo(ui);
            ui.add_space(12.0);
            vsep(ui);
            ui.add_space(8.0);

            let tip = if self.show_palette {
                "Hide palette"
            } else {
                "Show palette"
            };
            if icon_button(ui, Icon::Sidebar, self.show_palette)
                .on_hover_text(tip)
                .clicked()
            {
                self.show_palette = !self.show_palette;
            }
            ui.add_space(4.0);
            let doc = doc_button(ui, &self.doc.name(), self.doc.dirty);
            let recent = &self.settings.recent;
            if let Some(cmd) = egui::Popup::menu(&doc)
                .show(|ui| file_menu(ui, recent))
                .and_then(|r| r.inner)
            {
                self.run_file_command(cmd, now);
            }
            ui.add_space(8.0);
            vsep(ui);
            ui.add_space(8.0);
            let tracks = self.graph.tracks.view == View::Tracks;
            if icon_button(ui, Icon::Nodes, !tracks)
                .on_hover_text("Node view: the patch on a free canvas (V)")
                .clicked()
                && tracks
            {
                self.toggle_view();
            }
            if icon_button(ui, Icon::Tracks, tracks)
                .on_hover_text("Track view: each track's chain in a row (V)")
                .clicked()
                && !tracks
            {
                self.toggle_view();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let following = self.following().is_some();
                if transport_button(ui, self.running(), following).clicked() {
                    self.transport_pressed(now);
                }
                ui.add_space(4.0);
                let recording = self.recorder.recording();
                if record_button(ui, recording, self.recorder.seconds(), now).clicked() {
                    self.toggle_recording(now);
                }
                if !recording && self.recorder.take.is_some() {
                    ui.add_space(2.0);
                    let export = ui
                        .add(
                            egui::Button::new(
                                RichText::new("EXPORT").size(10.5).color(theme::TEXT_DIM),
                            )
                            .corner_radius(10.0),
                        )
                        .on_hover_text("Export the last recording");
                    if export.clicked() {
                        self.export.open = true;
                    }
                }
                ui.add_space(6.0);
                let on_output = self.output_control(ui);
                ui.add_space(6.0);
                let beat = self.running().then(|| self.shared.beat.get());
                let on_tempo = match self.settings.sync.source.clone() {
                    Some(source) => {
                        let t = crate::audio::sync::now_ns();
                        let clock = self.shared.sync.clock();
                        let bpm = clock.alive(t).then(|| clock.bpm() as f32);
                        if self.tempo.show_external(ui, bpm, beat, &source, now) {
                            self.settings_window.open = true;
                            self.settings_window.tab = super::settings::Tab::Sync;
                        }
                        on_output
                    }
                    None => self.tempo.show(ui, &mut self.graph.tempo, beat, now) || on_output,
                };
                // Dragging the BPM or the output (or selecting typed text)
                // must not drag the window: the toolbar is also the title bar.
                #[cfg(target_os = "macos")]
                if let Some(w) = &self.window {
                    w.set_movable(!on_tempo);
                }
                #[cfg(not(target_os = "macos"))]
                let _ = on_tempo;
                ui.add_space(8.0);
                if icon_button(ui, Icon::Monitor, self.show_monitor)
                    .on_hover_text(if self.show_monitor {
                        "Hide analyzers"
                    } else {
                        "Show analyzers"
                    })
                    .clicked()
                {
                    self.show_monitor = !self.show_monitor;
                }
                let canvas = self.graph.tracks.view == View::Nodes;
                let all_collapsed = !self.graph.nodes.is_empty()
                    && self.graph.nodes.iter().all(|n| n.collapsed)
                    && self.graph.groups.iter().all(|g| g.collapsed);
                let tip = if all_collapsed {
                    "Expand everything (Shift+C)"
                } else {
                    "Collapse everything (Shift+C)"
                };
                // Canvas tools: only in the node view.
                if canvas
                    && icon_button(ui, Icon::Compact, all_collapsed)
                        .on_hover_text(tip)
                        .clicked()
                {
                    self.graph.set_all_collapsed(!all_collapsed);
                }
                let tip = "Auto-fit: keep the whole patch in view (A)";
                if canvas
                    && icon_button(ui, Icon::Fit, self.editor.auto_fit)
                        .on_hover_text(tip)
                        .clicked()
                {
                    self.editor
                        .set_auto_fit(!self.editor.auto_fit, &self.graph, now);
                }
                let tip = "Auto layout: arrange by signal flow (L)";
                if canvas
                    && icon_button(ui, Icon::Layout, false)
                        .on_hover_text(tip)
                        .clicked()
                {
                    self.editor.auto_layout(&mut self.graph, now);
                }
                icon_button(ui, Icon::Help, false).on_hover_ui(shortcuts);
                let tip = "Settings: audio output and input (Cmd+,)";
                if icon_button(ui, Icon::Settings, self.settings_window.open)
                    .on_hover_text(tip)
                    .clicked()
                {
                    self.settings_window.open = !self.settings_window.open;
                }
                ui.add_space(10.0);
                let info = &self.audio.info;
                let (text, col, tip) = match &info.error {
                    Some(e) => ("Audio unavailable".to_owned(), theme::MUTE, e.clone()),
                    None => {
                        let input = info
                            .input
                            .as_ref()
                            .map_or(String::new(), |i| format!("\nInput: {i}"));
                        (
                            info.device.clone(),
                            theme::TEXT_DIM,
                            format!(
                                "Output: {}{input}\nClick for the audio settings",
                                info.device
                            ),
                        )
                    }
                };
                let device = ui
                    .add(
                        egui::Label::new(RichText::new(text).size(11.0).color(col))
                            .sense(Sense::click()),
                    )
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(tip);
                if device.clicked() {
                    self.settings_window.open = true;
                }
            });
        });
    }

    fn monitor_dock(&mut self, ui: &mut Ui, dt: f32) {
        let sr = self.audio.info.sample_rate as f32;
        let gap = 8.0;
        let size = vec2(
            ((ui.available_width() - gap) * 0.5).floor(),
            ui.available_height(),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            self.monitor.scope(ui, &self.shared, sr, size);
            self.monitor.spectrum(ui, sr, dt, size);
        });
    }

    fn run_view_command(&mut self, cmd: ViewCommand, now: f64) {
        let e = &mut self.editor;
        match cmd {
            ViewCommand::ZoomIn => e.zoom_by(1.25, now),
            ViewCommand::ZoomOut => e.zoom_by(0.8, now),
            ViewCommand::Actual => e.zoom_actual(now),
            ViewCommand::FitAll => e.fit_all(&self.graph, now),
            ViewCommand::FitSelection => e.fit_selection(&self.graph, now),
            ViewCommand::AutoFit => e.set_auto_fit(!e.auto_fit, &self.graph, now),
            ViewCommand::AutoLayout => e.auto_layout(&mut self.graph, now),
        }
    }

    fn status_bar(&mut self, ui: &mut Ui, now: f64) {
        ui.horizontal_centered(|ui| {
            let selected = &self.editor.selected;
            let single = (selected.len() == 1)
                .then(|| selected.iter().next().and_then(|id| self.graph.node(*id)))
                .flatten();
            let hint = if self.graph.tracks.view == View::Tracks {
                match self.tracks.selected.and_then(|id| self.graph.node(id)) {
                    Some(n) => format!("{} selected  ·  Del to remove (the chain closes up)  ·  C to open or close its panel  ·  Drag to move it", n.title),
                    None => "Drag devices to reorder  ·  Drop from the palette between two devices to insert  ·  Drag a track's name to move it  ·  V for the node view".to_owned(),
                }
            } else if let Some(g) = self.editor.selected_group(&self.graph) {
                format!(
                    "{} selected  ·  C to {}  ·  Cmd+D to duplicate  ·  Cmd+Shift+G to ungroup",
                    g.title,
                    if g.collapsed { "expand" } else { "collapse" }
                )
            } else if let Some(n) = single {
                format!(
                    "{} selected  ·  Del to delete  ·  Cmd+D to duplicate  ·  C to {}",
                    n.title,
                    if n.collapsed { "expand" } else { "collapse" }
                )
            } else if selected.len() > 1 {
                format!("{} nodes selected  ·  Cmd+G to make a module  ·  Del to delete", selected.len())
            } else {
                "Drag nodes from the palette  ·  Drag from an output to an input to connect  ·  Right-click for more"
                    .to_owned()
            };
            match self.toast.as_ref().filter(|t| t.until > now) {
                Some(t) => {
                    let col = if t.error { theme::MUTE } else { theme::TEXT };
                    ui.label(RichText::new(&t.text).size(10.5).color(col));
                }
                None => {
                    ui.label(RichText::new(hint).size(10.5).color(theme::TEXT_FAINT));
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                let item = |text: String| RichText::new(text).size(10.0).monospace().color(theme::TEXT_DIM);
                if self.graph.tracks.view == View::Nodes {
                    let auto = if self.editor.auto_fit { "  auto" } else { "" };
                    let zoom = ui
                        .add(egui::Label::new(item(format!("{:.0}%{auto}", self.editor.zoom * 100.0))).sense(Sense::click()))
                        .on_hover_cursor(CursorIcon::PointingHand);
                    if let Some(cmd) = egui::Popup::menu(&zoom).show(view_menu).and_then(|r| r.inner) {
                        self.run_view_command(cmd, now);
                    }
                }
                let devices: Vec<&str> = self.midi.names().collect();
                let active = now - self.midi_events.1 < 0.15;
                if active {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                }
                let (text, tip) = match (&self.midi.error, devices.len()) {
                    (Some(e), _) => ("MIDI off".to_owned(), e.clone()),
                    (None, 0) => ("MIDI —".to_owned(), "No MIDI input connected".to_owned()),
                    (None, n) => (format!("MIDI {n}"), devices.join("\n")),
                };
                let col = if active { theme::METER_GREEN } else { theme::TEXT_DIM };
                ui.label(item(text).color(col)).on_hover_text(tip);
                ui.label(item(format!(
                    "{} nodes · {} cables",
                    self.graph.nodes.len(),
                    self.graph.cables.len()
                )));
                self.audio_status(ui, now, &item);
            });
        });
    }
}

impl AudioBoardApp {
    /// Status bar, right to left: buffer size and latency (click to
    /// change), sample rate, DSP load with its recent peak, and dropouts.
    fn audio_status(&mut self, ui: &mut Ui, now: f64, item: &dyn Fn(String) -> RichText) {
        let info = &self.audio.info;
        let sr = info.sample_rate as f32;
        let ms = |frames: u32| frames as f32 / sr * 1000.0;
        let frames = self.shared.frames.load(Ordering::Relaxed);
        let buffer = ui
            .add(egui::Label::new(item(format!("{frames} smp · {:.1} ms", ms(frames)))).sense(Sense::click()))
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Audio buffer: larger gives heavy patches and plugins more headroom\n(fewer clicks), smaller gives less latency. Click to change.");
        let range = info.buffer_range;
        let current = info.buffer_size;
        let picked = egui::Popup::menu(&buffer)
            .show(|ui| {
                ui.label(
                    RichText::new("Buffer size")
                        .size(10.0)
                        .color(theme::TEXT_FAINT),
                );
                let mut picked = None;
                let mut option = |ui: &mut Ui, size: Option<u32>, label: String| {
                    let mark = if size == current { "●  " } else { "    " };
                    if ui
                        .button(RichText::new(format!("{mark}{label}")).size(11.5))
                        .clicked()
                    {
                        picked = Some(size);
                        ui.close();
                    }
                };
                option(ui, None, "Device default".to_owned());
                for size in BUFFER_SIZES {
                    if range.is_none_or(|(lo, hi)| (lo..=hi).contains(&size)) {
                        option(
                            ui,
                            Some(size),
                            format!("{size} samples · {:.1} ms", ms(size)),
                        );
                    }
                }
                picked
            })
            .and_then(|r| r.inner);
        if let Some(size) = picked {
            match self.audio.set_buffer_size(size) {
                Ok(()) => self.notify(now, "Audio buffer changed", false),
                Err(e) => self.notify(now, format!("Could not use that buffer size: {e}"), true),
            }
            self.settings.audio = self.audio.config.clone();
            save_settings(&self.settings);
        }
        let info = &self.audio.info;
        ui.label(item(format!(
            "{:.1} kHz · {} ch",
            info.sample_rate as f32 / 1000.0,
            info.channels
        )));

        // Load: the average, and the worst callback of the last two seconds.
        let peak = self.shared.peak_load.take();
        if peak >= self.peak_load.0 || now - self.peak_load.1 > 2.0 {
            self.peak_load = (peak, now);
        }
        let load = self.shared.load.get();
        let color = |l: f32| {
            if l > 0.85 {
                theme::METER_RED
            } else if l > 0.5 {
                theme::METER_YELLOW
            } else {
                theme::TEXT_DIM
            }
        };
        ui.label(
            item(format!("peak {:>3.0}%", self.peak_load.0 * 100.0)).color(color(self.peak_load.0)),
        )
        .on_hover_text(
            "The heaviest audio callback of the last seconds: near 100% the audio drops out",
        );
        ui.label(item(format!("DSP {:>4.1}%", load * 100.0)).color(color(load)));

        // The output limiter, while it works.
        let reduction = self.shared.limiting.take();
        if reduction >= self.limiting.0 || now - self.limiting.1 > 2.0 {
            self.limiting = (reduction, now);
        }
        if self.limiting.0 > 0.1 {
            ui.label(item(format!("limit −{:.1} dB", self.limiting.0)).color(theme::METER_YELLOW)).on_hover_text(
                "The output is louder than full scale, so the limiter turns it down\n(instead of clipping): lower the master or the plugins",
            );
        }

        // Dropouts: shown for a while after each one.
        let total = self.shared.overloads.load(Ordering::Relaxed);
        if total != self.overloads.0 {
            self.overloads = (total, now);
        }
        if total > 0 && now - self.overloads.1 < 10.0 {
            ui.label(
                item(format!(
                    "{total} dropout{}",
                    if total == 1 { "" } else { "s" }
                ))
                .color(theme::METER_RED),
            )
            .on_hover_text("The audio could not keep up: try a larger buffer size");
        }
    }
}

/// Frame interval for meters, analyzers and step lights when nothing
/// else asks for a frame: smooth to the eye at a fraction of the work.
const AMBIENT_FRAME: std::time::Duration = std::time::Duration::from_millis(33);

impl eframe::App for AudioBoardApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let (dt, now) = ui.input(|i| (i.stable_dt.min(0.1), i.time));
        self.file_shortcuts(ui, now);
        self.poll_midi(ui, now);
        self.shared.tempo.set(self.graph.tempo);
        for node in &mut self.graph.nodes {
            node.update_meters(dt);
        }
        if ui.memory(|m| m.focused().is_none()) {
            if ui.input(|i| i.key_pressed(Key::Space)) {
                self.transport_pressed(now);
            }
            if ui.input(|i| i.key_pressed(Key::T) && !i.modifiers.any())
                && self.following().is_none()
            {
                self.tempo.tap(&mut self.graph.tempo, now);
            }
            if ui.input(|i| i.key_pressed(Key::R) && !i.modifiers.any()) {
                self.toggle_recording(now);
            }
            if ui.input(|i| i.key_pressed(Key::V) && !i.modifiers.any()) {
                self.toggle_view();
            }
        }

        let bar_frame = |margin: Margin| Frame::NONE.fill(theme::PANEL).inner_margin(margin);
        Panel::top("toolbar")
            .exact_size(36.0)
            .frame(bar_frame(Margin::symmetric(10, 0)))
            .show(ui, |ui| self.toolbar(ui, now));
        Panel::bottom("status")
            .exact_size(22.0)
            .frame(bar_frame(Margin::symmetric(10, 0)))
            .show(ui, |ui| self.status_bar(ui, now));
        if self.show_monitor {
            Panel::bottom("monitor")
                .exact_size(150.0)
                .resizable(false)
                .frame(bar_frame(Margin::same(8)))
                .show(ui, |ui| self.monitor_dock(ui, dt));
        }
        if self.show_palette {
            let picked = Panel::left("palette")
                .exact_size(palette::WIDTH)
                .resizable(false)
                .frame(bar_frame(Margin::same(8)))
                .show(ui, |ui| {
                    let e = &mut self.editor;
                    self.palette
                        .show(ui, &e.library, &e.plugins, &mut e.requests)
                })
                .inner;
            if let Some(spawn) = picked {
                self.add_spawn(spawn);
            }
        }
        CentralPanel::default()
            .frame(Frame::NONE.fill(theme::BG))
            .show(ui, |ui| match self.graph.tracks.view {
                View::Nodes => self.editor.show(ui, &mut self.graph, &self.audio),
                View::Tracks => {
                    let e = &self.editor;
                    self.tracks
                        .show(ui, &mut self.graph, &self.audio, &e.library, &e.plugins)
                }
            });
        let notices = std::mem::take(&mut self.editor.notices);
        for notice in notices.into_iter().chain(std::mem::take(&mut self.tracks.notices)) {
            self.notify(now, notice, false);
        }
        self.export_window(ui.ctx(), now);
        let cx = SettingsContext {
            audio: &self.audio,
            shared: &self.shared,
            midi_inputs: self.midi.activity(),
            midi_error: self.midi.error.as_deref(),
            sync: &self.settings.sync,
            clock_out: self.clock_out.connected(),
            plugins: &self.settings.plugins,
            plugin_counts: plugin_counts(&self.editor.plugins),
        };
        let changes = self.settings_window.show(ui.ctx(), &cx, now, dt);
        for change in changes {
            match change {
                SettingsChange::Audio(config) => self.apply_audio(config, now),
                SettingsChange::Sync(sync) => self.apply_sync(sync),
                SettingsChange::Plugins(plugins) => {
                    self.settings.plugins = plugins;
                    save_settings(&self.settings);
                    self.refresh_plugins();
                }
                SettingsChange::RescanPlugins => {
                    self.handle_request(super::canvas::Request::RescanPlugins, now)
                }
            }
        }
        self.keep_audio_in_step();
        if self.recorder.recording() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let requests = std::mem::take(&mut self.editor.requests);
        for request in requests.into_iter().chain(std::mem::take(&mut self.tracks.requests)) {
            self.handle_request(request, now);
        }
        self.graph.service_plugins(now);
        self.audio.collect_garbage();
        self.take_au_scan();
        self.remember_plugins();
        self.track_document(ui, now);

        // Meters, analyzers and step lights move while audio runs or a
        // signal is dying away: redraw them at an easy pace. Input and
        // animations still get every frame (egui and the canvas ask).
        let minimized = ui.input(|i| i.viewport().minimized == Some(true));
        if !minimized
            && (self.running() || self.graph.nodes.iter().any(|n| n.signal_level() > 1e-3))
        {
            ui.ctx().request_repaint_after(AMBIENT_FRAME);
        }
    }
}

impl AudioBoardApp {
    /// Switches the audio to `config` and keeps it for the next launch.
    fn apply_audio(&mut self, config: AudioConfig, now: f64) {
        if let Err(e) = self.audio.apply(config) {
            self.notify(now, format!("Audio: {e}"), true);
        }
        self.settings.audio = self.audio.config.clone();
        save_settings(&self.settings);
        self.keep_audio_in_step();
    }

    /// Follows the clock chosen in Settings → Sync and sends the board's to
    /// the outputs chosen there; keeps the choice for the next launch.
    fn apply_sync(&mut self, sync: SyncSettings) {
        let was_following = self.settings.sync.source.is_some();
        self.midi
            .set_sync_source(sync.source.clone(), &self.shared.sync);
        self.clock_out.set_outputs(clock_outputs(&sync));
        // Back to the board's own clock: stopped, at the start.
        if was_following && sync.source.is_none() && self.running() {
            self.toggle_running();
        }
        self.settings.sync = sync;
        save_settings(&self.settings);
    }

    /// Every frame: the input open while an Audio In node (or the settings
    /// window) listens; every node built again after the engine changed
    /// (another sample rate).
    fn keep_audio_in_step(&mut self) {
        let listening = self.settings_window.open
            || self.graph.nodes.iter().any(
                |n| matches!(&n.kind, NodeKind::Module(m) if m.params.kind == ModuleKind::AudioIn),
            );
        self.audio.want_input(listening);
        if self.audio.generation != self.generation {
            self.generation = self.audio.generation;
            self.rebuild_nodes();
        }
    }

    /// Builds every node again, as it is, for the current engine.
    fn rebuild_nodes(&mut self) {
        let clean = !self.doc.dirty;
        let snapshot = project::snapshot(&self.graph);
        self.graph.clear(&self.audio);
        self.graph.release_plugin_host();
        project::restore(&mut self.graph, &snapshot, &self.audio);
        self.editor.reset();
        self.tracks.reset();
        // The same patch: still saved if it was (its nodes have new ids).
        if clean {
            self.doc.saved = project::snapshot(&self.graph);
        }
    }
}

// ---------------------------------------------------------------------------
// Demo patch
// ---------------------------------------------------------------------------

fn module(graph: &Graph, id: NodeId) -> Arc<ModuleParams> {
    match &graph.node(id).expect("node just created").kind {
        NodeKind::Module(m) => m.params.clone(),
        _ => unreachable!("not a module"),
    }
}

/// An ambient pad: a drone through a slowly wandering filter, spread into a
/// shimmering reverb and, an octave up, into a grain cloud with tape echoes.
pub(crate) fn demo_patch(graph: &mut Graph, audio: &AudioHandle) {
    let random = graph.add_module(ModuleKind::Random, pos2(0.0, 0.0), audio);
    let drone_id = graph.add_module(ModuleKind::Drone, pos2(0.0, 230.0), audio);
    let filter_id = graph.add_module(ModuleKind::Filter, pos2(340.0, 150.0), audio);
    let grains = graph.add_module(ModuleKind::Granular, pos2(680.0, 0.0), audio);
    let reverb_id = graph.add_module(ModuleKind::Reverb, pos2(712.0, 310.0), audio);
    let delay_id = graph.add_module(ModuleKind::Delay, pos2(1020.0, 12.0), audio);
    let mixer = graph.add_mixer(pos2(1300.0, 200.0), audio);

    let p = module(graph, random);
    p.set(random::RATE, 0.15);
    p.set(random::CHAOS, 0.4);
    p.set(random::SMOOTH, 1.0);

    let p = module(graph, drone_id);
    p.set_mode(1); // fifths
    p.set(drone::PITCH, 55.0);
    p.set(drone::DETUNE, 14.0);
    p.set(drone::DRIFT, 0.45);
    p.set(drone::TONE, 0.45);
    p.set(drone::SPREAD, 0.8);
    p.set(drone::LEVEL, 0.55);

    let p = module(graph, filter_id);
    p.set(filter::CUTOFF, 700.0);
    p.set(filter::RESO, 0.35);
    p.set(filter::MOD, 1.5);

    let p = module(graph, grains);
    p.set(granular::POSITION, 1.2);
    p.set(granular::SIZE, 0.22);
    p.set(granular::DENSITY, 9.0);
    p.set(granular::PITCH, 12.0);
    p.set(granular::JITTER, 0.6);
    p.set(granular::SPRAY, 0.15);
    p.set(granular::SPREAD, 0.9);
    p.set(granular::MIX, 1.0);

    let p = module(graph, delay_id);
    p.set(delay::TIME, 0.62);
    p.set(delay::FEEDBACK, 0.6);
    p.set(delay::TONE, 0.45);
    p.set(delay::WOW, 0.35);
    p.set(delay::PING, 0.7);
    p.set(delay::MIX, 0.45);

    let p = module(graph, reverb_id);
    p.set(reverb::SIZE, 0.85);
    p.set(reverb::DECAY, 11.0);
    p.set(reverb::DAMP, 0.5);
    p.set(reverb::MOD, 0.5);
    p.set(reverb::SHIMMER, 0.15);
    p.set(reverb::MIX, 0.55);

    if let Some(NodeKind::Mixer(m)) = graph.node(mixer).map(|n| &n.kind) {
        m.params.fader[1].set(0.63); // grains about -6 dB under the pad
        m.params.pan[1].set(0.15);
    }

    let port = |node, port| PortRef { node, port };
    graph.connect(port(random, 0), port(filter_id, 1), audio);
    graph.connect(port(drone_id, 0), port(filter_id, 0), audio);
    graph.connect(port(filter_id, 0), port(grains, 0), audio);
    graph.connect(port(filter_id, 0), port(reverb_id, 0), audio);
    graph.connect(port(grains, 0), port(delay_id, 0), audio);
    graph.connect(port(reverb_id, 0), port(mixer, 0), audio);
    graph.connect(port(delay_id, 0), port(mixer, 1), audio);
}

// ---------------------------------------------------------------------------
// Toolbar widgets
// ---------------------------------------------------------------------------

/// Yes / no question in a native dialog.
fn ask(question: &str) -> bool {
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_description(question)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    matches!(answer, rfd::MessageDialogResult::Yes)
}

/// The project's name, opening the file menu. A dot marks unsaved changes.
fn doc_button(ui: &mut Ui, name: &str, dirty: bool) -> Response {
    let galley =
        ui.painter()
            .layout_no_wrap(name.to_owned(), FontId::proportional(12.0), theme::TEXT);
    let text_w = galley.size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(text_w + 40.0, 24.0), Sense::CLICK);
    let hot = resp.hovered();
    let p = ui.painter();
    if hot {
        p.rect_filled(rect, 4.0, theme::CONTROL);
    }
    let text_pos = pos2(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5);
    p.galley(
        text_pos,
        galley,
        if hot { theme::TEXT } else { theme::TEXT_DIM },
    );
    if dirty {
        p.circle_filled(
            pos2(text_pos.x + text_w + 8.0, rect.center().y),
            2.5,
            theme::TEXT_DIM,
        );
    }
    let c = pos2(rect.right() - 9.0, rect.center().y);
    p.add(Shape::convex_polygon(
        vec![
            c + vec2(-3.0, -1.5),
            c + vec2(3.0, -1.5),
            c + vec2(0.0, 2.0),
        ],
        theme::TEXT_FAINT,
        Stroke::NONE,
    ));
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(if dirty {
            "Project · unsaved changes"
        } else {
            "Project"
        })
}

fn file_menu(ui: &mut Ui, recent: &[PathBuf]) -> Option<FileCommand> {
    let item = |ui: &mut Ui, label: &str, hint: &str, cmd: FileCommand| {
        menu_item(ui, label, hint).then(|| {
            ui.close();
            cmd
        })
    };
    let mut picked = item(ui, "New project", "Cmd+N", FileCommand::New);
    picked = picked.or(item(ui, "Open…", "Cmd+O", FileCommand::Open));
    let submenu = ui.menu_button(
        RichText::new("Open Recent").color(theme::TEXT).size(12.5),
        |ui| recent::menu(ui, recent),
    );
    picked = picked.or(match submenu.inner.flatten() {
        Some(recent::RecentPick::Open(path)) => Some(FileCommand::OpenRecent(path)),
        Some(recent::RecentPick::Clear) => Some(FileCommand::ClearRecent),
        None => None,
    });
    ui.separator();
    picked = picked.or(item(ui, "Save", "Cmd+S", FileCommand::Save));
    picked.or(item(ui, "Save as…", "Shift+Cmd+S", FileCommand::SaveAs))
}

fn logo(ui: &mut Ui) {
    let galley =
        ui.painter()
            .layout_no_wrap("AudioBoard".to_owned(), theme::strong(12.5), theme::TEXT);
    let (rect, _) = ui.allocate_exact_size(vec2(26.0 + galley.size().x, 24.0), Sense::hover());
    let p = ui.painter();
    let icon = Rect::from_center_size(pos2(rect.left() + 9.0, rect.center().y), vec2(18.0, 18.0));
    p.rect_filled(icon, 4.0, theme::ACCENT);
    let pts = super::widgets::wave_points(
        icon.shrink2(vec2(3.5, 5.0)),
        Waveform::Sine,
        0.0,
        1.0,
        1.0,
        0.0,
        0,
    );
    p.add(Shape::line(pts, Stroke::new(1.5, theme::INSET)));
    p.galley(
        pos2(icon.right() + 8.0, rect.center().y - galley.size().y * 0.5),
        galley,
        theme::TEXT,
    );
}

fn vsep(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL_BORDER);
}

enum Icon {
    Sidebar,
    /// The node view: blocks joined by a cable.
    Nodes,
    /// The track view: rows of devices.
    Tracks,
    Layout,
    Fit,
    Monitor,
    Compact,
    Help,
    Settings,
}

fn icon_button(ui: &mut Ui, icon: Icon, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::CLICK);
    let p = ui.painter();
    if active || resp.hovered() {
        p.rect_filled(
            rect,
            4.0,
            if resp.hovered() {
                theme::CONTROL_HI
            } else {
                theme::CONTROL
            },
        );
    }
    let col = if active || resp.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    let c = rect.center();
    match icon {
        Icon::Sidebar => {
            // A window with its left panel.
            let r = Rect::from_center_size(c, vec2(13.0, 11.0));
            p.rect_stroke(r, 2.0, Stroke::new(1.2, col), StrokeKind::Inside);
            p.vline(r.left() + 4.5, r.y_range(), Stroke::new(1.2, col));
        }
        Icon::Nodes => {
            // Two blocks joined by a curved cable.
            let stroke = Stroke::new(1.2, col);
            let a = Rect::from_center_size(c + vec2(-4.0, -3.0), vec2(5.0, 5.0));
            let b = Rect::from_center_size(c + vec2(4.0, 3.0), vec2(5.0, 5.0));
            p.rect_stroke(a, 1.0, stroke, StrokeKind::Inside);
            p.rect_stroke(b, 1.0, stroke, StrokeKind::Inside);
            p.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [a.right_center(), a.right_center() + vec2(3.0, 0.0), b.left_center() - vec2(3.0, 0.0), b.left_center()],
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
        }
        Icon::Tracks => {
            // Three rows, each a header and a chain of devices.
            let stroke = Stroke::new(1.2, col);
            for dy in [-4.5, 0.0, 4.5] {
                let y = c.y + dy;
                p.rect_filled(Rect::from_center_size(pos2(c.x - 5.5, y), vec2(2.5, 2.5)), 0.5, col);
                p.hline((c.x - 2.5)..=(c.x + 6.5), y, stroke);
            }
        }
        Icon::Layout => {
            // One block feeding two: a patch arranged left to right.
            let block = |x: f32, y: f32| Rect::from_center_size(c + vec2(x, y), vec2(4.5, 4.5));
            let stroke = Stroke::new(1.2, col);
            let (a, b, d) = (block(-4.5, 0.0), block(4.5, -3.5), block(4.5, 3.5));
            for r in [a, b, d] {
                p.rect_stroke(r, 1.0, stroke, StrokeKind::Inside);
            }
            p.line_segment([a.right_center(), b.left_center()], stroke);
            p.line_segment([a.right_center(), d.left_center()], stroke);
        }
        Icon::Fit => {
            // Four corners framing the view.
            let r = Rect::from_center_size(c, vec2(13.0, 11.0));
            let stroke = Stroke::new(1.2, col);
            for (corner, dx, dy) in [
                (r.left_top(), 1.0, 1.0),
                (r.right_top(), -1.0, 1.0),
                (r.left_bottom(), 1.0, -1.0),
                (r.right_bottom(), -1.0, -1.0),
            ] {
                p.line_segment([corner, corner + vec2(4.0 * dx, 0.0)], stroke);
                p.line_segment([corner, corner + vec2(0.0, 3.5 * dy)], stroke);
            }
        }
        Icon::Monitor => {
            // A window with its bottom dock.
            let r = Rect::from_center_size(c, vec2(13.0, 11.0));
            p.rect_stroke(r, 2.0, Stroke::new(1.2, col), StrokeKind::Inside);
            p.hline(r.x_range(), r.bottom() - 4.0, Stroke::new(1.2, col));
        }
        Icon::Compact => {
            // Two slim cards: nodes folded to their headers.
            for dy in [-3.0, 3.0] {
                let r = Rect::from_center_size(c + vec2(0.0, dy), vec2(13.0, 4.0));
                p.rect_stroke(r, 1.5, Stroke::new(1.2, col), StrokeKind::Inside);
            }
        }
        Icon::Settings => {
            // Three sliders: a mixer's faders, the audio setup.
            let stroke = Stroke::new(1.2, col);
            for (dx, knob) in [(-4.0, 2.0), (0.0, -2.5), (4.0, 1.0)] {
                p.vline(c.x + dx, (c.y - 6.0)..=(c.y + 6.0), stroke);
                p.rect_filled(
                    Rect::from_center_size(c + vec2(dx, knob), vec2(4.0, 2.6)),
                    1.0,
                    col,
                );
            }
        }
        Icon::Help => {
            p.circle_stroke(c, 6.5, Stroke::new(1.2, col));
            p.text(
                c + vec2(0.0, 0.5),
                Align2::CENTER_CENTER,
                "?",
                theme::strong(9.5),
                col,
            );
        }
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// The export window's state.
#[derive(Default)]
struct ExportWindow {
    open: bool,
    options: export::Options,
    job: Option<std::thread::JoinHandle<Result<PathBuf, String>>>,
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Minutes and seconds.
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// REC: a red dot; while recording, the time so far on red, the dot
/// pulsing.
fn record_button(ui: &mut Ui, recording: bool, seconds: f64, now: f64) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        vec2(if recording { 74.0 } else { 58.0 }, 24.0),
        Sense::CLICK,
    );
    let p = ui.painter();
    let hot = resp.hovered();
    let red = theme::MUTE;
    let (fill, border) = if recording {
        (alpha(red, if hot { 0.22 } else { 0.14 }), alpha(red, 0.6))
    } else {
        (
            if hot {
                theme::CONTROL
            } else {
                Color32::TRANSPARENT
            },
            theme::NODE_BORDER,
        )
    };
    p.rect(
        rect,
        12.0,
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let c = pos2(rect.left() + 14.0, rect.center().y);
    let pulse = if recording {
        0.55 + 0.45 * (now * 4.0).sin().abs() as f32
    } else {
        1.0
    };
    p.circle_filled(c, 4.0, alpha(red, pulse));
    let text = if recording {
        clock(seconds)
    } else {
        "REC".to_owned()
    };
    p.text(
        pos2(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::monospace(11.0),
        theme::TEXT,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(if recording {
            "Stop recording, and export the take (R)"
        } else {
            "Record what the board plays (R)"
        })
}

/// Start / Stop: a play triangle while stopped; while playing, a stop
/// square on green. Following a MIDI clock it shows that clock's state,
/// marked EXT.
fn transport_button(ui: &mut Ui, playing: bool, external: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(74.0, 24.0), Sense::CLICK);
    let p = ui.painter();
    let hot = resp.hovered();
    let (fill, border) = if playing {
        (
            alpha(theme::METER_GREEN, if hot { 0.18 } else { 0.10 }),
            alpha(theme::METER_GREEN, 0.45),
        )
    } else {
        (
            if hot {
                theme::CONTROL
            } else {
                Color32::TRANSPARENT
            },
            theme::NODE_BORDER,
        )
    };
    p.rect(
        rect,
        12.0,
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let c = pos2(rect.left() + 15.0, rect.center().y);
    if external {
        p.text(
            pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            "EXT",
            FontId::proportional(8.5),
            theme::ACCENT,
        );
    }
    if playing {
        p.rect_filled(
            Rect::from_center_size(c, vec2(7.0, 7.0)),
            1.5,
            theme::METER_GREEN,
        );
    } else {
        let triangle = vec![
            c + vec2(-3.0, -4.5),
            c + vec2(4.5, 0.0),
            c + vec2(-3.0, 4.5),
        ];
        p.add(Shape::convex_polygon(
            triangle,
            theme::METER_GREEN,
            Stroke::NONE,
        ));
    }
    p.text(
        pos2(rect.left() + 26.0, rect.center().y),
        Align2::LEFT_CENTER,
        if playing { "Stop" } else { "Start" },
        FontId::proportional(11.5),
        theme::TEXT,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(if external {
            "The transport follows a MIDI clock: its Start and Stop (Settings → Sync)"
        } else if playing {
            "Stop, and go back to the start (Space)"
        } else {
            "Play from the start (Space)"
        })
}

fn shortcuts(ui: &mut Ui) {
    ui.label(RichText::new("Shortcuts").size(11.5).color(theme::TEXT));
    ui.add_space(4.0);
    egui::Grid::new("shortcuts")
        .num_columns(2)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            for &(keys, what) in SHORTCUTS {
                ui.label(
                    RichText::new(keys)
                        .size(10.5)
                        .monospace()
                        .color(theme::TEXT),
                );
                ui.label(RichText::new(what).size(11.0).color(theme::TEXT_DIM));
                ui.end_row();
            }
        });
}

fn view_menu(ui: &mut Ui) -> Option<ViewCommand> {
    let mut picked = None;
    let mut item = |label: &str, hint: &str, cmd: ViewCommand| {
        if menu_item(ui, label, hint) {
            picked = Some(cmd);
            ui.close();
        }
    };
    item("Zoom in", "Cmd+=", ViewCommand::ZoomIn);
    item("Zoom out", "Cmd+−", ViewCommand::ZoomOut);
    item("Actual size", "Cmd+0", ViewCommand::Actual);
    item("Fit to view", "F", ViewCommand::FitAll);
    item("Fit selection", "Shift+F", ViewCommand::FitSelection);
    item("Auto-fit on / off", "A", ViewCommand::AutoFit);
    item("Auto layout", "L", ViewCommand::AutoLayout);
    picked
}

fn catalog_path() -> Option<PathBuf> {
    project::data_dir().map(|d| d.join("plugins.json"))
}

/// Preferences kept between launches.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Debug, PartialEq)]
pub(crate) struct AppSettings {
    /// The audio devices, rate and buffer size.
    #[serde(default)]
    pub(crate) audio: AudioConfig,
    /// The board's output level, dB.
    #[serde(default)]
    pub(crate) output_db: f32,
    /// Open Recent: projects opened or saved, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) recent: Vec<PathBuf>,
    /// MIDI sync: the clock followed and the outputs the clock goes to.
    #[serde(default)]
    pub(crate) sync: SyncSettings,
    /// Which plugins are listed (Audio Units, Apple's own).
    #[serde(default)]
    pub(crate) plugins: PluginSettings,
    /// Before the audio settings: the buffer size alone (read, then moved
    /// into `audio`).
    #[serde(default, skip_serializing)]
    buffer_size: Option<u32>,
}

impl AppSettings {
    /// Settings from the file's text (older files too).
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut settings: Self = serde_json::from_str(text).ok()?;
        if let Some(size) = settings.buffer_size.take() {
            settings.audio.buffer_size.get_or_insert(size);
        }
        Some(settings)
    }
}

fn settings_path() -> Option<PathBuf> {
    project::data_dir().map(|d| d.join("settings.json"))
}

fn load_settings() -> AppSettings {
    settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| AppSettings::parse(&text))
        .unwrap_or_default()
}

fn save_settings(settings: &AppSettings) {
    if let Some(path) = settings_path()
        && let Ok(json) = serde_json::to_string_pretty(settings)
    {
        let _ = path.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(path, json);
    }
}

/// What a plugin's first load told us about it.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
struct PluginInfo {
    /// "Fx · Delay".
    category: String,
    vendor: String,
}

fn load_catalog() -> BTreeMap<String, PluginInfo> {
    catalog_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| parse_catalog(&text))
        .unwrap_or_default()
}

/// The catalog file; older ones held "Fx · Delay · Maker" strings.
fn parse_catalog(text: &str) -> BTreeMap<String, PluginInfo> {
    if let Ok(catalog) = serde_json::from_str(text) {
        return catalog;
    }
    let old: BTreeMap<String, String> = serde_json::from_str(text).unwrap_or_default();
    old.into_iter()
        .map(|(path, hint)| {
            let (category, vendor) = hint.rsplit_once(" · ").unwrap_or((hint.as_str(), ""));
            (
                path,
                PluginInfo {
                    category: category.to_owned(),
                    vendor: vendor.to_owned(),
                },
            )
        })
        .collect()
}

fn save_catalog(catalog: &BTreeMap<String, PluginInfo>) {
    if let Some(path) = catalog_path()
        && let Ok(json) = serde_json::to_string_pretty(catalog)
    {
        let _ = path.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(path, json);
    }
}

/// The clock-out outputs: those chosen, but not the clock source itself
/// (a device that sends clock and hears it back would loop).
fn clock_outputs(sync: &SyncSettings) -> Vec<String> {
    sync.clock_out
        .iter()
        .filter(|o| sync.source.as_ref() != Some(*o))
        .cloned()
        .collect()
}

/// Creates `dir` if needed and opens it in the file manager.
fn reveal(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(dir)
        .spawn()
        .map(|_| ())
}

fn plugin_counts(plugins: &[PluginEntry]) -> PluginCounts {
    let mut c = PluginCounts::default();
    for p in plugins {
        match p.format {
            Format::Vst3 => c.vst3 += 1,
            Format::AudioUnit { v3 } => {
                c.au += 1;
                c.auv3 += v3 as usize;
            }
        }
    }
    c
}

/// Installed plugins, with what is known about each: for VST3, what their
/// first load told, else the maker named in the bundle's files; Audio
/// Units (if listed) as the system lists them. By name, VST3 first.
fn plugin_entries(
    catalog: &BTreeMap<String, PluginInfo>,
    settings: &PluginSettings,
    audio_units: &[crate::audio::au::Listed],
) -> Vec<PluginEntry> {
    use crate::audio::plugin::{bundle_name, bundle_vendor, clean_vendor};
    let mut entries: Vec<PluginEntry> = crate::audio::plugin::scan()
        .into_iter()
        .map(|path| {
            let known = catalog.get(path.to_string_lossy().as_ref());
            let vendor = known
                .map(|k| clean_vendor(&k.vendor))
                .filter(|v| !v.is_empty())
                .or_else(|| bundle_vendor(&path))
                .unwrap_or_default();
            PluginEntry {
                name: bundle_name(&path),
                hint: known.map_or_else(|| "VST3".to_owned(), |k| k.category.clone()),
                vendor,
                path,
                format: Format::Vst3,
            }
        })
        .collect();
    if settings.audio_units {
        let apple = u32::from_be_bytes(*b"appl");
        for unit in audio_units {
            if !settings.apple_units && unit.component.manufacturer == apple {
                continue;
            }
            let format = Format::AudioUnit { v3: unit.v3 };
            entries.push(PluginEntry {
                path: unit.component.path(),
                hint: format!(
                    "{} · {}",
                    format.label(),
                    plugin_node::category_label(unit.component.category())
                ),
                vendor: clean_vendor(&unit.vendor),
                name: unit.name.clone(),
                format,
            });
        }
    }
    entries.sort_by_key(|e| (e.name.to_lowercase(), e.format != Format::Vst3));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_catalogs_old_and_new() {
        let old = r#"{ "/a.vst3": "Fx · Delay · Valhalla DSP, LLC", "/b.vst3": "VST3" }"#;
        let catalog = parse_catalog(old);
        assert_eq!(
            catalog["/a.vst3"],
            PluginInfo {
                category: "Fx · Delay".into(),
                vendor: "Valhalla DSP, LLC".into()
            }
        );
        assert_eq!(catalog["/b.vst3"].vendor, "");
        let new = serde_json::to_string(&catalog).unwrap();
        assert_eq!(parse_catalog(&new), catalog);
    }
}
