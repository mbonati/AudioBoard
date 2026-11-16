//! The Settings window (Cmd+,), in four tabs down its left side:
//! - **Audio**: the audio system, the output (device, the channel pair the
//!   board plays on, sample rate, buffer size) and the input the Audio In
//!   nodes listen to, with a meter per input channel;
//! - **MIDI**: the MIDI inputs connected, with their activity;
//! - **Sync**: the transport's clock (the board's own, or a MIDI input's)
//!   and the MIDI outputs the board's clock is sent to;
//! - **Plugins**: where VST3 plugins are looked for, and which Audio Units
//!   are listed.
//!
//! Every tab is made of the same pieces: titled cards of rows, a label
//! column on the left and the control on the right, a status line under.
//! Changes apply at once and are kept.

use std::collections::HashMap;

use egui::{Align2, Color32, ComboBox, CursorIcon, FontId, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use super::theme::{self, alpha};
use crate::audio::AudioHandle;
use crate::audio::device::{self, AudioConfig, BUFFER_SIZES, DeviceList, InputChoice};
use crate::audio::input::MAX_CHANNELS;
use crate::audio::midi::InputActivity;
use crate::audio::params::EngineShared;
use crate::audio::sync::now_ns;

/// Read the devices again this often while the window is open: they come
/// and go (a USB interface plugged in).
const RELIST_SECONDS: f64 = 3.0;

/// The window's content size.
const SIZE: egui::Vec2 = vec2(700.0, 480.0);
const SIDEBAR_W: f32 = 150.0;
/// The label column of every row.
const LABEL_W: f32 = 118.0;
const COMBO_W: f32 = 320.0;
const ROW_H: f32 = 26.0;

/// MIDI sync, kept between launches.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Debug, PartialEq)]
pub struct SyncSettings {
    /// The MIDI input whose clock the transport follows (None: the board's
    /// own tempo and Start/Stop).
    #[serde(default)]
    pub source: Option<String>,
    /// MIDI outputs the board's clock is sent to.
    #[serde(default)]
    pub clock_out: Vec<String>,
}

/// Which plugins are listed, kept between launches.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct PluginSettings {
    /// List Audio Units (macOS) next to VST3 plugins.
    #[serde(default = "yes")]
    pub audio_units: bool,
    /// Apple's own Audio Units (AUDelay, AUReverb2, DLSMusicDevice…).
    #[serde(default = "yes")]
    pub apple_units: bool,
}

fn yes() -> bool {
    true
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self { audio_units: true, apple_units: true }
    }
}

/// What a change in the window asks the app for.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsChange {
    Audio(AudioConfig),
    Sync(SyncSettings),
    Plugins(PluginSettings),
    RescanPlugins,
}

/// What the window shows, from the app.
pub struct SettingsContext<'a> {
    pub audio: &'a AudioHandle,
    pub shared: &'a EngineShared,
    pub midi_inputs: Vec<InputActivity>,
    pub midi_error: Option<&'a str>,
    pub sync: &'a SyncSettings,
    /// The outputs the clock goes to now, and why one couldn't open.
    pub clock_out: (Vec<String>, Option<String>),
    pub plugins: &'a PluginSettings,
    /// Plugins listed: VST3, and Audio Units (v2, v3).
    pub plugin_counts: PluginCounts,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PluginCounts {
    pub vst3: usize,
    pub au: usize,
    pub auv3: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Audio,
    Midi,
    Sync,
    Plugins,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Audio, Tab::Midi, Tab::Sync, Tab::Plugins];

    fn title(self) -> &'static str {
        match self {
            Tab::Audio => "Audio",
            Tab::Midi => "MIDI",
            Tab::Sync => "Sync",
            Tab::Plugins => "Plugins",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Tab::Audio => "The sound card the board plays on and listens to",
            Tab::Midi => "Keyboards and controllers: every input plays the MIDI nodes",
            Tab::Sync => "Who keeps time: the board, or a MIDI clock",
            Tab::Plugins => "Where plugins are found, and which are listed",
        }
    }
}

/// When a MIDI input's counts last moved, for its lights.
#[derive(Clone, Copy, Default)]
struct Lights {
    events: u32,
    events_at: f64,
    clock: u32,
    clock_at: f64,
}

#[derive(Default)]
pub struct SettingsWindow {
    pub open: bool,
    pub tab: Tab,
    list: Option<DeviceList>,
    listed_at: f64,
    /// MIDI outputs on the system, read while the Sync tab shows.
    outputs: Option<Vec<String>>,
    outputs_at: f64,
    /// Input meters (falling slowly).
    levels: [f32; MAX_CHANNELS],
    lights: HashMap<String, Lights>,
    /// The external tempo shown (smoothed).
    shown_bpm: f32,
}

impl SettingsWindow {
    /// The window with a device list given rather than read (tests).
    #[cfg(test)]
    pub fn with_devices(list: DeviceList, outputs: Vec<String>) -> Self {
        Self { open: true, list: Some(list), listed_at: f64::INFINITY, outputs: Some(outputs), outputs_at: f64::INFINITY, ..Default::default() }
    }

    /// Draws the window; returns what the user changed.
    pub fn show(&mut self, ctx: &egui::Context, cx: &SettingsContext, now: f64, dt: f32) -> Vec<SettingsChange> {
        let mut changes = Vec::new();
        if !self.open {
            self.list = None;
            self.outputs = None;
            return changes;
        }
        if self.list.is_none() || now - self.listed_at > RELIST_SECONDS {
            self.list = Some(device::list(&cx.audio.config));
            self.listed_at = now;
        }
        if self.tab == Tab::Sync && (self.outputs.is_none() || now - self.outputs_at > RELIST_SECONDS) {
            self.outputs = Some(crate::audio::sync::output_names());
            self.outputs_at = now;
        }
        let peaks = cx.audio.shared.input.take_peaks();
        let fall = (-dt / 0.3).exp();
        for (level, peak) in self.levels.iter_mut().zip(peaks) {
            *level = peak.max(*level * fall);
        }
        for input in &cx.midi_inputs {
            let l = self.lights.entry(input.name.clone()).or_default();
            if input.events != l.events {
                (l.events, l.events_at) = (input.events, now);
            }
            if input.clock != l.clock {
                (l.clock, l.clock_at) = (input.clock, now);
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));

        let mut open = true;
        let mut relist = false;
        egui::Window::new("Settings")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .fixed_size(SIZE)
            .frame(egui::Frame::window(&ctx.style_of(egui::Theme::Dark)).inner_margin(0))
            .show(ctx, |ui| {
                ui.set_min_size(SIZE);
                ui.set_max_size(SIZE);
                // Outlined fields: combo boxes read as controls, flat.
                let v = ui.visuals_mut();
                v.widgets.inactive.bg_stroke = Stroke::new(1.0, theme::NODE_BORDER);
                v.widgets.hovered.bg_stroke = Stroke::new(1.0, theme::CONTROL_HI);
                v.widgets.open.bg_stroke = Stroke::new(1.0, theme::ACCENT);
                let full = Rect::from_min_size(ui.max_rect().min, SIZE);
                if close_button(ui, Rect::from_center_size(pos2(full.right() - 18.0, full.top() + 18.0), vec2(20.0, 20.0))).clicked()
                    || (!egui::Popup::is_any_open(ui.ctx()) && ui.input(|i| i.key_pressed(egui::Key::Escape)))
                {
                    open = false;
                }
                let side = Rect::from_min_size(full.min, vec2(SIDEBAR_W, SIZE.y));
                ui.painter().rect_filled(side, egui::CornerRadius { sw: 8, ..Default::default() }, theme::PANEL);
                ui.painter().vline(side.right(), side.y_range(), Stroke::new(1.0, theme::PANEL_BORDER));
                ui.scope_builder(egui::UiBuilder::new().max_rect(side.shrink2(vec2(8.0, 10.0))), |ui| self.sidebar(ui));
                let content = Rect::from_min_max(pos2(side.right() + 20.0, full.top() + 14.0), pos2(full.left() + SIZE.x - 20.0, full.top() + SIZE.y - 12.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                    ui.set_width(content.width());
                    ui.label(RichText::new(self.tab.title()).font(theme::strong(15.0)).color(theme::TEXT));
                    ui.label(RichText::new(self.tab.subtitle()).size(11.0).color(theme::TEXT_FAINT));
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(content.height() - 70.0).show(ui, |ui| {
                        ui.set_width(content.width() - 8.0);
                        match self.tab {
                            Tab::Audio => {
                                let list = self.list.clone().unwrap_or_default();
                                let mut config = cx.audio.config.clone();
                                output_card(ui, &list, cx.audio, &mut config);
                                ui.add_space(12.0);
                                input_card(ui, &list, cx.audio, &mut config, &self.levels);
                                if config != cx.audio.config {
                                    changes.push(SettingsChange::Audio(config));
                                }
                            }
                            Tab::Midi => midi_card(ui, cx, &self.lights, now),
                            Tab::Sync => {
                                let outputs = self.outputs.clone().unwrap_or_default();
                                if let Some(sync) = self.sync_cards(ui, cx, &outputs, &self.lights.clone(), now, dt) {
                                    changes.push(SettingsChange::Sync(sync));
                                }
                            }
                            Tab::Plugins => plugins_card(ui, cx, &mut changes),
                        }
                    });
                    // The footer, on the window's bottom edge.
                    let foot = Rect::from_min_max(pos2(content.left(), content.bottom() - 24.0), content.max);
                    ui.scope_builder(egui::UiBuilder::new().max_rect(foot), |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.label(RichText::new("Changes apply at once and are kept.").size(10.5).color(theme::TEXT_FAINT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match self.tab {
                                Tab::Audio => relist = pill_button(ui, "Refresh devices").clicked(),
                                Tab::Plugins => {
                                    if pill_button(ui, "Rescan plugins").clicked() {
                                        changes.push(SettingsChange::RescanPlugins);
                                    }
                                }
                                Tab::Sync => {
                                    if pill_button(ui, "Refresh ports").clicked() {
                                        self.outputs = None;
                                    }
                                }
                                Tab::Midi => {}
                            });
                        });
                    });
                });
            });
        self.open = open;
        if relist {
            self.list = None;
        }
        changes
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("SETTINGS").size(10.0).strong().color(theme::TEXT_FAINT));
        });
        ui.add_space(8.0);
        for tab in Tab::ALL {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::CLICK);
            let selected = self.tab == tab;
            let hot = resp.hovered();
            let p = ui.painter();
            if selected || hot {
                p.rect_filled(rect, 5.0, if selected { theme::CONTROL } else { alpha(theme::CONTROL, 0.5) });
            }
            if selected {
                p.rect_filled(Rect::from_min_size(rect.left_top() + vec2(0.0, 7.0), vec2(2.5, 16.0)), 1.0, theme::ACCENT);
            }
            let col = if selected { theme::TEXT } else if hot { theme::TEXT } else { theme::TEXT_DIM };
            let icon_col = if selected { theme::ACCENT } else { col };
            tab_icon(p, pos2(rect.left() + 20.0, rect.center().y), tab, icon_col);
            p.text(pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, tab.title(), FontId::proportional(12.5), col);
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                self.tab = tab;
            }
            ui.add_space(2.0);
        }
    }

    /// The Sync tab; returns new sync settings if changed.
    fn sync_cards(&mut self, ui: &mut Ui, cx: &SettingsContext, outputs: &[String], lights: &HashMap<String, Lights>, now: f64, dt: f32) -> Option<SyncSettings> {
        let mut sync = cx.sync.clone();
        let inputs: Vec<&str> = cx.midi_inputs.iter().map(|i| i.name.as_str()).collect();
        card(ui, "CLOCK SOURCE", |ui| {
            row(ui, "Transport", |ui| {
                let shown = match &sync.source {
                    None => "Internal · the board's tempo".to_owned(),
                    Some(s) if inputs.contains(&s.as_str()) => format!("MIDI clock · {s}"),
                    Some(s) => format!("MIDI clock · {s} (not connected)"),
                };
                ComboBox::from_id_salt("sync_source").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
                    ui.selectable_value(&mut sync.source, None, "Internal · the board's tempo");
                    for name in &inputs {
                        let label = format!("MIDI clock · {name}");
                        ui.selectable_value(&mut sync.source, Some((*name).to_owned()), label);
                    }
                });
            });
            let (text, color) = match &sync.source {
                None => ("The board keeps time: its BPM, TAP, and Start/Stop in the toolbar.".to_owned(), theme::TEXT_FAINT),
                Some(name) => {
                    let clock = cx.shared.sync.clock();
                    let t = now_ns();
                    if !inputs.contains(&name.as_str()) {
                        (format!("Waiting for {name} to be connected."), theme::SOLO)
                    } else if !clock.alive(t) {
                        (format!("No clock from {name} yet: start it there, or send clock from it."), theme::SOLO)
                    } else {
                        let bpm = clock.bpm() as f32;
                        self.shown_bpm = if (self.shown_bpm - bpm).abs() > 5.0 { bpm } else { self.shown_bpm + (bpm - self.shown_bpm) * (1.0 - (-dt / 0.4).exp()) };
                        let state = if clock.playing(t) {
                            let beat = clock.beat_at(t);
                            format!("playing · bar {} beat {}", (beat / 4.0).floor() as i64 + 1, (beat % 4.0).floor() as i64 + 1)
                        } else if clock.running {
                            "started, waiting for the first tick".to_owned()
                        } else {
                            "stopped".to_owned()
                        };
                        (format!("Receiving {:.1} BPM · {state}", self.shown_bpm), theme::METER_GREEN)
                    }
                }
            };
            status(ui, text, color);
            if let Some(name) = &sync.source {
                let l = lights.get(name).copied().unwrap_or_default();
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(LABEL_W + 12.0);
                    light(ui, "clock", now - l.clock_at < 0.15, theme::METER_GREEN);
                });
            }
            note(
                ui,
                "Follows 24 ticks a beat, smoothed; Start plays from the top, Stop holds, Continue and song position move to the clock's place.",
            );
        });
        ui.add_space(12.0);
        card(ui, "SEND CLOCK", |ui| {
            if outputs.is_empty() {
                line(ui, "No MIDI outputs on this computer.", theme::TEXT_FAINT);
            }
            for name in outputs {
                let mut on = sync.clock_out.contains(name);
                let loops = sync.source.as_deref() == Some(name.as_str());
                item(ui, |ui| {
                    let resp = switch(ui, &mut on, name).on_hover_text(if loops {
                        "Also the clock source: the clock would go round in a loop, so it isn't sent there"
                    } else {
                        "Send the board's clock and transport here"
                    });
                    if resp.changed() {
                        if on {
                            sync.clock_out.push(name.clone());
                        } else {
                            sync.clock_out.retain(|n| n != name);
                        }
                    }
                });
            }
            let (sending, error) = &cx.clock_out;
            if let Some(e) = error {
                line(ui, e, theme::MUTE);
            } else if !sending.is_empty() {
                line(ui, format!("Sending to {}", sending.join(", ")), theme::METER_GREEN);
            }
            note(
                ui,
                "24 ticks a beat at the board's tempo, also while stopped; Start, Stop, and song position plus Continue. Ticks leave when their beat is heard.",
            );
        });
        (sync != *cx.sync).then_some(sync)
    }
}

// ---------------------------------------------------------------------------
// Pieces: cards, rows, notes
// ---------------------------------------------------------------------------

/// A titled card: thin outline, the rows inside.
fn card(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(title).size(10.0).strong().color(theme::TEXT_DIM));
    ui.add_space(4.0);
    egui::Frame::NONE
        .fill(alpha(theme::INSET, 0.55))
        .stroke(Stroke::new(1.0, theme::PANEL_BORDER))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            body(ui);
        });
}

/// A row: the label on the left, the control in the right column.
fn row<R>(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_H);
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_W, ROW_H), Sense::hover());
        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(12.0), theme::TEXT_DIM);
        ui.add_space(6.0);
        control(ui)
    })
    .inner
}

/// A row of a list (inputs, outputs, folders): from the card's left edge.
fn item<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_H);
        body(ui)
    })
    .inner
}

/// A status line under a list, from the card's left edge.
fn line(ui: &mut Ui, text: impl Into<String>, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).size(11.0).color(color)).wrap());
}

/// The window's close button: a line-art cross.
fn close_button(ui: &mut Ui, rect: Rect) -> Response {
    let resp = ui.interact(rect, egui::Id::unique("settings_close"), Sense::CLICK);
    let col = if resp.hovered() { theme::TEXT } else { theme::TEXT_DIM };
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, theme::CONTROL);
    }
    let c = rect.center();
    for d in [vec2(4.5, 4.5), vec2(4.5, -4.5)] {
        ui.painter().line_segment([c - d, c + d], Stroke::new(1.3, col));
    }
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Close (Esc)")
}

/// A status line under the rows, in the control column.
fn status(ui: &mut Ui, text: impl Into<String>, color: Color32) {
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + 12.0);
        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(color)).wrap());
    });
}

/// A quiet explanation at the bottom of a card.
fn note(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.add(egui::Label::new(RichText::new(text).size(10.5).color(theme::TEXT_FAINT)).wrap());
}

/// A small outlined button.
fn pill_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(11.0).color(theme::TEXT))
            .stroke(Stroke::new(1.0, theme::NODE_BORDER))
            .fill(Color32::TRANSPARENT)
            .corner_radius(11.0)
            .min_size(vec2(0.0, 22.0)),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
}

/// An on/off switch with its label: an outlined pill, its dot right and
/// filled when on.
fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let font = FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, theme::TEXT);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(34.0 + 10.0 + galley.size().x, ROW_H), Sense::CLICK);
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let p = ui.painter();
    let track = Rect::from_min_size(pos2(rect.left(), rect.center().y - 8.0), vec2(30.0, 16.0));
    let col = if *on { theme::ACCENT } else { theme::NODE_BORDER };
    p.rect(track, 8.0, if *on { alpha(theme::ACCENT, 0.18) } else { Color32::TRANSPARENT }, Stroke::new(1.2, col), StrokeKind::Inside);
    let x = if *on { track.right() - 8.0 } else { track.left() + 8.0 };
    let dot = pos2(x, track.center().y);
    if *on {
        p.circle_filled(dot, 4.5, theme::ACCENT);
    } else {
        p.circle_stroke(dot, 4.0, Stroke::new(1.2, theme::TEXT_DIM));
    }
    let text_col = if resp.hovered() || *on { theme::TEXT } else { theme::TEXT_DIM };
    p.galley(pos2(track.right() + 10.0, rect.center().y - galley.size().y * 0.5), galley, text_col);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// An activity light: a ring that fills while active, and its label.
fn light(ui: &mut Ui, label: &str, active: bool, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0 + 6.0 + 7.0 * label.len() as f32, 16.0), Sense::hover());
    let c = pos2(rect.left() + 5.0, rect.center().y);
    let p = ui.painter();
    if active {
        p.circle_filled(c, 4.0, color);
    } else {
        p.circle_stroke(c, 3.5, Stroke::new(1.0, theme::TEXT_FAINT));
    }
    p.text(pos2(c.x + 9.0, c.y), Align2::LEFT_CENTER, label, FontId::monospace(9.5), if active { color } else { theme::TEXT_FAINT });
}

/// Line-art icons for the tabs.
fn tab_icon(p: &egui::Painter, c: Pos2, tab: Tab, col: Color32) {
    let s = Stroke::new(1.3, col);
    match tab {
        Tab::Audio => {
            // A speaker and two waves.
            let body = vec![c + vec2(-7.0, -2.5), c + vec2(-4.0, -2.5), c + vec2(0.0, -6.0), c + vec2(0.0, 6.0), c + vec2(-4.0, 2.5), c + vec2(-7.0, 2.5)];
            p.add(Shape::closed_line(body, s));
            for r in [3.5f32, 6.5] {
                let pts: Vec<Pos2> = (0..=8).map(|k| {
                    let a = -0.9 + 1.8 * k as f32 / 8.0;
                    c + vec2(1.0 + r * a.cos(), r * a.sin())
                }).collect();
                p.add(Shape::line(pts, s));
            }
        }
        Tab::Midi => {
            // A DIN socket: a ring, five pins and the key.
            p.circle_stroke(c, 7.0, s);
            for k in 0..5 {
                let a = std::f32::consts::PI * (1.0 + k as f32 / 4.0);
                p.circle_filled(c + vec2(4.2 * a.cos(), 4.2 * a.sin()), 0.9, col);
            }
            p.line_segment([c + vec2(-1.5, 7.0), c + vec2(1.5, 7.0)], Stroke::new(2.0, theme::PANEL));
        }
        Tab::Sync => {
            // A metronome: its body and the arm.
            let body = vec![c + vec2(-5.0, 7.0), c + vec2(5.0, 7.0), c + vec2(2.5, -7.0), c + vec2(-2.5, -7.0)];
            p.add(Shape::closed_line(body, s));
            p.line_segment([c + vec2(0.0, 4.0), c + vec2(4.5, -5.0)], s);
            p.circle_filled(c + vec2(2.6, -1.2), 1.4, col);
        }
        Tab::Plugins => {
            // A plug: its head and two prongs.
            let head = Rect::from_center_size(c + vec2(0.0, 1.5), vec2(10.0, 7.0));
            p.rect_stroke(head, 2.0, s, StrokeKind::Inside);
            p.line_segment([c + vec2(-2.5, -2.0), c + vec2(-2.5, -6.5)], s);
            p.line_segment([c + vec2(2.5, -2.0), c + vec2(2.5, -6.5)], s);
            p.line_segment([c + vec2(0.0, 5.0), c + vec2(0.0, 8.0)], s);
        }
    }
}

fn rate_label(rate: u32) -> String {
    if rate.is_multiple_of(1000) { format!("{} kHz", rate / 1000) } else { format!("{:.1} kHz", rate as f32 / 1000.0) }
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

fn output_card(ui: &mut Ui, list: &DeviceList, audio: &AudioHandle, config: &mut AudioConfig) {
    let info = &audio.info;
    let device = list.output(config.output.as_deref());
    card(ui, "OUTPUT", |ui| {
        if list.hosts.len() > 1 {
            row(ui, "Audio system", |ui| {
                let current = config.host.clone().unwrap_or_else(|| list.host.clone());
                ComboBox::from_id_salt("host").width(COMBO_W).selected_text(&current).show_ui(ui, |ui| {
                    for host in &list.hosts {
                        if ui.selectable_label(*host == current, host).clicked() && *host != current {
                            // Another system: its own devices, from their defaults.
                            *config = AudioConfig { host: Some(host.clone()), buffer_size: config.buffer_size, ..Default::default() };
                        }
                    }
                });
            });
        }
        row(ui, "Device", |ui| {
            let default_name = list.output(None).map_or("none", |d| d.name.as_str());
            let shown = match &config.output {
                None => format!("System default ({default_name})"),
                Some(id) => list.outputs.iter().find(|d| d.id == *id).map_or_else(|| "Missing device".to_owned(), |d| d.name.clone()),
            };
            ComboBox::from_id_salt("output").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
                if ui.selectable_label(config.output.is_none(), format!("System default ({default_name})")).clicked() {
                    config.output = None;
                    config.output_channel = 0;
                }
                for d in &list.outputs {
                    let on = config.output.as_deref() == Some(d.id.as_str());
                    if ui.selectable_label(on, format!("{}  ·  {} ch", d.name, d.channels)).clicked() && !on {
                        config.output = Some(d.id.clone());
                        config.output_channel = 0;
                    }
                }
            });
        });
        let channels = device.map_or(info.channels, |d| d.channels);
        if channels > 2 {
            row(ui, "Channels", |ui| {
                let pairs = device::pairs(channels);
                let shown = pairs.iter().find(|(c, _)| *c == config.output_channel).map_or("1–2", |(_, l)| l.as_str());
                ComboBox::from_id_salt("out_pair").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
                    for (c, label) in pairs.iter().filter(|(c, _)| c + 1 < channels) {
                        ui.selectable_value(&mut config.output_channel, *c, label);
                    }
                });
            });
        }
        row(ui, "Sample rate", |ui| {
            let shown = match config.sample_rate {
                None => format!("Device's own ({})", rate_label(info.sample_rate)),
                Some(rate) => rate_label(rate),
            };
            ComboBox::from_id_salt("rate").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut config.sample_rate, None, "Device's own");
                for &rate in device.map_or(&[][..], |d| &d.rates) {
                    ui.selectable_value(&mut config.sample_rate, Some(rate), rate_label(rate));
                }
            });
        });
        row(ui, "Buffer size", |ui| {
            let ms = |frames: u32| frames as f32 / info.sample_rate.max(1) as f32 * 1000.0;
            let shown = match config.buffer_size {
                None => "Device's own".to_owned(),
                Some(size) => format!("{size} samples · {:.1} ms", ms(size)),
            };
            let range = device.and_then(|d| d.buffer_range).or(info.buffer_range);
            ComboBox::from_id_salt("buffer").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut config.buffer_size, None, "Device's own");
                for size in BUFFER_SIZES {
                    if range.is_none_or(|(lo, hi)| (lo..=hi).contains(&size)) {
                        ui.selectable_value(&mut config.buffer_size, Some(size), format!("{size} samples · {:.1} ms", ms(size)));
                    }
                }
            });
        });
        match &info.error {
            Some(e) => status(ui, e, theme::MUTE),
            None => {
                let frames = audio.shared.frames.load(std::sync::atomic::Ordering::Relaxed);
                let pair = device::pairs(info.channels).into_iter().find(|(c, _)| *c == info.output_channel).map_or_else(String::new, |(_, l)| format!(" · out {l}"));
                let ms = frames as f32 / info.sample_rate.max(1) as f32 * 1000.0;
                status(ui, format!("Playing on {} · {} · {frames} samples ({ms:.1} ms){pair}", info.device, rate_label(info.sample_rate)), theme::METER_GREEN);
            }
        }
        if config.sample_rate.is_some_and(|r| r != info.sample_rate) {
            note(ui, "A new sample rate rebuilds every node (the patch stays as it is).");
        }
    });
}

fn input_card(ui: &mut Ui, list: &DeviceList, audio: &AudioHandle, config: &mut AudioConfig, levels: &[f32; MAX_CHANNELS]) {
    let info = &audio.info;
    card(ui, "INPUT", |ui| {
        row(ui, "Device", |ui| {
            let default_name = list.input(&InputChoice::SystemDefault).map_or("none", |d| d.name.as_str());
            let shown = match &config.input {
                InputChoice::Off => "Off".to_owned(),
                InputChoice::SystemDefault => format!("System default ({default_name})"),
                InputChoice::Device(id) => list.inputs.iter().find(|d| d.id == *id).map_or_else(|| "Missing device".to_owned(), |d| d.name.clone()),
            };
            ComboBox::from_id_salt("input").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut config.input, InputChoice::Off, "Off");
                ui.selectable_value(&mut config.input, InputChoice::SystemDefault, format!("System default ({default_name})"));
                for d in &list.inputs {
                    let rate_ok = d.rates.contains(&info.sample_rate);
                    let label = format!("{}  ·  {} ch{}", d.name, d.channels, if rate_ok { "" } else { "  ·  other rate" });
                    ui.selectable_value(&mut config.input, InputChoice::Device(d.id.clone()), label);
                }
            });
        });
        if let Some(e) = &info.input_error {
            status(ui, e, theme::MUTE);
            return;
        }
        let Some(name) = &info.input else {
            if config.input != InputChoice::Off {
                status(ui, "Opens while an Audio In node is on the board.", theme::TEXT_FAINT);
            }
            return;
        };
        let channels = (info.input_channels as usize).min(MAX_CHANNELS);
        status(ui, format!("Listening to {name} · {channels} ch · Audio In nodes pick their channels"), theme::METER_GREEN);
        ui.add_space(4.0);
        for (c, &level) in levels.iter().enumerate().take(channels) {
            meter(ui, c + 1, level);
        }
        let lost = audio.shared.input.dropouts.load(std::sync::atomic::Ordering::Relaxed);
        if lost > 0 {
            status(ui, format!("{lost} input dropouts (a larger buffer, or the same device for input and output, helps)"), theme::SOLO);
        }
    });
}

/// One input channel's level, −60 … 0 dBFS: an outlined bar, its fill
/// green to red, ticks at −18 and −6.
fn meter(ui: &mut Ui, channel: usize, level: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 12.0), Sense::hover());
    let p = ui.painter();
    p.text(pos2(rect.left() + LABEL_W, rect.center().y), Align2::RIGHT_CENTER, format!("in {channel}"), theme::mono(9.0), theme::TEXT_DIM);
    let bar = Rect::from_min_max(pos2(rect.left() + LABEL_W + 12.0, rect.top() + 2.0), rect.right_bottom() - vec2(0.0, 2.0));
    p.rect_stroke(bar, 2.0, Stroke::new(1.0, theme::PANEL_BORDER), StrokeKind::Inside);
    let x = (1.0 + 20.0 * level.max(1e-6).log10() / 60.0).clamp(0.0, 1.0);
    let color = if level >= 0.89 { theme::METER_RED } else if level >= 0.5 { theme::METER_YELLOW } else { theme::METER_GREEN };
    if x > 0.0 {
        p.rect_filled(Rect::from_min_size(bar.min + vec2(1.0, 1.0), vec2((bar.width() - 2.0) * x, bar.height() - 2.0)), 1.5, color);
    }
    for db in [-18.0, -6.0] {
        let x = bar.left() + bar.width() * (1.0 + db / 60.0);
        p.vline(x, bar.y_range(), Stroke::new(1.0, alpha(theme::TEXT, 0.18)));
    }
}

// ---------------------------------------------------------------------------
// MIDI
// ---------------------------------------------------------------------------

fn midi_card(ui: &mut Ui, cx: &SettingsContext, lights: &HashMap<String, Lights>, now: f64) {
    card(ui, "INPUTS", |ui| {
        if let Some(e) = cx.midi_error {
            line(ui, format!("MIDI is unavailable: {e}"), theme::MUTE);
            return;
        }
        if cx.midi_inputs.is_empty() {
            line(ui, "No MIDI input connected. Plug one in: it is picked up within seconds.", theme::TEXT_FAINT);
        }
        for input in &cx.midi_inputs {
            let l = lights.get(&input.name).copied().unwrap_or_default();
            item(ui, |ui| {
                ui.label(RichText::new(&input.name).size(12.0).color(theme::TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    light(ui, "clock", now - l.clock_at < 0.15, theme::ACCENT);
                    ui.add_space(8.0);
                    light(ui, "notes", now - l.events_at < 0.15, theme::METER_GREEN);
                    if cx.sync.source.as_deref() == Some(input.name.as_str()) {
                        ui.add_space(8.0);
                        ui.label(RichText::new("SYNC SOURCE").size(9.5).color(theme::ACCENT));
                    }
                });
            });
        }
        note(
            ui,
            "Every input is connected: notes and controllers reach the MIDI nodes, MIDI learn and plugins' KEYBOARD; clock and transport reach the MIDI Looper and, if chosen under Sync, the board's transport.",
        );
    });
}

// ---------------------------------------------------------------------------
// Plugins
// ---------------------------------------------------------------------------

fn plugins_card(ui: &mut Ui, cx: &SettingsContext, changes: &mut Vec<SettingsChange>) {
    let counts = cx.plugin_counts;
    card(ui, "VST3", |ui| {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        for dir in crate::audio::plugin::plugin_dirs() {
            item(ui, |ui| {
                let there = dir.exists();
                // "~/Library/…" for the user's own folder.
                let text = match home.as_deref().and_then(|h| dir.strip_prefix(h).ok()) {
                    Some(rest) => format!("~/{}", rest.display()),
                    None => dir.display().to_string(),
                };
                ui.label(RichText::new(text).font(FontId::monospace(10.5)).color(if there { theme::TEXT } else { theme::TEXT_FAINT }));
                if !there {
                    ui.label(RichText::new("not there").size(10.0).color(theme::TEXT_FAINT));
                }
            });
        }
        line(ui, format!("{} VST3 plugin{} found", counts.vst3, if counts.vst3 == 1 { "" } else { "s" }), theme::METER_GREEN);
    });
    ui.add_space(12.0);
    card(ui, "AUDIO UNITS", |ui| {
        if !cfg!(target_os = "macos") {
            line(ui, "Audio Units are a macOS format.", theme::TEXT_FAINT);
            return;
        }
        let mut p = cx.plugins.clone();
        item(ui, |ui| switch(ui, &mut p.audio_units, "List Audio Units: AUv3 extensions and AU components"));
        ui.add_enabled_ui(p.audio_units, |ui| {
            item(ui, |ui| {
                switch(ui, &mut p.apple_units, "Include Apple's own").on_hover_text("AUDelay, AUReverb2, AUNBandEQ, DLSMusicDevice, AUSampler…")
            });
        });
        if p.audio_units {
            let v2 = counts.au - counts.auv3;
            line(ui, format!("{} Audio Unit{} listed · {} AUv3 · {v2} AU", counts.au, if counts.au == 1 { "" } else { "s" }, counts.auv3), theme::METER_GREEN);
        }
        note(
            ui,
            "AUv3 app extensions run in a process of their own (a crash there leaves AudioBoard playing); classic AU components run inside AudioBoard, like VST3.",
        );
        if p != *cx.plugins {
            changes.push(SettingsChange::Plugins(p));
        }
    });
}
