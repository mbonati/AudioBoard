//! [`NodeView`]: draws a node panel in node units, whatever the canvas zoom.

use std::cell::RefCell;

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Ui, Vec2, vec2};

use crate::theme;

/// Text smaller than this (in screen points) is skipped: unreadable when
/// zoomed far out, and costly to lay out.
pub const MIN_TEXT: f32 = 4.0;

/// A control drawn this frame, so the host can map MIDI to it (click one to
/// map it). `P` is the host's name for the control.
#[derive(Clone, Copy, Debug)]
pub struct Hit<P> {
    pub rect: Rect,
    pub node: u32,
    pub param: P,
}

/// Maps node-local coordinates (node units, origin at the node's top-left
/// corner) to the screen. Every panel draws through one: positions with
/// [`NodeView::p`] and [`NodeView::r`], sizes times [`NodeView::z`].
pub struct NodeView<'a, P = ()> {
    pub ui: &'a Ui,
    pub painter: &'a Painter,
    /// The node's top-left corner on screen.
    pub min: Pos2,
    /// The canvas zoom.
    pub z: f32,
    pub node: u32,
    /// Collects every knob and fader drawn.
    pub hits: &'a RefCell<Vec<Hit<P>>>,
    /// The global tempo, for tempo-synced displays.
    pub tempo: f32,
}

impl<P> NodeView<'_, P> {
    /// A point in node units, on screen.
    pub fn p(&self, x: f32, y: f32) -> Pos2 {
        self.min + vec2(x, y) * self.z
    }

    /// A rectangle in node units, on screen.
    pub fn r(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.p(x0, y0), self.p(x1, y1))
    }

    /// Records a control for MIDI mapping.
    pub fn hit(&self, rect: Rect, param: P) {
        self.hits.borrow_mut().push(Hit {
            rect,
            node: self.node,
            param,
        });
    }

    /// The proportional font at a size in node units.
    pub fn font(&self, size: f32) -> FontId {
        theme::font(size * self.z)
    }

    /// The monospace font at a size in node units.
    pub fn mono(&self, size: f32) -> FontId {
        theme::mono(size * self.z)
    }

    /// Text at a point in node units. Text too small to read when zoomed
    /// far out is skipped.
    pub fn text(
        &self,
        x: f32,
        y: f32,
        align: Align2,
        text: impl ToString,
        font: FontId,
        color: Color32,
    ) -> Rect {
        if font.size < MIN_TEXT {
            return Rect::from_center_size(self.p(x, y), Vec2::ZERO);
        }
        self.painter.text(self.p(x, y), align, text, font, color)
    }
}
