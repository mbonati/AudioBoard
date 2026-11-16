//! The panel of a plugin node (VST3 or Audio Unit): ports (audio, and MIDI
//! for plugins that take it), name and maker, buttons for the plugin's own
//! editor, its parameter list and the MIDI keyboard, and knobs for up to
//! eight parameters.

use egui::{Align2, Color32, Id, Popup, PopupCloseBehavior, RichText, ScrollArea, Vec2, vec2};

use super::nodes::NodeView;
use super::theme::{self, alpha};
use super::widgets::{self, knob};
use crate::audio::engine::NodeId;
use crate::graph::{PINNED_MAX, ParamRef, PluginNode};

pub const SIZE: Vec2 = vec2(280.0, 222.0);
const PAD: f32 = 12.0;
const PORT_Y: f32 = 42.0;
const PORT_DY: f32 = 18.0;
const BUTTONS_Y: [f32; 2] = [72.0, 92.0];
const KNOBS_TOP: f32 = 108.0;
const ROW_H: f32 = 58.0;
const COL_W: f32 = 64.0;

pub fn input_offset(i: usize) -> Vec2 {
    vec2(0.0, PORT_Y + PORT_DY * i as f32)
}

pub fn output_offset(i: usize) -> Vec2 {
    vec2(SIZE.x, PORT_Y + PORT_DY * i as f32)
}

/// "Fx · Delay" for "Fx|Delay".
pub fn category_label(category: &str) -> String {
    let label = category.replace('|', " · ");
    if label.is_empty() { "VST3".to_owned() } else { label }
}

/// The EDITOR button was clicked (opening runs plugin code; the canvas
/// reports failures).
pub struct PluginEvents {
    pub toggle_editor: bool,
}

pub fn draw(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    p: &mut PluginNode,
    (input, midi_in): (bool, bool),
    output: bool,
    now: f64,
) -> PluginEvents {
    let z = v.z;
    let w = SIZE.x;
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };

    if p.meta.inputs > 0 {
        v.text(PAD, PORT_Y, Align2::LEFT_CENTER, "IN", v.font(9.0), col(input));
    }
    if p.meta.midi {
        let y = if p.meta.inputs > 0 { PORT_Y + PORT_DY } else { PORT_Y };
        v.text(PAD, y, Align2::LEFT_CENTER, "MIDI", v.font(9.0), col(midi_in));
    }
    v.text(w - PAD, PORT_Y, Align2::RIGHT_CENTER, "OUT", v.font(9.0), col(output));
    v.text(w * 0.5, PORT_Y, Align2::CENTER_CENTER, category_label(&p.meta.category), v.font(11.0), theme::TEXT);
    // The maker, and the format when it isn't VST3.
    let maker = match p.meta.format {
        crate::audio::plugin::Format::Vst3 => p.meta.vendor.clone(),
        f if p.meta.vendor.is_empty() => f.label().to_owned(),
        f => format!("{} · {}", p.meta.vendor, f.label()),
    };
    v.text(w * 0.5, PORT_Y + PORT_DY, Align2::CENTER_CENTER, maker, v.font(9.5), theme::TEXT_DIM);

    // Buttons: EDITOR · PARAMS · KEYBOARD.
    let gap = 6.0;
    let bw = (w - 2.0 * PAD - 2.0 * gap) / 3.0;
    let slot = |k: usize| {
        let x = PAD + k as f32 * (bw + gap);
        v.r(x, BUTTONS_Y[0], x + bw, BUTTONS_Y[1])
    };
    let editor_open = p.window.is_some();
    let editor = widgets::toggle(v.ui, v.painter, Id::unique(("plug_editor", id)), slot(0), "EDITOR", editor_open, accent, z)
        .on_hover_text(if p.meta.editor { "The plugin's own window" } else { "This plugin has no editor" });
    let params = widgets::toggle(v.ui, v.painter, Id::unique(("plug_params", id)), slot(1), "PARAMS", false, accent, z)
        .on_hover_text("All parameters; pin up to eight as knobs");
    let midi_on = p.shared.midi_in.load(std::sync::atomic::Ordering::Relaxed);
    let midi = widgets::toggle(v.ui, v.painter, Id::unique(("plug_midi", id)), slot(2), "KEYBOARD", midi_on, accent, z)
        .on_hover_text(if p.meta.midi {
            "Play it from the MIDI keyboard (or patch a sequencer into its MIDI port)"
        } else {
            "Send the MIDI keyboard to it"
        });
    if midi.clicked() {
        p.shared.midi_in.store(!midi_on, std::sync::atomic::Ordering::Relaxed);
    }
    parameter_list(&params, p, now);

    // Knobs for the pinned parameters.
    if p.pinned.is_empty() {
        v.text(w * 0.5, KNOBS_TOP + 30.0, Align2::CENTER_CENTER, "No parameters", v.font(10.0), theme::TEXT_FAINT);
    }
    for (k, i) in p.pinned.clone().into_iter().enumerate() {
        let cx = PAD + COL_W * ((k % 4) as f32 + 0.5);
        let row_y = KNOBS_TOP + ROW_H * (k / 4) as f32;
        let param = &p.params[i];
        let label: String = param.name.chars().take(11).collect();
        v.text(cx, row_y, Align2::CENTER_CENTER, label.to_uppercase(), v.font(8.0), theme::TEXT_FAINT);
        let mut norm = p.values[i] as f32;
        let resp = knob(
            v.ui,
            v.painter,
            Id::unique(("plug_knob", id, param.id)),
            v.p(cx, row_y + 19.0),
            12.0 * z,
            &mut norm,
            param.default as f32,
            false,
            accent,
            z,
        )
        .on_hover_text(&param.name);
        v.hit(resp.rect, ParamRef::Plugin(param.id));
        if resp.dragged() || resp.double_clicked() {
            p.set_param(i, norm as f64, now);
        }
        let text = if p.texts[i].is_empty() { format!("{:.0}%", p.values[i] * 100.0) } else { p.texts[i].clone() };
        let text: String = text.chars().take(10).collect();
        let hot = resp.dragged() || resp.hovered();
        v.text(
            cx,
            row_y + 38.0,
            Align2::CENTER_CENTER,
            text,
            v.mono(9.0),
            if hot { theme::TEXT } else { theme::TEXT_DIM },
        );
    }

    PluginEvents {
        toggle_editor: editor.clicked() && p.meta.editor,
    }
}

/// The PARAMS popup: every parameter, searchable, with a slider and a pin.
fn parameter_list(button: &egui::Response, p: &mut PluginNode, now: f64) {
    Popup::menu(button)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(340.0);
            ui.add(egui::TextEdit::singleline(&mut p.query).hint_text("Search parameters"));
            let query = p.query.trim().to_lowercase();
            let shown: Vec<usize> = (0..p.params.len())
                .filter(|&i| query.is_empty() || p.params[i].name.to_lowercase().contains(&query))
                .collect();
            ui.label(
                RichText::new(format!("{} parameters  ·  ● pins up to {PINNED_MAX} as knobs", shown.len()))
                    .size(10.0)
                    .color(theme::TEXT_FAINT),
            );
            ScrollArea::vertical().max_height(380.0).show_rows(ui, 22.0, shown.len(), |ui, rows| {
                for &i in &shown[rows] {
                    p.refresh(i, now);
                    ui.horizontal(|ui| {
                        let pinned = p.pinned.contains(&i);
                        let full = p.pinned.len() >= PINNED_MAX;
                        let pin_col = if pinned { theme::ACCENT } else { alpha(theme::TEXT_FAINT, 0.8) };
                        let pin = ui
                            .add_enabled(pinned || !full, egui::Button::new(RichText::new("●").color(pin_col)).frame(false))
                            .on_hover_text(if pinned { "Unpin" } else { "Show as a knob" });
                        if pin.clicked() {
                            if pinned {
                                p.pinned.retain(|&k| k != i);
                            } else {
                                p.pinned.push(i);
                            }
                        }
                        let name: String = p.params[i].name.chars().take(28).collect();
                        ui.add_sized([170.0, 18.0], egui::Label::new(RichText::new(name).size(11.0)).truncate());
                        let mut value = p.values[i] as f32;
                        if ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false)).changed() {
                            p.set_param(i, value as f64, now);
                        }
                    });
                }
            });
        });
}
