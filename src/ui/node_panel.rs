//! What a plugin node's generic panel shows beyond knobs and ports: its
//! display, drawn from its description with the UI Kit's display primitives
//! (`audioboard_ui::display`), and its readout line.

use egui::{Color32, Painter, Rect};

use audioboard_node::{Display, ParamSpec, Readout};
use audioboard_ui::display::Screen;

use super::theme;
use crate::audio::modules::ModuleParams;
use crate::audio::node_plugin::{ExtId, ext};
use crate::graph::{ModuleNode, TRACE_LEN};

/// Draws plugin node `e`'s display in `rect` (already padded and clipped).
pub fn display(painter: &Painter, rect: Rect, accent: Color32, z: f32, m: &ModuleNode, e: ExtId) {
    let node = ext(e);
    let s = Screen {
        p: painter,
        rect,
        accent,
        z,
    };
    let viz = |i: usize| m.params.viz.get(i).map_or(0.0, |v| v.get());
    match node.panel.display {
        Display::None => {}
        Display::Level => s.level(&m.meter),
        Display::Bars { from, count } => s.bars((from..from + count).map(viz)),
        Display::Curve { from, count } => s.curve((from..from + count).map(viz)),
        Display::Trace { .. } => s.trace(m.trace.iter().copied(), TRACE_LEN),
        Display::Dot { x, y } => s.dot(viz(x), viz(y)),
    }
    if let Some(trouble) = node.trouble() {
        painter.rect_filled(rect, 2.0 * z, theme::alpha(theme::INSET, 0.8));
        s.message(trouble, theme::METER_RED);
    }
}

/// The readout's two lines for plugin node `e`.
pub fn readout(p: &ModuleParams, e: ExtId) -> (String, String) {
    let node = ext(e);
    let def = node.def;
    let mode = def.modes.get(p.mode()).copied().unwrap_or("");
    let main = match node.panel.readout {
        Readout::Name => def.name.to_owned(),
        Readout::Param(i) => def
            .params
            .get(i)
            .map(|spec| spec.format(p.get(i)))
            .unwrap_or_default(),
        Readout::Viz { index, unit } => {
            let v = p.viz.get(index).map_or(0.0, |v| v.get());
            ParamSpec::lin("", f32::MIN, f32::MAX, 0.0, unit).format(v)
        }
        Readout::Mode => mode.to_owned(),
    };
    let sub = if let Some(trouble) = node.trouble() {
        trouble.to_owned()
    } else if !mode.is_empty() && node.panel.readout != Readout::Mode {
        mode.to_owned()
    } else if def.hint.is_empty() {
        node.maker.to_owned()
    } else {
        def.hint.to_owned()
    };
    (main, sub)
}

/// The display value plugin node `e` traces, if its display is a trace.
pub fn traced(e: ExtId) -> Option<usize> {
    match ext(e).panel.display {
        Display::Trace { index } => Some(index),
        _ => None,
    }
}
