//! The SONG dock, under the board in either view: the scenes on the left
//! (pads: click to recall, + NEW keeps the board as a new scene, SAVE keeps
//! it in the scene on the board), the song in the middle (a block per step,
//! as wide as its bars, in its scene's colour, with its key, MORPH and what
//! follows; the playhead runs through the step playing and the step coming
//! blinks) and the selected step's settings on the right. Drawn flat: solid
//! blocks and outlines, no shading.

use egui::{
    Align2, Color32, CursorIcon, Id, Painter, Pos2, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, pos2, vec2,
};

use super::canvas;
use super::theme::{self, alpha, mix};
use crate::audio::engine::NodeId;
use crate::graph::Graph;
use crate::song::{
    self, BEATS_PER_BAR, COLORS, MAX_BARS, MAX_STEPS, MORPHS, Quant, Song, Step, Then,
};

/// The dock's height.
pub const HEIGHT: f32 = 192.0;
const HEADER_H: f32 = 22.0;
pub const SCENES_W: f32 = 252.0;
const INSPECTOR_W: f32 = 236.0;
const PAD: egui::Vec2 = vec2(56.0, 34.0);
const GAP: f32 = 6.0;
const RULER_H: f32 = 14.0;

/// The scenes' colours: flat, like the nodes' accents.
pub const SCENE_COLORS: [Color32; COLORS as usize] = [
    Color32::from_rgb(96, 165, 238),
    Color32::from_rgb(226, 112, 150),
    Color32::from_rgb(132, 194, 108),
    Color32::from_rgb(158, 138, 230),
    Color32::from_rgb(228, 142, 88),
    Color32::from_rgb(86, 188, 172),
    Color32::from_rgb(214, 166, 92),
    Color32::from_rgb(170, 176, 192),
];

pub fn scene_color(c: u8) -> Color32 {
    SCENE_COLORS[c as usize % SCENE_COLORS.len()]
}

/// What the panel asks the app to do (the rest it edits in the song).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    /// Recall a scene by hand.
    Recall(u32),
    /// Keep the board as a new scene (and a step that plays it).
    NewScene,
    /// Keep the board in a scene.
    Store(u32),
    /// Play this step next.
    Queue(usize),
}

/// What is playing, for the panel.
pub struct Info {
    pub running: bool,
    pub beat: f64,
    pub now: f64,
    pub current: Option<usize>,
    pub started: f64,
    pub coming: Option<usize>,
    pub coming_scene: Option<u32>,
    pub loaded: Option<u32>,
    /// The board differs from the scene on it.
    pub edited: bool,
    pub tempo: f32,
}

#[derive(Default)]
pub struct SongPanel {
    /// The step whose settings are shown.
    pub selected: Option<usize>,
    renaming: Option<(u32, String)>,
    /// A step being dragged to another place, or a scene onto the song.
    drag_step: Option<usize>,
    drag_scene: Option<u32>,
    pub acts: Vec<Act>,
}

impl SongPanel {
    /// Draws the dock; returns whether the song was edited.
    pub fn show(&mut self, ui: &mut Ui, song: &mut Song, graph: &Graph, info: &Info) -> bool {
        let full = ui.max_rect();
        let before = song.clone();
        if self.selected.is_some_and(|s| s >= song.steps.len()) {
            self.selected = song.steps.len().checked_sub(1);
        }
        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
        let body = Rect::from_min_max(pos2(full.left(), header.bottom() + 6.0), full.max);
        let scenes = Rect::from_min_size(body.min, vec2(SCENES_W, body.height()));
        let inspector = Rect::from_min_max(pos2(body.right() - INSPECTOR_W, body.top()), body.max);
        let timeline = Rect::from_min_max(
            pos2(scenes.right() + 12.0, body.top()),
            pos2(inspector.left() - 12.0, body.bottom()),
        );
        let p = ui.painter().clone();
        for x in [scenes.right() + 6.0, inspector.left() - 6.0] {
            p.vline(x, body.y_range(), Stroke::new(1.0, theme::panel()_BORDER));
        }
        self.header(ui, header, song, graph, info);
        self.scenes(ui, scenes, song, info);
        self.timeline(ui, timeline, song, info);
        self.inspector(ui, inspector, song);
        // A drag let go anywhere else ends.
        if ui.input(|i| i.pointer.any_released()) {
            self.drag_step = None;
            self.drag_scene = None;
        }
        *song != before
    }

    fn header(&mut self, ui: &mut Ui, r: Rect, song: &mut Song, graph: &Graph, info: &Info) {
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("SONG").font(theme::strong(11.0)).color(theme::text()_DIM));
                if pill(ui, "ON", song.on, theme::accent())
                    .on_hover_text("The song plays with the transport: Start plays it from the start step")
                    .clicked()
                {
                    song.on = !song.on;
                }
                if pill(ui, "HOLD", song.hold, theme::mixer_accent())
                    .on_hover_text("Live: every step repeats until you pick the next (click it twice, 1–9, ← →)")
                    .clicked()
                {
                    song.hold = !song.hold;
                }
                ui.add_space(4.0);
                caption(ui, "QUANT");
                let resp = menu_pill(ui, song.quant.name())
                    .on_hover_text("When a step you pick while playing comes in");
                egui::Popup::menu(&resp).show(|ui| {
                    for q in Quant::ALL {
                        if canvas::menu_item(ui, q.name(), if q == song.quant { "✓" } else { "" }) {
                            song.quant = q;
                            ui.close();
                        }
                    }
                });
                caption(ui, "KEY");
                let resp = menu_pill(ui, song::key_name(song.home))
                    .on_hover_text("The song's home key: steps in another key are transposed from it");
                egui::Popup::menu(&resp).show(|ui| {
                    for k in 0..12u8 {
                        let mark = if k == song.home { "✓" } else { "" };
                        if canvas::menu_item(ui, song::key_name(k), mark) {
                            song.home = k;
                            ui.close();
                        }
                    }
                });
                caption(ui, "MIDI");
                let ch = if song.channel == 0 {
                    "OFF".to_owned()
                } else {
                    format!("CH {}", song.channel)
                };
                let resp = menu_pill(ui, &ch).on_hover_text(
                    "A MIDI channel whose notes pick steps (C3 step 1, C#3 step 2…; B2 next, A#2 back) and whose program changes pick step 1, 2…",
                );
                egui::Popup::menu(&resp).show(|ui| {
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        for c in 0..=16u8 {
                            let label = if c == 0 { "Off".to_owned() } else { format!("Channel {c}") };
                            let mark = if c == song.channel { "✓" } else { "" };
                            if canvas::menu_item(ui, &label, mark) {
                                song.channel = c;
                                ui.close();
                            }
                        }
                    });
                });
                let resp = menu_pill(ui, "NODES").on_hover_text(
                    "Which nodes the scenes set, and which follow the key",
                );
                egui::Popup::menu(&resp).show(|ui| nodes_menu(ui, song, graph));

                // What plays, on the right.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let bars: u32 = song.steps.iter().map(|s| s.bars).sum();
                    let secs = song.beats() * 60.0 / info.tempo.max(1.0) as f64;
                    if !song.steps.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "{} step{} · {bars} bars · {}:{:02}",
                                song.steps.len(),
                                if song.steps.len() == 1 { "" } else { "s" },
                                secs as u32 / 60,
                                secs as u32 % 60
                            ))
                            .size(10.5)
                            .color(theme::text()_FAINT),
                        );
                        ui.add_space(10.0);
                    }
                    if let Some(text) = status(song, info) {
                        ui.label(RichText::new(text).font(theme::strong(11.0)).color(theme::text()));
                    }
                });
            });
        });
    }

    fn scenes(&mut self, ui: &mut Ui, r: Rect, song: &mut Song, info: &Info) {
        let p = ui.painter_at(r.expand(2.0));
        p.text(r.left_top() + vec2(0.0, 8.0), Align2::LEFT_CENTER, "SCENES", theme::font(9.5), theme::text()_FAINT);
        // + NEW and SAVE on the right of the heading.
        let new_r = Rect::from_min_size(pos2(r.right() - 50.0, r.top()), vec2(50.0, 17.0));
        let save_r = new_r.translate(vec2(-56.0, 0.0));
        if flat_button(ui, &p, Id::unique("song-new"), new_r, "+ NEW", true, theme::accent())
            .on_hover_text("Keep the board as it is as a new scene, played by a new step at the end of the song")
            .clicked()
        {
            self.acts.push(Act::NewScene);
        }
        let can_save = info.loaded.is_some();
        let save = flat_button(ui, &p, Id::unique("song-save"), save_r, "SAVE", can_save && info.edited, theme::mixer_accent());
        let tip = match info.loaded.and_then(|id| song.scene(id)) {
            Some(s) if info.edited => format!("Keep the board's changes in scene {}", s.name),
            Some(s) => format!("Scene {} is on the board, unchanged", s.name),
            None => "Recall a scene, change the board, then SAVE keeps the changes in it".to_owned(),
        };
        if save.on_hover_text(tip).clicked()
            && let Some(id) = info.loaded
        {
            self.acts.push(Act::Store(id));
        }

        let grid = Rect::from_min_max(pos2(r.left(), r.top() + 24.0), r.max);
        if song.scenes.is_empty() {
            p.text(
                grid.left_top() + vec2(0.0, 4.0),
                Align2::LEFT_TOP,
                "Set the board up, then + NEW\nkeeps it as scene A.",
                theme::font(10.5),
                theme::text()_FAINT,
            );
            return;
        }
        let cols = (((grid.width() + GAP) / (PAD.x + GAP)).floor() as usize).max(1);
        let rows = song.scenes.len().div_ceil(cols);
        let content_h = rows as f32 * (PAD.y + GAP);
        ui.scope_builder(egui::UiBuilder::new().max_rect(grid), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("song-scenes")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (area, _) = ui.allocate_exact_size(vec2(grid.width(), content_h), Sense::hover());
                    let p = ui.painter().clone();
                    let mut remove = None;
                    let mut duplicate = None;
                    let mut add = None;
                    for (i, scene) in song.scenes.iter_mut().enumerate() {
                        let at = area.min + vec2((i % cols) as f32 * (PAD.x + GAP), (i / cols) as f32 * (PAD.y + GAP));
                        let pad = Rect::from_min_size(at, PAD);
                        let id = Id::unique(("song-scene", scene.id));
                        let resp = ui.interact(pad, id, Sense::click_and_drag());
                        let loaded = info.loaded == Some(scene.id);
                        let coming = info.coming_scene == Some(scene.id);
                        let col = scene_color(scene.color);
                        let blink = coming && (info.now * 4.0) as i64 % 2 == 0;
                        let fill = if loaded { col } else if resp.hovered() { mix(theme::node_body(), col, 0.4) } else { mix(theme::node_body(), col, 0.22) };
                        p.rect(pad, 4.0, fill, Stroke::new(if blink { 2.0 } else { 1.0 }, if blink { theme::text() } else { alpha(col, 0.85) }), StrokeKind::Inside);
                        let text_col = if loaded { theme::inset() } else { theme::text() };
                        if self.renaming.as_ref().is_some_and(|(r, _)| *r == scene.id) {
                            let (_, text) = self.renaming.as_mut().unwrap();
                            let edit = ui.put(pad.shrink(4.0), egui::TextEdit::singleline(text).font(theme::strong(11.0)).char_limit(12).align(Align2::CENTER_CENTER));
                            edit.request_focus();
                            let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                            if enter || edit.lost_focus() && !esc {
                                let name = text.trim().to_owned();
                                if !name.is_empty() {
                                    scene.name = name;
                                }
                                self.renaming = None;
                            } else if esc {
                                self.renaming = None;
                            }
                        } else {
                            p.text(pad.center(), Align2::CENTER_CENTER, &scene.name, theme::strong(11.5), text_col);
                        }
                        if loaded && info.edited {
                            p.circle_filled(pad.right_top() + vec2(-7.0, 7.0), 2.5, theme::inset());
                        }
                        let tip = format!(
                            "Scene {}: click to recall it{}; drag it onto the song to add a step; right-click for more",
                            scene.name,
                            if info.running { " on the next bar" } else { "" }
                        );
                        let resp = resp.on_hover_text(tip);
                        if resp.clicked() {
                            self.acts.push(Act::Recall(scene.id));
                        }
                        if resp.double_clicked() {
                            self.renaming = Some((scene.id, scene.name.clone()));
                        }
                        if resp.drag_started() {
                            self.drag_scene = Some(scene.id);
                        }
                        if self.drag_scene == Some(scene.id) {
                            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                        }
                        egui::Popup::context_menu(&resp).show(|ui| {
                            ui.label(RichText::new(format!("Scene {}", scene.name)).size(10.5).color(theme::accent()));
                            if canvas::menu_item(ui, "Recall", "click") {
                                self.acts.push(Act::Recall(scene.id));
                                ui.close();
                            }
                            if canvas::menu_item(ui, "Keep the board here", "") {
                                self.acts.push(Act::Store(scene.id));
                                ui.close();
                            }
                            if canvas::menu_item(ui, "Add to the song", "") {
                                add = Some(scene.id);
                                ui.close();
                            }
                            if canvas::menu_item(ui, "Rename…", "double-click") {
                                self.renaming = Some((scene.id, scene.name.clone()));
                                ui.close();
                            }
                            if canvas::menu_item(ui, "Duplicate", "") {
                                duplicate = Some(scene.id);
                                ui.close();
                            }
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for c in 0..COLORS {
                                    let (sw, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                                    let on = scene.color == c;
                                    ui.painter().rect(sw, 3.0, scene_color(c), Stroke::new(if on { 2.0 } else { 0.0 }, theme::text()), StrokeKind::Inside);
                                    if resp.clicked() {
                                        scene.color = c;
                                    }
                                }
                            });
                            if canvas::menu_item(ui, "Delete", "") {
                                remove = Some(scene.id);
                                ui.close();
                            }
                        });
                    }
                    if let Some(id) = remove {
                        song.remove_scene(id);
                    }
                    if let Some(id) = duplicate {
                        song.duplicate_scene(id);
                    }
                    if let Some(id) = add {
                        self.selected = song.add_step(id, None);
                    }
                });
        });
    }

    fn timeline(&mut self, ui: &mut Ui, r: Rect, song: &mut Song, info: &Info) {
        let p = ui.painter_at(r.expand(2.0));
        if song.steps.is_empty() {
            let text = if song.scenes.is_empty() {
                "A song is scenes played in order. Keep the board as scene A with + NEW,\nchange it and + NEW again for B: each becomes a step. Turn the song ON,\nand Start plays it, each step landing on the bar."
            } else {
                "Drag a scene here to add a step, or right-click it → Add to the song."
            };
            p.text(r.left_top() + vec2(0.0, 4.0), Align2::LEFT_TOP, text, theme::font(10.5), theme::text()_FAINT);
            self.drop_scene(ui, r, song, 0);
            return;
        }
        let total_bars: u32 = song.steps.iter().map(|s| s.bars).sum();
        let plus_w = 26.0;
        let avail = (r.width() - plus_w - 4.0).max(40.0);
        let ppb = (avail / total_bars.max(1) as f32).clamp(5.0, 22.0);
        let widths: Vec<f32> = song.steps.iter().map(|s| (s.bars as f32 * ppb).max(30.0)).collect();
        let content_w = widths.iter().sum::<f32>() + plus_w + 4.0;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("song-timeline")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (area, _) = ui.allocate_exact_size(vec2(content_w.max(r.width()), r.height() - 8.0), Sense::hover());
                    let p = ui.painter().clone();
                    let top = area.top() + RULER_H;
                    // The blocks fill the height, under the ruler, over the scroll bar.
                    let block_h = (area.height() - RULER_H - 8.0).clamp(48.0, 140.0);
                    let mut x = area.left();
                    let mut lefts = Vec::with_capacity(widths.len());
                    let mut bar_at = 0u32;
                    let mut menu: Option<(usize, StepMenu)> = None;
                    for (i, step) in song.steps.iter().enumerate() {
                        let w = widths[i];
                        lefts.push(x);
                        let block = Rect::from_min_size(pos2(x + 1.0, top), vec2(w - 2.0, block_h));
                        // The ruler: the bar each step starts on.
                        p.text(pos2(x + 2.0, area.top() + 6.0), Align2::LEFT_CENTER, format!("{}", bar_at + 1), theme::mono(8.5), theme::text()_FAINT);
                        if i == song.start.min(song.steps.len() - 1) {
                            start_flag(&p, pos2(x + 1.0, top - 1.0));
                        }
                        let id = Id::unique(("song-step", i));
                        let resp = ui.interact(block, id, Sense::click_and_drag());
                        self.draw_step(&p, block, i, step, song, info, resp.hovered(), ppb);
                        let tip = step_tip(song, i, step, info.running);
                        let resp = resp.on_hover_text(tip);
                        if resp.clicked() {
                            self.selected = Some(i);
                            if !info.running {
                                song.start = i;
                            }
                        }
                        if resp.double_clicked() && info.running {
                            self.acts.push(Act::Queue(i));
                        }
                        if resp.drag_started() {
                            self.drag_step = Some(i);
                            self.selected = Some(i);
                        }
                        egui::Popup::context_menu(&resp).show(|ui| {
                            ui.label(RichText::new(format!("Step {}", i + 1)).size(10.5).color(theme::accent()));
                            if info.running && canvas::menu_item(ui, "Play next", "double-click") {
                                menu = Some((i, StepMenu::Queue));
                                ui.close();
                            }
                            if canvas::menu_item(ui, "Start here", "") {
                                menu = Some((i, StepMenu::Start));
                                ui.close();
                            }
                            if canvas::menu_item(ui, "Duplicate", "") {
                                menu = Some((i, StepMenu::Duplicate));
                                ui.close();
                            }
                            if canvas::menu_item(ui, "Remove", "") {
                                menu = Some((i, StepMenu::Remove));
                                ui.close();
                            }
                        });
                        x += w;
                        bar_at += step.bars;
                    }
                    p.text(pos2(x + 2.0, area.top() + 6.0), Align2::LEFT_CENTER, format!("{}", bar_at + 1), theme::mono(8.5), theme::text()_FAINT);
                    // + adds a step at the end.
                    let plus = Rect::from_min_size(pos2(x + 4.0, top), vec2(plus_w - 4.0, block_h));
                    let resp = ui.interact(plus, Id::unique("song-add-step"), Sense::click());
                    let col = if resp.hovered() { theme::text() } else { theme::text()_FAINT };
                    p.rect_stroke(plus, 4.0, Stroke::new(1.0, alpha(col, 0.6)), StrokeKind::Inside);
                    p.text(plus.center(), Align2::CENTER_CENTER, "+", theme::strong(14.0), col);
                    if song.steps.len() < MAX_STEPS
                        && resp
                            .on_hover_text("Add a step: the selected step's scene, or the scene on the board")
                            .clicked()
                    {
                        let scene = self
                            .selected
                            .and_then(|s| song.steps.get(s))
                            .map(|s| s.scene)
                            .or(info.loaded)
                            .or(song.scenes.first().map(|s| s.id));
                        if let Some(scene) = scene {
                            self.selected = song.add_step(scene, Some(song.steps.len() - 1));
                        }
                    }

                    // Where a dragged step or scene would go.
                    let pointer = ui.input(|i| i.pointer.hover_pos());
                    let slot = pointer.filter(|p| area.contains(*p)).map(|pt| {
                        lefts
                            .iter()
                            .zip(&widths)
                            .position(|(l, w)| pt.x < l + w * 0.5)
                            .unwrap_or(lefts.len())
                    });
                    let dragging = self.drag_step.is_some() || self.drag_scene.is_some();
                    if dragging && let Some(slot) = slot {
                        let lx = lefts.get(slot).copied().unwrap_or(x);
                        p.vline(lx, top - 2.0..=top + block_h + 2.0, Stroke::new(2.0, theme::accent()));
                    }
                    if ui.input(|i| i.pointer.any_released()) && let Some(slot) = slot {
                        if let Some(from) = self.drag_step.take() {
                            let to = if slot > from { slot - 1 } else { slot };
                            if to != from {
                                song.move_step(from, to);
                                self.selected = Some(to);
                            }
                        }
                        if let Some(scene) = self.drag_scene.take()
                            && song.steps.len() < MAX_STEPS
                        {
                            let after = slot.checked_sub(1);
                            self.selected = song.add_step(scene, after);
                            if after.is_none() && let Some(s) = self.selected {
                                // Inserted first: it doesn't take the previous step's length.
                                song.steps[s].bars = 4;
                            }
                        }
                    }
                    if let Some((i, m)) = menu {
                        match m {
                            StepMenu::Queue => self.acts.push(Act::Queue(i)),
                            StepMenu::Start => song.start = i,
                            StepMenu::Duplicate => {
                                if song.steps.len() < MAX_STEPS {
                                    let copy = song.steps[i];
                                    song.steps.insert(i + 1, copy);
                                    self.selected = Some(i + 1);
                                }
                            }
                            StepMenu::Remove => {
                                song.remove_step(i);
                                self.selected = None;
                            }
                        }
                    }
                });
        });
    }

    /// A scene dropped on the empty song becomes its first step.
    fn drop_scene(&mut self, ui: &Ui, r: Rect, song: &mut Song, at: usize) {
        let over = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| r.contains(p));
        if over && ui.input(|i| i.pointer.any_released()) && let Some(scene) = self.drag_scene.take() {
            self.selected = song.add_step(scene, at.checked_sub(1));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_step(&self, p: &Painter, block: Rect, i: usize, step: &Step, song: &Song, info: &Info, hovered: bool, ppb: f32) {
        let scene = song.scene(step.scene);
        let col = scene.map_or(theme::text()_FAINT, |s| scene_color(s.color));
        let playing = info.current == Some(i) && info.running && song.on;
        let coming = info.coming == Some(i) && song.on;
        let fill = if playing {
            mix(theme::node_body(), col, 0.45)
        } else if hovered {
            mix(theme::node_body(), col, 0.34)
        } else {
            mix(theme::node_body(), col, 0.24)
        };
        p.rect_filled(block, 4.0, fill);
        // The part of the step already played, solid, and the playhead.
        if playing {
            let done = ((info.beat - info.started) / step.beats()).clamp(0.0, 1.0) as f32;
            let head = block.left() + block.width() * done;
            let played = Rect::from_min_max(block.min, pos2(head, block.bottom()));
            p.rect_filled(played, 4.0, alpha(col, 0.55));
            p.vline(head, block.y_range(), Stroke::new(2.0, theme::text()));
        }
        // Bar ticks along the bottom.
        if ppb >= 7.0 {
            for b in 1..step.bars {
                let x = block.left() + b as f32 * ppb;
                if x < block.right() - 2.0 {
                    let h = if b % 4 == 0 { 7.0 } else { 4.0 };
                    p.vline(x, (block.bottom() - h)..=block.bottom(), Stroke::new(1.0, alpha(theme::text(), 0.25)));
                }
            }
        }
        let selected = self.selected == Some(i);
        let blink = coming && (info.now * 4.0) as i64 % 2 == 0;
        let (w, c) = if blink {
            (2.0, theme::text())
        } else if selected {
            (1.5, theme::text())
        } else {
            (1.0, alpha(col, 0.8))
        };
        p.rect_stroke(block, 4.0, Stroke::new(w, c), StrokeKind::Inside);
        let clip = p.with_clip_rect(block.shrink(2.0));
        let name = scene.map_or("?", |s| s.name.as_str());
        clip.text(block.left_top() + vec2(6.0, 6.0), Align2::LEFT_TOP, format!("{}", i + 1), theme::mono(9.0), alpha(theme::text(), 0.7));
        clip.text(block.left_top() + vec2(6.0, 18.0), Align2::LEFT_TOP, name, theme::strong(12.5), theme::text());
        // The key: dim when it's the home key.
        let key = song::key_name(song.key_of(step));
        let key_col = if step.key.is_some_and(|k| k % 12 != song.home) { theme::text() } else { theme::text()_FAINT };
        clip.text(block.left_bottom() + vec2(6.0, -8.0), Align2::LEFT_BOTTOM, key, theme::strong(10.5), key_col);
        // MORPH, the tempo and what follows, from the right.
        let mut gx = block.right() - 8.0;
        let gy = block.bottom() - 14.0;
        match step.then {
            Then::Next => {}
            Then::Loop => {
                loop_glyph(&clip, pos2(gx - 4.0, gy), theme::text()_DIM);
                gx -= 14.0;
            }
            Then::Stop => {
                clip.rect_filled(Rect::from_center_size(pos2(gx - 4.0, gy), vec2(7.0, 7.0)), 1.0, theme::text()_DIM);
                gx -= 14.0;
            }
        }
        if step.morph > 0 {
            let r = clip.text(pos2(gx, gy), Align2::RIGHT_CENTER, format!("{}", step.morph), theme::font(9.5), theme::text()_DIM);
            morph_glyph(&clip, pos2(r.left() - 12.0, gy), theme::text()_DIM);
            gx = r.left() - 16.0;
        }
        if let Some(t) = step.tempo {
            clip.text(pos2(gx, gy), Align2::RIGHT_CENTER, format!("{t:.0}"), theme::font(9.5), theme::text()_DIM);
        }
        if coming {
            clip.text(block.right_top() + vec2(-6.0, 6.0), Align2::RIGHT_TOP, "NEXT", theme::strong(8.5), theme::text());
        } else if playing && let Some(bar) = Some(((info.beat - info.started).max(0.0) / BEATS_PER_BAR) as u32 + 1) {
            clip.text(block.right_top() + vec2(-6.0, 6.0), Align2::RIGHT_TOP, format!("{}/{}", bar.min(step.bars), step.bars), theme::mono(9.0), theme::text());
        }
    }

    fn inspector(&mut self, ui: &mut Ui, r: Rect, song: &mut Song) {
        let p = ui.painter_at(r.expand(2.0));
        let Some(i) = self.selected.filter(|&i| i < song.steps.len()) else {
            p.text(
                r.left_top() + vec2(0.0, 4.0),
                Align2::LEFT_TOP,
                "Click a step to set its bars,\nkey, MORPH, tempo and what\nfollows it.",
                theme::font(10.5),
                theme::text()_FAINT,
            );
            return;
        };
        p.text(r.left_top() + vec2(0.0, 8.0), Align2::LEFT_CENTER, format!("STEP {}", i + 1), theme::font(9.5), theme::text()_FAINT);
        let x_r = Rect::from_min_size(pos2(r.right() - 17.0, r.top()), vec2(17.0, 17.0));
        let remove = ui.interact(x_r, Id::unique("song-step-remove"), Sense::click());
        p.text(x_r.center(), Align2::CENTER_CENTER, "×", theme::strong(13.0), if remove.hovered() { theme::mute() } else { theme::text()_FAINT });
        if remove.on_hover_text("Remove this step").clicked() {
            song.remove_step(i);
            self.selected = None;
            return;
        }
        let home = song.home;
        let shift = song.shift(&song.steps[i]);
        let scene_names: Vec<(u32, String, u8)> = song.scenes.iter().map(|s| (s.id, s.name.clone(), s.color)).collect();
        let step = &mut song.steps[i];
        let rows = Rect::from_min_max(pos2(r.left(), r.top() + 20.0), r.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rows), |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 3.0);
            ui.spacing_mut().interact_size.y = 18.0;
            let row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
                ui.horizontal(|ui| {
                    let (lr, _) = ui.allocate_exact_size(vec2(46.0, 18.0), Sense::hover());
                    ui.painter().text(lr.left_center(), Align2::LEFT_CENTER, label, theme::font(9.5), theme::text()_DIM);
                    add(ui);
                });
            };
            row(ui, "SCENE", &mut |ui| {
                let name = scene_names.iter().find(|s| s.0 == step.scene).map_or("?".to_owned(), |s| s.1.clone());
                let resp = menu_pill(ui, &name);
                egui::Popup::menu(&resp).show(|ui| {
                    for (id, name, _) in &scene_names {
                        if canvas::menu_item(ui, name, if *id == step.scene { "✓" } else { "" }) {
                            step.scene = *id;
                            ui.close();
                        }
                    }
                });
            });
            row(ui, "BARS", &mut |ui| {
                ui.add(egui::DragValue::new(&mut step.bars).range(1..=MAX_BARS).speed(0.1).suffix(" bars"))
                    .on_hover_text("How long the step plays");
            });
            row(ui, "KEY", &mut |ui| {
                let label = match step.key {
                    Some(k) if shift != 0 => format!("{} ({:+})", song::key_name(k), shift),
                    Some(k) => song::key_name(k).to_owned(),
                    None => format!("{} (home)", song::key_name(home)),
                };
                let resp = menu_pill(ui, &label).on_hover_text(
                    "The step's key: the notes and pitch knobs that follow the key move from the home key to it",
                );
                egui::Popup::menu(&resp).show(|ui| {
                    if canvas::menu_item(ui, &format!("Home ({})", song::key_name(home)), if step.key.is_none() { "✓" } else { "" }) {
                        step.key = None;
                        ui.close();
                    }
                    for k in 0..12u8 {
                        let d = (k as i32 - home as i32).rem_euclid(12);
                        let d = if d > 6 { d - 12 } else { d };
                        let hint = if step.key == Some(k) { "✓".to_owned() } else { format!("{d:+}") };
                        if canvas::menu_item(ui, song::key_name(k), &hint) {
                            step.key = Some(k);
                            ui.close();
                        }
                    }
                });
            });
            row(ui, "MORPH", &mut |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for m in MORPHS {
                    let label = if m == 0 { "CUT".to_owned() } else { m.to_string() };
                    if pill(ui, &label, step.morph == m, theme::accent())
                        .on_hover_text(if m == 0 { "The scene comes in at once".to_owned() } else { format!("Continuous knobs glide into the scene over {m} bar{}", if m == 1 { "" } else { "s" }) })
                        .clicked()
                    {
                        step.morph = m;
                    }
                }
            });
            row(ui, "TEMPO", &mut |ui| {
                let set = step.tempo.is_some();
                if pill(ui, if set { "SET" } else { "KEEP" }, set, theme::accent())
                    .on_hover_text("Keep the tempo as it is, or set it when the step starts")
                    .clicked()
                {
                    step.tempo = if set { None } else { Some(120.0) };
                }
                if let Some(t) = step.tempo.as_mut() {
                    ui.add(egui::DragValue::new(t).range(crate::audio::params::TEMPO_RANGE.0..=crate::audio::params::TEMPO_RANGE.1).speed(0.2).fixed_decimals(1).suffix(" BPM"));
                }
            });
            row(ui, "THEN", &mut |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for t in Then::ALL {
                    let tip = match t {
                        Then::Next => "Then the next step (after the last, the first)",
                        Then::Loop => "Then this step again, until you pick another",
                        Then::Stop => "Then the transport stops",
                    };
                    if pill(ui, t.name(), step.then == t, theme::accent()).on_hover_text(tip).clicked() {
                        step.then = t;
                    }
                }
            });
        });
    }
}

#[derive(Clone, Copy)]
enum StepMenu {
    Queue,
    Start,
    Duplicate,
    Remove,
}

/// What plays, for the header: "▶ 3 · B · F" or where Start begins.
fn status(song: &Song, info: &Info) -> Option<String> {
    if !song.on || song.steps.is_empty() {
        return None;
    }
    let name = |i: usize| song.steps.get(i).and_then(|s| song.scene(s.scene)).map_or("?", |s| s.name.as_str());
    let key = |i: usize| song.steps.get(i).map_or("", |s| song::key_name(song.key_of(s)));
    match info.current.filter(|_| info.running) {
        Some(i) => {
            let next = info.coming.map_or(String::new(), |n| format!("   next {} · {}", n + 1, name(n)));
            Some(format!("▶ {} · {} · {}{next}", i + 1, name(i), key(i)))
        }
        None => {
            let s = song.start.min(song.steps.len() - 1);
            Some(format!("Start plays from {} · {}", s + 1, name(s)))
        }
    }
}

fn step_tip(song: &Song, i: usize, step: &Step, running: bool) -> String {
    let scene = song.scene(step.scene).map_or("?", |s| s.name.as_str());
    let key = song::key_name(song.key_of(step));
    let morph = if step.morph > 0 { format!(", MORPH {} bar{}", step.morph, if step.morph == 1 { "" } else { "s" }) } else { String::new() };
    let tempo = step.tempo.map_or(String::new(), |t| format!(", {t:.1} BPM"));
    let then = match step.then {
        Then::Next => "then the next step",
        Then::Loop => "then again",
        Then::Stop => "then stop",
    };
    let how = if running { "double-click to play it next" } else { "click to start the song here" };
    format!("Step {}: scene {scene}, {} bars in {key}{morph}{tempo}, {then}\n{how}; drag to move it", i + 1, step.bars)
}

/// A small flat button: lit (filled) when `on`.
fn pill(ui: &mut Ui, text: &str, on: bool, col: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::strong(9.5), theme::text());
    let size = vec2(galley.size().x + 14.0, 18.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let p = ui.painter();
    let fill = if on { col } else if resp.hovered() { theme::control()_HI } else { theme::control() };
    p.rect_filled(r, 9.0, fill);
    p.text(r.center(), Align2::CENTER_CENTER, text, theme::strong(9.5), if on { theme::inset() } else { theme::text()_DIM });
    resp
}

/// A value that opens a menu: "END ▾".
fn menu_pill(ui: &mut Ui, text: &str) -> Response {
    let label = format!("{text} ▾");
    let galley = ui.painter().layout_no_wrap(label.clone(), theme::font(10.5), theme::text());
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 18.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let p = ui.painter();
    p.rect(r, 9.0, theme::inset(), Stroke::new(1.0, if resp.hovered() { theme::control()_HI } else { theme::node_border() }), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, label, theme::font(10.5), theme::text());
    resp
}

fn caption(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(9.5).color(theme::text()_FAINT));
}

/// A flat button in a rect: filled with `col` when `on`.
fn flat_button(ui: &Ui, p: &Painter, id: Id, r: Rect, text: &str, on: bool, col: Color32) -> Response {
    let resp = ui.interact(r, id, Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    let fill = if on { col } else if resp.hovered() { theme::control()_HI } else { theme::control() };
    p.rect_filled(r, 8.5, fill);
    p.text(r.center(), Align2::CENTER_CENTER, text, theme::strong(9.5), if on { theme::inset() } else { theme::text()_DIM });
    resp
}

/// The start step's flag, above its block.
fn start_flag(p: &Painter, at: Pos2) {
    let pts = vec![at + vec2(0.0, -10.0), at + vec2(7.0, -6.5), at + vec2(0.0, -3.0)];
    p.add(egui::Shape::convex_polygon(pts, theme::accent(), Stroke::NONE));
}

/// A ramp: MORPH.
fn morph_glyph(p: &Painter, at: Pos2, col: Color32) {
    let pts = vec![at + vec2(0.0, 4.0), at + vec2(9.0, -4.0), at + vec2(9.0, 4.0)];
    p.add(egui::Shape::convex_polygon(pts, alpha(col, 0.8), Stroke::NONE));
}

/// A ring with an arrowhead: the step repeats.
fn loop_glyph(p: &Painter, c: Pos2, col: Color32) {
    let pts = super::widgets::arc(c, 4.0, -2.4, 2.0);
    p.add(egui::Shape::line(pts, Stroke::new(1.3, col)));
    let tip = c + vec2(4.0 * 2.0f32.cos(), 4.0 * 2.0f32.sin());
    p.add(egui::Shape::convex_polygon(vec![tip + vec2(-3.0, 0.0), tip + vec2(3.0, 0.0), tip + vec2(0.0, 3.5)], col, Stroke::NONE));
}

/// The NODES menu: for each node, whether the scenes set it and whether it
/// follows the key.
fn nodes_menu(ui: &mut Ui, song: &mut Song, graph: &Graph) {
    ui.label(RichText::new("Set by the scenes · follows the key").size(10.5).color(theme::text()_DIM));
    ui.add_space(2.0);
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
        let mut nodes: Vec<&crate::graph::Node> = graph.nodes.iter().collect();
        nodes.sort_by(|a, b| a.title.cmp(&b.title));
        for n in nodes {
            let plugin = matches!(n.kind, crate::graph::NodeKind::Plugin(_));
            ui.horizontal(|ui| {
                ui.set_min_width(260.0);
                let mut drives = song.drives(n);
                let resp = ui.add_enabled(!plugin, egui::Checkbox::new(&mut drives, RichText::new(&n.title).size(11.5)));
                let resp = if plugin {
                    resp.on_disabled_hover_text("Plugins keep their own state: scenes leave them alone")
                } else {
                    resp.on_hover_text("Scenes set this node (off: you play it by hand)")
                };
                if resp.changed() {
                    set_free(song, n.id, !drives);
                }
                if song::keyable(n) {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut follows = song.follows(n);
                        let what = if n.midi_out() { "its notes" } else { "its pitch" };
                        if ui
                            .checkbox(&mut follows, RichText::new("KEY").size(10.0))
                            .on_hover_text(format!("The step's key moves {what}"))
                            .changed()
                        {
                            if follows == song::follows_by_default(n) {
                                song.keys.remove(&n.id);
                            } else {
                                song.keys.insert(n.id, follows);
                            }
                        }
                    });
                }
            });
        }
    });
}

fn set_free(song: &mut Song, id: NodeId, free: bool) {
    if free {
        song.free.insert(id);
    } else {
        song.free.remove(&id);
    }
}

/// The toolbar's song chip: the step playing and its bar, or SONG.
pub fn chip(ui: &mut Ui, song: &Song, info: &Info, open: bool) -> Response {
    let playing = song.on && info.running && info.current.is_some();
    let (text, col) = match info.current.filter(|_| playing) {
        Some(i) => {
            let step = song.steps[i.min(song.steps.len() - 1)];
            let scene = song.scene(step.scene);
            let bar = (((info.beat - info.started).max(0.0) / BEATS_PER_BAR) as u32 + 1).min(step.bars);
            (
                format!("{} · {}/{}", scene.map_or("?", |s| s.name.as_str()), bar, step.bars),
                scene.map_or(theme::text()_DIM, |s| scene_color(s.color)),
            )
        }
        None => ("SONG".to_owned(), if song.on { theme::accent() } else { theme::text()_DIM }),
    };
    let galley = ui.painter().layout_no_wrap(text.clone(), theme::strong(10.0), theme::text());
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 26.0, 24.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let p = ui.painter();
    let bg = if open || resp.hovered() { theme::control() } else { Color32::TRANSPARENT };
    p.rect(r, 12.0, bg, Stroke::new(1.0, theme::node_border()), StrokeKind::Inside);
    // Three blocks of different lengths: a song.
    let x = r.left() + 8.0;
    let y = r.center().y;
    let lit = song.on;
    for (k, (dx, w)) in [(0.0, 3.0), (4.0, 2.0), (7.0, 4.0)].into_iter().enumerate() {
        let c = if lit && (k == 0 || playing) { col } else { theme::text()_FAINT };
        p.rect_filled(Rect::from_min_size(pos2(x + dx, y - 4.0), vec2(w, 8.0)), 1.0, c);
    }
    p.text(pos2(r.left() + 20.0, y), Align2::LEFT_CENTER, text, theme::strong(10.0), if song.on { theme::text() } else { theme::text()_DIM });
    resp
}
