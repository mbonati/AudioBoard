use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::{
    Align, Align2, CentralPanel, Color32, CursorIcon, FontId, Frame, Id, Key, Layout, Margin, Panel,
    Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Ui, ViewportCommand, pos2, vec2,
};

use super::canvas::Editor;
use super::monitor::Monitor;
use super::canvas::add_menu;
use super::theme::{self, alpha};
use crate::audio::engine::{BLOCK, NodeId};
use crate::audio::modules::{ModuleKind, ModuleParams, delay, drone, filter, granular, random, reverb};
use crate::audio::params::{EngineShared, Waveform};
use crate::audio::{self, AudioHandle};
use crate::graph::{Graph, NodeKind, PortRef};

const SHORTCUTS: [(&str, &str); 21] = [
    ("Right-click", "Add node / menu"),
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
    ("C", "Collapse / expand node or module"),
    ("Double-click title", "Collapse / expand"),
    ("Shift+C", "Collapse / expand all"),
    ("F", "Frame all"),
    ("Space", "Audio on / off"),
];

pub struct AudioBoardApp {
    shared: Arc<EngineShared>,
    audio: AudioHandle,
    graph: Graph,
    editor: Editor,
    monitor: Monitor,
    show_monitor: bool,
}


impl AudioBoardApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let shared = Arc::new(EngineShared::new());
        let audio = audio::start(shared.clone());
        let mut graph = Graph::default();
        demo_patch(&mut graph, &audio);
        Self {
            shared,
            audio,
            graph,
            editor: Editor::new(),
            monitor: Monitor::new(),
            show_monitor: true,
        }
    }

    fn running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    fn toggle_running(&self) {
        self.shared.running.fetch_xor(true, Ordering::Relaxed);
    }

    fn toolbar(&mut self, ui: &mut Ui) {
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

            let add = tool_button(ui, "Add node", theme::ACCENT);
            if let Some(ty) = egui::Popup::menu(&add).show(add_menu).and_then(|r| r.inner) {
                let p = self.editor.spawn_point(&self.graph, ty.size());
                let id = self.graph.add(ty, p, &self.audio);
                self.editor.select_only(id);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if power_button(ui, self.running()).on_hover_text("Space").clicked() {
                    self.toggle_running();
                }
                ui.add_space(8.0);
                if icon_button(ui, Icon::Monitor, self.show_monitor)
                    .on_hover_text(if self.show_monitor { "Hide analyzers" } else { "Show analyzers" })
                    .clicked()
                {
                    self.show_monitor = !self.show_monitor;
                }
                let all_collapsed = !self.graph.nodes.is_empty()
                    && self.graph.nodes.iter().all(|n| n.collapsed)
                    && self.graph.groups.iter().all(|g| g.collapsed);
                let tip = if all_collapsed { "Expand everything (Shift+C)" } else { "Collapse everything (Shift+C)" };
                if icon_button(ui, Icon::Compact, all_collapsed).on_hover_text(tip).clicked() {
                    self.graph.set_all_collapsed(!all_collapsed);
                }
                icon_button(ui, Icon::Help, false).on_hover_ui(shortcuts);
                ui.add_space(10.0);
                let info = &self.audio.info;
                match &info.error {
                    Some(e) => {
                        ui.label(RichText::new("Audio unavailable").size(11.0).color(theme::MUTE))
                            .on_hover_text(e);
                    }
                    None => {
                        ui.label(RichText::new(&info.device).size(11.0).color(theme::TEXT_DIM));
                    }
                }
            });
        });
    }

    fn monitor_dock(&mut self, ui: &mut Ui, dt: f32) {
        let sr = self.audio.info.sample_rate as f32;
        let gap = 8.0;
        let size = vec2(((ui.available_width() - gap) * 0.5).floor(), ui.available_height());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            self.monitor.scope(ui, &self.shared, sr, size);
            self.monitor.spectrum(ui, sr, dt, size);
        });
    }

    fn status_bar(&self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            let selected = &self.editor.selected;
            let single = (selected.len() == 1)
                .then(|| selected.iter().next().and_then(|id| self.graph.node(*id)))
                .flatten();
            let hint = if let Some(g) = self.editor.selected_group(&self.graph) {
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
                "Drag from an output to an input to connect  ·  Drag on empty space to select  ·  Right-click to add nodes"
                    .to_owned()
            };
            ui.label(RichText::new(hint).size(10.5).color(theme::TEXT_FAINT));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                let item = |text: String| RichText::new(text).size(10.0).monospace().color(theme::TEXT_DIM);
                ui.label(item(format!("{:.0}%", self.editor.zoom * 100.0)));
                ui.label(item(format!(
                    "{} nodes · {} cables",
                    self.graph.nodes.len(),
                    self.graph.cables.len()
                )));
                let info = &self.audio.info;
                ui.label(item(format!(
                    "{:.1} kHz · {} ch · {BLOCK} smp",
                    info.sample_rate as f32 / 1000.0,
                    info.channels
                )));
                let load = self.shared.load.get();
                let load_col = if load > 0.7 {
                    theme::METER_RED
                } else if load > 0.4 {
                    theme::METER_YELLOW
                } else {
                    theme::TEXT_DIM
                };
                ui.label(item(format!("DSP {:>4.1}%", load * 100.0)).color(load_col));
            });
        });
    }
}

impl eframe::App for AudioBoardApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let dt = ui.input(|i| i.stable_dt.min(0.1));
        for node in &mut self.graph.nodes {
            node.update_meters(dt);
        }
        if ui.input(|i| i.key_pressed(Key::Space)) && ui.memory(|m| m.focused().is_none()) {
            self.toggle_running();
        }

        let bar_frame = |margin: Margin| Frame::NONE.fill(theme::PANEL).inner_margin(margin);
        Panel::top("toolbar")
            .exact_size(36.0)
            .frame(bar_frame(Margin::symmetric(10, 0)))
            .show(ui, |ui| self.toolbar(ui));
        Panel::bottom("status")
            .exact_size(22.0)
            .frame(bar_frame(Margin::symmetric(10, 0)))
            .show(ui, |ui| self.status_bar(ui));
        if self.show_monitor {
            Panel::bottom("monitor")
                .exact_size(150.0)
                .resizable(false)
                .frame(bar_frame(Margin::same(8)))
                .show(ui, |ui| self.monitor_dock(ui, dt));
        }
        CentralPanel::default()
            .frame(Frame::NONE.fill(theme::BG))
            .show(ui, |ui| self.editor.show(ui, &mut self.graph, &self.audio));

        // Meters and analyzers only move while there is signal; otherwise
        // egui repaints on input alone.
        if self.running() || self.graph.nodes.iter().any(|n| n.signal_level() > 1e-3) {
            ui.ctx().request_repaint();
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
fn demo_patch(graph: &mut Graph, audio: &AudioHandle) {
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

fn logo(ui: &mut Ui) {
    let galley = ui
        .painter()
        .layout_no_wrap("AudioBoard".to_owned(), theme::strong(12.5), theme::TEXT);
    let (rect, _) = ui.allocate_exact_size(vec2(26.0 + galley.size().x, 24.0), Sense::hover());
    let p = ui.painter();
    let icon = Rect::from_center_size(pos2(rect.left() + 9.0, rect.center().y), vec2(18.0, 18.0));
    p.rect_filled(icon, 4.0, theme::ACCENT);
    let pts = super::widgets::wave_points(icon.shrink2(vec2(3.5, 5.0)), Waveform::Sine, 0.0, 1.0, 1.0, 0.0, 0);
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

/// Flat toolbar button with a small "+" in the node's colour.
fn tool_button(ui: &mut Ui, label: &str, color: Color32) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(12.0), theme::TEXT);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 30.0, 24.0), Sense::CLICK);
    let hot = resp.hovered();
    let p = ui.painter();
    if hot {
        p.rect_filled(rect, 4.0, theme::CONTROL);
    }
    let c = pos2(rect.left() + 12.0, rect.center().y);
    let d = 3.5;
    p.line_segment([c - vec2(d, 0.0), c + vec2(d, 0.0)], Stroke::new(1.4, color));
    p.line_segment([c - vec2(0.0, d), c + vec2(0.0, d)], Stroke::new(1.4, color));
    p.galley(
        pos2(rect.left() + 22.0, rect.center().y - galley.size().y * 0.5),
        galley,
        if hot { theme::TEXT } else { theme::TEXT_DIM },
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

enum Icon {
    Monitor,
    Compact,
    Help,
}

fn icon_button(ui: &mut Ui, icon: Icon, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::CLICK);
    let p = ui.painter();
    if active || resp.hovered() {
        p.rect_filled(rect, 4.0, if resp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
    }
    let col = if active || resp.hovered() { theme::TEXT } else { theme::TEXT_DIM };
    let c = rect.center();
    match icon {
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
        Icon::Help => {
            p.circle_stroke(c, 6.5, Stroke::new(1.2, col));
            p.text(c + vec2(0.0, 0.5), Align2::CENTER_CENTER, "?", theme::strong(9.5), col);
        }
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn power_button(ui: &mut Ui, on: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(80.0, 24.0), Sense::CLICK);
    let p = ui.painter();
    let (fill, border, dot, text) = if on {
        (alpha(theme::METER_GREEN, 0.10), alpha(theme::METER_GREEN, 0.45), theme::METER_GREEN, theme::TEXT)
    } else {
        (Color32::TRANSPARENT, theme::NODE_BORDER, theme::TEXT_FAINT, theme::TEXT_DIM)
    };
    let fill = if resp.hovered() && !on { theme::CONTROL } else { fill };
    p.rect(rect, 12.0, fill, Stroke::new(1.0, border), StrokeKind::Inside);
    p.circle_filled(pos2(rect.left() + 13.0, rect.center().y), 3.0, dot);
    p.text(
        pos2(rect.left() + 23.0, rect.center().y),
        Align2::LEFT_CENTER,
        if on { "Audio on" } else { "Audio off" },
        FontId::proportional(11.5),
        text,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn shortcuts(ui: &mut Ui) {
    ui.label(RichText::new("Shortcuts").size(11.5).color(theme::TEXT));
    ui.add_space(4.0);
    egui::Grid::new("shortcuts").num_columns(2).spacing([18.0, 4.0]).show(ui, |ui| {
        for (keys, what) in SHORTCUTS {
            ui.label(RichText::new(keys).size(10.5).monospace().color(theme::TEXT));
            ui.label(RichText::new(what).size(11.0).color(theme::TEXT_DIM));
            ui.end_row();
        }
    });
}
