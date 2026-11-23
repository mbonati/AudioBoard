//! The track view (README, "Track view"): the patch as one row per track,
//! its chain of devices left to right, Ableton style. It reads and edits
//! the same `Graph` as the canvas (see `crate::tracks` for the rules): a
//! row's header shows where the track goes, its channel's level, mute and
//! solo; its lane shows the devices, compact or with their panel open.
//! Devices are dragged to reorder (the cables follow), dropped in from the
//! palette at any point, removed with the gap closing behind them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::{
    Align2, Color32, CursorIcon, Id, Painter, PointerButton, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

use super::canvas::{self, AddChoice, Request};
use super::nodes::{self, KnobHit, NodeView};
use super::palette::{PluginEntry, Spawn};
use super::theme::{self, alpha};
use super::{modules, plugin_node, widgets};
use crate::audio::AudioHandle;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleKind;
use crate::audio::params::MixerParams;
use crate::graph::{Console, Graph, MIDI_PORT, Node, NodeKind, PortRef, is_midi};
use crate::project::{self, Preset};
use crate::tracks::{self, Input, Tag, Track, TrackKey, TrackMap};

/// Width of the track headers.
pub const HEADER_W: f32 = 236.0;
/// Room above and below the devices in a row.
const PAD: f32 = 10.0;
const BOX_W: f32 = 156.0;
const ATTACHED_W: f32 = 132.0;
const BOX_H: f32 = 76.0;
/// Between two devices: the arrow, and where a dropped device goes.
const GAP: f32 = 26.0;
const LANE_PAD: f32 = 14.0;
const FOLDED_H: f32 = 30.0;
const NEW_ROW_H: f32 = 64.0;
/// Open panels are scaled down to this height at most.
const PANEL_H: f32 = 360.0;
/// Colour of MIDI notes and tags.
const MIDI_COL: Color32 = theme::osc_accents()[3];

/// Where something picked in an add menu (or dropped) goes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum AddAt {
    /// Gap `1` of a track (an effect there, a MIDI node before its
    /// instrument, anything else on a track of its own).
    Gap(TrackKey, usize),
    NewTrack,
}

enum Act {
    Select(Option<NodeId>),
    ToggleOpen(NodeId),
    Fold(TrackKey, bool),
    Bypass(NodeId),
    Remove(NodeId),
    Move(NodeId, TrackKey, usize),
    Reorder(usize, usize),
    Route(TrackKey, Option<PortRef>),
    Add(AddAt, AddChoice),
    SetConsole(NodeId, Console),
    ToggleEditor(NodeId),
}

/// What a device shows, read before the panels borrow the graph.
struct Dev {
    title: String,
    accent: Color32,
    level: f32,
    /// Effects: bypassed or not.
    bypass: Option<bool>,
    summary: String,
    tags: Vec<Tag>,
    /// Its panel's size, open.
    full: Vec2,
    inline: bool,
    /// Audio inputs: the colour and name of what is plugged in.
    sources: Vec<Option<(Color32, String)>>,
    /// Audio outputs patched.
    outs: Vec<bool>,
    /// MIDI in and out patched.
    midi: (bool, bool),
    /// MIDI nodes: the control they drive.
    target: Option<String>,
}

/// A track header's contents.
struct Head {
    name: String,
    kind: &'static str,
    color: Color32,
    from: Option<String>,
    dest: String,
    strip: Strip,
}

enum Strip {
    None,
    /// The mixer channel the track ends in.
    Channel {
        params: Arc<MixerParams>,
        ch: usize,
        meter: widgets::MeterState,
    },
    /// A mixer's own row: its master.
    Master {
        params: Arc<MixerParams>,
        meters: [widgets::MeterState; 2],
    },
    /// Unrouted: the level of its last device.
    Level(f32),
}

/// A row as laid out this frame, for drops.
struct RowGeo {
    key: TrackKey,
    /// Among the tracks (master and loose rows: None).
    index: Option<usize>,
    top: f32,
    bottom: f32,
    /// Gaps a device can go in, and where they are on screen.
    gaps: Vec<(usize, f32)>,
    /// Where a MIDI node would go, before an instrument.
    midi_x: Option<f32>,
}

#[derive(Default)]
pub struct TrackView {
    scroll: f32,
    hscroll: HashMap<TrackKey, f32>,
    /// Each lane's content width last frame.
    widths: HashMap<TrackKey, f32>,
    pub selected: Option<NodeId>,
    hovered: Option<NodeId>,
    /// A device being dragged to another place.
    drag: Option<NodeId>,
    /// A track being dragged up or down (its index among the tracks).
    row_drag: Option<usize>,
    /// Short messages for the status bar, drained by the app.
    pub notices: Vec<String>,
    /// Requests for the app (from the add menus), drained after each frame.
    pub requests: Vec<Request>,
}

impl TrackView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets gestures and scrolling (another project).
    pub fn reset(&mut self) {
        *self = Self {
            notices: std::mem::take(&mut self.notices),
            requests: std::mem::take(&mut self.requests),
            ..Self::default()
        };
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        graph: &mut Graph,
        audio: &AudioHandle,
        library: &[Preset],
        plugins: &[PluginEntry],
    ) {
        let view = ui.max_rect();
        let now = ui.input(|i| i.time);
        ui.painter_at(view).rect_filled(view, 0.0, theme::bg());
        tick_iris(ui, graph);
        if let Some(id) = self.selected
            && graph.node(id).is_none()
        {
            self.selected = None;
        }

        let map = TrackMap::build(graph);
        let devs = dev_info(graph, &map);
        let heads: Vec<Head> = map.rows.iter().map(|t| head_info(graph, t)).collect();
        let heights: Vec<f32> = map
            .rows
            .iter()
            .map(|t| row_height(graph, t, &devs))
            .collect();
        let free: Vec<(PortRef, String)> = tracks::free_channels(graph)
            .into_iter()
            .map(|p| (p, channel_label(graph, p)))
            .collect();

        // Scrolling: up and down the rows, sideways along the lane under
        // the pointer.
        let content_h: f32 = heights.iter().sum::<f32>() + NEW_ROW_H + PAD;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let mut hovered_row = None;
        if let Some(p) = pointer.filter(|p| view.contains(*p)) {
            let mut y = view.top() - self.scroll;
            for (t, h) in map.rows.iter().zip(&heights) {
                if (y..y + h).contains(&p.y) {
                    hovered_row = Some(t.key);
                }
                y += h;
            }
            let delta = ui.input(|i| i.smooth_scroll_delta);
            self.scroll -= delta.y;
            if let Some(key) = hovered_row.filter(|_| p.x > view.left() + HEADER_W) {
                *self.hscroll.entry(key).or_default() -= delta.x;
            }
        }
        self.scroll = self.scroll.clamp(0.0, (content_h - view.height()).max(0.0));
        let lane_w = view.width() - HEADER_W;
        for (key, s) in self.hscroll.iter_mut() {
            let w = self.widths.get(key).copied().unwrap_or(0.0);
            *s = s.clamp(0.0, (w - lane_w).max(0.0));
        }

        let mut acts = Vec::new();
        let mut geos = Vec::new();
        self.hovered = None;
        let mut y = view.top() - self.scroll;
        let mut index = 0;
        for (r, t) in map.rows.iter().enumerate() {
            let h = heights[r];
            let row = Rect::from_min_size(pos2(view.left(), y), vec2(view.width(), h));
            y += h;
            let track_index = t.is_track().then(|| {
                index += 1;
                index - 1
            });
            let folded = graph.tracks.is_folded(t.key);
            if !row.intersects(view) {
                geos.push(RowGeo {
                    key: t.key,
                    index: track_index,
                    top: row.top(),
                    bottom: row.bottom(),
                    gaps: Vec::new(),
                    midi_x: None,
                });
                continue;
            }
            let geo = self.lane(ui, graph, t, &devs, row, view, folded, now, library, plugins, &mut acts);
            geos.push(RowGeo {
                index: track_index,
                ..geo
            });
            self.header(ui, graph, t, &heads[r], row, view, folded, track_index, &free, &mut acts);
            ui.painter_at(view).hline(
                row.x_range(),
                row.bottom() - 0.5,
                Stroke::new(1.0, theme::panel()_BORDER),
            );
        }
        // The header column runs to the bottom.
        let below = Rect::from_min_max(pos2(view.left(), y), pos2(view.left() + HEADER_W, view.bottom()));
        if below.height() > 0.0 {
            ui.painter_at(view).rect_filled(below, 0.0, theme::panel());
        }
        let new_row = self.new_track_row(ui, view, y, graph.nodes.is_empty(), library, plugins, &mut acts);

        self.drops(ui, graph, &map, &devs, view, &geos, new_row, pointer, &mut acts);
        self.row_drag(ui, view, &geos, pointer, &mut acts);
        if ui.memory(|m| m.focused().is_none()) {
            self.keyboard(ui, &devs, &mut acts);
        }
        self.apply(acts, graph, audio, library, plugins);
    }

    /// One row's devices; returns where its gaps are.
    #[allow(clippy::too_many_arguments)]
    fn lane(
        &mut self,
        ui: &mut Ui,
        graph: &mut Graph,
        t: &Track,
        devs: &HashMap<NodeId, Dev>,
        row: Rect,
        view: Rect,
        folded: bool,
        now: f64,
        library: &[Preset],
        plugins: &[PluginEntry],
        acts: &mut Vec<Act>,
    ) -> RowGeo {
        let lane = Rect::from_min_max(pos2(row.left() + HEADER_W, row.top()), row.max);
        let clip = lane.intersect(view);
        let mut lui = ui.new_child(UiBuilder::new().max_rect(lane));
        lui.set_clip_rect(clip);
        let painter = lui.painter().clone();
        let scroll = self.hscroll.get(&t.key).copied().unwrap_or(0.0);
        let left = lane.left() + LANE_PAD - scroll;
        let top = if folded { row.top() + 4.0 } else { row.top() + PAD };
        let line_y = if folded { row.center().y } else { top + nodes::HEADER_H * 0.5 };
        let tempo = graph.tempo;
        let mut x = left;
        let mut gaps = Vec::new();
        let mut midi_x = None;

        // Where the chain comes from, when not from its own source.
        let from = match t.input {
            Input::Branch(p) | Input::Bus(p) => Some(p),
            _ => None,
        };
        // The box before (right edge, colour, what it serves if attached),
        // the chain's last element (where the audio arrow starts), and the
        // attached boxes since: the audio runs under them.
        let mut prev: Option<(f32, Color32, Option<NodeId>)> = None;
        let mut chain_prev: Option<(f32, Color32)> = None;
        let mut under: Option<f32> = None;
        if let Some(p) = from
            && let Some(n) = graph.node(p.node)
        {
            let text = format!("◂ {}", port_name(n, p.port));
            let r = chip(&painter, pos2(x, line_y), &text, n.accent);
            x = r.right();
            prev = Some((x, n.accent, None));
            chain_prev = Some((x, n.accent));
        }

        let gap_ok = |g: usize| t.gaps().is_some_and(|r| r.contains(&g));
        let mut chain_index = 0;
        let mut group_start = true;
        for s in &t.slots {
            let Some(d) = devs.get(&s.node) else {
                continue;
            };
            if prev.is_some() {
                x += GAP;
            }
            if group_start {
                if gap_ok(chain_index) {
                    gaps.push((chain_index, x - GAP * 0.5));
                }
                if chain_index == 0 && t.input == Input::Source {
                    midi_x = Some(x - GAP * 0.5);
                }
                group_start = false;
            }
            let open = !folded && graph.tracks.is_open(s.node);
            let (size, z) = if folded {
                let w = theme_text_w(ui, &d.title, 10.5) + 22.0;
                (vec2(w, FOLDED_H - 8.0), 1.0)
            } else if open {
                let z = (PANEL_H / d.full.y).min(1.0);
                (d.full * z, z)
            } else if s.serves.is_some() {
                (vec2(ATTACHED_W, BOX_H), 1.0)
            } else {
                (vec2(BOX_W, BOX_H), 1.0)
            };
            let r = if folded {
                Rect::from_min_size(pos2(x, row.center().y - size.y * 0.5), size)
            } else {
                Rect::from_min_size(pos2(x, top), size)
            };
            // An attached box points at what it serves (dashed).
            if let Some((px, col, Some(served))) = prev
                && served == s.node
            {
                arrow(&painter, pos2(px + 3.0, line_y), pos2(r.left() - 3.0, line_y), alpha(col, 0.75), true);
            }
            if s.serves.is_some() {
                under = Some(under.map_or(r.bottom(), |b: f32| b.max(r.bottom())));
            } else if let below = under.take()
                && let Some((cx, col)) = chain_prev
            {
                match below {
                    // The audio goes round under the attached boxes.
                    Some(bottom) if !folded => elbow(&painter, cx, r.left(), line_y, bottom + 5.0, line_y + 22.0, col),
                    Some(_) => arrow(&painter, pos2(prev.map_or(cx, |p| p.0) + 3.0, line_y), pos2(r.left() - 3.0, line_y), col, false),
                    None => arrow(&painter, pos2(cx + 3.0, line_y), pos2(r.left() - 3.0, line_y), col, false),
                }
            }
            if r.intersects(clip) {
                if folded {
                    self.pill(&lui, &painter, s.node, d, r, acts);
                } else {
                    self.device(&lui, &painter, graph, t, s, d, r, z, open, tempo, now, acts);
                }
            }
            prev = Some((r.right(), d.accent, s.serves));
            x = r.right();
            if s.serves.is_none() {
                chain_prev = Some((r.right(), d.accent));
                chain_index += 1;
                group_start = true;
            }
        }
        if t.is_track() {
            if prev.is_some() {
                x += GAP;
            }
            if gap_ok(chain_index) {
                gaps.push((chain_index, x - GAP * 0.5));
            }
            // Where it goes.
            let (text, col) = match t.dest {
                Some(p) => (format!("▸ {}", channel_label(graph, p)), theme::mixer_accent()),
                None => ("not routed".to_owned(), theme::text()_FAINT),
            };
            if let Some((px, c)) = chain_prev
                && t.dest.is_some()
            {
                arrow(&painter, pos2(px + 3.0, line_y), pos2(x - 3.0, line_y), c, false);
            }
            let r = chip(&painter, pos2(x, line_y), &text, col);
            x = r.right() + 10.0;
            // Add an effect at the end.
            if !folded {
                let plus = Rect::from_center_size(pos2(x + 12.0, line_y), vec2(24.0, 22.0));
                let resp = lui
                    .interact(plus, Id::unique(("trk-plus", t.key)), Sense::CLICK)
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text("Add a device at the end of this track");
                let col = if resp.hovered() { theme::text() } else { theme::text()_FAINT };
                dashed_rect(&painter, plus, Stroke::new(1.0, col));
                plus_icon(&painter, plus.center(), col);
                if let Some(choice) = egui::Popup::menu(&resp)
                    .show(|ui| canvas::add_menu(ui, library, plugins))
                    .and_then(|r| r.inner)
                {
                    acts.push(Act::Add(AddAt::Gap(t.key, chain_index), choice));
                }
                x = plus.right();
            }
        } else if t.slots.is_empty() {
            painter.text(
                pos2(x, line_y),
                Align2::LEFT_CENTER,
                "nothing here",
                theme::font(10.5),
                theme::text()_FAINT,
            );
        }
        self.widths.insert(t.key, x + scroll - lane.left() + LANE_PAD);
        RowGeo {
            key: t.key,
            index: None,
            top: row.top(),
            bottom: row.bottom(),
            gaps: if folded { Vec::new() } else { gaps },
            midi_x: midi_x.filter(|_| !folded),
        }
    }

    /// A device in a folded row: its name.
    fn pill(&mut self, ui: &Ui, p: &Painter, id: NodeId, d: &Dev, r: Rect, acts: &mut Vec<Act>) {
        let resp = ui
            .interact(r, Id::unique(("trk-pill", id)), Sense::CLICK)
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() {
            acts.push(Act::Select(Some(id)));
        }
        if resp.hovered() {
            self.hovered = Some(id);
        }
        let selected = self.selected == Some(id);
        p.rect(
            r,
            r.height() * 0.5,
            theme::node_body(),
            Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { d.accent } else { theme::node_border() }),
            StrokeKind::Inside,
        );
        p.circle_filled(r.left_center() + vec2(10.0, 0.0), 3.0, theme::mix(theme::control()_HI, d.accent, 0.35 + 0.65 * d.level.clamp(0.0, 1.0).sqrt()));
        let col = if d.bypass == Some(true) { theme::text()_FAINT } else { theme::text() };
        p.text(r.left_center() + vec2(18.0, 0.0), Align2::LEFT_CENTER, &d.title, theme::strong(10.5), col);
    }

    /// A device: its card, compact or with its panel.
    #[allow(clippy::too_many_arguments)]
    fn device(
        &mut self,
        ui: &Ui,
        painter: &Painter,
        graph: &mut Graph,
        t: &Track,
        s: &tracks::Slot,
        d: &Dev,
        r: Rect,
        z: f32,
        open: bool,
        tempo: f32,
        now: f64,
        acts: &mut Vec<Act>,
    ) {
        let id = s.node;
        let body = ui.interact(r, Id::unique(("trk-dev", id)), Sense::CLICK | Sense::DRAG);
        if body.hovered() {
            self.hovered = Some(id);
        }
        if body.clicked() {
            acts.push(Act::Select(Some(id)));
        }
        let on_header = body
            .interact_pointer_pos()
            .is_some_and(|p| p.y < r.top() + nodes::HEADER_H * z);
        if body.double_clicked() && on_header {
            acts.push(Act::ToggleOpen(id));
        }
        // Effects in a track's chain move; sources and attached devices stay.
        let movable = d.inline && s.serves.is_none() && t.is_track();
        if body.drag_started_by(PointerButton::Primary) && movable {
            self.drag = Some(id);
            acts.push(Act::Select(Some(id)));
        }
        if movable && body.hovered() && self.drag.is_none() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        egui::Popup::context_menu(&body).show(|ui| {
            ui.label(egui::RichText::new(&d.title).size(10.5).color(theme::text()_DIM));
            if canvas::menu_item(ui, if open { "Close panel" } else { "Open panel" }, "C") {
                acts.push(Act::ToggleOpen(id));
                ui.close();
            }
            if let Some(off) = d.bypass
                && canvas::menu_item(ui, if off { "Switch on" } else { "Bypass" }, "B")
            {
                acts.push(Act::Bypass(id));
                ui.close();
            }
            if canvas::menu_item(ui, "Remove", "Del") {
                acts.push(Act::Remove(id));
                ui.close();
            }
        });

        let hits = RefCell::new(Vec::<KnobHit>::new());
        let v = NodeView {
            ui,
            painter,
            min: r.min,
            z,
            node: id,
            hits: &hits,
            tempo,
        };
        let size = r.size() / z;
        let card = audioboard_ui::card::card(
            &v,
            &audioboard_ui::card::Card {
                size,
                title: &d.title,
                accent: d.accent,
                level: d.level,
                collapsed: !open,
                selected: self.selected == Some(id),
                bypass: d.bypass,
            },
        );
        if card.close {
            acts.push(Act::Remove(id));
        }
        if card.toggle_collapse {
            acts.push(Act::ToggleOpen(id));
        }
        if card.bypass {
            acts.push(Act::Bypass(id));
        }
        if open {
            if let Some(node) = graph.node_mut(id)
                && let Some(act) = panel(&v, node, d, now, &mut self.notices)
            {
                acts.push(act);
            }
        } else {
            compact(&v, s, d, size);
            if !d.tags.is_empty() {
                let all: Vec<&str> = d.tags.iter().map(|t| t.text.as_str()).collect();
                body.on_hover_text(all.join("\n"));
            }
        }
        if d.bypass == Some(true) {
            audioboard_ui::card::veil(&v, size);
        }
        if self.drag == Some(id) {
            painter.rect_filled(r, audioboard_ui::card::RADIUS * z, alpha(theme::bg(), 0.6));
        }
    }

    /// A row's header: name, where it goes, its channel.
    #[allow(clippy::too_many_arguments)]
    fn header(
        &mut self,
        ui: &mut Ui,
        graph: &Graph,
        t: &Track,
        h: &Head,
        row: Rect,
        view: Rect,
        folded: bool,
        index: Option<usize>,
        free: &[(PortRef, String)],
        acts: &mut Vec<Act>,
    ) {
        let hr = Rect::from_min_size(row.min, vec2(HEADER_W, row.height()));
        let mut hui = ui.new_child(UiBuilder::new().max_rect(hr));
        hui.set_clip_rect(hr.intersect(view));
        let p = hui.painter().clone();
        p.rect_filled(hr, 0.0, theme::panel());
        p.vline(hr.right() - 0.5, hr.y_range(), Stroke::new(1.0, theme::panel()_BORDER));
        p.rect_filled(Rect::from_min_size(hr.min, vec2(3.0, hr.height())), 0.0, h.color);
        let top = hr.top();
        let name_y = if folded { hr.center().y } else { top + 16.0 };

        // Fold chevron.
        if t.is_track() {
            let fold = Rect::from_center_size(pos2(hr.left() + 15.0, name_y), Vec2::splat(16.0));
            let resp = hui
                .interact(fold, Id::unique(("trk-fold", t.key)), Sense::CLICK)
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(if folded { "Show the devices" } else { "Fold to a thin row" });
            if resp.clicked() {
                acts.push(Act::Fold(t.key, !folded));
            }
            let c = fold.center();
            let s = 3.0;
            let tri = if folded {
                vec![c + vec2(-0.6 * s, -s), c + vec2(0.9 * s, 0.0), c + vec2(-0.6 * s, s)]
            } else {
                vec![c + vec2(-s, -0.6 * s), c + vec2(s, -0.6 * s), c + vec2(0.0, 0.9 * s)]
            };
            let col = if resp.hovered() { theme::text() } else { theme::text()_FAINT };
            p.add(Shape::convex_polygon(tri, col, Stroke::NONE));
        }
        let name_x = hr.left() + 27.0;
        p.text(pos2(hr.right() - 10.0, name_y), Align2::RIGHT_CENTER, h.kind, theme::strong(8.0), theme::text()_FAINT);
        let name_clip = p.with_clip_rect(Rect::from_min_max(pos2(name_x, top), pos2(hr.right() - 70.0, hr.bottom())).intersect(view));
        name_clip.text(pos2(name_x, name_y), Align2::LEFT_CENTER, &h.name, theme::strong(12.0), theme::text());
        // Drag the name to move the track up or down.
        if let Some(i) = index {
            let grip = Rect::from_min_max(pos2(name_x - 2.0, name_y - 10.0), pos2(hr.right() - 70.0, name_y + 10.0));
            let resp = hui
                .interact(grip, Id::unique(("trk-grip", t.key)), Sense::DRAG)
                .on_hover_cursor(CursorIcon::Grab);
            if resp.drag_started_by(PointerButton::Primary) {
                self.row_drag = Some(i);
            }
        }

        if folded {
            if let Strip::Channel { params, ch, .. } = &h.strip {
                let ms = Rect::from_min_size(pos2(hr.right() - 112.0, name_y - 7.0), vec2(18.0, 14.0));
                mute_solo(&hui, &p, params, *ch, ms, t.key);
            }
            return;
        }

        // Where it goes (a menu of free channels), and where it comes from.
        let dest_pos = pos2(name_x, top + 37.0);
        if t.is_track() {
            let galley = p.layout_no_wrap(h.dest.clone(), theme::font(10.5), theme::mixer_accent());
            let r = Rect::from_min_size(dest_pos - vec2(0.0, galley.size().y * 0.5), galley.size()).expand(2.0);
            let resp = hui
                .interact(r, Id::unique(("trk-dest", t.key)), Sense::CLICK)
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Send this track to another channel");
            let col = if t.dest.is_none() { theme::text()_FAINT } else if resp.hovered() { theme::text() } else { theme::mixer_accent() };
            p.text(dest_pos, Align2::LEFT_CENTER, &h.dest, theme::font(10.5), col);
            let current = t.dest;
            if let Some(dest) = egui::Popup::menu(&resp)
                .show(|ui| {
                    let mut picked = None;
                    if current.is_some() && canvas::menu_item(ui, "Not routed", "") {
                        picked = Some(None);
                        ui.close();
                    }
                    if free.is_empty() {
                        ui.label(egui::RichText::new("No free channel: add a Mixer or Console").size(10.5).color(theme::text()_FAINT));
                    }
                    for (port, label) in free {
                        if canvas::menu_item(ui, label, "") {
                            picked = Some(Some(*port));
                            ui.close();
                        }
                    }
                    picked
                })
                .and_then(|r| r.inner)
            {
                acts.push(Act::Route(t.key, dest));
            }
        } else {
            p.text(dest_pos, Align2::LEFT_CENTER, &h.dest, theme::font(10.5), theme::text()_DIM);
        }
        if let Some(from) = &h.from {
            p.text(pos2(name_x, top + 53.0), Align2::LEFT_CENTER, from, theme::font(9.5), theme::text()_FAINT);
        }

        // Its channel: mute, solo, fader, meter.
        let strip_y = top + 74.0;
        match &h.strip {
            Strip::Channel { params, ch, meter } => {
                let ms = Rect::from_min_size(pos2(name_x, strip_y - 7.0), vec2(18.0, 14.0));
                mute_solo(&hui, &p, params, *ch, ms, t.key);
                let fr = Rect::from_min_max(pos2(name_x + 52.0, strip_y - 6.0), pos2(hr.right() - 14.0, strip_y + 2.0));
                let mut pos = params.fader[*ch].get();
                widgets::hfader(&hui, &p, Id::unique(("trk-fader", t.key)), fr, &mut pos, h.color, true, 1.0);
                params.fader[*ch].set(pos);
                widgets::hmeter(&p, Rect::from_min_max(pos2(fr.left(), strip_y + 7.0), pos2(fr.right(), strip_y + 10.0)), meter, 1.0);
            }
            Strip::Master { params, meters } => {
                let mr = Rect::from_min_size(pos2(name_x, strip_y - 7.0), vec2(18.0, 14.0));
                let muted = params.master_mute.load(Ordering::Relaxed);
                if widgets::toggle(&hui, &p, Id::unique(("trk-mm", t.key)), mr, "M", muted, theme::mute(), 1.0).clicked() {
                    params.master_mute.store(!muted, Ordering::Relaxed);
                }
                let fr = Rect::from_min_max(pos2(name_x + 52.0, strip_y - 6.0), pos2(hr.right() - 14.0, strip_y + 2.0));
                let mut pos = params.master.get();
                widgets::hfader(&hui, &p, Id::unique(("trk-master", t.key)), fr, &mut pos, h.color, true, 1.0);
                params.master.set(pos);
                for (i, m) in meters.iter().enumerate() {
                    let y = strip_y + 6.0 + 4.0 * i as f32;
                    widgets::hmeter(&p, Rect::from_min_max(pos2(fr.left(), y), pos2(fr.right(), y + 2.5)), m, 1.0);
                }
            }
            Strip::Level(level) => {
                let r = Rect::from_min_max(pos2(name_x, strip_y - 1.5), pos2(hr.right() - 14.0, strip_y + 1.5));
                p.rect_filled(r, 1.5, theme::control());
                let w = r.width() * widgets::meter_norm(*level);
                p.rect_filled(Rect::from_min_size(r.min, vec2(w, r.height())), 1.5, alpha(h.color, 0.8));
            }
            Strip::None => {}
        }
        let _ = graph;
    }

    /// The last row: a new track, from a click or a drop.
    #[allow(clippy::too_many_arguments)]
    fn new_track_row(
        &mut self,
        ui: &mut Ui,
        view: Rect,
        y: f32,
        empty: bool,
        library: &[Preset],
        plugins: &[PluginEntry],
        acts: &mut Vec<Act>,
    ) -> Rect {
        let r = Rect::from_min_size(
            pos2(view.left() + HEADER_W + LANE_PAD, y + 12.0),
            vec2((view.width() - HEADER_W - 2.0 * LANE_PAD).clamp(120.0, 440.0), NEW_ROW_H - 24.0),
        );
        let mut nui = ui.new_child(UiBuilder::new().max_rect(view));
        nui.set_clip_rect(view);
        let resp = nui
            .interact(r, Id::unique("trk-new"), Sense::CLICK)
            .on_hover_cursor(CursorIcon::PointingHand);
        let col = if resp.hovered() { theme::text()_DIM } else { theme::text()_FAINT };
        let p = nui.painter();
        dashed_rect(p, r, Stroke::new(1.0, col));
        let text = if empty {
            "+   Add a track: click to pick a source, or drag one from the palette"
        } else {
            "+   New track: click to pick a source, or drop one here"
        };
        p.text(r.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, text, theme::font(11.0), col);
        if let Some(choice) = egui::Popup::menu(&resp)
            .show(|ui| canvas::add_menu(ui, library, plugins))
            .and_then(|r| r.inner)
        {
            acts.push(Act::Add(AddAt::NewTrack, choice));
        }
        r
    }

    /// Devices dragged to another place, and palette entries dropped.
    #[allow(clippy::too_many_arguments)]
    fn drops(
        &mut self,
        ui: &Ui,
        graph: &Graph,
        map: &TrackMap,
        devs: &HashMap<NodeId, Dev>,
        view: Rect,
        geos: &[RowGeo],
        new_row: Rect,
        pointer: Option<Pos2>,
        acts: &mut Vec<Act>,
    ) {
        let payload = egui::DragAndDrop::payload::<Spawn>(ui.ctx()).map(|s| *s);
        let released = ui.input(|i| i.pointer.any_released());
        // A drag whose release went elsewhere (another window) ends here.
        if !released && !ui.input(|i| i.pointer.primary_down()) {
            self.drag = None;
        }
        if self.drag.is_none() && payload.is_none() {
            return;
        }
        let painter = ui.painter_at(view);
        let Some(p) = pointer.filter(|p| view.contains(*p)) else {
            if released {
                self.drag = None;
            }
            return;
        };
        // What it is: an effect (into a gap), a MIDI node (before an
        // instrument), or anything else (a track of its own).
        enum Kind {
            Gap,
            Midi,
            Track,
        }
        let kind = match payload {
            None => Kind::Gap,
            Some(Spawn::Node(ty)) if tracks::inline_type(ty) => Kind::Gap,
            Some(Spawn::Node(ty)) if tracks::midi_type(ty) => Kind::Midi,
            Some(Spawn::Plugin(_)) => Kind::Gap,
            Some(_) => Kind::Track,
        };
        let row = geos.iter().find(|g| (g.top..g.bottom).contains(&p.y) && p.x > view.left() + HEADER_W);
        let target = match (&kind, row) {
            (Kind::Gap, Some(g)) => g
                .gaps
                .iter()
                .min_by(|a, b| (a.1 - p.x).abs().total_cmp(&(b.1 - p.x).abs()))
                .map(|&(gap, x)| (AddAt::Gap(g.key, gap), x, g)),
            (Kind::Midi, Some(g)) => {
                let instrument = map
                    .find(g.key)
                    .and_then(|t| t.head())
                    .and_then(|h| graph.node(h))
                    .is_some_and(|n| n.midi_in());
                g.midi_x
                    .filter(|_| instrument)
                    .map(|x| (AddAt::Gap(g.key, 0), x, g))
            }
            _ => None,
        };
        // A device only moves to gaps; anything else can go on a track of its own.
        let to_new = target.is_none() && payload.is_some();
        match target {
            Some((_, x, g)) => {
                let col = theme::accent();
                painter.vline(x, (g.top + 6.0)..=(g.bottom - 6.0), Stroke::new(2.0, col));
                for (y, d) in [(g.top + 6.0, 1.0), (g.bottom - 6.0, -1.0)] {
                    painter.add(Shape::convex_polygon(
                        vec![pos2(x - 4.0, y - 2.0 * d), pos2(x + 4.0, y - 2.0 * d), pos2(x, y + 3.0 * d)],
                        col,
                        Stroke::NONE,
                    ));
                }
            }
            None if to_new => {
                painter.rect(new_row, 4.0, alpha(theme::accent(), 0.06), Stroke::new(1.0, alpha(theme::accent(), 0.7)), StrokeKind::Inside);
            }
            None => {}
        }
        // The device under the pointer while it moves.
        if let Some(id) = self.drag
            && let Some(d) = devs.get(&id)
        {
            let r = chip(&painter, p + vec2(10.0, 0.0), &d.title, d.accent);
            painter.rect_stroke(r, 4.0, Stroke::new(1.0, d.accent), StrokeKind::Outside);
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
        if released {
            if let Some(spawn) = payload {
                egui::DragAndDrop::clear_payload(ui.ctx());
                let at = target.map_or(AddAt::NewTrack, |(at, _, _)| at);
                acts.push(Act::Add(at, spawn_choice(spawn)));
            }
            if let Some(id) = self.drag.take()
                && let Some((AddAt::Gap(key, gap), _, _)) = target
            {
                acts.push(Act::Move(id, key, gap));
            }
        }
    }

    /// A track dragged up or down by its name.
    fn row_drag(&mut self, ui: &Ui, view: Rect, geos: &[RowGeo], pointer: Option<Pos2>, acts: &mut Vec<Act>) {
        let Some(from) = self.row_drag else {
            return;
        };
        let rows: Vec<&RowGeo> = geos.iter().filter(|g| g.index.is_some()).collect();
        let at = pointer.map_or(from, |p| rows.iter().filter(|g| p.y > (g.top + g.bottom) * 0.5).count());
        let y = match rows.get(at) {
            Some(g) => g.top,
            None => rows.last().map_or(view.top(), |g| g.bottom),
        };
        ui.painter_at(view).hline(view.x_range(), y, Stroke::new(2.0, theme::accent()));
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        if !ui.input(|i| i.pointer.primary_down()) {
            self.row_drag = None;
            let to = if at > from { at - 1 } else { at };
            if to != from {
                acts.push(Act::Reorder(from, to));
            }
        }
    }

    fn keyboard(&mut self, ui: &Ui, devs: &HashMap<NodeId, Dev>, acts: &mut Vec<Act>) {
        let pressed = |k: egui::Key| ui.input(|i| i.key_pressed(k) && !i.modifiers.command);
        if let Some(id) = self.selected {
            if pressed(egui::Key::Delete) || pressed(egui::Key::Backspace) {
                acts.push(Act::Remove(id));
            }
            if pressed(egui::Key::C) {
                acts.push(Act::ToggleOpen(id));
            }
        }
        if pressed(egui::Key::B)
            && let Some(id) = self.hovered.or(self.selected)
            && devs.get(&id).is_some_and(|d| d.bypass.is_some())
        {
            acts.push(Act::Bypass(id));
        }
        if pressed(egui::Key::Escape) {
            acts.push(Act::Select(None));
        }
    }

    /// Adds a palette entry clicked (not dragged): after the selected
    /// device when that's where it fits, else on a track of its own.
    pub fn add_spawn(&mut self, spawn: Spawn, graph: &mut Graph, audio: &AudioHandle, library: &[Preset], plugins: &[PluginEntry]) {
        let map = TrackMap::build(graph);
        let at = self
            .selected
            .and_then(|id| {
                let (r, _) = map.locate(id)?;
                let t = &map.rows[r];
                let i = t.chain_nodes().iter().position(|n| *n == id)?;
                t.is_track().then_some(AddAt::Gap(t.key, i + 1))
            })
            .unwrap_or(AddAt::NewTrack);
        self.add(graph, audio, at, spawn_choice(spawn), library, plugins);
    }

    fn add(&mut self, graph: &mut Graph, audio: &AudioHandle, at: AddAt, choice: AddChoice, library: &[Preset], plugins: &[PluginEntry]) {
        match choice {
            AddChoice::Node(ty) => {
                let placed = match at {
                    AddAt::Gap(key, gap) if tracks::inline_type(ty) => tracks::insert_new(graph, key, gap, ty, audio),
                    AddAt::Gap(key, _) if tracks::midi_type(ty) => TrackMap::build(graph)
                        .find(key)
                        .and_then(|t| t.head())
                        .and_then(|head| tracks::insert_midi_new(graph, head, ty, audio)),
                    _ => None,
                };
                let id = placed.unwrap_or_else(|| tracks::new_track(graph, ty, audio));
                self.selected = Some(id);
            }
            AddChoice::Plugin(i) => {
                let Some(entry) = plugins.get(i) else {
                    return;
                };
                match graph.add_plugin(&entry.path, tracks::below_content(graph), audio) {
                    Ok(id) => {
                        let inserted = matches!(at, AddAt::Gap(key, gap) if tracks::insert_placed(graph, key, gap, id, audio));
                        if !inserted {
                            tracks::route_new(graph, id, audio);
                        }
                        self.selected = Some(id);
                    }
                    Err(e) => self.notices.push(format!("Could not load {}: {e}", entry.name)),
                }
            }
            AddChoice::Preset(i) => {
                if let Some(preset) = library.get(i) {
                    let at = tracks::below_content(graph);
                    project::insert_preset(graph, preset, at, audio);
                }
            }
            AddChoice::Request(r) => self.requests.push(r),
        }
    }

    fn apply(&mut self, acts: Vec<Act>, graph: &mut Graph, audio: &AudioHandle, library: &[Preset], plugins: &[PluginEntry]) {
        for act in acts {
            match act {
                Act::Select(id) => self.selected = id,
                Act::ToggleOpen(id) => {
                    let open = graph.tracks.is_open(id);
                    graph.tracks.set_open(id, !open);
                }
                Act::Fold(key, folded) => graph.tracks.set_folded(key, folded),
                Act::Bypass(id) => {
                    if let Some(n) = graph.node(id) {
                        n.set_bypass(!n.bypassed());
                    }
                }
                Act::Remove(id) => {
                    if canvas::confirm_removal(graph, &[id]) {
                        tracks::remove(graph, id, audio);
                        if self.selected == Some(id) {
                            self.selected = None;
                        }
                    }
                }
                Act::Move(id, key, gap) => {
                    if !tracks::move_device(graph, id, key, gap, audio) {
                        let name = graph.node(id).map_or(String::new(), |n| n.title.clone());
                        self.notices.push(format!("{name} can't go there: it would feed itself"));
                    }
                }
                Act::Reorder(from, to) => tracks::reorder(graph, from, to),
                Act::Route(key, dest) => {
                    if !tracks::route(graph, key, dest, audio) {
                        self.notices.push("That channel is taken".to_owned());
                    }
                }
                Act::Add(at, choice) => self.add(graph, audio, at, choice, library, plugins),
                Act::SetConsole(id, shape) => {
                    let lost = graph.console_losses(id, shape);
                    if lost == 0 || canvas::confirm_unplug(lost) {
                        graph.set_console(id, shape, audio);
                    }
                }
                Act::ToggleEditor(id) => {
                    if let Some(NodeKind::Plugin(p)) = graph.node_mut(id).map(|n| &mut n.kind) {
                        #[cfg(target_os = "macos")]
                        let before = super::macos::windows();
                        match p.toggle_editor() {
                            #[cfg(target_os = "macos")]
                            Ok(()) => super::macos::float_new_windows(&before),
                            #[cfg(not(target_os = "macos"))]
                            Ok(()) => {}
                            Err(e) => self.notices.push(format!("Could not open the editor: {e}")),
                        }
                    }
                }
            }
        }
    }
}

fn spawn_choice(spawn: Spawn) -> AddChoice {
    match spawn {
        Spawn::Node(ty) => AddChoice::Node(ty),
        Spawn::Plugin(i) => AddChoice::Plugin(i),
        Spawn::Preset(i) => AddChoice::Preset(i),
    }
}

/// Iris runs its scene changes and its window even when not drawn (the
/// canvas does the same).
fn tick_iris(ui: &Ui, graph: &Graph) {
    for node in &graph.nodes {
        if let NodeKind::Module(m) = &node.kind
            && m.params.kind == ModuleKind::Iris
        {
            super::iris::tick(ui.ctx(), node.id, &node.title, node.accent, &m.params, graph.tempo);
        }
    }
}

/// A node's panel size, open (whatever it is on the canvas).
fn full_size(n: &Node) -> Vec2 {
    match &n.kind {
        NodeKind::Module(m) => modules::size_of(&m.params),
        NodeKind::Mixer(m) => match m.console {
            Some(c) => super::console::size(c),
            None => nodes::type_size(n.kind.node_type()),
        },
        _ => nodes::type_size(n.kind.node_type()),
    }
}

fn dev_info(g: &Graph, map: &TrackMap) -> HashMap<NodeId, Dev> {
    let mut devs = HashMap::new();
    for n in &g.nodes {
        let sources = (0..n.inputs().len())
            .map(|port| {
                let c = g.cables.iter().find(|c| c.to == PortRef { node: n.id, port })?;
                let src = g.node(c.from.node)?;
                Some((src.accent, src.title.clone()))
            })
            .collect();
        let out = |port| g.cables.iter().any(|c| c.from == PortRef { node: n.id, port });
        let midi_in = g.cables.iter().any(|c| c.to == PortRef { node: n.id, port: MIDI_PORT });
        devs.insert(
            n.id,
            Dev {
                title: n.title.clone(),
                accent: n.accent,
                level: n.signal_level(),
                bypass: n.is_effect().then(|| n.bypassed()),
                summary: nodes::summary(n, g.tempo),
                tags: map.tags(g, n.id),
                full: full_size(n),
                inline: tracks::inline(n),
                sources,
                outs: (0..n.outputs().len()).map(out).collect(),
                midi: (midi_in, out(MIDI_PORT)),
                target: g.target_of(n.id).and_then(|t| g.target_label(t)),
            },
        );
    }
    devs
}

/// "MIXER 1 · 3".
fn channel_label(g: &Graph, p: PortRef) -> String {
    let name = g.node(p.node).map_or("?", |n| n.title.as_str());
    format!("{name} · {}", p.port + 1)
}

/// "FILTER 1", or "FJORD 1 · CH 2" for an output other than the first.
fn port_name(n: &Node, port: usize) -> String {
    if port == 0 || is_midi(port) {
        n.title.clone()
    } else {
        format!("{} · {}", n.title, n.port_label(port, true))
    }
}

fn head_info(g: &Graph, t: &Track) -> Head {
    let head = t.head().and_then(|h| g.node(h));
    let input = match t.input {
        Input::Branch(p) | Input::Bus(p) => Some(p),
        _ => None,
    };
    let input_node = input.and_then(|p| g.node(p.node));
    let from = input.zip(input_node).map(|(p, n)| format!("from {}", port_name(n, p.port)));
    let kind = match t.input {
        Input::Source if head.is_some_and(|n| n.midi_in()) => "INSTRUMENT",
        Input::Source => "AUDIO",
        Input::Empty => "EFFECTS",
        Input::Branch(_) => "BRANCH",
        Input::Bus(p) if p.port == 1 => "RETURN",
        Input::Bus(_) => "BUS",
        Input::Master => "MASTER",
        Input::Loose => "",
    };
    let (name, color) = match (t.input, head, input_node) {
        (Input::Loose, ..) => ("NOT IN A TRACK".to_owned(), theme::text()_FAINT),
        (_, Some(n), _) => (n.title.clone(), n.accent),
        (_, None, Some(n)) => (from.clone().unwrap_or_default(), n.accent),
        _ => (String::new(), theme::text()_FAINT),
    };
    let dest = match t.input {
        Input::Master => {
            let out = t.head().and_then(|m| g.cables.iter().find(|c| c.from == PortRef { node: m, port: 0 }));
            match out.and_then(|c| g.node(c.to.node)) {
                Some(n) => format!("▸ {}", n.title),
                None => "▸ audio out".to_owned(),
            }
        }
        Input::Loose => "MIDI and modulation patched to no track".to_owned(),
        _ => match t.dest {
            Some(p) => format!("▸ {}", channel_label(g, p)),
            None => "▸ not routed".to_owned(),
        },
    };
    let strip = match (t.input, t.dest) {
        (Input::Master, _) => match head.map(|n| &n.kind) {
            Some(NodeKind::Mixer(m)) => Strip::Master {
                params: m.params.clone(),
                meters: m.master,
            },
            _ => Strip::None,
        },
        (_, Some(p)) => match g.node(p.node).map(|n| &n.kind) {
            Some(NodeKind::Mixer(m)) if p.port < m.channels.len() => Strip::Channel {
                params: m.params.clone(),
                ch: p.port,
                meter: m.channels[p.port],
            },
            _ => Strip::None,
        },
        (Input::Loose, _) => Strip::None,
        _ => match t.chain().last().and_then(|s| g.node(s.node)) {
            Some(n) => Strip::Level(n.signal_level()),
            None => Strip::None,
        },
    };
    Head {
        name,
        kind,
        color,
        from: from.filter(|_| head.is_some()),
        dest,
        strip,
    }
}

fn row_height(g: &Graph, t: &Track, devs: &HashMap<NodeId, Dev>) -> f32 {
    if g.tracks.is_folded(t.key) {
        return FOLDED_H;
    }
    let tallest = t
        .slots
        .iter()
        .filter(|s| g.tracks.is_open(s.node))
        .filter_map(|s| devs.get(&s.node))
        .map(|d| d.full.y.min(PANEL_H))
        .fold(BOX_H, f32::max);
    tallest + 2.0 * PAD
}

/// A closed device's body: its main value and its tags.
fn compact(v: &NodeView, s: &tracks::Slot, d: &Dev, size: Vec2) {
    let clip = v.painter.with_clip_rect(v.r(4.0, nodes::HEADER_H, size.x - 4.0, size.y - 2.0).intersect(v.painter.clip_rect()));
    let x = 10.0;
    let mut lines: Vec<(String, Color32)> = Vec::new();
    if s.serves.is_none() {
        lines.push((d.summary.clone(), theme::text()));
    }
    for t in &d.tags {
        let col = if t.midi {
            MIDI_COL
        } else if t.incoming {
            theme::text()_DIM
        } else {
            theme::aux()
        };
        lines.push((t.text.clone(), col));
    }
    let max = 3;
    let more = lines.len().saturating_sub(max);
    for (i, (text, col)) in lines.iter().take(max).enumerate() {
        let y = nodes::HEADER_H + 13.0 + 14.0 * i as f32;
        let font = if i == 0 && s.serves.is_none() { theme::mono(10.5) } else { theme::font(9.0) };
        let text = if i + 1 == max && more > 0 { format!("{text}  +{more}") } else { text.clone() };
        clip.text(v.p(x, y), Align2::LEFT_CENTER, text, font, *col);
    }
}

/// An open device's panel, as on the canvas.
fn panel(v: &NodeView, node: &mut Node, d: &Dev, now: f64, notices: &mut Vec<String>) -> Option<Act> {
    let id = node.id;
    let accent = node.accent;
    let connected: Vec<bool> = d.sources.iter().map(Option::is_some).collect();
    let out = d.outs.first().copied().unwrap_or(false);
    match &mut node.kind {
        NodeKind::Osc(o) => nodes::osc(v, id, accent, o, &connected, out),
        NodeKind::Mixer(m) => {
            let outs = (out, d.outs.get(1).copied().unwrap_or(false));
            if m.console.is_some() {
                if let Some(shape) = super::console::draw(v, id, accent, m, &d.sources, outs) {
                    return Some(Act::SetConsole(id, shape));
                }
            } else {
                nodes::mixer(v, id, accent, m, &d.sources, outs);
            }
        }
        NodeKind::Plugin(p) => {
            let input = connected.first().copied().unwrap_or(false);
            let events = plugin_node::draw(v, id, accent, p, (input, d.midi.0), out, now);
            if let Some(e) = p.editor_error.take() {
                notices.push(format!("Could not open the editor: {e}"));
            }
            if events.toggle_editor {
                return Some(Act::ToggleEditor(id));
            }
        }
        NodeKind::Module(m) => {
            let map = modules::MapState {
                target: d.target.as_deref(),
                picking: false,
            };
            let button = modules::draw(v, id, accent, m, (&connected, &d.outs), d.midi, map);
            if button.clicked {
                notices.push("MIDI mapping is done in the node view (V)".to_owned());
            }
        }
    }
    None
}

fn mute_solo(ui: &Ui, p: &Painter, params: &MixerParams, ch: usize, r: Rect, key: TrackKey) {
    let muted = params.mute[ch].load(Ordering::Relaxed);
    if widgets::toggle(ui, p, Id::unique(("trk-m", key)), r, "M", muted, theme::mute(), 1.0)
        .on_hover_text("Mute")
        .clicked()
    {
        params.mute[ch].store(!muted, Ordering::Relaxed);
    }
    let solo = params.solo[ch].load(Ordering::Relaxed);
    let sr = r.translate(vec2(r.width() + 4.0, 0.0));
    if widgets::toggle(ui, p, Id::unique(("trk-s", key)), sr, "S", solo, theme::solo(), 1.0)
        .on_hover_text("Solo")
        .clicked()
    {
        params.solo[ch].store(!solo, Ordering::Relaxed);
    }
}

fn theme_text_w(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), theme::strong(size), theme::text())
        .size()
        .x
}

/// A rounded label in a colour's outline, its left edge centred on `at`.
fn chip(p: &Painter, at: Pos2, text: &str, col: Color32) -> Rect {
    let galley = p.layout_no_wrap(text.to_owned(), theme::font(10.0), col);
    let r = Rect::from_min_size(at - vec2(0.0, 10.0), vec2(galley.size().x + 16.0, 20.0));
    p.rect(r, 10.0, theme::node_body(), Stroke::new(1.0, alpha(col, 0.8)), StrokeKind::Inside);
    p.galley(r.center() - galley.size() * 0.5, galley, col);
    r
}

fn arrow(p: &Painter, a: Pos2, b: Pos2, col: Color32, dashed: bool) {
    if b.x - a.x < 6.0 {
        return;
    }
    let stroke = Stroke::new(1.4, col);
    let tip = b - vec2(5.0, 0.0);
    if dashed {
        p.extend(Shape::dashed_line(&[a, tip], stroke, 3.0, 3.0));
    } else {
        p.line_segment([a, tip], stroke);
    }
    p.add(Shape::convex_polygon(
        vec![b, b + vec2(-6.0, -3.5), b + vec2(-6.0, 3.5)],
        col,
        Stroke::NONE,
    ));
}

/// The audio from `from_x` to the box at `to_x`, round under what lies
/// between (attached boxes) at `low`, coming in at `entry`.
fn elbow(p: &Painter, from_x: f32, to_x: f32, y: f32, low: f32, entry: f32, col: Color32) {
    let (a, b) = (from_x + GAP * 0.5, to_x - GAP * 0.5);
    let pts = vec![pos2(from_x + 3.0, y), pos2(a, y), pos2(a, low), pos2(b, low), pos2(b, entry)];
    p.add(Shape::line(pts, Stroke::new(1.4, col)));
    arrow(p, pos2(b, entry), pos2(to_x - 3.0, entry), col, false);
}

fn dashed_rect(p: &Painter, r: Rect, stroke: Stroke) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.extend(Shape::dashed_line(&pts, stroke, 5.0, 4.0));
}

fn plus_icon(p: &Painter, c: Pos2, col: Color32) {
    let s = Stroke::new(1.4, col);
    p.line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], s);
    p.line_segment([c - vec2(0.0, 5.0), c + vec2(0.0, 5.0)], s);
}
