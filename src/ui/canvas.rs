//! The patching canvas: grid, pan/zoom, nodes and modules, selection, and
//! cable drag & drop.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use egui::{
    Align2, Color32, CursorIcon, FontId, Id, Key, Painter, PointerButton, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use super::groups;
use super::layout;
use super::modules;
use super::nodes::{self, KnobHit, NodeView};
use super::palette::{PluginEntry, Spawn};
use super::plugin_node;
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::AudioHandle;
use crate::audio::engine::NodeId;
use crate::graph::{
    Cable, Console, Graph, Group, GroupId, MIDI_PORT, NodeKind, NodeType, PortRef, Target, is_midi,
};
use crate::project::{self, Preset};

const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 2.0;
/// Length of layout glides and view animations.
const ANIM_SECS: f64 = 0.5;
/// Glides start in a wave from left to right, over this long.
const GLIDE_STAGGER: f64 = 0.09;

/// Nodes gliding to new positions: (node, from, to, start delay).
struct Glide {
    moves: Vec<(NodeId, Pos2, Pos2, f64)>,
    start: f64,
}

impl Glide {
    /// Each node sets off a little after the ones to its left, so the
    /// patch flows into place rather than jumping all at once.
    fn new(moves: Vec<(NodeId, Pos2, Pos2)>, start: f64) -> Self {
        let (lo, hi) = moves
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), m| {
                (lo.min(m.2.x), hi.max(m.2.x))
            });
        let span = (hi - lo).max(1.0);
        let moves = moves
            .into_iter()
            .map(|(id, from, to)| (id, from, to, GLIDE_STAGGER * ((to.x - lo) / span) as f64))
            .collect();
        Self { moves, start }
    }

    fn end(&self) -> f64 {
        self.start + ANIM_SECS + GLIDE_STAGGER
    }
}

/// Animation curve: the motion of a critically damped spring, rescaled to
/// land exactly at 1. It leaves gently, moves briskly and settles softly,
/// with no overshoot.
fn spring(t: f64) -> f32 {
    const W: f64 = 8.0;
    let t = t.clamp(0.0, 1.0);
    let curve = |t: f64| 1.0 - (1.0 + W * t) * (-W * t).exp();
    (curve(t) / curve(1.0)) as f32
}

/// The view moving to a new (pan, zoom).
struct ViewAnim {
    from: (Vec2, f32),
    to: (Vec2, f32),
    start: f64,
}

/// Work that needs the app (files, dialogs) rather than the canvas.
pub enum Request {
    SaveToLibrary(GroupId),
    DeletePreset(usize),
    RevealLibrary,
    RescanPlugins,
    /// Open the folder node plugins are installed into.
    RevealNodePlugins,
}

/// What was picked in the "add" menu.
pub enum AddChoice {
    Node(NodeType),
    Preset(usize),
    Plugin(usize),
    Request(Request),
}

enum Action {
    Add(NodeType, Pos2),
    AddPreset(usize, Pos2),
    AddPlugin(usize, Pos2),
    Request(Request),
    Remove(NodeId),
    Connect {
        from: PortRef,
        to: PortRef,
    },
    Disconnect(Cable),
    /// Select these nodes, replacing the selection unless `true`.
    Select(Vec<NodeId>, bool),
    ToggleSelect(NodeId),
    SelectAll,
    Deselect,
    MoveSelection(Vec2),
    AutoLayout,
    UndoLayout,
    FitAll,
    FitSelection,
    /// Fill the view with these nodes, in front; the view to go back to is
    /// kept (see `Editor::focus`).
    Focus(Vec<NodeId>),
    FocusSelection,
    Unfocus,
    ZoomBy(f32),
    ZoomActual,
    ToggleAutoFit,
    RemoveSelection,
    DuplicateSelection,
    ToggleCollapse(NodeId),
    CollapseSelection,
    CollapseAll(bool),
    GroupSelection,
    UngroupSelection,
    Ungroup(GroupId),
    ToggleGroup(GroupId),
    StartRename(GroupId),
    RenameGroup(GroupId, String),
    /// Wait for a click on a control to map this MIDI node to.
    StartMapping(NodeId),
    CancelMapping,
    SetTarget(NodeId, Option<Target>),
    /// Reshape a Console (12/24 channels, horizontal/vertical).
    SetConsole(NodeId, Console),
    /// Bypass these effects, or switch them back on if all are bypassed.
    ToggleBypass(Vec<NodeId>),
    BypassSelection,
}

/// A module title being edited in place.
struct Rename {
    gid: GroupId,
    text: String,
    /// Focus the field on its first frame.
    fresh: bool,
    /// Select the old name once the field has focus (egui drops a
    /// selection made before that).
    select: bool,
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

/// What the pointer rests on, for the type label.
#[derive(Clone, Copy, PartialEq)]
enum TipTarget {
    Node(NodeId),
    Group(GroupId),
}

/// The type label waits this long before fading in, so passing over the
/// canvas doesn't flash labels everywhere.
const TIP_DELAY: f64 = 0.35;
const TIP_FADE: f64 = 0.15;

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
    /// Modules saved in the library, offered in the add menu.
    pub library: Vec<Preset>,
    /// Installed VST3 plugins, offered in the add menu.
    pub plugins: Vec<PluginEntry>,
    /// Requests for the app, drained after each frame.
    pub requests: Vec<Request>,
    renaming: Option<Rename>,
    /// The node or module under the pointer, and since when.
    tip: Option<(TipTarget, f64)>,
    /// MIDI node waiting for a click on the control it should drive.
    mapping: Option<NodeId>,
    /// Keep the whole patch in view as it changes.
    pub auto_fit: bool,
    glide: Option<Glide>,
    view_anim: Option<ViewAnim>,
    /// Positions before the last auto layout, for Cmd+Z.
    layout_undo: Option<Vec<(NodeId, Pos2)>>,
    /// Zoomed onto these nodes (Z): them, and the view to go back to.
    focus: Option<(Vec<NodeId>, (Vec2, f32))>,
    /// Short messages for the status bar, drained by the app.
    pub notices: Vec<String>,
    /// Time of the current frame.
    now: f64,
    /// A menu (or another popup) was open when the pointer last went down:
    /// that click only closes it, and leaves the selection alone.
    menu_at_press: bool,
    /// The board's stickers and background on screen (see `ui::board`).
    pub board_ui: super::board::BoardUi,
    /// Where the selection bar was drawn last frame (screen), if it was,
    /// and its buttons.
    bar: Option<Rect>,
    bar_buttons: Vec<(BarItem, Rect)>,
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
            library: Vec::new(),
            plugins: Vec::new(),
            requests: Vec::new(),
            renaming: None,
            tip: None,
            mapping: None,
            auto_fit: false,
            glide: None,
            view_anim: None,
            layout_undo: None,
            focus: None,
            notices: Vec::new(),
            now: 0.0,
            menu_at_press: false,
            board_ui: super::board::BoardUi::default(),
            bar: None,
            bar_buttons: Vec::new(),
        }
    }

    /// Forgets selection and gestures, and frames the patch on the next
    /// frame (after a project is opened or created).
    pub fn reset(&mut self) {
        self.selected.clear();
        self.drag = None;
        self.marquee = None;
        self.renaming = None;
        self.mapping = None;
        self.glide = None;
        self.view_anim = None;
        self.layout_undo = None;
        self.focus = None;
        self.needs_framing = true;
    }

    pub fn select_only(&mut self, id: NodeId) {
        self.selected.clear();
        self.selected.insert(id);
    }

    /// The module whose nodes are exactly the selection, if any.
    pub fn selected_group<'a>(&self, graph: &'a Graph) -> Option<&'a Group> {
        graph.groups.iter().find(|g| {
            g.members.len() == self.selected.len()
                && g.members.iter().all(|m| self.selected.contains(m))
        })
    }

    /// The selection bar as last drawn (screen), and the centre of each of
    /// its buttons by label ("Make module", "Delete"…), for tests.
    #[cfg(test)]
    pub fn selection_bar_layout(&self) -> Option<(Rect, Vec<(&'static str, Pos2)>)> {
        let names = |i: BarItem| match i {
            BarItem::Module => "Make module",
            BarItem::Collapse => "Collapse",
            BarItem::Duplicate => "Duplicate",
            BarItem::Delete => "Delete",
        };
        self.bar.map(|b| {
            (
                b,
                self.bar_buttons
                    .iter()
                    .map(|&(i, r)| (names(i), r.center()))
                    .collect(),
            )
        })
    }

    /// Selected nodes, left to right (the order modules list their ports in).
    fn ordered_selection(&self, graph: &Graph) -> Vec<NodeId> {
        let mut nodes: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| self.selected.contains(&n.id))
            .collect();
        nodes.sort_by(|a, b| {
            a.pos
                .x
                .total_cmp(&b.pos.x)
                .then(a.pos.y.total_cmp(&b.pos.y))
        });
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
        match content_bounds(graph) {
            Some(bounds) => (self.pan, self.zoom) = self.fit_params(bounds),
            None => {
                self.zoom = 1.0;
                self.pan = self.rect.size() * 0.5;
            }
        }
    }

    /// The view (pan, zoom) that shows `bounds` with a margin, at most 100%.
    fn fit_params(&self, bounds: Rect) -> (Vec2, f32) {
        let avail = self.rect.size() - vec2(120.0, 120.0);
        let zoom = (avail.x / bounds.width())
            .min(avail.y / bounds.height())
            .clamp(MIN_ZOOM, 1.0);
        (
            self.rect.size() * 0.5 - bounds.center().to_vec2() * zoom,
            zoom,
        )
    }

    /// Where the view is heading (or is, when it isn't moving).
    fn view_target(&self) -> (Vec2, f32) {
        self.view_anim
            .as_ref()
            .map_or((self.pan, self.zoom), |a| a.to)
    }

    fn animate_view(&mut self, pan: Vec2, zoom: f32, now: f64) {
        self.view_anim = Some(ViewAnim {
            from: (self.pan, self.zoom),
            to: (pan, zoom),
            start: now,
        });
    }

    /// Manual navigation takes over from auto-fit and view animations.
    fn take_over_view(&mut self) {
        self.auto_fit = false;
        self.view_anim = None;
    }

    pub fn fit_all(&mut self, graph: &Graph, now: f64) {
        if let Some(bounds) = content_bounds(graph) {
            let (pan, zoom) = self.fit_params(bounds);
            self.animate_view(pan, zoom, now);
        }
    }

    /// The node under a screen point, topmost first (not one folded into a
    /// collapsed module).
    fn node_under(&self, graph: &Graph, q: Pos2) -> Option<NodeId> {
        graph
            .nodes
            .iter()
            .rev()
            .filter(|n| graph.group_of(n.id).is_none_or(|g| !g.collapsed))
            .find(|n| {
                self.to_screen_rect(Rect::from_min_size(n.pos, n.size()))
                    .contains(q)
            })
            .map(|n| n.id)
    }

    /// Zooms until `ids` fill the view (past 100%, unlike fitting), brings
    /// them to the front and selects them. The view before the first
    /// focus is kept for going back.
    fn focus_on(&mut self, graph: &mut Graph, ids: Vec<NodeId>, now: f64) {
        let mut bounds: Option<Rect> = None;
        for id in &ids {
            if let Some(n) = graph.node(*id) {
                let r = Rect::from_min_size(n.pos, n.size());
                bounds = Some(bounds.map_or(r, |b| b.union(r)));
            }
        }
        let Some(bounds) = bounds else { return };
        let avail = self.rect.size() - vec2(48.0, 48.0);
        let zoom = (avail.x / bounds.width())
            .min(avail.y / bounds.height())
            .clamp(MIN_ZOOM, MAX_ZOOM);
        let pan = self.rect.size() * 0.5 - bounds.center().to_vec2() * zoom;
        let back = self
            .focus
            .take()
            .map_or_else(|| self.view_target(), |f| f.1);
        self.auto_fit = false;
        self.animate_view(pan, zoom, now);
        for id in &ids {
            graph.bring_to_front(*id);
        }
        self.selected = ids.iter().copied().collect();
        if let [one] = ids.as_slice()
            && let Some(n) = graph.node(*one)
        {
            self.notices
                .push(format!("{}  ·  Z or Esc to go back", n.title));
        }
        self.focus = Some((ids, back));
    }

    /// Back to the view from before the zoom onto a node.
    fn unfocus(&mut self, now: f64) {
        if let Some((_, (pan, zoom))) = self.focus.take() {
            self.animate_view(pan, zoom, now);
        }
    }

    pub fn fit_selection(&mut self, graph: &Graph, now: f64) {
        if let Some(bounds) = selection_bounds(graph, &self.selected) {
            let (pan, zoom) = self.fit_params(bounds);
            self.auto_fit = false;
            self.animate_view(pan, zoom, now);
        }
    }

    /// Zooms by `factor` around the middle of the view.
    pub fn zoom_by(&mut self, factor: f32, now: f64) {
        let (pan, zoom) = self.view_target();
        let new_zoom = (zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let centre = self.rect.size() * 0.5;
        let world = (centre - pan) / zoom;
        self.take_over_view();
        self.animate_view(centre - world * new_zoom, new_zoom, now);
    }

    pub fn zoom_actual(&mut self, now: f64) {
        let zoom = self.view_target().1;
        self.zoom_by(1.0 / zoom, now);
    }

    /// Auto-fit keeps the whole patch in view as it changes.
    pub fn set_auto_fit(&mut self, on: bool, graph: &Graph, now: f64) {
        self.auto_fit = on;
        if on {
            self.fit_all(graph, now);
        }
    }

    /// Arranges the selection (two or more nodes, or a module) or else the
    /// whole patch, gliding the nodes into place.
    pub fn auto_layout(&mut self, graph: &mut Graph, now: f64) {
        let partial = self.selected.len() >= 2 || self.selected_group(graph).is_some();
        let only = partial.then(|| self.selected.clone());
        let moves: Vec<(NodeId, Pos2, Pos2)> = layout::plan(graph, only.as_ref())
            .into_iter()
            .filter_map(|(id, to)| Some((id, graph.node(id)?.pos, to)))
            .collect();
        if moves
            .iter()
            .all(|(_, from, to)| (*from - *to).length() < 0.5)
        {
            self.notices.push("Already tidy".to_owned());
            return;
        }
        if !partial {
            // Zoom to where everything will end up.
            for (id, _, to) in &moves {
                graph.node_mut(*id).expect("planned node").pos = *to;
            }
            let bounds = content_bounds(graph);
            for (id, from, _) in &moves {
                graph.node_mut(*id).expect("planned node").pos = *from;
            }
            if let Some(b) = bounds {
                let (pan, zoom) = self.fit_params(b);
                self.animate_view(pan, zoom, now);
            }
        }
        self.layout_undo = Some(moves.iter().map(|(id, from, _)| (*id, *from)).collect());
        self.glide = Some(Glide::new(moves, now));
        self.notices
            .push("Auto layout  ·  Cmd+Z to undo".to_owned());
    }

    /// Puts nodes back where they were before the last auto layout.
    pub fn undo_layout(&mut self, graph: &Graph, now: f64) {
        let Some(before) = self.layout_undo.take() else {
            return;
        };
        let moves = before
            .into_iter()
            .filter_map(|(id, to)| Some((id, graph.node(id)?.pos, to)))
            .collect();
        self.glide = Some(Glide::new(moves, now));
        self.notices.push("Layout undone".to_owned());
    }

    /// Advances node glides and view animations; true while one is running.
    fn animate(&mut self, graph: &mut Graph, now: f64) -> bool {
        let mut busy = false;
        if let Some(g) = &self.glide {
            for &(id, from, to, delay) in &g.moves {
                if let Some(n) = graph.node_mut(id) {
                    n.pos = from + (to - from) * spring((now - g.start - delay) / ANIM_SECS);
                }
            }
            let done = now >= g.end();
            busy |= !done;
            if done {
                self.glide = None;
            }
        }
        if let Some(a) = &self.view_anim {
            let t = (now - a.start) / ANIM_SECS;
            let e = spring(t);
            // Zoom moves geometrically and the world point at the centre of
            // the view moves in a straight line, so the patch neither
            // swims sideways nor rushes through the zoom.
            let ((p0, z0), (p1, z1)) = (a.from, a.to);
            let centre = self.rect.size() * 0.5;
            let (c0, c1) = ((centre - p0) / z0, (centre - p1) / z1);
            self.zoom = z0 * (z1 / z0).powf(e);
            self.pan = centre - (c0 + (c1 - c0) * e) * self.zoom;
            busy |= t < 1.0;
            if t >= 1.0 {
                (self.pan, self.zoom) = (p1, z1);
                self.view_anim = None;
            }
        }
        busy
    }

    pub fn show(&mut self, ui: &mut Ui, graph: &mut Graph, audio: &AudioHandle) {
        let rect = ui.max_rect();
        self.rect = rect;
        if self.needs_framing {
            self.needs_framing = false;
            self.frame_all(graph);
        }
        let now = ui.input(|i| i.time);
        self.now = now;
        if self.animate(graph, now) {
            ui.ctx().request_repaint();
        }

        let painter = ui.painter_at(rect);
        let bg = ui.interact(rect, Id::unique("canvas_bg"), Sense::CLICK | Sense::DRAG);
        let (pointer, mods) = ui.input(|i| (i.pointer.hover_pos(), i.modifiers));
        let additive = mods.shift || mods.command;
        // Clicks that only pick: not one that closes a menu (it was open
        // when the pointer went down), and not a macOS Ctrl+click, which is
        // a right-click.
        let menu_open = egui::Popup::is_any_open(ui.ctx());
        if ui.input(|i| i.pointer.any_pressed()) {
            self.menu_at_press = menu_open;
        }
        let ctrl_click = cfg!(target_os = "macos") && mods.ctrl;
        let picks = !ctrl_click && !self.menu_at_press;
        let mut actions = Vec::new();

        // --- Navigation: scroll, middle/right drag or Alt+drag pan ----------
        let panning = bg.dragged_by(PointerButton::Middle)
            || bg.dragged_by(PointerButton::Secondary)
            || (bg.dragged_by(PointerButton::Primary) && mods.alt && self.marquee.is_none());
        if panning {
            self.take_over_view();
            self.pan += bg.drag_delta();
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
        if ui.rect_contains_pointer(rect) {
            let (zoom_delta, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
            if zoom_delta != 1.0 || scroll != Vec2::ZERO {
                self.take_over_view();
            }
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

        // The board's background, then its stickers: under everything
        // else, and under the nodes for the pointer too.
        let board_view = super::board::View {
            origin: rect.min + self.pan,
            zoom: z,
        };
        self.board_ui
            .draw_background(ui, &painter, rect, board_view, &graph.board);
        let stickers = self.board_ui.show(
            ui,
            &painter,
            rect,
            board_view,
            &mut graph.board,
            &bg,
            &mut self.selected,
            &mut self.notices,
        );
        self.pan += stickers.pan;

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
                Some(Card {
                    gid: g.id,
                    rect: self.to_screen_rect(r),
                    ins,
                    outs,
                })
            })
            .collect();
        let node_rects: Vec<Option<Rect>> = graph
            .nodes
            .iter()
            .map(|n| {
                (!hidden.contains(&n.id))
                    .then(|| self.to_screen_rect(Rect::from_min_size(n.pos, n.size())))
            })
            .collect();

        // Screen position of every visible port. A collapsed module shows
        // its members' outer ports on its card; inner ports are hidden.
        let mut in_pos: HashMap<PortRef, Pos2> = HashMap::new();
        let mut out_pos: HashMap<PortRef, Pos2> = HashMap::new();
        // Inputs on a node's top edge: their cables drop in from above.
        let mut on_top: HashSet<PortRef> = HashSet::new();
        for n in graph.nodes.iter().filter(|n| !hidden.contains(&n.id)) {
            for port in n.input_ports() {
                in_pos.insert(
                    PortRef { node: n.id, port },
                    self.to_screen(n.input_pos(port)),
                );
                if n.inputs_on_top() {
                    on_top.insert(PortRef { node: n.id, port });
                }
            }
            for port in n.output_ports() {
                out_pos.insert(
                    PortRef { node: n.id, port },
                    self.to_screen(n.output_pos(port)),
                );
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
            top: bool,
            color: Color32,
            level: f32,
            midi: bool,
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
                    top: on_top.contains(&c.to),
                    color: src.accent,
                    level: src.signal_level(),
                    midi: is_midi(c.to.port),
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
                    .map(|w| (w.cable, widgets::cable_distance(w.a, w.b, w.top, p, z)))
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
                // MIDI only into MIDI, audio only into audio.
                let (from, to) = if d.from_output {
                    (d.anchor.node, pr.node)
                } else {
                    (pr.node, d.anchor.node)
                };
                let valid =
                    is_midi(pr.port) == is_midi(d.anchor.port) && !graph.would_cycle(from, to);
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
        // Not on the selection bar (as last drawn), which is for left clicks.
        let on_bar = pointer.is_some_and(|p| self.bar.is_some_and(|b| b.contains(p)));
        let open_menu = ui.rect_contains_pointer(rect)
            && !on_bar
            && !stickers.hovered
            && ui.input(|i| {
                i.pointer.secondary_clicked()
                    || (cfg!(target_os = "macos")
                        && i.modifiers.ctrl
                        && i.pointer.primary_clicked())
            });
        if open_menu && let Some(p) = pointer {
            let card = cards
                .iter()
                .rev()
                .find(|c| c.rect.contains(p))
                .map(|c| c.gid);
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
            // What is right-clicked joins the menu's subject: the selection
            // stays if it holds it, else it is replaced, or (Shift or Cmd
            // held) grows to take it in.
            self.menu = if let Some(gid) = card.or(if node.is_none() { frame } else { None }) {
                if let Some(g) = graph.group(gid)
                    && !g.members.iter().all(|m| self.selected.contains(m))
                {
                    actions.push(Action::Select(g.members.clone(), additive));
                }
                MenuTarget::Group(gid)
            } else if let Some(id) = node {
                if !self.selected.contains(&id) {
                    actions.push(Action::Select(vec![id], additive));
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
        } else if bg.clicked() && !additive && picks {
            actions.push(Action::Deselect);
        }

        // A right or middle drag that starts on a node or a module pans the
        // view, as on empty canvas (and never selects).
        let mut item_pan = Vec2::ZERO;

        // --- Expanded modules: frames behind everything else ---------------
        for &(gid, frame) in &frames {
            if !frame.intersects(rect) {
                continue;
            }
            let g = graph.group(gid).expect("frame of an existing module");
            let selected = g.members.iter().all(|m| self.selected.contains(m));
            let head = Rect::from_min_size(frame.min, vec2(frame.width(), nodes::HEADER_H * z));
            let resp = ui.interact(head, Id::unique(("group", gid)), Sense::CLICK | Sense::DRAG);
            item_pan += module_pointer(ui, &resp, g, selected, (additive, picks), z, &mut actions);
            let renaming = self.renaming.as_ref().is_some_and(|r| r.gid == gid);
            let h = groups::draw_frame(ui, &painter, g, frame, selected, renaming, z);
            self.title_editor(ui, gid, h.title_rect, &mut actions);
            push_header(h, gid, &mut actions);
        }

        // --- Cables ------------------------------------------------------
        for w in &wires {
            widgets::draw_cable(
                &painter,
                w.a,
                w.b,
                w.top,
                w.color,
                w.level,
                hovered_cable == Some(w.cable),
                w.midi,
                z,
            );
        }

        // --- Nodes -------------------------------------------------------
        let ctx = PortCtx {
            inputs: &inputs,
            outputs: &outputs,
            drop_target,
            dragging: self.drag,
        };
        let hits = RefCell::new(Vec::<KnobHit>::new());
        let target_labels: HashMap<NodeId, String> = graph
            .nodes
            .iter()
            .filter_map(|n| Some((n.id, graph.target_label(graph.target_of(n.id)?)?)))
            .collect();
        let mut map_button = None;
        let tempo = graph.tempo;
        for (idx, node) in graph.nodes.iter_mut().enumerate() {
            // Iris runs its scene changes and its window even when not drawn.
            if let NodeKind::Module(m) = &node.kind
                && m.params.kind == crate::audio::modules::ModuleKind::Iris
            {
                super::iris::tick(
                    ui.ctx(),
                    node.id,
                    &node.title,
                    node.accent,
                    &m.params,
                    tempo,
                );
            }
            let Some(rect_s) = node_rects[idx] else {
                continue;
            };
            if !rect_s.expand(40.0).intersects(rect) {
                continue;
            }
            let id = node.id;
            let selected = self.selected.contains(&id);
            let body = ui.interact(rect_s, Id::unique(("node", id)), Sense::CLICK | Sense::DRAG);
            if body.clicked() && picks {
                actions.push(if additive {
                    Action::ToggleSelect(id)
                } else {
                    Action::Select(vec![id], false)
                });
            } else if body.drag_started_by(PointerButton::Primary) && !selected {
                // Only a left drag moves (a right drag pans the view).
                actions.push(Action::Select(vec![id], additive));
            }
            if body.dragged_by(PointerButton::Primary) {
                actions.push(Action::MoveSelection(body.drag_delta() / z));
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
            if body.dragged_by(PointerButton::Secondary) || body.dragged_by(PointerButton::Middle) {
                item_pan += body.drag_delta();
            }
            let collapsed = node.collapsed;

            let view = NodeView {
                ui,
                painter: &painter,
                min: rect_s.min,
                z,
                node: id,
                hits: &hits,
                tempo,
            };
            let header = nodes::frame(&view, node, selected);
            if header.close {
                actions.push(Action::Remove(id));
            }
            if header.bypass {
                actions.push(Action::ToggleBypass(vec![id]));
            }
            let on_header = body
                .interact_pointer_pos()
                .is_some_and(|p| p.y < rect_s.top() + nodes::HEADER_H * z);
            if header.toggle_collapse
                || (body.double_clicked() && on_header && !nodes::frameless(node))
            {
                actions.push(Action::ToggleCollapse(id));
            }

            let in_ports = node.input_ports();
            let out_ports = node.output_ports();
            let sources: Vec<Option<(Color32, String)>> = (0..node.inputs().len())
                .map(|i| {
                    inputs
                        .get(&PortRef { node: id, port: i })
                        .map(|(_, c, n)| (*c, n.clone()))
                })
                .collect();
            let accent = node.accent;
            let connected: Vec<bool> = sources.iter().map(Option::is_some).collect();
            let out = outputs.contains(&PortRef { node: id, port: 0 });
            let midi_in = inputs.contains_key(&PortRef {
                node: id,
                port: MIDI_PORT,
            });
            let midi_out = outputs.contains(&PortRef {
                node: id,
                port: MIDI_PORT,
            });
            if collapsed {
                let ins: Vec<bool> = in_ports
                    .iter()
                    .map(|&port| inputs.contains_key(&PortRef { node: id, port }))
                    .collect();
                let outs: Vec<bool> = out_ports
                    .iter()
                    .map(|&port| outputs.contains(&PortRef { node: id, port }))
                    .collect();
                nodes::collapsed(&view, node, &ins, &outs);
            } else {
                match &mut node.kind {
                    NodeKind::Osc(o) => nodes::osc(&view, id, accent, o, &connected, out),
                    NodeKind::Mixer(m) => {
                        let outs = (out, outputs.contains(&PortRef { node: id, port: 1 }));
                        if m.console.is_some() {
                            if let Some(shape) =
                                super::console::draw(&view, id, accent, m, &sources, outs)
                            {
                                actions.push(Action::SetConsole(id, shape));
                            }
                        } else {
                            nodes::mixer(&view, id, accent, m, &sources, outs);
                        }
                    }
                    NodeKind::Plugin(p) => {
                        let input = connected.first().copied().unwrap_or(false);
                        let events =
                            plugin_node::draw(&view, id, accent, p, (input, midi_in), out, now);
                        if let Some(e) = p.editor_error.take() {
                            self.notices.push(format!("Could not open the editor: {e}"));
                        }
                        if events.toggle_editor {
                            #[cfg(target_os = "macos")]
                            let before = super::macos::windows();
                            match p.toggle_editor() {
                                // The editor stays in front of the board.
                                #[cfg(target_os = "macos")]
                                Ok(()) => super::macos::float_new_windows(&before),
                                #[cfg(not(target_os = "macos"))]
                                Ok(()) => {}
                                Err(e) => {
                                    self.notices.push(format!("Could not open the editor: {e}"))
                                }
                            }
                        }
                    }
                    NodeKind::Module(m) => {
                        let outs: Vec<bool> = (0..m.params.kind.def().outputs.len())
                            .map(|port| outputs.contains(&PortRef { node: id, port }))
                            .collect();
                        let midi = (midi_in, midi_out);
                        let picking = self.mapping == Some(id);
                        let map = modules::MapState {
                            target: target_labels.get(&id).map(String::as_str),
                            picking,
                        };
                        let button =
                            modules::draw(&view, id, accent, m, (&connected, &outs), midi, map);
                        if button.clicked {
                            actions.push(if picking {
                                Action::CancelMapping
                            } else if m.target.is_some() {
                                Action::SetTarget(id, None)
                            } else {
                                Action::StartMapping(id)
                            });
                        }
                        if picking {
                            map_button = button.rect;
                        }
                    }
                }
            }

            if node.bypassed() {
                nodes::bypass_veil(&view, node);
            }

            // Ports last so they sit on top of everything else in the node.
            for port in in_ports {
                let pr = PortRef { node: id, port };
                self.port(
                    ui,
                    &painter,
                    pr,
                    true,
                    in_pos[&pr],
                    accent,
                    &ctx,
                    &mut actions,
                );
            }
            for port in out_ports {
                let pr = PortRef { node: id, port };
                self.port(
                    ui,
                    &painter,
                    pr,
                    false,
                    out_pos[&pr],
                    accent,
                    &ctx,
                    &mut actions,
                );
            }
        }

        // --- Collapsed modules -------------------------------------------
        for card in &cards {
            if !card.rect.expand(40.0).intersects(rect) {
                continue;
            }
            let g = graph.group(card.gid).expect("card of an existing module");
            let selected = g.members.iter().all(|m| self.selected.contains(m));
            let resp = ui.interact(
                card.rect,
                Id::unique(("card", card.gid)),
                Sense::CLICK | Sense::DRAG,
            );
            item_pan += module_pointer(ui, &resp, g, selected, (additive, picks), z, &mut actions);
            let gid = g.id;
            let connected = |pr: PortRef, input: bool| {
                if input {
                    inputs.contains_key(&pr)
                } else {
                    outputs.contains(&pr)
                }
            };
            let ports = (card.ins.as_slice(), card.outs.as_slice());
            let renaming = self.renaming.as_ref().is_some_and(|r| r.gid == gid);
            let h = groups::draw_card(
                ui, &painter, graph, g, card.rect, ports, connected, selected, renaming, z,
            );
            self.title_editor(ui, gid, h.title_rect, &mut actions);
            push_header(h, gid, &mut actions);

            let accent_of = |pr: &PortRef| graph.node(pr.node).map_or(theme::accent(), |n| n.accent);
            for pr in &card.ins {
                self.port(
                    ui,
                    &painter,
                    *pr,
                    true,
                    in_pos[pr],
                    accent_of(pr),
                    &ctx,
                    &mut actions,
                );
            }
            for pr in &card.outs {
                self.port(
                    ui,
                    &painter,
                    *pr,
                    false,
                    out_pos[pr],
                    accent_of(pr),
                    &ctx,
                    &mut actions,
                );
            }
        }

        self.midi_mapping(
            ui,
            &painter,
            graph,
            &node_rects,
            hits.into_inner(),
            map_button,
            pointer,
            &mut actions,
        );
        if item_pan != Vec2::ZERO {
            self.take_over_view();
            self.pan += item_pan;
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
        // After the nodes and cards, so its buttons win the clicks.
        self.selection_bar(ui, &painter, graph, rect, &mut actions);

        // --- Cable being dragged -----------------------------------------
        if let Some(d) = self.drag {
            let anchor = if d.from_output {
                out_pos.get(&d.anchor)
            } else {
                in_pos.get(&d.anchor)
            };
            if let (Some(&anchor), Some(src), Some(p)) =
                (anchor, graph.node(d.anchor.node), pointer)
            {
                let end = drop_target.map_or(p, |(_, pos)| pos);
                let (a, b) = if d.from_output {
                    (anchor, end)
                } else {
                    (end, anchor)
                };
                let input = if d.from_output {
                    drop_target.map(|(pr, _)| pr)
                } else {
                    Some(d.anchor)
                };
                let top = input.is_some_and(|pr| on_top.contains(&pr));
                let color = if drop_target.is_some() {
                    src.accent
                } else {
                    alpha(src.accent, 0.7)
                };
                widgets::draw_cable(
                    &painter,
                    a,
                    b,
                    top,
                    color,
                    0.0,
                    drop_target.is_some(),
                    is_midi(d.anchor.port),
                    z,
                );
                if drop_target.is_none() {
                    painter.circle_filled(p, 3.0 * z, color);
                }
            }
            if ui.input(|i| i.pointer.any_released()) {
                if let Some((target, _)) = drop_target {
                    let (from, to) = if d.from_output {
                        (d.anchor, target)
                    } else {
                        (target, d.anchor)
                    };
                    actions.push(Action::Connect { from, to });
                }
                self.drag = None;
            }
        }

        // --- Dropping a palette entry ------------------------------------
        if let Some(spawn) = egui::DragAndDrop::payload::<Spawn>(ui.ctx())
            && let Some(p) = pointer
            && rect.contains(p)
        {
            // Lands with its header under the pointer.
            let size = match *spawn {
                Spawn::Node(ty) => ty.size(),
                Spawn::Preset(_) => groups::card_size(2, 1),
                Spawn::Plugin(_) => plugin_node::SIZE,
            };
            let at = self.to_world(p) - vec2(size.x * 0.5, nodes::HEADER_H * 0.5);
            let preview = Rect::from_min_size(self.to_screen(at), size * z);
            painter.rect(
                preview,
                6.0 * z,
                alpha(theme::accent(), 0.06),
                Stroke::new(1.0, alpha(theme::accent(), 0.7)),
                StrokeKind::Inside,
            );
            if ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                actions.push(match *spawn {
                    Spawn::Node(ty) => Action::Add(ty, at),
                    Spawn::Preset(i) => Action::AddPreset(i, at),
                    Spawn::Plugin(i) => Action::AddPlugin(i, at),
                });
            }
        }

        // --- Rubber-band selection ---------------------------------------
        if let Some(start) = self.marquee {
            let end = pointer.unwrap_or(start);
            let band = Rect::from_two_pos(start, end);
            painter.rect(
                band,
                2.0,
                alpha(theme::accent(), 0.08),
                Stroke::new(1.0, alpha(theme::accent(), 0.7)),
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
                theme::text()_FAINT,
            );
        }

        // --- Type label --------------------------------------------------
        // The topmost node or module under the pointer (a module's frame
        // only by its header), as for the context menu.
        let hovered = pointer
            .filter(|_| ui.rect_contains_pointer(rect) && !on_bar)
            .and_then(|p| {
                if let Some(c) = cards.iter().rev().find(|c| c.rect.contains(p)) {
                    return Some((TipTarget::Group(c.gid), c.rect));
                }
                let node = graph
                    .nodes
                    .iter()
                    .zip(&node_rects)
                    .rev()
                    .find_map(|(n, r)| {
                        r.filter(|r| r.contains(p))
                            .map(|r| (TipTarget::Node(n.id), r))
                    });
                node.or_else(|| {
                    frames
                        .iter()
                        .rev()
                        .find(|(_, f)| p.y < f.top() + nodes::HEADER_H * z && f.contains(p))
                        .map(|&(gid, f)| (TipTarget::Group(gid), f))
                })
            });
        self.type_tip(ui, graph, rect, hovered, now);

        // --- Context menu -------------------------------------------------
        // Each menu says first what it acts on, so it is clear what Make
        // module (and the rest) will take.
        let menu = self.menu;
        let has_selection = !self.selected.is_empty();
        let subject = menu_subject(graph, &self.selected, menu);
        egui::Popup::menu(&bg)
            .open_memory(open_menu.then_some(egui::SetOpenCommand::Bool(true)))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .at_pointer_fixed()
            .show(|ui| match menu {
                MenuTarget::Canvas(pos) => {
                    canvas_menu(
                        ui,
                        pos,
                        subject.as_deref().filter(|_| has_selection),
                        &self.library,
                        &self.plugins,
                        &mut actions,
                    );
                    self.board_ui.canvas_menu(ui, pos, &graph.board);
                }
                MenuTarget::Cable(cable) => {
                    if menu_item(ui, "Disconnect", "Double-click") {
                        actions.push(Action::Disconnect(cable));
                        ui.close();
                    }
                }
                MenuTarget::Node(id) => match graph.node(id) {
                    Some(n) => {
                        subject_heading(ui, subject.as_deref().unwrap_or(&n.title));
                        node_menu(
                            ui,
                            n.collapsed,
                            n.is_effect(),
                            graph.group_of(id).is_some(),
                            &mut actions,
                        )
                    }
                    None => ui.close(),
                },
                MenuTarget::Group(gid) => match graph.group(gid) {
                    Some(g) => {
                        subject_heading(ui, subject.as_deref().unwrap_or(&g.title));
                        // More than this module selected: Make module takes it all.
                        let more = self.selected.iter().any(|id| !g.members.contains(id));
                        group_menu(ui, gid, g.collapsed, more, &mut actions)
                    }
                    None => ui.close(),
                },
            });

        // Auto-fit: follow the patch as it changes (not mid-gesture).
        if self.auto_fit
            && self.glide.is_none()
            && !ui.input(|i| i.pointer.any_down())
            && let Some(bounds) = content_bounds(graph)
        {
            let (pan, zoom) = self.fit_params(bounds);
            let (to_pan, to_zoom) = self.view_target();
            if (to_zoom - zoom).abs() > 1e-3 || (to_pan - pan).length() > 1.0 {
                self.animate_view(pan, zoom, now);
                ui.ctx().request_repaint();
            }
        }

        // --- Keyboard (not while typing, e.g. a module name) --------------
        if ui.memory(|m| m.focused().is_none()) {
            self.keyboard(ui, graph, menu_open, &mut actions);
        }

        self.apply(actions, graph, audio);
    }

    /// While the pointer rests on a node or a module: a small translucent
    /// label just above it with its type (titles can be renamed, and a
    /// module's name says nothing of what is inside). It stays put rather
    /// than following the pointer, takes no clicks, and keeps out of the
    /// way of drags, menus and renaming.
    fn type_tip(
        &mut self,
        ui: &Ui,
        graph: &Graph,
        canvas: Rect,
        hovered: Option<(TipTarget, Rect)>,
        now: f64,
    ) {
        let busy = self.drag.is_some()
            || self.marquee.is_some()
            || self.mapping.is_some()
            || self.renaming.is_some()
            || egui::DragAndDrop::has_any_payload(ui.ctx())
            || egui::Popup::is_any_open(ui.ctx())
            || ui.input(|i| i.pointer.any_down());
        let Some((target, item)) = hovered.filter(|_| !busy) else {
            self.tip = None;
            return;
        };
        let since = match self.tip {
            Some((t, since)) if t == target => since,
            _ => now,
        };
        self.tip = Some((target, since));
        let shown = now - since - TIP_DELAY;
        if shown < 0.0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(-shown));
            return;
        }
        let opacity = (shown / TIP_FADE).min(1.0) as f32;
        if opacity < 1.0 {
            ui.ctx().request_repaint();
        }
        let Some((kind, detail)) = type_label(graph, target) else {
            return;
        };

        let mut job = egui::text::LayoutJob::default();
        let format = |size: f32, color: Color32| {
            egui::TextFormat::simple(FontId::proportional(size), alpha(color, opacity))
        };
        job.append(&kind, 0.0, format(11.0, theme::text()));
        if !detail.is_empty() {
            job.append(&detail, 8.0, format(10.5, theme::text()_DIM));
        }
        let galley = ui.painter().layout_job(job);
        let pad = vec2(8.0, 4.0);
        let size = galley.size() + pad * 2.0;
        // Just above the item, left-aligned; below it if there's no room.
        let mut min = pos2(item.left(), item.top() - 6.0 - size.y);
        if min.y < canvas.top() + 4.0 {
            min.y = item.bottom() + 6.0;
        }
        min.x = min.x.clamp(
            canvas.left() + 4.0,
            (canvas.right() - size.x - 4.0).max(canvas.left() + 4.0),
        );
        let pill = Rect::from_min_size(min, size);
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                Id::unique("type_tip"),
            ))
            .with_clip_rect(canvas);
        painter.rect(
            pill,
            5.0,
            alpha(theme::node_header(), 0.72 * opacity),
            Stroke::new(1.0, alpha(theme::node_border(), 0.8 * opacity)),
            StrokeKind::Inside,
        );
        painter.galley(pill.min + pad, galley, theme::text());
    }

    /// MIDI mappings: a dashed line from each MIDI node to the control it
    /// drives and, while picking one, highlighted controls and a hint.
    #[allow(clippy::too_many_arguments)]
    fn midi_mapping(
        &mut self,
        ui: &Ui,
        painter: &Painter,
        graph: &Graph,
        node_rects: &[Option<Rect>],
        hits: Vec<KnobHit>,
        map_button: Option<Rect>,
        pointer: Option<Pos2>,
        actions: &mut Vec<Action>,
    ) {
        let z = self.zoom;
        for (node, rect) in graph.nodes.iter().zip(node_rects) {
            let (NodeKind::Module(m), Some(rect)) = (&node.kind, rect) else {
                continue;
            };
            let Some(t) = m.target else {
                continue;
            };
            let Some(hit) = hits.iter().find(|h| h.node == t.node && h.param == t.param) else {
                continue;
            };
            let from = pos2(rect.right(), rect.top() + nodes::HEADER_H * 0.5 * z);
            let to = hit.rect.center();
            let stroke = Stroke::new(1.2 * z.max(0.6), alpha(node.accent, 0.65));
            painter.extend(Shape::dashed_line(&[from, to], stroke, 6.0 * z, 4.0 * z));
            painter.circle_stroke(to, hit.rect.width().min(hit.rect.height()) * 0.5, stroke);
        }

        let Some(source) = self.mapping else {
            return;
        };
        if graph.node(source).is_none() {
            self.mapping = None;
            return;
        }
        let targets = || hits.iter().filter(|h| h.node != source);
        for h in targets() {
            painter.rect_stroke(
                h.rect.expand(2.0 * z),
                4.0 * z,
                Stroke::new(1.2, alpha(theme::accent(), 0.8)),
                StrokeKind::Outside,
            );
        }
        let area = painter.clip_rect();
        let text = "Click a knob or fader to drive it with MIDI  ·  Esc to cancel";
        let galley =
            painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.0), theme::text());
        let banner = Rect::from_center_size(
            pos2(area.center().x, area.top() + 24.0),
            galley.size() + vec2(24.0, 12.0),
        );
        painter.rect(
            banner,
            6.0,
            theme::node_header(),
            Stroke::new(1.0, theme::accent()),
            StrokeKind::Inside,
        );
        painter.galley(banner.center() - galley.size() * 0.5, galley, theme::text());

        let Some(p) = pointer else {
            return;
        };
        let under = targets().find(|h| h.rect.contains(p)).copied();
        if under.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if ui.input(|i| i.pointer.primary_pressed()) {
            match under {
                Some(h) => {
                    actions.push(Action::SetTarget(
                        source,
                        Some(Target {
                            node: h.node,
                            param: h.param,
                        }),
                    ));
                    actions.push(Action::CancelMapping);
                }
                // The MAP button itself toggles picking off on its own.
                None if map_button.is_some_and(|r| r.contains(p)) => {}
                None => actions.push(Action::CancelMapping),
            }
        }
    }

    /// `menu_open`: a menu was open as the frame began (then Esc only
    /// closes it).
    fn keyboard(&mut self, ui: &Ui, graph: &Graph, menu_open: bool, actions: &mut Vec<Action>) {
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
        // Esc that closes a menu does only that.
        if pressed(Key::Escape) && !menu_open {
            actions.push(if self.mapping.is_some() {
                Action::CancelMapping
            } else if self.focus.is_some() {
                Action::Unfocus
            } else {
                Action::Deselect
            });
        }
        if !cmd && !shift && pressed(Key::Z) {
            // The node under the pointer, else the selection; Z again (on
            // the same node, or off any) goes back.
            let hovered = ui
                .input(|i| i.pointer.hover_pos())
                .and_then(|q| self.node_under(graph, q));
            let focused = self.focus.as_ref().map(|f| f.0.clone());
            match (hovered, focused) {
                (Some(id), Some(f)) if f != [id] => actions.push(Action::Focus(vec![id])),
                (_, Some(_)) => actions.push(Action::Unfocus),
                (Some(id), None) => actions.push(Action::Focus(vec![id])),
                (None, None) if has_selection => actions.push(Action::FocusSelection),
                (None, None) => {}
            }
        }
        if (pressed(Key::Enter) || pressed(Key::F2))
            && let Some(g) = self.selected_group(graph)
        {
            actions.push(Action::StartRename(g.id));
        }
        if cmd && pressed(Key::G) {
            actions.push(if shift {
                Action::UngroupSelection
            } else {
                Action::GroupSelection
            });
        }
        if !cmd && pressed(Key::C) {
            if shift {
                // Collapse everything, or expand if all is already collapsed.
                let any_open = graph.nodes.iter().any(|n| !n.collapsed)
                    || graph.groups.iter().any(|g| !g.collapsed);
                actions.push(Action::CollapseAll(any_open));
            } else if has_selection {
                actions.push(Action::CollapseSelection);
            }
        }
        if !cmd && !shift && pressed(Key::B) {
            // The effect under the pointer, else the selected ones.
            let hovered = ui
                .input(|i| i.pointer.hover_pos())
                .and_then(|q| self.node_under(graph, q));
            match hovered {
                Some(id) => actions.push(Action::ToggleBypass(vec![id])),
                None if has_selection => actions.push(Action::BypassSelection),
                None => {}
            }
        }
        if !cmd && pressed(Key::F) {
            actions.push(if shift {
                Action::FitSelection
            } else {
                Action::FitAll
            });
        }
        if !cmd && pressed(Key::L) {
            actions.push(Action::AutoLayout);
        }
        if !cmd && pressed(Key::A) {
            actions.push(Action::ToggleAutoFit);
        }
        if cmd && (pressed(Key::Equals) || pressed(Key::Plus)) {
            actions.push(Action::ZoomBy(1.25));
        }
        if cmd && pressed(Key::Minus) {
            actions.push(Action::ZoomBy(0.8));
        }
        if cmd && pressed(Key::Num0) {
            actions.push(Action::ZoomActual);
        }
        if cmd && pressed(Key::Z) {
            actions.push(Action::UndoLayout);
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
            .interact(
                Rect::from_center_size(c, Vec2::splat(20.0 * z)),
                id,
                Sense::CLICK | Sense::DRAG,
            )
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
                        CableDrag {
                            anchor: cable.from,
                            from_output: true,
                        }
                    }
                    None => CableDrag {
                        anchor: pr,
                        from_output: false,
                    },
                });
            }
            let color = existing.map_or(accent, |(_, c, _)| *c);
            widgets::port(
                painter,
                c,
                existing.is_some(),
                hot,
                color,
                is_midi(pr.port),
                z,
            );
        } else {
            if resp.drag_started() {
                self.drag = Some(CableDrag {
                    anchor: pr,
                    from_output: true,
                });
            }
            widgets::port(
                painter,
                c,
                ctx.outputs.contains(&pr),
                hot,
                accent,
                is_midi(pr.port),
                z,
            );
        }
    }

    /// While renaming a module: a text field over its title. Enter or a
    /// click elsewhere keeps the new name, Esc cancels.
    fn title_editor(&mut self, ui: &mut Ui, gid: GroupId, rect: Rect, actions: &mut Vec<Action>) {
        let Some(r) = self.renaming.as_mut().filter(|r| r.gid == gid) else {
            return;
        };
        let id = Id::unique(("rename", gid));
        let edit = egui::TextEdit::singleline(&mut r.text)
            .id(id)
            .font(theme::strong(11.5 * self.zoom))
            .text_color(theme::text())
            .margin(egui::Margin::symmetric(3, 1))
            .desired_width(rect.width());
        let resp = ui.put(rect, edit);
        if r.select && resp.has_focus() {
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            let all = egui::text_selection::CCursorRange::select_all_str(&r.text);
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
            r.select = false;
        }
        if r.fresh {
            resp.request_focus();
            r.fresh = false;
        } else if resp.lost_focus() {
            let name = r.text.trim().to_owned();
            if !ui.input(|i| i.key_pressed(Key::Escape)) && !name.is_empty() {
                actions.push(Action::RenameGroup(gid, name));
            }
            self.renaming = None;
        }
    }

    fn apply(&mut self, actions: Vec<Action>, graph: &mut Graph, audio: &AudioHandle) {
        for action in actions {
            match action {
                Action::Add(ty, p) => {
                    let id = graph.add(ty, p, audio);
                    self.select_only(id);
                }
                Action::AddPreset(i, p) => {
                    if let Some(preset) = self.library.get(i) {
                        self.selected = project::insert_preset(graph, preset, p, audio)
                            .into_iter()
                            .collect();
                    }
                }
                Action::AddPlugin(i, p) => {
                    let Some(entry) = self.plugins.get(i) else {
                        continue;
                    };
                    match graph.add_plugin(&entry.path, p, audio) {
                        Ok(id) => self.select_only(id),
                        Err(e) => self
                            .notices
                            .push(format!("Could not load {}: {e}", entry.name)),
                    }
                }
                Action::Request(r) => self.requests.push(r),
                Action::Remove(id) => {
                    if confirm_removal(graph, &[id]) {
                        graph.remove_node(id, audio);
                        self.selected.remove(&id);
                    }
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
                Action::AutoLayout => self.auto_layout(graph, self.now),
                Action::UndoLayout => self.undo_layout(graph, self.now),
                Action::FitAll => {
                    self.focus = None;
                    self.fit_all(graph, self.now);
                }
                Action::FitSelection => {
                    self.focus = None;
                    self.fit_selection(graph, self.now);
                }
                Action::Focus(ids) => self.focus_on(graph, ids, self.now),
                Action::FocusSelection => {
                    let ids = self.ordered_selection(graph);
                    self.focus_on(graph, ids, self.now);
                }
                Action::Unfocus => self.unfocus(self.now),
                Action::ZoomBy(f) => self.zoom_by(f, self.now),
                Action::ZoomActual => self.zoom_actual(self.now),
                Action::ToggleAutoFit => self.set_auto_fit(!self.auto_fit, graph, self.now),
                Action::RemoveSelection => {
                    let ids = self.ordered_selection(graph);
                    if confirm_removal(graph, &ids) {
                        for id in ids {
                            graph.remove_node(id, audio);
                        }
                        self.selected.clear();
                    }
                }
                Action::DuplicateSelection => {
                    let ids = self.ordered_selection(graph);
                    self.selected = graph.duplicate_nodes(&ids, audio).into_iter().collect();
                }
                Action::ToggleCollapse(id) => graph.toggle_collapsed(id),
                Action::ToggleBypass(ids) => toggle_bypass(graph, &ids),
                Action::BypassSelection => {
                    let ids = self.ordered_selection(graph);
                    toggle_bypass(graph, &ids);
                }
                Action::SetConsole(id, shape) => {
                    let lost = graph.console_losses(id, shape);
                    if lost == 0 || confirm_unplug(lost) {
                        graph.set_console(id, shape, audio);
                    }
                }
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
                Action::StartRename(gid) => {
                    if let Some(g) = graph.group(gid) {
                        self.renaming = Some(Rename {
                            gid,
                            text: g.title.clone(),
                            fresh: true,
                            select: true,
                        });
                    }
                }
                Action::RenameGroup(gid, title) => graph.rename_group(gid, &title),
                Action::StartMapping(id) => self.mapping = Some(id),
                Action::CancelMapping => self.mapping = None,
                Action::SetTarget(id, target) => graph.set_target(id, target),
            }
        }
    }

    /// C: when the selection is made of whole modules, fold those modules;
    /// otherwise fold the selected nodes themselves.
    /// While two or more nodes are selected, a bar with what to do with
    /// them: Make module, Collapse (or Expand), Duplicate, Delete. No menu
    /// is needed, so no stray click can lose the selection on the way. At a
    /// fixed size whatever the zoom, centred above the selection (below it
    /// when there is no room), inside the canvas; hidden while nodes are
    /// dragged, a rectangle drawn, a module renamed or a control mapped.
    fn selection_bar(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        graph: &Graph,
        canvas: Rect,
        actions: &mut Vec<Action>,
    ) {
        self.bar = None;
        self.bar_buttons.clear();
        let count = self
            .selected
            .iter()
            .filter(|id| graph.node(**id).is_some())
            .count();
        let busy = self.marquee.is_some()
            || self.drag.is_some()
            || self.renaming.is_some()
            || self.mapping.is_some()
            || egui::DragAndDrop::has_any_payload(ui.ctx())
            || actions
                .iter()
                .any(|a| matches!(a, Action::MoveSelection(_)));
        if count < 2 || busy {
            return;
        }
        // The selection, and the frames of open modules it reaches into, so
        // the bar never covers a module's header.
        let frames = graph
            .groups
            .iter()
            .filter(|g| !g.collapsed && g.members.iter().any(|m| self.selected.contains(m)))
            .filter_map(|g| groups::frame_rect(graph, g));
        let Some(bounds) = selection_bounds(graph, &self.selected)
            .into_iter()
            .chain(frames)
            .reduce(|a, b| a.union(b))
        else {
            return;
        };
        let bounds = self.to_screen_rect(bounds);
        if !bounds.intersects(canvas) {
            return;
        }

        // Make module only when it would make something new.
        let one_module = self.selected_group(graph).is_some();
        let open = self.selected.iter().any(|id| {
            graph.node(*id).is_some_and(|n| !n.collapsed)
                && !graph.group_of(*id).is_some_and(|g| g.collapsed)
        });
        let items: Vec<BarItem> = [
            BarItem::Module,
            BarItem::Collapse,
            BarItem::Duplicate,
            BarItem::Delete,
        ]
        .into_iter()
        .filter(|i| *i != BarItem::Module || !one_module)
        .collect();
        let label = |i: BarItem| match i {
            BarItem::Module => "Make module",
            BarItem::Collapse if open => "Collapse",
            BarItem::Collapse => "Expand",
            BarItem::Duplicate => "Duplicate",
            BarItem::Delete => "Delete",
        };

        // Layout: the count, a rule, then the buttons.
        const H: f32 = 34.0;
        const BUTTON_H: f32 = 26.0;
        let font = FontId::proportional(11.5);
        let count_text = format!("{count} nodes");
        let count_w = painter
            .layout_no_wrap(count_text.clone(), font.clone(), theme::text()_DIM)
            .size()
            .x;
        let widths: Vec<f32> = items
            .iter()
            .map(|&i| {
                10.0 + 14.0
                    + 6.0
                    + painter
                        .layout_no_wrap(label(i).to_owned(), font.clone(), theme::text())
                        .size()
                        .x
                    + 10.0
            })
            .collect();
        let w = 14.0
            + count_w
            + 12.0
            + 1.0
            + 6.0
            + widths.iter().sum::<f32>()
            + 2.0 * (items.len() - 1) as f32
            + 4.0;
        let x = (bounds.center().x - w * 0.5).clamp(
            canvas.left() + 8.0,
            (canvas.right() - 8.0 - w).max(canvas.left() + 8.0),
        );
        let above = bounds.top() - 10.0 - H;
        let below = bounds.bottom() + 10.0;
        let y = if above >= canvas.top() + 8.0 {
            above
        } else if below + H <= canvas.bottom() - 8.0 {
            below
        } else {
            canvas.top() + 8.0
        };
        let bar = Rect::from_min_size(pos2(x, y), vec2(w, H));
        // The bar itself takes clicks and drags, so a near miss of a button
        // doesn't reach the canvas (and deselect, or move nodes).
        ui.interact(bar, Id::unique("selection_bar"), Sense::CLICK | Sense::DRAG);
        self.bar = Some(bar);
        painter.rect(
            bar,
            H * 0.5,
            theme::panel(),
            Stroke::new(1.0, theme::node_border()),
            StrokeKind::Inside,
        );
        painter.text(
            pos2(bar.left() + 14.0, bar.center().y),
            Align2::LEFT_CENTER,
            count_text,
            font.clone(),
            theme::text()_DIM,
        );
        let rule_x = bar.left() + 14.0 + count_w + 12.0;
        painter.vline(
            rule_x,
            (bar.top() + 8.0)..=(bar.bottom() - 8.0),
            Stroke::new(1.0, theme::panel()_BORDER),
        );

        let mut bx = rule_x + 6.0;
        for (&item, &bw) in items.iter().zip(&widths) {
            let r = Rect::from_min_size(
                pos2(bx, bar.center().y - BUTTON_H * 0.5),
                vec2(bw, BUTTON_H),
            );
            bx += bw + 2.0;
            self.bar_buttons.push((item, r));
            let resp = ui
                .interact(r, Id::unique(("selection_bar", item as u8)), Sense::CLICK)
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(match item {
                    BarItem::Module => "Gather the selected nodes into one module (Cmd+G); modules among them join it",
                    BarItem::Collapse => "Collapse or expand the selection (C)",
                    BarItem::Duplicate => "Duplicate the selection, with the cables between its nodes (Cmd+D)",
                    BarItem::Delete => "Delete the selection (Del)",
                });
            let hot = resp.hovered();
            if hot {
                painter.rect_filled(r, BUTTON_H * 0.5, theme::control());
            }
            let ink = match item {
                BarItem::Delete if hot => theme::mute(),
                BarItem::Module => theme::accent(),
                _ if hot => theme::text(),
                _ => theme::text()_DIM,
            };
            bar_icon(
                painter,
                pos2(r.left() + 10.0 + 7.0, r.center().y),
                item,
                ink,
            );
            painter.text(
                pos2(r.left() + 10.0 + 14.0 + 6.0, r.center().y),
                Align2::LEFT_CENTER,
                label(item),
                font.clone(),
                theme::text(),
            );
            if resp.clicked() {
                actions.push(match item {
                    BarItem::Module => Action::GroupSelection,
                    BarItem::Collapse => Action::CollapseSelection,
                    BarItem::Duplicate => Action::DuplicateSelection,
                    BarItem::Delete => Action::RemoveSelection,
                });
            }
        }
    }

    fn collapse_selection(&self, graph: &mut Graph) {
        let whole: Vec<(GroupId, bool)> = graph
            .groups
            .iter()
            .filter(|g| g.members.iter().all(|m| self.selected.contains(m)))
            .map(|g| (g.id, g.collapsed))
            .collect();
        let covered = self.selected.iter().all(|id| {
            graph
                .group_of(*id)
                .is_some_and(|g| whole.iter().any(|(gid, _)| *gid == g.id))
        });
        if !whole.is_empty() && covered {
            let collapse = whole.iter().any(|(_, c)| !c);
            for (gid, _) in whole {
                graph.set_group_collapsed(gid, collapse);
            }
        } else {
            let collapse = self
                .selected
                .iter()
                .any(|id| graph.node(*id).is_some_and(|n| !n.collapsed));
            for id in &self.selected {
                graph.set_collapsed(*id, collapse);
            }
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
/// of its nodes (Shift or Cmd+click adds it to the selection, or takes it
/// out); double-clicking folds it. `picks`: see `Editor::show`. Returns how
/// far a right or middle drag on it pans the view.
fn module_pointer(
    ui: &Ui,
    resp: &Response,
    g: &Group,
    selected: bool,
    (additive, picks): (bool, bool),
    z: f32,
    actions: &mut Vec<Action>,
) -> Vec2 {
    if resp.clicked() && picks && additive && selected {
        actions.extend(g.members.iter().map(|&m| Action::ToggleSelect(m)));
    } else if (resp.clicked() && picks)
        || (resp.drag_started_by(PointerButton::Primary) && !selected)
    {
        actions.push(Action::Select(g.members.clone(), additive));
    }
    if resp.dragged_by(PointerButton::Primary) {
        actions.push(Action::MoveSelection(resp.drag_delta() / z));
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    if resp.double_clicked() {
        actions.push(Action::ToggleGroup(g.id));
    }
    if resp.dragged_by(PointerButton::Secondary) || resp.dragged_by(PointerButton::Middle) {
        resp.drag_delta()
    } else {
        Vec2::ZERO
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

pub(super) fn menu_item(ui: &mut Ui, label: &str, hint: &str) -> bool {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(theme::text()).size(12.5))
            .right_text(
                egui::RichText::new(hint)
                    .color(theme::text()_FAINT)
                    .size(10.5),
            )
            .min_size(vec2(220.0, 22.0)),
    )
    .clicked()
}

fn menu_heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text).size(9.5).color(theme::text()_FAINT));
}

/// What a menu acts on, at its top: "4 nodes selected", a node's or a
/// module's title.
fn subject_heading(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(text).size(10.5).color(theme::accent()));
    });
    ui.separator();
}

/// The subject of the menu on `target`, when it is more than its own title
/// would say: the whole selection when the target is in it with others.
/// None: the target alone (its title says it), or nothing selected.
fn menu_subject(graph: &Graph, selected: &HashSet<NodeId>, target: MenuTarget) -> Option<String> {
    let count = selected
        .iter()
        .filter(|id| graph.node(**id).is_some())
        .count();
    let many = |n: usize| {
        if n == 1 {
            "1 node selected".to_owned()
        } else {
            format!("{n} nodes selected")
        }
    };
    match target {
        MenuTarget::Canvas(_) | MenuTarget::Cable(_) => (count > 0).then(|| many(count)),
        MenuTarget::Node(id) => (count > 1 && selected.contains(&id)).then(|| many(count)),
        MenuTarget::Group(gid) => {
            let g = graph.group(gid)?;
            let more = selected.iter().any(|id| !g.members.contains(id));
            Some(if more {
                many(count)
            } else {
                format!(
                    "{} · {}",
                    g.title,
                    many(g.members.len()).replace(" selected", "")
                )
            })
        }
    }
}

/// Node types in groups, then the module library; returns the pick (and closes).
pub fn add_menu(ui: &mut Ui, library: &[Preset], plugins: &[PluginEntry]) -> Option<AddChoice> {
    let mut picked = None;
    for (group, types) in NodeType::MENU {
        menu_heading(ui, group);
        for &ty in types {
            if menu_item(ui, ty.name(), ty.hint()) {
                picked = Some(AddChoice::Node(ty));
                ui.close();
            }
        }
    }

    // Plugin nodes (dylibs built with the node SDK): one submenu per maker.
    let makers = crate::audio::node_plugin::by_maker();
    if !makers.is_empty() {
        menu_heading(ui, "Plugin nodes");
        for (maker, list) in makers {
            ui.menu_button(format!("{maker}  ({})", list.len()), |ui| {
                for e in list {
                    let ty = NodeType::Module(crate::audio::modules::ModuleKind::Ext(e));
                    if menu_item(ui, ty.name(), ty.hint()) {
                        picked = Some(AddChoice::Node(ty));
                        ui.close();
                    }
                }
            });
        }
    }

    if !plugins.is_empty() {
        menu_heading(ui, "Plugins");
        // One submenu per maker.
        for (vendor, members) in super::palette::by_vendor(plugins) {
            ui.menu_button(format!("{vendor}  ({})", members.len()), |ui| {
                for i in members {
                    if menu_item(ui, &plugins[i].name, &plugins[i].hint) {
                        picked = Some(AddChoice::Plugin(i));
                        ui.close();
                    }
                }
            });
        }
    }

    menu_heading(ui, "Library");
    if library.is_empty() {
        ui.label(
            egui::RichText::new("Right-click a module → Save to library")
                .size(10.5)
                .color(theme::text()_FAINT),
        );
    }
    for (i, preset) in library.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let n = preset.node_count();
            let hint = if n == 1 {
                "1 node".to_owned()
            } else {
                format!("{n} nodes")
            };
            if menu_item(ui, &preset.name, &hint) {
                picked = Some(AddChoice::Preset(i));
                ui.close();
            }
            let remove = ui
                .add(
                    egui::Button::new(egui::RichText::new("×").color(theme::text()_FAINT))
                        .min_size(vec2(22.0, 22.0)),
                )
                .on_hover_text("Remove from the library");
            if remove.clicked() {
                picked = Some(AddChoice::Request(Request::DeletePreset(i)));
                ui.close();
            }
        });
    }
    if menu_item(ui, "Open library folder", "") {
        picked = Some(AddChoice::Request(Request::RevealLibrary));
        ui.close();
    }
    picked
}

/// The canvas menu: with a selection, first what to do with it; then what
/// to add; then the view.
fn canvas_menu(
    ui: &mut Ui,
    pos: Pos2,
    selection: Option<&str>,
    library: &[Preset],
    plugins: &[PluginEntry],
    actions: &mut Vec<Action>,
) {
    if let Some(subject) = selection {
        subject_heading(ui, subject);
        if menu_item(ui, "Make module from selection", "Cmd+G") {
            actions.push(Action::GroupSelection);
            ui.close();
        }
        if menu_item(ui, "Deselect", "Esc") {
            actions.push(Action::Deselect);
            ui.close();
        }
        ui.separator();
    }
    match add_menu(ui, library, plugins) {
        Some(AddChoice::Node(ty)) => actions.push(Action::Add(ty, pos)),
        Some(AddChoice::Preset(i)) => actions.push(Action::AddPreset(i, pos)),
        Some(AddChoice::Plugin(i)) => actions.push(Action::AddPlugin(i, pos)),
        Some(AddChoice::Request(r)) => actions.push(Action::Request(r)),
        None => {}
    }
    ui.separator();
    if menu_item(ui, "Auto layout", "L") {
        actions.push(Action::AutoLayout);
        ui.close();
    }
    if menu_item(ui, "Fit to view", "F") {
        actions.push(Action::FitAll);
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

/// Asks before deleting nodes, which can't be undone: "Delete “FILTER 1”?"
/// in a native dialog (Enter deletes, Esc cancels).
pub(super) fn confirm_removal(graph: &Graph, ids: &[NodeId]) -> bool {
    let names: Vec<&str> = ids
        .iter()
        .filter_map(|&id| graph.node(id).map(|n| n.title.as_str()))
        .collect();
    let what = match names.as_slice() {
        [] => return false,
        [one] => format!("“{one}”"),
        many if many.len() <= 4 => format!("{} nodes ({})", many.len(), many.join(", ")),
        many => format!("{} nodes ({}, …)", many.len(), many[..4].join(", ")),
    };
    let lost = if names.len() == 1 {
        "Its settings and cables"
    } else {
        "Their settings and cables"
    };
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(if names.len() == 1 {
            "Delete node"
        } else {
            "Delete nodes"
        })
        .set_description(format!(
            "Delete {what}?\n{lost} will be lost: this can't be undone."
        ))
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            "Delete".into(),
            "Cancel".into(),
        ))
        .show();
    match answer {
        rfd::MessageDialogResult::Ok => true,
        rfd::MessageDialogResult::Custom(label) => label == "Delete",
        _ => false,
    }
}

/// Asks before going down to 12 channels unplugs `lost` cables.
pub(super) fn confirm_unplug(lost: usize) -> bool {
    let cables = if lost == 1 {
        "1 cable".to_owned()
    } else {
        format!("{lost} cables")
    };
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("12 channels")
        .set_description(format!(
            "Channels 13–24 have {cables} plugged in.\nGoing to 12 channels unplugs them."
        ))
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            "Unplug".into(),
            "Cancel".into(),
        ))
        .show();
    match answer {
        rfd::MessageDialogResult::Ok => true,
        rfd::MessageDialogResult::Custom(label) => label == "Unplug",
        _ => false,
    }
}

/// Acts on the selection (right-clicking a node selects it first).
/// Bypasses the effects among `ids`, or switches them all back on when
/// every one already is.
fn toggle_bypass(graph: &Graph, ids: &[NodeId]) {
    let effects: Vec<_> = ids
        .iter()
        .filter_map(|&id| graph.node(id))
        .filter(|n| n.is_effect())
        .collect();
    let on = effects.iter().any(|n| !n.bypassed());
    for n in effects {
        n.set_bypass(on);
    }
}

fn node_menu(
    ui: &mut Ui,
    collapsed: bool,
    effect: bool,
    in_group: bool,
    actions: &mut Vec<Action>,
) {
    let fold = if collapsed { "Expand" } else { "Collapse" };
    let mut item = |label: &str, hint: &str, action: Action| {
        if menu_item(ui, label, hint) {
            actions.push(action);
            ui.close();
        }
    };
    item(fold, "C", Action::CollapseSelection);
    item("Zoom to node", "Z", Action::FocusSelection);
    if effect {
        item("Bypass on / off", "B", Action::BypassSelection);
    }
    item("Make module", "Cmd+G", Action::GroupSelection);
    item("Duplicate", "Cmd+D", Action::DuplicateSelection);
    item("Delete", "Del", Action::RemoveSelection);
    if in_group {
        item("Ungroup module", "Cmd+Shift+G", Action::UngroupSelection);
    }
}

/// `more`: other nodes are selected besides this module; Make module then
/// gathers them all into one module (this one dissolves into it).
fn group_menu(ui: &mut Ui, gid: GroupId, collapsed: bool, more: bool, actions: &mut Vec<Action>) {
    if more {
        if menu_item(ui, "Make module from selection", "Cmd+G") {
            actions.push(Action::GroupSelection);
            ui.close();
        }
        ui.separator();
    }
    if menu_item(ui, "Rename…", "Enter") {
        actions.push(Action::StartRename(gid));
        ui.close();
    }
    if menu_item(ui, if collapsed { "Expand" } else { "Collapse" }, "C") {
        actions.push(Action::ToggleGroup(gid));
        ui.close();
    }
    if menu_item(ui, "Save to library", "") {
        actions.push(Action::Request(Request::SaveToLibrary(gid)));
        ui.close();
    }
    // These act on the whole selection: say so when it is more than this.
    if menu_item(
        ui,
        if more {
            "Duplicate selection"
        } else {
            "Duplicate module"
        },
        "Cmd+D",
    ) {
        actions.push(Action::DuplicateSelection);
        ui.close();
    }
    if menu_item(ui, "Ungroup", "Cmd+Shift+G") {
        actions.push(Action::Ungroup(gid));
        ui.close();
    }
    if menu_item(
        ui,
        if more {
            "Delete selection"
        } else {
            "Delete module and its nodes"
        },
        "Del",
    ) {
        actions.push(Action::RemoveSelection);
        ui.close();
    }
}

/// A button of the selection bar (see `Editor::selection_bar`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum BarItem {
    Module,
    Collapse,
    Duplicate,
    Delete,
}

/// The selection bar's line-art icons, 14 points across.
fn bar_icon(p: &Painter, c: Pos2, item: BarItem, col: Color32) {
    let s = Stroke::new(1.2, col);
    match item {
        BarItem::Module => {
            // Two blocks joined, inside a frame drawn by its corners.
            let r = Rect::from_center_size(c, vec2(14.0, 12.0));
            for (corner, dx, dy) in [
                (r.left_top(), 1.0, 1.0),
                (r.right_top(), -1.0, 1.0),
                (r.left_bottom(), 1.0, -1.0),
                (r.right_bottom(), -1.0, -1.0),
            ] {
                p.line_segment([corner, corner + vec2(3.5 * dx, 0.0)], s);
                p.line_segment([corner, corner + vec2(0.0, 3.0 * dy)], s);
            }
            let (a, b) = (
                Rect::from_center_size(c + vec2(-2.8, 0.0), vec2(3.6, 3.6)),
                Rect::from_center_size(c + vec2(2.8, 0.0), vec2(3.6, 3.6)),
            );
            p.rect_stroke(a, 0.5, s, StrokeKind::Inside);
            p.rect_stroke(b, 0.5, s, StrokeKind::Inside);
            p.line_segment([a.right_center(), b.left_center()], s);
        }
        BarItem::Collapse => {
            // Two slim cards: nodes folded to their headers.
            for dy in [-3.0, 3.0] {
                p.rect_stroke(
                    Rect::from_center_size(c + vec2(0.0, dy), vec2(13.0, 4.0)),
                    1.5,
                    s,
                    StrokeKind::Inside,
                );
            }
        }
        BarItem::Duplicate => {
            // Two overlapping cards.
            p.rect_stroke(
                Rect::from_min_size(c + vec2(-6.0, -6.0), vec2(8.5, 8.5)),
                1.5,
                s,
                StrokeKind::Inside,
            );
            let front = Rect::from_min_size(c + vec2(-2.5, -2.5), vec2(8.5, 8.5));
            p.rect_filled(front, 1.5, theme::panel());
            p.rect_stroke(front, 1.5, s, StrokeKind::Inside);
        }
        BarItem::Delete => {
            // A bin: its lid and handle, its body with two ribs.
            p.line_segment([c + vec2(-6.0, -4.0), c + vec2(6.0, -4.0)], s);
            p.line_segment([c + vec2(-2.0, -6.0), c + vec2(2.0, -6.0)], s);
            let body = vec![
                c + vec2(-4.5, -4.0),
                c + vec2(-3.5, 6.0),
                c + vec2(3.5, 6.0),
                c + vec2(4.5, -4.0),
            ];
            p.add(Shape::line(body, s));
            for x in [-1.2, 1.2] {
                p.line_segment([c + vec2(x, -1.5), c + vec2(x, 3.5)], s);
            }
        }
    }
}

/// Bounds of the selected nodes as drawn (a folded module counts whole).
fn selection_bounds(graph: &Graph, selected: &HashSet<NodeId>) -> Option<Rect> {
    let mut bounds: Option<Rect> = None;
    let mut add = |r: Rect| bounds = Some(bounds.map_or(r, |b| b.union(r)));
    for g in &graph.groups {
        if g.collapsed
            && g.members.iter().any(|m| selected.contains(m))
            && let Some((r, ..)) = groups::card_layout(graph, g)
        {
            add(r);
        }
    }
    for n in &graph.nodes {
        let folded = graph.group_of(n.id).is_some_and(|g| g.collapsed);
        if selected.contains(&n.id) && !folded {
            add(Rect::from_min_size(n.pos, n.size()));
        }
    }
    bounds
}

/// The type of a node or module for its label: ("Reverb", "hall · shimmer ·
/// freeze"), ("Originals - Cinematic Pads", "VST3 instrument · Spitfire
/// Audio") or ("Module", "Drone · Filter · 2× Reverb").
fn type_label(graph: &Graph, target: TipTarget) -> Option<(String, String)> {
    let node_type = |n: &crate::graph::Node| match &n.kind {
        NodeKind::Plugin(p) => p.meta.name.clone(),
        k => k.node_type().name().to_owned(),
    };
    match target {
        TipTarget::Node(id) => {
            let n = graph.node(id)?;
            Some(match &n.kind {
                NodeKind::Plugin(p) => {
                    let kind = if p.meta.instrument {
                        "instrument"
                    } else {
                        "effect"
                    };
                    let what = format!("{} {kind}", p.meta.format.label());
                    let detail = if p.meta.vendor.is_empty() {
                        what
                    } else {
                        format!("{what} · {}", p.meta.vendor)
                    };
                    (p.meta.name.clone(), detail)
                }
                k => (
                    k.node_type().name().to_owned(),
                    k.node_type().hint().to_owned(),
                ),
            })
        }
        TipTarget::Group(gid) => {
            // Member types in order of appearance, repeats counted.
            let mut kinds: Vec<(String, usize)> = Vec::new();
            for n in graph
                .group(gid)?
                .members
                .iter()
                .filter_map(|id| graph.node(*id))
            {
                let name = node_type(n);
                match kinds.iter_mut().find(|(k, _)| *k == name) {
                    Some((_, count)) => *count += 1,
                    None => kinds.push((name, 1)),
                }
            }
            let mut parts: Vec<String> = kinds
                .iter()
                .take(5)
                .map(|(k, c)| {
                    if *c > 1 {
                        format!("{c}× {k}")
                    } else {
                        k.clone()
                    }
                })
                .collect();
            if kinds.len() > 5 {
                parts.push("…".to_owned());
            }
            Some(("Module".to_owned(), parts.join(" · ")))
        }
    }
}
