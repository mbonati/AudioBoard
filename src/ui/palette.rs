//! The palette: every node type and every library module, searchable.
//! Drag an entry onto the canvas to place it there, or click it to add it
//! in the middle of the view.

use egui::{Align2, Color32, CursorIcon, FontId, Id, LayerId, Order, Rect, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2};

use super::canvas::Request;
use super::theme::{self, alpha};
use crate::graph::NodeType;
use crate::project::Preset;

pub const WIDTH: f32 = 212.0;
const ROW_H: f32 = 34.0;

/// What a palette entry adds. Also the drag-and-drop payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spawn {
    Node(NodeType),
    Preset(usize),
}

pub struct Palette {
    query: String,
}

/// Colour of each palette group's swatch.
fn group_color(group: &str) -> Color32 {
    match group {
        "Sources" => theme::OSC_ACCENTS[0],
        "Texture" => theme::OSC_ACCENTS[3],
        "Effects" => theme::OSC_ACCENTS[5],
        "MIDI" => theme::OSC_ACCENTS[1],
        "Output" => theme::MIXER_ACCENT,
        _ => theme::ACCENT,
    }
}

impl Palette {
    pub fn new() -> Self {
        Self { query: String::new() }
    }

    /// Returns an entry that was clicked; drags are dropped on the canvas.
    pub fn show(&mut self, ui: &mut Ui, library: &[Preset], requests: &mut Vec<Request>) -> Option<Spawn> {
        let mut picked = None;
        ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("Search")
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::symmetric(8, 4)),
        );
        ui.add_space(4.0);
        let query = self.query.trim().to_lowercase();
        let matches = |name: &str, hint: &str| {
            query.is_empty() || name.to_lowercase().contains(&query) || hint.to_lowercase().contains(&query)
        };

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (group, types) in NodeType::MENU {
                let shown: Vec<NodeType> = types.iter().copied().filter(|t| matches(t.name(), t.hint())).collect();
                if shown.is_empty() {
                    continue;
                }
                heading(ui, group);
                for ty in shown {
                    let entry = Entry {
                        name: ty.name(),
                        hint: ty.hint(),
                        color: group_color(group),
                        spawn: Spawn::Node(ty),
                        removable: false,
                    };
                    let r = row(ui, &entry);
                    if r.clicked {
                        picked = Some(entry.spawn);
                    }
                }
            }

            let presets: Vec<(usize, &Preset)> = library
                .iter()
                .enumerate()
                .filter(|(_, p)| matches(&p.name, "module library"))
                .collect();
            if !presets.is_empty() || query.is_empty() {
                heading(ui, "Library");
            }
            if library.is_empty() && query.is_empty() {
                ui.add_space(2.0);
                ui.label(
                    RichText::new("Right-click a module → Save to library to keep it here.")
                        .size(10.5)
                        .color(theme::TEXT_FAINT),
                );
            }
            for (i, preset) in presets {
                let n = preset.node_count();
                let hint = if n == 1 { "module · 1 node".to_owned() } else { format!("module · {n} nodes") };
                let entry = Entry {
                    name: &preset.name,
                    hint: &hint,
                    color: theme::ACCENT,
                    spawn: Spawn::Preset(i),
                    removable: true,
                };
                let r = row(ui, &entry);
                if r.clicked {
                    picked = Some(entry.spawn);
                }
                if r.removed {
                    requests.push(Request::DeletePreset(i));
                }
            }
            if query.is_empty() {
                ui.add_space(6.0);
                let open = ui.add(
                    egui::Label::new(RichText::new("Open library folder").size(10.5).color(theme::TEXT_DIM))
                        .sense(Sense::click()),
                );
                if open.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    requests.push(Request::RevealLibrary);
                }
            }
        });

        drag_ghost(ui, library);
        picked
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text.to_uppercase()).size(9.5).color(theme::TEXT_FAINT));
    ui.add_space(4.0);
}

struct Entry<'a> {
    name: &'a str,
    hint: &'a str,
    color: Color32,
    spawn: Spawn,
    removable: bool,
}

struct RowResponse {
    clicked: bool,
    removed: bool,
}

/// One entry: swatch, name and hint. Click adds it; dragging carries it.
fn row(ui: &mut Ui, e: &Entry) -> RowResponse {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
    let resp = resp
        .on_hover_cursor(CursorIcon::Grab)
        .on_hover_text("Drag onto the canvas, or click to add");
    if resp.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), e.spawn);
    }
    let p = ui.painter();
    let hot = resp.hovered() || resp.dragged();
    if hot {
        p.rect_filled(rect, 4.0, theme::CONTROL);
    }
    let swatch = Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), Vec2::splat(12.0));
    p.rect_filled(swatch, 3.0, alpha(e.color, 0.25));
    p.rect_stroke(swatch, 3.0, Stroke::new(1.2, e.color), egui::StrokeKind::Inside);
    p.text(
        pos2(rect.left() + 30.0, rect.center().y - 7.0),
        Align2::LEFT_CENTER,
        e.name,
        FontId::proportional(12.0),
        theme::TEXT,
    );
    p.text(
        pos2(rect.left() + 30.0, rect.center().y + 8.0),
        Align2::LEFT_CENTER,
        e.hint,
        FontId::proportional(9.5),
        theme::TEXT_FAINT,
    );

    let mut removed = false;
    if e.removable && hot {
        let x = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), Vec2::splat(18.0));
        let r = ui
            .interact(x, Id::unique(("palette_remove", e.name)), Sense::click())
            .on_hover_text("Remove from the library");
        let col = if r.hovered() { theme::TEXT } else { theme::TEXT_FAINT };
        ui.painter()
            .text(x.center(), Align2::CENTER_CENTER, "×", FontId::proportional(14.0), col);
        removed = r.clicked();
    }
    RowResponse {
        clicked: resp.clicked() && !removed,
        removed,
    }
}

/// While an entry is being dragged: its name follows the pointer.
fn drag_ghost(ui: &Ui, library: &[Preset]) {
    let Some(spawn) = egui::DragAndDrop::payload::<Spawn>(ui.ctx()) else {
        return;
    };
    let Some(p) = ui.ctx().pointer_hover_pos() else {
        return;
    };
    let name = match *spawn {
        Spawn::Node(ty) => ty.name().to_owned(),
        Spawn::Preset(i) => library.get(i).map_or("Module".to_owned(), |p| p.name.clone()),
    };
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    let painter = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, Id::unique("palette_ghost")));
    let galley = painter.layout_no_wrap(name, FontId::proportional(12.0), theme::TEXT);
    let pill = Rect::from_min_size(p + vec2(14.0, 10.0), galley.size() + vec2(20.0, 10.0));
    painter.rect(pill, 6.0, theme::NODE_HEADER, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
    painter.galley(pill.min + vec2(10.0, 5.0), galley, theme::TEXT);
}
