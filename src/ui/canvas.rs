//! The patching canvas: grid, pan/zoom, nodes and modules, selection, and
//! cable drag & drop.

use std::collections::{HashMap, HashSet};

use egui::{
    Align2, Color32, CursorIcon, FontId, Id, Key, Painter, PointerButton, Pos2, Rect, Response,
    Sense, Stroke, StrokeKind, Ui, Vec2, vec2,
};

use super::groups;
use super::modules;
use super::nodes::{self, NodeView};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::AudioHandle;
use crate::audio::engine::NodeId;
use crate::graph::{Cable, Graph, Group, GroupId, NodeKind, NodeType, PortRef};

const GRID: f32 = 24.0;
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 2.0;

enum Action {
    Add(NodeType, Pos2),
    Remove(NodeId),
    Connect { from: PortRef, to: PortRef },
    Disconnect(Cable),
    /// Select these nodes, replacing the selection unless `true`.
    Select(Vec<NodeId>, bool),
    ToggleSelect(NodeId),
    SelectAll,
    Deselect,
    MoveSelection(Vec2),
    RemoveSelection,
    DuplicateSelection,
    ToggleCollapse(NodeId),
    CollapseSelection,
    CollapseAll(bool),
    GroupSelection,
    UngroupSelection,
    Ungroup(GroupId),
    ToggleGroup(GroupId),
    RenameGroup(GroupId, String),
}

/// A cable being dragged out of a port. `anchor` is the fixed end.
#[derive(Clone, Copy)]
struct CableDrag {
    anchor: PortRef,
    from_output: bool,
}

/// What the context menu was opened on.
#[derive(Clone, Copy)]
enum MenuTarget {
    Canvas(Pos2),
    Cable(Cable),
    Node(NodeId),
    Group(GroupId),
}

/// A collapsed module as laid out this frame (screen space).
struct Card {
    gid: GroupId,
    rect: Rect,
    ins: Vec<PortRef>,
    outs: Vec<PortRef>,
}

/// Per-frame facts shared by every port widget.
struct PortCtx<'a> {
    /// Input port → (its cable, the source's colour, the source's title).
    inputs: &'a HashMap<PortRef, (Cable, Color32, String)>,
    /// Output ports with at least one cable.
    outputs: &'a HashSet<PortRef>,
    drop_target: Option<(PortRef, Pos2)>,
    dragging: Option<CableDrag>,
}

pub struct Editor {
    pub pan: Vec2,
    pub zoom: f32,
    pub selected: HashSet<NodeId>,
    rect: Rect,
    drag: Option<CableDrag>,
    /// Screen position where a rubber-band selection started.
    marquee: Option<Pos2>,
    menu: MenuTarget,
    needs_framing: bool,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: HashSet::new(),
            rect: Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)),
            drag: None,
            marquee: None,
            menu: MenuTarget::Canvas(Pos2::ZERO),
            needs_framing: true,
        }
    }

    pub fn select_only(&mut self, id: NodeId) {
        self.selected.clear();
        self.selected.insert(id);
    }

    /// The module whose nodes are exactly the selection, if any.
    pub fn selected_group<'a>(&self, graph: &'a Graph) -> Option<&'a Group> {
        graph.groups.iter().find(|g| {
            g.members.len() == self.selected.len() && g.members.iter().all(|m| self.selected.contains(m))
        })
    }

    /// Selected nodes, left to right (the order modules list their ports in).
    fn ordered_selection(&self, graph: &Graph) -> Vec<NodeId> {
        let mut nodes: Vec<_> = graph.nodes.iter().filter(|n| self.selected.contains(&n.id)).collect();
        nodes.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x).then(a.pos.y.total_cmp(&b.pos.y)));
        nodes.into_iter().map(|n| n.id).collect()
    }

    fn to_screen(&self, w: Pos2) -> Pos2 {
        self.rect.min + self.pan + w.to_vec2() * self.zoom
    }

    fn to_world(&self, s: Pos2) -> Pos2 {
        ((s - self.rect.min - self.pan) / self.zoom).to_pos2()
    }

    fn to_screen_rect(&self, r: Rect) -> Rect {
        Rect::from_min_size(self.to_screen(r.min), r.size() * self.zoom)
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
        let Some(bounds) = content_bounds(graph) else {
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
        let (pointer, mods) = ui.input(|i| (i.pointer.hover_pos(), i.modifiers));
        let additive = mods.shift || mods.command;
        let mut actions = Vec::new();

        // --- Navigation: scroll, middle/right drag or Alt+drag pan ----------
        let panning = bg.dragged_by(PointerButton::Middle)
            || bg.dragged_by(PointerButton::Secondary)
            || (bg.dragged_by(PointerButton::Primary) && mods.alt && self.marquee.is_none());
        if panning {
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

        // A plain drag on empty canvas draws a selection rectangle.
        if bg.drag_started_by(PointerButton::Primary) && !mods.alt {
            self.marquee = bg.interact_pointer_pos();
        }

        // --- Geometry snapshot (before anything moves) ----------------------
        let hidden: HashSet<NodeId> = graph
            .groups
            .iter()
            .filter(|g| g.collapsed)
            .flat_map(|g| g.members.iter().copied())
            .collect();
        let frames: Vec<(GroupId, Rect)> = graph
            .groups
            .iter()
            .filter(|g| !g.collapsed)
            .filter_map(|g| Some((g.id, self.to_screen_rect(groups::frame_rect(graph, g)?))))
            .collect();
        let cards: Vec<Card> = graph
            .groups
            .iter()
            .filter(|g| g.collapsed)
            .filter_map(|g| {
                let (r, ins, outs) = groups::card_layout(graph, g)?;
                Some(Card { gid: g.id, rect: self.to_screen_rect(r), ins, outs })
            })
            .collect();
        let node_rects: Vec<Option<Rect>> = graph
            .nodes
            .iter()
            .map(|n| (!hidden.contains(&n.id)).then(|| self.to_screen_rect(Rect::from_min_size(n.pos, n.size()))))
            .collect();

        // Screen position of every visible port. A collapsed module shows
        // its members' outer ports on its card; inner ports are hidden.
        let mut in_pos: HashMap<PortRef, Pos2> = HashMap::new();
        let mut out_pos: HashMap<PortRef, Pos2> = HashMap::new();
        for n in graph.nodes.iter().filter(|n| !hidden.contains(&n.id)) {
            for port in 0..n.inputs().len() {
                in_pos.insert(PortRef { node: n.id, port }, self.to_screen(n.input_pos(port)));
            }
            for port in 0..n.outputs().len() {
                out_pos.insert(PortRef { node: n.id, port }, self.to_screen(n.output_pos(port)));
            }
        }
        for card in &cards {
            for (k, pr) in card.ins.iter().enumerate() {
                in_pos.insert(*pr, card.rect.min + groups::card_port(k, false) * z);
            }
            for (k, pr) in card.outs.iter().enumerate() {
                out_pos.insert(*pr, card.rect.min + groups::card_port(k, true) * z);
            }
        }

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
                Some(Wire {
                    cable: *c,
                    a: *out_pos.get(&c.from)?,
                    b: *in_pos.get(&c.to)?,
                    color: src.accent,
                    level: src.signal_level(),
                })
            })
            .collect();

        let mut inputs: HashMap<PortRef, (Cable, Color32, String)> = HashMap::new();
        let mut outputs: HashSet<PortRef> = HashSet::new();
        for c in &graph.cables {
            if let Some(src) = graph.node(c.from.node) {
                inputs.insert(c.to, (*c, src.accent, src.title.clone()));
                outputs.insert(c.from);
            }
        }

        let hovered_cable = pointer
            .filter(|_| self.drag.is_none() && self.marquee.is_none() && bg.hovered())
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
            let candidates = if d.from_output { &in_pos } else { &out_pos };
            let mut best: Option<(PortRef, Pos2, f32)> = None;
            for (&pr, &pos) in candidates {
                let valid = if d.from_output {
                    !graph.would_cycle(d.anchor.node, pr.node)
                } else {
                    !graph.would_cycle(pr.node, d.anchor.node)
                };
                let dist = (pos - p).length();
                if valid && dist < 22.0 * z.max(0.8) && best.is_none_or(|b| dist < b.2) {
                    best = Some((pr, pos, dist));
                }
            }
            best.map(|(pr, pos, _)| (pr, pos))
        });

        // --- Canvas clicks & context menu --------------------------------
        // One menu for the whole canvas, aimed at whatever is on top under
        // the pointer, so right-clicking a knob or display still reaches its
        // node. Ctrl+click works too on macOS.
        let open_menu = ui.rect_contains_pointer(rect)
            && ui.input(|i| {
                i.pointer.secondary_clicked()
                    || (cfg!(target_os = "macos") && i.modifiers.ctrl && i.pointer.primary_clicked())
            });
        if open_menu && let Some(p) = pointer {
            let card = cards.iter().rev().find(|c| c.rect.contains(p)).map(|c| c.gid);
            let node = graph
                .nodes
                .iter()
                .zip(&node_rects)
                .rev()
                .find(|(_, r)| r.is_some_and(|r| r.contains(p)))
                .map(|(n, _)| n.id);
            let frame = frames
                .iter()
                .rev()
                .find(|(_, f)| p.y < f.top() + nodes::HEADER_H * z && f.contains(p))
                .map(|(gid, _)| *gid);
            self.menu = if let Some(gid) = card.or(if node.is_none() { frame } else { None }) {
                if let Some(g) = graph.group(gid)
                    && !g.members.iter().all(|m| self.selected.contains(m))
                {
                    actions.push(Action::Select(g.members.clone(), false));
                }
                MenuTarget::Group(gid)
            } else if let Some(id) = node {
                if !self.selected.contains(&id) {
                    actions.push(Action::Select(vec![id], false));
                }
                MenuTarget::Node(id)
            } else if let Some(c) = hovered_cable {
                MenuTarget::Cable(c)
            } else {
                MenuTarget::Canvas(self.to_world(p))
            };
        }
        if bg.double_clicked()
            && let Some(c) = hovered_cable
        {
            actions.push(Action::Disconnect(c));
        } else if bg.clicked() && !additive {
            actions.push(Action::Deselect);
        }

        // --- Expanded modules: frames behind everything else ---------------
        for &(gid, frame) in &frames {
            if !frame.intersects(rect) {
                continue;
            }
            let g = graph.group(gid).expect("frame of an existing module");
            let selected = g.members.iter().all(|m| self.selected.contains(m));
            let head = Rect::from_min_size(frame.min, vec2(frame.width(), nodes::HEADER_H * z));
            let resp = ui.interact(head, Id::unique(("group", gid)), Sense::CLICK | Sense::DRAG);
            module_pointer(ui, &resp, g, selected, additive, z, &mut actions);
            let h = groups::draw_frame(ui, &painter, g, frame, selected, z);
            push_header(h, gid, &mut actions);
        }

        // --- Cables ------------------------------------------------------
        for w in &wires {
            widgets::draw_cable(&painter, w.a, w.b, w.color, w.level, hovered_cable == Some(w.cable), z);
        }

        // --- Nodes -------------------------------------------------------
        let ctx = PortCtx {
            inputs: &inputs,
            outputs: &outputs,
            drop_target,
            dragging: self.drag,
        };
        for (idx, node) in graph.nodes.iter_mut().enumerate() {
            let Some(rect_s) = node_rects[idx] else {
                continue;
            };
            if !rect_s.expand(40.0).intersects(rect) {
                continue;
            }
            let id = node.id;
            let selected = self.selected.contains(&id);
            let body = ui.interact(rect_s, Id::unique(("node", id)), Sense::CLICK | Sense::DRAG);
            if body.clicked() {
                actions.push(if additive { Action::ToggleSelect(id) } else { Action::Select(vec![id], false) });
            } else if body.drag_started() && !selected {
                actions.push(Action::Select(vec![id], additive));
            }
            if body.dragged_by(PointerButton::Primary) {
                actions.push(Action::MoveSelection(body.drag_delta() / z));
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
            let collapsed = node.collapsed;

            let view = NodeView {
                ui,
                painter: &painter,
                min: rect_s.min,
                z,
            };
            let header = nodes::frame(&view, node, selected);
            if header.close {
                actions.push(Action::Remove(id));
            }
            let on_header = body
                .interact_pointer_pos()
                .is_some_and(|p| p.y < rect_s.top() + nodes::HEADER_H * z);
            if header.toggle_collapse || (body.double_clicked() && on_header) {
                actions.push(Action::ToggleCollapse(id));
            }

            let in_count = node.inputs().len();
            let sources: Vec<Option<(Color32, String)>> = (0..in_count)
                .map(|i| inputs.get(&PortRef { node: id, port: i }).map(|(_, c, n)| (*c, n.clone())))
                .collect();
            let accent = node.accent;
            let connected: Vec<bool> = sources.iter().map(Option::is_some).collect();
            let out = outputs.contains(&PortRef { node: id, port: 0 });
            if collapsed {
                nodes::collapsed(&view, node, &connected, out);
            } else {
                match &mut node.kind {
                    NodeKind::Osc(o) => nodes::osc(&view, id, accent, o, &connected, out),
                    NodeKind::Mixer(m) => nodes::mixer(&view, id, accent, m, &sources),
                    NodeKind::Module(m) => modules::draw(&view, id, accent, m, &connected, out),
                }
            }

            // Ports last so they sit on top of everything else in the node.
            for port in 0..in_count {
                let pr = PortRef { node: id, port };
                self.port(ui, &painter, pr, true, in_pos[&pr], accent, &ctx, &mut actions);
            }
            for port in 0..node.outputs().len() {
                let pr = PortRef { node: id, port };
                self.port(ui, &painter, pr, false, out_pos[&pr], accent, &ctx, &mut actions);
            }
        }

        // --- Collapsed modules -------------------------------------------
        for card in &cards {
            if !card.rect.expand(40.0).intersects(rect) {
                continue;
            }
            let g = graph.group(card.gid).expect("card of an existing module");
            let selected = g.members.iter().all(|m| self.selected.contains(m));
            let resp = ui.interact(card.rect, Id::unique(("card", card.gid)), Sense::CLICK | Sense::DRAG);
            module_pointer(ui, &resp, g, selected, additive, z, &mut actions);
            let gid = g.id;
            let connected = |pr: PortRef, input: bool| {
                if input { inputs.contains_key(&pr) } else { outputs.contains(&pr) }
            };
            let ports = (card.ins.as_slice(), card.outs.as_slice());
            let h = groups::draw_card(ui, &painter, graph, g, card.rect, ports, connected, selected, z);
            push_header(h, gid, &mut actions);

            let accent_of = |pr: &PortRef| graph.node(pr.node).map_or(theme::ACCENT, |n| n.accent);
            for pr in &card.ins {
                self.port(ui, &painter, *pr, true, in_pos[pr], accent_of(pr), &ctx, &mut actions);
            }
            for pr in &card.outs {
                self.port(ui, &painter, *pr, false, out_pos[pr], accent_of(pr), &ctx, &mut actions);
            }
        }

        // --- Cable being dragged -----------------------------------------
        if let Some(d) = self.drag {
            let anchor = if d.from_output { out_pos.get(&d.anchor) } else { in_pos.get(&d.anchor) };
            if let (Some(&anchor), Some(src), Some(p)) = (anchor, graph.node(d.anchor.node), pointer) {
                let end = drop_target.map_or(p, |(_, pos)| pos);
                let (a, b) = if d.from_output { (anchor, end) } else { (end, anchor) };
                let color = if drop_target.is_some() { src.accent } else { alpha(src.accent, 0.7) };
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

        // --- Rubber-band selection ---------------------------------------
        if let Some(start) = self.marquee {
            let end = pointer.unwrap_or(start);
            let band = Rect::from_two_pos(start, end);
            painter.rect(
                band,
                2.0,
                alpha(theme::ACCENT, 0.08),
                Stroke::new(1.0, alpha(theme::ACCENT, 0.7)),
                StrokeKind::Inside,
            );
            if !ui.input(|i| i.pointer.primary_down()) {
                let mut hit: Vec<NodeId> = graph
                    .nodes
                    .iter()
                    .zip(&node_rects)
                    .filter(|(_, r)| r.is_some_and(|r| r.intersects(band)))
                    .map(|(n, _)| n.id)
                    .collect();
                for card in cards.iter().filter(|c| c.rect.intersects(band)) {
                    hit.extend(graph.group(card.gid).map_or(&[][..], |g| &g.members[..]));
                }
                actions.push(Action::Select(hit, additive));
                self.marquee = None;
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

        // --- Context menu -------------------------------------------------
        let menu = self.menu;
        let has_selection = !self.selected.is_empty();
        egui::Popup::menu(&bg)
            .open_memory(open_menu.then_some(egui::SetOpenCommand::Bool(true)))
            .at_pointer_fixed()
            .show(|ui| match menu {
                MenuTarget::Canvas(pos) => canvas_menu(ui, pos, has_selection, &mut actions),
                MenuTarget::Cable(cable) => {
                    if menu_item(ui, "Disconnect", "Double-click") {
                        actions.push(Action::Disconnect(cable));
                        ui.close();
                    }
                }
                MenuTarget::Node(id) => match graph.node(id) {
                    Some(n) => node_menu(ui, n.collapsed, graph.group_of(id).is_some(), &mut actions),
                    None => ui.close(),
                },
                MenuTarget::Group(gid) => match graph.group(gid) {
                    Some(g) => group_menu(ui, gid, &g.title, g.collapsed, &mut actions),
                    None => ui.close(),
                },
            });

        // --- Keyboard (not while typing, e.g. a module name) --------------
        if ui.memory(|m| m.focused().is_none()) {
            self.keyboard(ui, graph, &mut actions);
        }

        self.apply(actions, graph, audio);
    }

    fn keyboard(&mut self, ui: &Ui, graph: &Graph, actions: &mut Vec<Action>) {
        let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
        let has_selection = !self.selected.is_empty();

        if has_selection && (pressed(Key::Delete) || pressed(Key::Backspace)) {
            actions.push(Action::RemoveSelection);
        }
        if has_selection && cmd && pressed(Key::D) {
            actions.push(Action::DuplicateSelection);
        }
        if cmd && pressed(Key::A) {
            actions.push(Action::SelectAll);
        }
        if pressed(Key::Escape) {
            actions.push(Action::Deselect);
        }
        if cmd && pressed(Key::G) {
            actions.push(if shift { Action::UngroupSelection } else { Action::GroupSelection });
        }
        if !cmd && pressed(Key::C) {
            if shift {
                // Collapse everything, or expand if all is already collapsed.
                let any_open = graph.nodes.iter().any(|n| !n.collapsed) || graph.groups.iter().any(|g| !g.collapsed);
                actions.push(Action::CollapseAll(any_open));
            } else if has_selection {
                actions.push(Action::CollapseSelection);
            }
        }
        if !cmd && pressed(Key::F) {
            self.frame_all(graph);
        }
    }

    /// One port: drawing, hover and starting a cable drag from it.
    #[allow(clippy::too_many_arguments)]
    fn port(
        &mut self,
        ui: &Ui,
        painter: &Painter,
        pr: PortRef,
        input: bool,
        c: Pos2,
        accent: Color32,
        ctx: &PortCtx,
        actions: &mut Vec<Action>,
    ) {
        let z = self.zoom;
        let id = Id::unique((if input { "in" } else { "out" }, pr.node, pr.port));
        let resp = ui
            .interact(Rect::from_center_size(c, Vec2::splat(20.0 * z)), id, Sense::CLICK | Sense::DRAG)
            .on_hover_cursor(CursorIcon::Crosshair);
        let targeted = ctx
            .drop_target
            .is_some_and(|(t, _)| t == pr && ctx.dragging.is_some_and(|d| d.from_output == input));
        let hot = resp.hovered() || targeted;
        if input {
            let existing = ctx.inputs.get(&pr);
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
            let color = existing.map_or(accent, |(_, c, _)| *c);
            widgets::port(painter, c, existing.is_some(), hot, color, z);
        } else {
            if resp.drag_started() {
                self.drag = Some(CableDrag { anchor: pr, from_output: true });
            }
            widgets::port(painter, c, ctx.outputs.contains(&pr), hot, accent, z);
        }
    }

    fn apply(&mut self, actions: Vec<Action>, graph: &mut Graph, audio: &AudioHandle) {
        for action in actions {
            match action {
                Action::Add(ty, p) => {
                    let id = graph.add(ty, p, audio);
                    self.select_only(id);
                }
                Action::Remove(id) => {
                    graph.remove_node(id, audio);
                    self.selected.remove(&id);
                }
                Action::Connect { from, to } => {
                    graph.connect(from, to, audio);
                }
                Action::Disconnect(c) => graph.disconnect(c, audio),
                Action::Select(ids, additive) => {
                    if !additive {
                        self.selected.clear();
                    }
                    for id in ids {
                        if graph.node(id).is_some() {
                            self.selected.insert(id);
                            graph.bring_to_front(id);
                        }
                    }
                }
                Action::ToggleSelect(id) => {
                    if !self.selected.remove(&id) {
                        self.selected.insert(id);
                        graph.bring_to_front(id);
                    }
                }
                Action::SelectAll => self.selected = graph.nodes.iter().map(|n| n.id).collect(),
                Action::Deselect => self.selected.clear(),
                Action::MoveSelection(delta) => graph.move_nodes(&self.selected, delta),
                Action::RemoveSelection => {
                    for id in self.ordered_selection(graph) {
                        graph.remove_node(id, audio);
                    }
                    self.selected.clear();
                }
                Action::DuplicateSelection => {
                    let ids = self.ordered_selection(graph);
                    self.selected = graph.duplicate_nodes(&ids, audio).into_iter().collect();
                }
                Action::ToggleCollapse(id) => graph.toggle_collapsed(id),
                Action::CollapseSelection => self.collapse_selection(graph),
                Action::CollapseAll(collapsed) => graph.set_all_collapsed(collapsed),
                Action::GroupSelection => {
                    let ids = self.ordered_selection(graph);
                    graph.create_group(&ids);
                }
                Action::UngroupSelection => {
                    let touched: Vec<GroupId> = graph
                        .groups
                        .iter()
                        .filter(|g| g.members.iter().any(|m| self.selected.contains(m)))
                        .map(|g| g.id)
                        .collect();
                    for gid in touched {
                        graph.ungroup(gid);
                    }
                }
                Action::Ungroup(gid) => graph.ungroup(gid),
                Action::ToggleGroup(gid) => {
                    if let Some(collapsed) = graph.group(gid).map(|g| g.collapsed) {
                        graph.set_group_collapsed(gid, !collapsed);
                    }
                }
                Action::RenameGroup(gid, title) => graph.rename_group(gid, &title),
            }
        }
    }

    /// C: when the selection is made of whole modules, fold those modules;
    /// otherwise fold the selected nodes themselves.
    fn collapse_selection(&self, graph: &mut Graph) {
        let whole: Vec<(GroupId, bool)> = graph
            .groups
            .iter()
            .filter(|g| g.members.iter().all(|m| self.selected.contains(m)))
            .map(|g| (g.id, g.collapsed))
            .collect();
        let covered = self
            .selected
            .iter()
            .all(|id| graph.group_of(*id).is_some_and(|g| whole.iter().any(|(gid, _)| *gid == g.id)));
        if !whole.is_empty() && covered {
            let collapse = whole.iter().any(|(_, c)| !c);
            for (gid, _) in whole {
                graph.set_group_collapsed(gid, collapse);
            }
        } else {
            let collapse = self.selected.iter().any(|id| graph.node(*id).is_some_and(|n| !n.collapsed));
            for id in &self.selected {
                graph.set_collapsed(*id, collapse);
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

/// Bounds of everything drawn: free nodes, module frames and module cards.
fn content_bounds(graph: &Graph) -> Option<Rect> {
    let hidden: HashSet<NodeId> = graph
        .groups
        .iter()
        .filter(|g| g.collapsed)
        .flat_map(|g| g.members.iter().copied())
        .collect();
    let nodes = graph
        .nodes
        .iter()
        .filter(|n| !hidden.contains(&n.id))
        .map(|n| Rect::from_min_size(n.pos, n.size()));
    let modules = graph.groups.iter().filter_map(|g| {
        if g.collapsed {
            groups::card_layout(graph, g).map(|(r, ..)| r)
        } else {
            groups::frame_rect(graph, g)
        }
    });
    nodes.chain(modules).reduce(|a, b| a.union(b))
}

/// Clicking or dragging a module's header (or card) selects and moves all
/// of its nodes; double-clicking folds it.
fn module_pointer(ui: &Ui, resp: &Response, g: &Group, selected: bool, additive: bool, z: f32, actions: &mut Vec<Action>) {
    if resp.clicked() || (resp.drag_started() && !selected) {
        actions.push(Action::Select(g.members.clone(), additive));
    }
    if resp.dragged_by(PointerButton::Primary) {
        actions.push(Action::MoveSelection(resp.drag_delta() / z));
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    if resp.double_clicked() {
        actions.push(Action::ToggleGroup(g.id));
    }
}

fn push_header(h: groups::HeaderActions, gid: GroupId, actions: &mut Vec<Action>) {
    if h.toggle_collapse {
        actions.push(Action::ToggleGroup(gid));
    }
    if h.ungroup {
        actions.push(Action::Ungroup(gid));
    }
}

fn menu_item(ui: &mut Ui, label: &str, hint: &str) -> bool {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(theme::TEXT).size(12.5))
            .right_text(egui::RichText::new(hint).color(theme::TEXT_FAINT).size(10.5))
            .min_size(vec2(220.0, 22.0)),
    )
    .clicked()
}

fn menu_heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text).size(9.5).color(theme::TEXT_FAINT));
}

/// The grouped list of node types; returns the one picked (and closes).
pub fn add_menu(ui: &mut Ui) -> Option<NodeType> {
    let mut picked = None;
    for (group, types) in NodeType::MENU {
        menu_heading(ui, group);
        for &ty in types {
            if menu_item(ui, ty.name(), ty.hint()) {
                picked = Some(ty);
                ui.close();
            }
        }
    }
    picked
}

fn canvas_menu(ui: &mut Ui, pos: Pos2, has_selection: bool, actions: &mut Vec<Action>) {
    if let Some(ty) = add_menu(ui) {
        actions.push(Action::Add(ty, pos));
    }
    ui.separator();
    if has_selection && menu_item(ui, "Make module from selection", "Cmd+G") {
        actions.push(Action::GroupSelection);
        ui.close();
    }
    if menu_item(ui, "Select all", "Cmd+A") {
        actions.push(Action::SelectAll);
        ui.close();
    }
    if menu_item(ui, "Collapse all", "Shift+C") {
        actions.push(Action::CollapseAll(true));
        ui.close();
    }
    if menu_item(ui, "Expand all", "Shift+C") {
        actions.push(Action::CollapseAll(false));
        ui.close();
    }
}

/// Acts on the selection (right-clicking a node selects it first).
fn node_menu(ui: &mut Ui, collapsed: bool, in_group: bool, actions: &mut Vec<Action>) {
    let fold = if collapsed { "Expand" } else { "Collapse" };
    let mut item = |label: &str, hint: &str, action: Action| {
        if menu_item(ui, label, hint) {
            actions.push(action);
            ui.close();
        }
    };
    item(fold, "C", Action::CollapseSelection);
    item("Make module", "Cmd+G", Action::GroupSelection);
    item("Duplicate", "Cmd+D", Action::DuplicateSelection);
    item("Delete", "Del", Action::RemoveSelection);
    if in_group {
        item("Ungroup module", "Cmd+Shift+G", Action::UngroupSelection);
    }
}

fn group_menu(ui: &mut Ui, gid: GroupId, title: &str, collapsed: bool, actions: &mut Vec<Action>) {
    menu_heading(ui, "Module name");
    let mut name = title.to_owned();
    if ui.add(egui::TextEdit::singleline(&mut name).desired_width(220.0)).changed() {
        actions.push(Action::RenameGroup(gid, name));
    }
    ui.separator();
    if menu_item(ui, if collapsed { "Expand" } else { "Collapse" }, "C") {
        actions.push(Action::ToggleGroup(gid));
        ui.close();
    }
    if menu_item(ui, "Duplicate module", "Cmd+D") {
        actions.push(Action::DuplicateSelection);
        ui.close();
    }
    if menu_item(ui, "Ungroup", "Cmd+Shift+G") {
        actions.push(Action::Ungroup(gid));
        ui.close();
    }
    if menu_item(ui, "Delete module and its nodes", "Del") {
        actions.push(Action::RemoveSelection);
        ui.close();
    }
}
