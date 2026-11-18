//! The palette: every node type and every library module, searchable.
//! Drag an entry onto the canvas to place it there, or click it to add it
//! in the middle of the view.

use egui::collapsing_header::CollapsingState;
use egui::{Align2, Color32, CursorIcon, FontId, Id, LayerId, Order, Rect, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2};

use super::canvas::Request;
use super::theme::{self, alpha};
use crate::audio::modules::ModuleKind;
use crate::graph::NodeType;
use crate::project::Preset;

pub const WIDTH: f32 = 212.0;
const ROW_H: f32 = 34.0;

/// What a palette entry adds. Also the drag-and-drop payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spawn {
    Node(NodeType),
    Preset(usize),
    /// A plugin (VST3 or Audio Unit) from the installed list.
    Plugin(usize),
}

/// An installed plugin, as listed (its code is not loaded to list it).
#[derive(Clone, Debug)]
pub struct PluginEntry {
    /// Its bundle, or an Audio Unit's pseudo-path (see `audio::au`).
    pub path: std::path::PathBuf,
    pub name: String,
    /// Its category once it has been loaded, else "VST3".
    pub hint: String,
    /// Its maker, or empty if unknown.
    pub vendor: String,
    pub format: crate::audio::plugin::Format,
}

/// Plugins grouped by maker: (maker, indices into `plugins`), makers in
/// alphabetical order and unknown ones last as "Other". Spellings of one
/// maker ("Valhalla DSP", "ValhallaDSP") share a group, named by the
/// fullest spelling.
pub fn by_vendor(plugins: &[PluginEntry]) -> Vec<(String, Vec<usize>)> {
    use crate::audio::plugin::vendor_key;
    let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new();
    for (i, p) in plugins.iter().enumerate() {
        let key = vendor_key(&p.vendor);
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => {
                let fuller = |s: &str| (s.matches(' ').count(), s.len());
                if fuller(&p.vendor) > fuller(&g.1) {
                    g.1 = p.vendor.clone();
                }
                g.2.push(i);
            }
            None => groups.push((key, p.vendor.clone(), vec![i])),
        }
    }
    groups.sort_by_key(|g| (g.0.is_empty(), g.1.to_lowercase()));
    groups
        .into_iter()
        .map(|(key, name, members)| (if key.is_empty() { "Other".to_owned() } else { name }, members))
        .collect()
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
    pub fn show(
        &mut self,
        ui: &mut Ui,
        library: &[Preset],
        plugins: &[PluginEntry],
        requests: &mut Vec<Request>,
    ) -> Option<Spawn> {
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

        let searching = !query.is_empty();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (group, types) in NodeType::MENU {
                let shown: Vec<NodeType> = types.iter().copied().filter(|t| matches(t.name(), t.hint())).collect();
                if shown.is_empty() {
                    continue;
                }
                section(ui, group, shown.len(), searching, |ui| {
                    for ty in shown {
                        let entry = Entry {
                            name: ty.name(),
                            hint: ty.hint(),
                            color: group_color(group),
                            spawn: Spawn::Node(ty),
                            removable: false,
                        };
                        if row(ui, &entry).clicked {
                            picked = Some(entry.spawn);
                        }
                    }
                });
            }

            // Plugin nodes (dylibs built with the node SDK), by maker.
            let makers = crate::audio::node_plugin::by_maker();
            let node_groups: Vec<(&str, Vec<NodeType>)> = makers
                .iter()
                .map(|(maker, list)| {
                    let types = list.iter().map(|e| NodeType::Module(ModuleKind::Ext(*e)));
                    (*maker, types.filter(|t| matches(t.name(), t.hint()) || matches(maker, "")).collect::<Vec<_>>())
                })
                .filter(|(_, types)| !types.is_empty())
                .collect();
            let node_count: usize = node_groups.iter().map(|(_, t)| t.len()).sum();
            if node_count > 0 || !searching {
                section(ui, "Plugin nodes", node_count, searching, |ui| {
                    if makers.is_empty() && !searching {
                        ui.label(
                            RichText::new("Node plugins you install in the Nodes folder show here.")
                                .size(10.5)
                                .color(theme::TEXT_FAINT),
                        );
                    }
                    for (maker, types) in node_groups {
                        node_maker_section(ui, maker, types.len(), searching, |ui| {
                            let mut rows = ui.available_rect_before_wrap();
                            rows.min.x += 10.0;
                            ui.scope_builder(egui::UiBuilder::new().max_rect(rows), |ui| {
                                for ty in types {
                                    let entry = Entry {
                                        name: ty.name(),
                                        hint: ty.hint(),
                                        color: theme::OSC_ACCENTS[2],
                                        spawn: Spawn::Node(ty),
                                        removable: false,
                                    };
                                    if row(ui, &entry).clicked {
                                        picked = Some(entry.spawn);
                                    }
                                }
                            });
                        });
                    }
                    if !searching {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let link = |ui: &mut Ui, text: &str| {
                                ui.add(egui::Label::new(RichText::new(text).size(10.5).color(theme::TEXT_DIM)).sense(Sense::click()))
                                    .on_hover_cursor(CursorIcon::PointingHand)
                                    .clicked()
                            };
                            if link(ui, "Rescan") {
                                requests.push(Request::RescanPlugins);
                            }
                            ui.label(RichText::new("·").size(10.5).color(theme::TEXT_FAINT));
                            if link(ui, "Open Nodes folder") {
                                requests.push(Request::RevealNodePlugins);
                            }
                        });
                    }
                });
            }

            // Plugins, by maker; a search also matches the maker's name.
            let shown: Vec<usize> = (0..plugins.len())
                .filter(|&i| matches(&plugins[i].name, &plugins[i].hint) || matches(&plugins[i].vendor, ""))
                .collect();
            if !shown.is_empty() || !searching {
                section(ui, "Plugins", shown.len(), searching, |ui| {
                    if plugins.is_empty() && !searching {
                        ui.label(
                            RichText::new("No plugins in the system plug-in folders.")
                                .size(10.5)
                                .color(theme::TEXT_FAINT),
                        );
                    }
                    for (vendor, members) in by_vendor(plugins) {
                        let members: Vec<usize> = members.into_iter().filter(|i| shown.contains(i)).collect();
                        if members.is_empty() {
                            continue;
                        }
                        vendor_section(ui, &vendor, members.len(), searching, |ui| {
                            // One step in, under their maker.
                            let mut rows = ui.available_rect_before_wrap();
                            rows.min.x += 10.0;
                            ui.scope_builder(egui::UiBuilder::new().max_rect(rows), |ui| {
                                for i in members {
                                    let plugin = &plugins[i];
                                    let entry = Entry {
                                        name: &plugin.name,
                                        hint: &plugin.hint,
                                        color: theme::OSC_ACCENTS[4],
                                        spawn: Spawn::Plugin(i),
                                        removable: false,
                                    };
                                    if row(ui, &entry).clicked {
                                        picked = Some(entry.spawn);
                                    }
                                }
                            });
                        });
                    }
                    if !searching {
                        ui.add_space(6.0);
                        let rescan = ui.add(
                            egui::Label::new(RichText::new("Rescan plugins").size(10.5).color(theme::TEXT_DIM))
                                .sense(Sense::click()),
                        );
                        if rescan.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            requests.push(Request::RescanPlugins);
                        }
                    }
                });
            }

            let presets: Vec<(usize, &Preset)> = library
                .iter()
                .enumerate()
                .filter(|(_, p)| matches(&p.name, "module library"))
                .collect();
            if !presets.is_empty() || !searching {
                section(ui, "Library", presets.len(), searching, |ui| {
                    if library.is_empty() && !searching {
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
                    if !searching {
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
            }
        });

        drag_ghost(ui, library, plugins);
        picked
    }
}

/// How a fold's heading looks.
#[derive(Clone, Copy, PartialEq)]
enum Level {
    /// "SOURCES", "PLUGINS".
    Category,
    /// A plugin maker, inside Plugins.
    Vendor,
}

/// A category: a heading that folds and unfolds its entries (open at
/// first; folded ones show how many entries they hold). While searching,
/// every category with matches is open, and the folds are kept for later.
fn section(ui: &mut Ui, name: &str, count: usize, searching: bool, body: impl FnOnce(&mut Ui)) {
    fold(ui, Level::Category, "", name, count, searching, body);
}

/// A plugin maker inside Plugins: folds like a category, one step in.
fn vendor_section(ui: &mut Ui, name: &str, count: usize, searching: bool, body: impl FnOnce(&mut Ui)) {
    fold(ui, Level::Vendor, "", name, count, searching, body);
}

/// A maker inside Plugin nodes (folds apart from the same maker's plugins).
fn node_maker_section(ui: &mut Ui, name: &str, count: usize, searching: bool, body: impl FnOnce(&mut Ui)) {
    fold(ui, Level::Vendor, "nodes", name, count, searching, body);
}

fn fold(ui: &mut Ui, level: Level, group: &str, name: &str, count: usize, searching: bool, body: impl FnOnce(&mut Ui)) {
    let vendor = level == Level::Vendor;
    let id = Id::unique(("palette_section", vendor, group, name));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, true);
    ui.add_space(if vendor { 2.0 } else { 6.0 });
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), if vendor { 22.0 } else { 24.0 }), Sense::click());
    let resp = if searching {
        resp
    } else {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    };
    if resp.clicked() && !searching {
        state.toggle(ui);
    }
    let openness = if searching { 1.0 } else { state.openness(ui.ctx()) };

    let p = ui.painter();
    let (rest, hot) = if vendor { (theme::TEXT_DIM, theme::TEXT) } else { (theme::TEXT_FAINT, theme::TEXT_DIM) };
    let color = if resp.hovered() && !searching { hot } else { rest };
    let indent = if vendor { 10.0 } else { 0.0 };
    // A chevron that turns from pointing right (folded) to down (open).
    let c = pos2(rect.left() + indent + 6.0, rect.center().y);
    let angle = openness * std::f32::consts::FRAC_PI_2;
    let turn = |v: Vec2| c + Vec2::angled(v.angle() + angle) * v.length();
    let chevron = vec![turn(vec2(-2.0, -3.5)), turn(vec2(3.0, 0.0)), turn(vec2(-2.0, 3.5))];
    p.add(egui::Shape::convex_polygon(chevron, color, Stroke::NONE));
    let (label, size) = if vendor { (name.to_owned(), 11.0) } else { (name.to_uppercase(), 9.5) };
    p.text(pos2(rect.left() + indent + 16.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(size), color);
    if openness < 1.0 {
        let fade = alpha(theme::TEXT_FAINT, 1.0 - openness);
        p.text(pos2(rect.right() - 6.0, rect.center().y), Align2::RIGHT_CENTER, count.to_string(), FontId::proportional(9.5), fade);
    }

    if searching {
        body(ui);
    } else {
        state.show_body_unindented(ui, body);
    }
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
fn drag_ghost(ui: &Ui, library: &[Preset], plugins: &[PluginEntry]) {
    let Some(spawn) = egui::DragAndDrop::payload::<Spawn>(ui.ctx()) else {
        return;
    };
    let Some(p) = ui.ctx().pointer_hover_pos() else {
        return;
    };
    let name = match *spawn {
        Spawn::Node(ty) => ty.name().to_owned(),
        Spawn::Preset(i) => library.get(i).map_or("Module".to_owned(), |p| p.name.clone()),
        Spawn::Plugin(i) => plugins.get(i).map_or("Plugin".to_owned(), |p| p.name.clone()),
    };
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    let painter = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, Id::unique("palette_ghost")));
    let galley = painter.layout_no_wrap(name, FontId::proportional(12.0), theme::TEXT);
    let pill = Rect::from_min_size(p + vec2(14.0, 10.0), galley.size() + vec2(20.0, 10.0));
    painter.rect(pill, 6.0, theme::NODE_HEADER, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
    painter.galley(pill.min + vec2(10.0, 5.0), galley, theme::TEXT);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, vendor: &str) -> PluginEntry {
        PluginEntry {
            path: name.into(),
            name: name.to_owned(),
            hint: "VST3".to_owned(),
            vendor: vendor.to_owned(),
            format: crate::audio::plugin::Format::Vst3,
        }
    }

    #[test]
    fn plugins_group_by_maker() {
        let plugins = [
            entry("Vital", "Vital Audio"),
            entry("Mystery", ""),
            entry("FreqEcho", "Valhalla DSP"),
            entry("SpaceModulator", "ValhallaDSP"),
            entry("BBC SO", "Spitfire Audio"),
            entry("Supermassive", "Valhalla DSP"),
        ];
        let groups = by_vendor(&plugins);
        let names: Vec<&str> = groups.iter().map(|g| g.0.as_str()).collect();
        assert_eq!(names, ["Spitfire Audio", "Valhalla DSP", "Vital Audio", "Other"]);
        assert_eq!(groups[1].1, [2, 3, 5], "spellings of one maker share a group");
        assert_eq!(groups[3].1, [1]);
    }
}
