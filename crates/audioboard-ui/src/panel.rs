//! The generic node panel, laid out from a [`NodeDef`] alone: port labels
//! down the sides, a readout line, the switch, the display inset, extra
//! controls, the mode selector and a grid of knob cells. Built-in modules
//! without a custom panel and every plugin node are drawn on it.
//!
//! ```text
//!  ┌──────────────────────────────┐
//!  │ ▾ ● TITLE               ○  × │  header (card::HEADER_H)
//!  │ IN      main readout     OUT │  PORT_Y
//!  │ CV       sub line  [SWITCH]  │  + PORT_DY per port row
//!  │ ┌──────────────────────────┐ │  display_top
//!  │ │        display           │ │  NodeDef::display_h
//!  │ └──────────────────────────┘ │
//!  │ [ mode | mode | mode ]       │  selector_y
//!  │  KNOB   KNOB   KNOB   KNOB   │  knobs_top, ROW_H per row
//!  └──────────────────────────────┘
//! ```

use egui::{Vec2, vec2};

use audioboard_node::NodeDef;

/// Side padding.
pub const PAD: f32 = 12.0;
/// Width of one knob column.
pub const COL_W: f32 = 64.0;
/// y of the first port row (and the readout).
pub const PORT_Y: f32 = 42.0;
/// Distance between port rows.
pub const PORT_DY: f32 = 18.0;
/// Height of one knob row.
pub const ROW_H: f32 = 58.0;

/// Knob columns: 4 for four knobs or seven and more, else 3.
pub fn columns(def: &NodeDef) -> usize {
    match def.params.len() {
        4 => 4,
        n if n >= 7 => 4,
        _ => 3,
    }
}

/// Node-local top of the display: below the port rows.
pub fn display_top(def: &NodeDef) -> f32 {
    let rows =
        (def.inputs.len() + def.midi_in as usize).max(def.outputs.len() + def.midi_out as usize);
    (PORT_Y + PORT_DY * rows.saturating_sub(1) as f32 + 12.0).max(72.0)
}

pub fn display_bottom(def: &NodeDef) -> f32 {
    display_top(def) + def.display_h
}

/// Node-local bottom of the extra controls under the display, if any.
pub fn controls_bottom(def: &NodeDef) -> f32 {
    display_bottom(def)
        + if def.controls_h > 0.0 {
            6.0 + def.controls_h
        } else {
            0.0
        }
}

/// Node-local top and bottom of the mode selector row.
pub fn selector_y(def: &NodeDef) -> [f32; 2] {
    let d = controls_bottom(def);
    [d + 6.0, d + 26.0]
}

/// Node-local y of the first knob row's labels.
pub fn knobs_top(def: &NodeDef) -> f32 {
    controls_bottom(def)
        + if def.modes.is_empty() && !def.map {
            18.0
        } else {
            42.0
        }
}

/// The panel's size.
pub fn size(def: &NodeDef) -> Vec2 {
    let cols = columns(def);
    let rows = def.params.len().div_ceil(cols);
    vec2(
        PAD * 2.0 + COL_W * cols as f32,
        knobs_top(def) + ROW_H * rows as f32 - 4.0,
    )
}

/// Knob `k`'s cell: (centre x, label y), node-local.
pub fn knob_cell_at(def: &NodeDef, k: usize) -> (f32, f32) {
    let cols = columns(def);
    (
        PAD + COL_W * ((k % cols) as f32 + 0.5),
        knobs_top(def) + ROW_H * (k / cols) as f32,
    )
}

/// Where input `i` sits on the left edge.
pub fn input_offset(i: usize) -> Vec2 {
    vec2(0.0, PORT_Y + PORT_DY * i as f32)
}

/// Where output `i` sits on the right edge of a panel `width` wide.
pub fn output_offset(width: f32, i: usize) -> Vec2 {
    vec2(width, PORT_Y + PORT_DY * i as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use audioboard_node::{ParamSpec, Unit};

    #[test]
    fn panels_grow_with_their_description() {
        const P: [ParamSpec; 5] = [ParamSpec::lin("A", 0.0, 1.0, 0.5, Unit::Percent); 5];
        let small = NodeDef {
            name: "A",
            inputs: &["IN"],
            outputs: &["OUT"],
            params: &P[..3],
            ..NodeDef::EMPTY
        };
        let big = NodeDef {
            params: &P,
            modes: &["X", "Y"],
            display_h: 40.0,
            ..small
        };
        assert_eq!(columns(&small), 3);
        assert!(size(&big).y > size(&small).y + 40.0);
        assert_eq!(knob_cell_at(&big, 3).1, knobs_top(&big) + ROW_H);
        assert!(selector_y(&big)[0] >= display_bottom(&big));
    }
}
