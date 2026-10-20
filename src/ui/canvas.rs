//! The patching canvas: grid, pan/zoom, nodes, and cable drag & drop.

use std::collections::{HashMap, HashSet};

use egui::{
    Align2, Color32, CursorIcon, FontId, Id, Key, Painter, PointerButton, Pos2, Rect, Sense,
    Stroke, Ui, Vec2, vec2,
};

use super::nodes::{self, NodeView};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::AudioHandle;
use crate::audio::engine::NodeId;
use crate::graph::{Cable, Graph, NodeKind, PortRef};

const GRID: f32 = 24.0;
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 2.0;

enum Action {
    AddOsc(Pos2),
    AddMixer(Pos2),
    Remove(NodeId),
    Duplicate(NodeId),
    Connect { from: PortRef, to: PortRef },
    Disconnect(Cable),
    Select(NodeId),
    Deselect,
}

/// A cable being dragged out of a port. `anchor` is the fixed end.
#[derive(Clone, Copy)]
struct CableDrag {
    anchor: PortRef,
    from_output: bool,
}

#[derive(Clone, Copy)]
enum MenuTarget {
    Canvas(Pos2),
    Cable(Cable),
}

pub struct Editor {
    pub pan: Vec2,
    pub zoom: f32,
    pub selected: Option<NodeId>,
    rect: Rect,
    drag: Option<CableDrag>,
    menu: MenuTarget,
    needs_framing: bool,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            rect: Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)),
            drag: None,
            menu: MenuTarget::Canvas(Pos2::ZERO),
            needs_framing: true,
        }
    }

    fn to_screen(&self, w: Pos2) -> Pos2 {
        self.rect.min + self.pan + w.to_vec2() * self.zoom
    }

    fn to_world(&self, s: Pos2) -> Pos2 {
        ((s - self.rect.min - self.pan) / self.zoom).to_pos2()
    }

    /// Where to drop a node added from the toolbar: the middle of the view,
    /// nudged so repeated adds don't stack exactly.
    pub fn spawn_point(&self, graph: &Graph, size: Vec2) -> Pos2 {
        let mut p = self.to_world(self.rect.center()) - size * 0.5;
        while graph.nodes.iter().any(|n| (n.pos - p).length() < 12.0) {
            p += vec2(28.0, 28.0);
        }
        p
    }

    fn frame_all(&mut self, graph: &Graph) {
        let Some(bounds) = graph
            .nodes
            .iter()
            .map(|n| Rect::from_min_size(n.pos, n.size()))
            .reduce(|a, b| a.union(b))
        else {
            self.zoom = 1.0;
            self.pan = self.rect.size() * 0.5;
            return;
        };
        let avail = self.rect.size() - vec2(120.0, 120.0);
        self.zoom = (avail.x / bounds.width())
            .min(avail.y / bounds.height())
            .clamp(MIN_ZOOM, 1.0);
        self.pan = self.rect.size() * 0.5 - bounds.center().to_vec2() * self.zoom;
    }

    pub fn show(&mut self, ui: &mut Ui, graph: &mut Graph, audio: &AudioHandle) {
        let rect = ui.max_rect();
        self.rect = rect;
        if self.needs_framing {
            self.needs_framing = false;
            self.frame_all(graph);
        }

        let painter = ui.painter_at(rect);
        let bg = ui.interact(rect, Id::unique("canvas_bg"), Sense::CLICK | Sense::DRAG);
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let mut actions = Vec::new();

        // --- Navigation ---------------------------------------------------
        if bg.dragged_by(PointerButton::Primary) || bg.dragged_by(PointerButton::Middle) {
            self.pan += bg.drag_delta();
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
        if ui.rect_contains_pointer(rect) {
            let (zoom_delta, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
            if let Some(hp) = pointer
                && zoom_delta != 1.0
            {
                let anchor = self.to_world(hp);
                self.zoom = (self.zoom * zoom_delta).clamp(MIN_ZOOM, MAX_ZOOM);
                self.pan = hp - rect.min - anchor.to_vec2() * self.zoom;
            }
            self.pan += scroll;
        }
        let z = self.zoom;

        painter.rect_filled(rect, 0.0, theme::BG);
        self.draw_grid(&painter, rect);

        // --- Geometry snapshot (before nodes are mutated) -------------------
        let node_rects: Vec<Rect> = graph
            .nodes
            .iter()
            .map(|n| Rect::from_min_size(self.to_screen(n.pos), n.size() * z))
            .collect();

        struct Wire {
            cable: Cable,
            a: Pos2,
            b: Pos2,
            color: Color32,
            level: f32,
        }
        let wires: Vec<Wire> = graph
            .cables
            .iter()
            .filter_map(|c| {
                let src = graph.node(c.from.node)?;
                let dst = graph.node(c.to.node)?;
                Some(Wire {
                    cable: *c,
                    a: self.to_screen(src.output_pos(c.from.port)),
                    b: self.to_screen(dst.input_pos(c.to.port)),
                    color: src.accent,
                    level: src.signal_level(),
                })
            })
            .collect();

        // input port -> (cable, source colour, source name)
        let mut inputs: HashMap<PortRef, (Cable, Color32, String)> = HashMap::new();
        let mut outputs: HashSet<PortRef> = HashSet::new();
        for c in &graph.cables {
            if let Some(src) = graph.node(c.from.node) {
                inputs.insert(c.to, (*c, src.accent, src.title.clone()));
                outputs.insert(c.from);
            }
        }

        let hovered_cable = pointer
            .filter(|_| self.drag.is_none() && bg.hovered())
            .and_then(|p| {
                wires
                    .iter()
                    .map(|w| (w.cable, widgets::cable_distance(w.a, w.b, p, z)))
                    .filter(|(_, d)| *d < 7.0 * z.max(0.7))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(c, _)| c)
            });
        if hovered_cable.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }

        // Where a dragged cable would land.
        let drop_target = self.drag.zip(pointer).and_then(|(d, p)| {
            let mut best: Option<(PortRef, Pos2, f32)> = None;
            for n in &graph.nodes {
                let ports = if d.from_output { n.inputs().len() } else { n.outputs().len() };
                for i in 0..ports {
                    let pr = PortRef { node: n.id, port: i };
                    let pos = self.to_screen(if d.from_output { n.input_pos(i) } else { n.output_pos(i) });
                    let dist = (pos - p).length();
                    let valid = if d.from_output {
                        !graph.would_cycle(d.anchor.node, n.id)
                    } else {
                        !graph.would_cycle(n.id, d.anchor.node)
                    };
                    if valid && dist < 22.0 * z.max(0.8) && best.is_none_or(|b| dist < b.2) {
                        best = Some((pr, pos, dist));
                    }
                }
            }
            best.map(|(pr, pos, _)| (pr, pos))
        });

        // --- Canvas clicks & context menu --------------------------------
        if bg.secondary_clicked() {
            self.menu = match hovered_cable {
                Some(c) => MenuTarget::Cable(c),
                None => MenuTarget::Canvas(self.to_world(pointer.unwrap_or(rect.center()))),
            };
        }
        if bg.double_clicked()
            && let Some(c) = hovered_cable
        {
            actions.push(Action::Disconnect(c));
        } else if bg.clicked() {
            actions.push(Action::Deselect);
        }
        let menu = self.menu;
        bg.context_menu(|ui| canvas_menu(ui, menu, &mut actions));

        // --- Cables ------------------------------------------------------
        for w in &wires {
            widgets::draw_cable(&painter, w.a, w.b, w.color, w.level, hovered_cable == Some(w.cable), z);
        }

        // --- Nodes -------------------------------------------------------
        let dragging = self.drag;
        for (idx, node) in graph.nodes.iter_mut().enumerate() {
            let rect_s = node_rects[idx];
            if !rect_s.expand(40.0).intersects(rect) {
                continue;
            }
            let id = node.id;
            let body = ui.interact(rect_s, Id::unique(("node", id)), Sense::CLICK | Sense::DRAG);
            if body.drag_started() || body.clicked() || body.secondary_clicked() {
                actions.push(Action::Select(id));
            }
            if body.dragged_by(PointerButton::Primary) {
                node.pos += body.drag_delta() / z;
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
            body.context_menu(|ui| node_menu(ui, id, &mut actions));

            let view = NodeView {
                ui,
                painter: &painter,
                min: rect_s.min,
                z,
            };
            if nodes::frame(&view, node, self.selected == Some(id)).clicked() {
                actions.push(Action::Remove(id));
            }

            let in_count = node.inputs().len();
            let sources: Vec<Option<(Color32, String)>> = (0..in_count)
                .map(|i| inputs.get(&PortRef { node: id, port: i }).map(|(_, c, n)| (*c, n.clone())))
                .collect();
            let accent = node.accent;
            match &mut node.kind {
                NodeKind::Osc(o) => {
                    let connected: Vec<bool> = sources.iter().map(Option::is_some).collect();
                    let out = outputs.contains(&PortRef { node: id, port: 0 });
                    nodes::osc(&view, id, accent, o, &connected, out);
                }
                NodeKind::Mixer(m) => nodes::mixer(&view, id, accent, m, &sources),
            }

            // Ports last so they sit on top of everything else in the node.
            for i in 0..in_count {
                let pr = PortRef { node: id, port: i };
                let c = self.to_screen(node.input_pos(i));
                let resp = ui
                    .interact(Rect::from_center_size(c, Vec2::splat(20.0 * z)), Id::unique(("in", id, i)), Sense::CLICK | Sense::DRAG)
                    .on_hover_cursor(CursorIcon::Crosshair);
                let existing = inputs.get(&pr);
                if resp.drag_started() {
                    // Pick up the plugged cable (Max/Pd style) or start a new one.
                    self.drag = Some(match existing {
                        Some((cable, ..)) => {
                            actions.push(Action::Disconnect(*cable));
                            CableDrag { anchor: cable.from, from_output: true }
                        }
                        None => CableDrag { anchor: pr, from_output: false },
                    });
                }
                let hot = resp.hovered() || drop_target.is_some_and(|(t, _)| t == pr && dragging.is_some_and(|d| d.from_output));
                let color = existing.map_or(accent, |(_, c, _)| *c);
                widgets::port(&painter, c, existing.is_some(), hot, color, z);
            }
            for i in 0..node.outputs().len() {
                let pr = PortRef { node: id, port: i };
                let c = self.to_screen(node.output_pos(i));
                let resp = ui
                    .interact(Rect::from_center_size(c, Vec2::splat(20.0 * z)), Id::unique(("out", id, i)), Sense::CLICK | Sense::DRAG)
                    .on_hover_cursor(CursorIcon::Crosshair);
                if resp.drag_started() {
                    self.drag = Some(CableDrag { anchor: pr, from_output: true });
                }
                let hot = resp.hovered() || drop_target.is_some_and(|(t, _)| t == pr && dragging.is_some_and(|d| !d.from_output));
                widgets::port(&painter, c, outputs.contains(&pr), hot, accent, z);
            }
        }

        // --- Cable being dragged -----------------------------------------
        if let Some(d) = self.drag {
            if let (Some(anchor_node), Some(p)) = (graph.node(d.anchor.node), pointer) {
                let anchor = self.to_screen(if d.from_output {
                    anchor_node.output_pos(d.anchor.port)
                } else {
                    anchor_node.input_pos(d.anchor.port)
                });
                let end = drop_target.map_or(p, |(_, pos)| pos);
                let (a, b) = if d.from_output { (anchor, end) } else { (end, anchor) };
                let color = if drop_target.is_some() { anchor_node.accent } else { alpha(anchor_node.accent, 0.7) };
                widgets::draw_cable(&painter, a, b, color, 0.0, drop_target.is_some(), z);
                if drop_target.is_none() {
                    painter.circle_filled(p, 3.0 * z, color);
                }
            }
            if ui.input(|i| i.pointer.any_released()) {
                if let Some((target, _)) = drop_target {
                    let (from, to) = if d.from_output { (d.anchor, target) } else { (target, d.anchor) };
                    actions.push(Action::Connect { from, to });
                }
                self.drag = None;
            }
        }

        if graph.nodes.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Right-click to add a node",
                FontId::proportional(13.0),
                theme::TEXT_FAINT,
            );
        }

        // --- Keyboard ----------------------------------------------------
        let (delete, duplicate, frame) = ui.input(|i| {
            (
                i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                i.modifiers.command && i.key_pressed(Key::D),
                i.key_pressed(Key::F) && !i.modifiers.command,
            )
        });
        if let Some(sel) = self.selected {
            if delete {
                actions.push(Action::Remove(sel));
            } else if duplicate {
                actions.push(Action::Duplicate(sel));
            }
        }
        if frame {
            self.frame_all(graph);
        }

        self.apply(actions, graph, audio);
    }

    fn apply(&mut self, actions: Vec<Action>, graph: &mut Graph, audio: &AudioHandle) {
        for action in actions {
            match action {
                Action::AddOsc(p) => self.selected = Some(graph.add_osc(p, audio)),
                Action::AddMixer(p) => self.selected = Some(graph.add_mixer(p, audio)),
                Action::Remove(id) => {
                    graph.remove_node(id, audio);
                    if self.selected == Some(id) {
                        self.selected = None;
                    }
                }
                Action::Duplicate(id) => {
                    if let Some(new) = graph.duplicate(id, audio) {
                        self.selected = Some(new);
                    }
                }
                Action::Connect { from, to } => {
                    graph.connect(from, to, audio);
                }
                Action::Disconnect(c) => graph.disconnect(c, audio),
                Action::Select(id) => {
                    self.selected = Some(id);
                    graph.bring_to_front(id);
                }
                Action::Deselect => self.selected = None,
            }
        }
    }

    fn draw_grid(&self, p: &Painter, rect: Rect) {
        let z = self.zoom;
        let mut step = GRID * z;
        let mut major_every = 5;
        if step < 10.0 {
            // Zoomed out: majors only.
            step *= 5.0;
            major_every = 1;
        }
        let origin = rect.min + self.pan;
        let first = |o: f32, lo: f32| lo + (o - lo).rem_euclid(step);
        let color = |v: f32, o: f32| {
            if ((v - o) / step).round() as i64 % major_every == 0 {
                theme::GRID_MAJOR
            } else {
                theme::GRID_MINOR
            }
        };

        let mut x = first(origin.x, rect.left());
        while x < rect.right() {
            p.vline(x, rect.y_range(), Stroke::new(1.0, color(x, origin.x)));
            x += step;
        }
        let mut y = first(origin.y, rect.top());
        while y < rect.bottom() {
            p.hline(rect.x_range(), y, Stroke::new(1.0, color(y, origin.y)));
            y += step;
        }
    }
}

/// Context-menu entry; `color` adds a small swatch for node types.
fn menu_item(ui: &mut Ui, color: Option<Color32>, label: &str, hint: &str) -> bool {
    let text = if color.is_some() { format!("    {label}") } else { label.to_owned() };
    let resp = ui.add(
        egui::Button::new(egui::RichText::new(text).color(theme::TEXT).size(12.5))
            .right_text(egui::RichText::new(hint).color(theme::TEXT_FAINT).size(10.5))
            .min_size(vec2(196.0, 22.0)),
    );
    if let Some(c) = color {
        ui.painter().circle_filled(resp.rect.left_center() + vec2(13.0, 0.0), 3.0, c);
    }
    resp.clicked()
}

fn menu_heading(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).size(9.5).color(theme::TEXT_FAINT));
}

fn canvas_menu(ui: &mut Ui, target: MenuTarget, actions: &mut Vec<Action>) {
    match target {
        MenuTarget::Canvas(pos) => {
            menu_heading(ui, "Add node");
            if menu_item(ui, Some(theme::OSC_ACCENTS[0]), "Oscillator", "sine · saw · pulse") {
                actions.push(Action::AddOsc(pos));
                ui.close();
            }
            if menu_item(ui, Some(theme::MIXER_ACCENT), "Mixer", "4 channels") {
                actions.push(Action::AddMixer(pos));
                ui.close();
            }
        }
        MenuTarget::Cable(cable) => {
            if menu_item(ui, None, "Disconnect", "Double-click") {
                actions.push(Action::Disconnect(cable));
                ui.close();
            }
        }
    }
}

fn node_menu(ui: &mut Ui, id: NodeId, actions: &mut Vec<Action>) {
    if menu_item(ui, None, "Duplicate", "Cmd+D") {
        actions.push(Action::Duplicate(id));
        ui.close();
    }
    if menu_item(ui, None, "Delete", "Del") {
        actions.push(Action::Remove(id));
        ui.close();
    }
}
