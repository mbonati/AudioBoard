//! The global tempo in the toolbar: a BPM field that is dragged to change
//! the tempo or clicked to type it, a beat light, and a TAP button. While
//! the transport follows a MIDI clock, the clock's tempo instead.

use egui::{
    Align2, Color32, CursorIcon, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};

use super::theme;
use crate::audio::params::{DEFAULT_TEMPO, TEMPO_RANGE};

/// Taps further apart than this start a new count.
const TAP_RESET: f64 = 2.0;
/// Intervals averaged for the tempo.
const TAP_INTERVALS: usize = 4;

/// Tap tempo: the tempo of the last few taps.
#[derive(Default)]
pub struct TapTempo {
    taps: Vec<f64>,
}

impl TapTempo {
    /// Records a tap at `now` (seconds); the tempo once there are two.
    pub fn tap(&mut self, now: f64) -> Option<f32> {
        if self
            .taps
            .last()
            .is_some_and(|&last| now - last > TAP_RESET || now <= last)
        {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > TAP_INTERVALS + 1 {
            self.taps.remove(0);
        }
        let (first, last) = (*self.taps.first()?, *self.taps.last()?);
        let intervals = self.taps.len() - 1;
        (intervals > 0).then(|| {
            let bpm = 60.0 * intervals as f64 / (last - first);
            round_tempo(bpm as f32)
        })
    }

    /// Taps counted so far (for the button's hint).
    pub fn count(&self, now: f64) -> usize {
        match self.taps.last() {
            Some(&last) if now - last <= TAP_RESET => self.taps.len(),
            _ => 0,
        }
    }
}

/// A tempo clamped to the range, to a tenth of a BPM.
pub fn round_tempo(bpm: f32) -> f32 {
    let (slowest, fastest) = TEMPO_RANGE;
    (bpm.clamp(slowest, fastest) * 10.0).round() / 10.0
}

/// A typed tempo: "120", "97.5" or "97,5".
pub fn parse_tempo(text: &str) -> Option<f32> {
    let bpm: f32 = text.trim().replace(',', ".").parse().ok()?;
    bpm.is_finite().then(|| round_tempo(bpm))
}

fn format_tempo(bpm: f32) -> String {
    if bpm.fract() == 0.0 {
        format!("{bpm:.0}")
    } else {
        format!("{bpm:.1}")
    }
}

#[derive(Default)]
pub struct TempoControl {
    /// The text being typed, while typing.
    editing: Option<String>,
    /// Select all of it once the field has focus.
    select: bool,
    taps: TapTempo,
    /// When the TAP button was last hit, for its flash.
    tapped_at: f64,
    /// A followed MIDI clock's tempo as shown (smoothed).
    external_bpm: Option<f32>,
}

impl TempoControl {
    /// A tap from the keyboard shortcut.
    pub fn tap(&mut self, tempo: &mut f32, now: f64) {
        self.tapped_at = now;
        if let Some(bpm) = self.taps.tap(now) {
            *tempo = bpm;
        }
    }

    /// Draws the field and the TAP button in a right-to-left toolbar.
    /// `beat` is the transport position while audio runs. Returns whether
    /// the pointer is on them, so the window doesn't move under a drag.
    pub fn show(&mut self, ui: &mut Ui, tempo: &mut f32, beat: Option<f32>, now: f64) -> bool {
        let tap = self.tap_button(ui, now);
        if tap.clicked() {
            self.tap(tempo, now);
        }
        ui.add_space(4.0);
        let field = self.field(ui, tempo, beat);
        tap.hovered() || field.hovered() || field.dragged() || self.editing.is_some()
    }

    /// While the transport follows a MIDI clock: the clock's tempo (None:
    /// none coming) in the field, read-only, and SYNC where TAP was, both
    /// outlined in the accent. Returns whether either was clicked (the app
    /// opens Settings → Sync).
    pub fn show_external(
        &mut self,
        ui: &mut Ui,
        bpm: Option<f32>,
        beat: Option<f32>,
        source: &str,
        now: f64,
    ) -> bool {
        // Smoothed for reading; a jump is shown at once.
        if let Some(b) = bpm {
            let shown = self.external_bpm.get_or_insert(b);
            *shown = if (*shown - b).abs() > 3.0 {
                b
            } else {
                *shown + (b - *shown) * 0.08
            };
        } else {
            self.external_bpm = None;
        }
        let tip = match bpm {
            Some(_) => format!("Following the MIDI clock from {source}\nClick for Settings → Sync"),
            None => format!("Waiting for a MIDI clock from {source}\nClick for Settings → Sync"),
        };
        let (tag, tag_resp) = ui.allocate_exact_size(vec2(40.0, 24.0), Sense::CLICK);
        ui.add_space(4.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(92.0, 24.0), Sense::CLICK);
        let p = ui.painter();
        let border = if bpm.is_some() {
            theme::ACCENT
        } else {
            theme::NODE_BORDER
        };
        for (r, hot) in [(tag, tag_resp.hovered()), (rect, resp.hovered())] {
            p.rect(
                r,
                12.0,
                if hot {
                    theme::CONTROL
                } else {
                    Color32::TRANSPARENT
                },
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );
        }
        p.text(
            tag.center(),
            Align2::CENTER_CENTER,
            "SYNC",
            FontId::proportional(9.5),
            if bpm.is_some() {
                theme::ACCENT
            } else {
                theme::TEXT_DIM
            },
        );
        let flash = beat.map_or(0.0, |b| (1.0 - b.fract() * 4.0).max(0.0));
        p.circle_filled(
            pos2(rect.left() + 13.0, rect.center().y),
            3.0,
            theme::mix(theme::TEXT_FAINT, theme::ACCENT, flash),
        );
        let text = self.external_bpm.map_or_else(
            || "– –".to_owned(),
            |b| format_tempo((b * 10.0).round() / 10.0),
        );
        p.text(
            pos2(rect.left() + 23.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::monospace(11.5),
            theme::TEXT,
        );
        p.text(
            pos2(rect.right() - 11.0, rect.center().y + 0.5),
            Align2::RIGHT_CENTER,
            "BPM",
            FontId::proportional(9.5),
            theme::TEXT_FAINT,
        );
        if bpm.is_some() || now.fract() < 0.5 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let clicked = tag_resp.clicked() || resp.clicked();
        tag_resp
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(&tip);
        resp.on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(&tip);
        clicked
    }

    fn tap_button(&self, ui: &mut Ui, now: f64) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 24.0), Sense::CLICK);
        let count = self.taps.count(now);
        let hint = match count {
            0 => "Tap tempo: click (or press T) on each beat".to_owned(),
            1 => "Keep tapping…".to_owned(),
            n => format!("{n} taps"),
        };
        let resp = resp
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(hint);
        // A short flash on each tap.
        let flash = (1.0 - (now - self.tapped_at) / 0.25).clamp(0.0, 1.0) as f32;
        if flash > 0.0 {
            ui.ctx().request_repaint();
        }
        let p = ui.painter();
        let fill = if resp.hovered() {
            theme::CONTROL
        } else {
            theme::PANEL
        };
        p.rect(
            rect,
            12.0,
            theme::mix(fill, theme::mix(theme::PANEL, theme::ACCENT, 0.35), flash),
            Stroke::new(1.0, theme::mix(theme::NODE_BORDER, theme::ACCENT, flash)),
            StrokeKind::Inside,
        );
        let color = if count > 0 {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        };
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "TAP",
            FontId::proportional(10.0),
            color,
        );
        resp
    }

    fn field(&mut self, ui: &mut Ui, tempo: &mut f32, beat: Option<f32>) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(84.0, 24.0), Sense::CLICK | Sense::DRAG);
        let typing = self.editing.is_some();
        let resp = if typing {
            resp
        } else {
            resp.on_hover_cursor(CursorIcon::ResizeVertical)
                .on_hover_text(
                    "Tempo for sequencers, arpeggiators and synced plugins\n\
                 Drag to change (Shift: fine) · click to type · Alt+click: 90",
                )
        };
        if !typing {
            if resp.dragged() {
                let fine = ui.input(|i| i.modifiers.shift);
                let d = resp.drag_delta();
                let (slowest, fastest) = TEMPO_RANGE;
                *tempo =
                    (*tempo + (d.x - d.y) * if fine { 0.05 } else { 0.4 }).clamp(slowest, fastest);
            } else {
                *tempo = round_tempo(*tempo);
            }
            if resp.clicked() {
                if ui.input(|i| i.modifiers.alt) {
                    *tempo = DEFAULT_TEMPO;
                } else {
                    self.editing = Some(format_tempo(*tempo));
                    self.select = true;
                }
            }
        }

        let p = ui.painter();
        let hot = resp.hovered() || resp.dragged() || typing;
        p.rect(
            rect,
            12.0,
            if hot {
                theme::CONTROL
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(
                1.0,
                if typing {
                    theme::ACCENT
                } else {
                    theme::NODE_BORDER
                },
            ),
            StrokeKind::Inside,
        );
        // Bright on the beat, fading over it.
        let flash = beat.map_or(0.0, |b| (1.0 - b.fract() * 4.0).max(0.0));
        p.circle_filled(
            pos2(rect.left() + 13.0, rect.center().y),
            3.0,
            theme::mix(theme::TEXT_FAINT, theme::ACCENT, flash),
        );
        let x = rect.left() + 23.0;
        p.text(
            pos2(rect.right() - 11.0, rect.center().y + 0.5),
            Align2::RIGHT_CENTER,
            "BPM",
            FontId::proportional(9.5),
            theme::TEXT_FAINT,
        );
        if typing {
            // One text row, centred like the number it replaces.
            let row = Rect::from_x_y_ranges(
                x - 2.0..=rect.right() - 34.0,
                rect.center().y - 7.0..=rect.center().y + 9.0,
            );
            self.text_entry(ui, row, tempo);
        } else {
            p.text(
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                format_tempo(*tempo),
                FontId::monospace(11.5),
                theme::TEXT,
            );
        }
        resp
    }

    /// Typing a tempo: Enter or a click elsewhere sets it, Esc cancels.
    fn text_entry(&mut self, ui: &mut Ui, rect: Rect, tempo: &mut f32) {
        let Some(text) = self.editing.as_mut() else {
            return;
        };
        let id = Id::unique("tempo_entry");
        let edit = egui::TextEdit::singleline(text)
            .id(id)
            .font(FontId::monospace(11.5))
            .text_color(theme::TEXT)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::symmetric(2, 0))
            .char_limit(6)
            .desired_width(rect.width());
        let resp = ui.place(rect, edit);
        if self.select {
            resp.request_focus();
            if resp.has_focus() {
                let mut state =
                    egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                state.cursor.set_char_range(Some(
                    egui::text_selection::CCursorRange::select_all_str(text),
                ));
                state.store(ui.ctx(), id);
                self.select = false;
            }
            return;
        }
        if resp.lost_focus() {
            if !ui.input(|i| i.key_pressed(egui::Key::Escape))
                && let Some(bpm) = parse_tempo(text)
            {
                *tempo = bpm;
            }
            self.editing = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tapping_sets_the_tempo() {
        let mut t = TapTempo::default();
        assert_eq!(t.tap(10.0), None, "one tap is not a tempo");
        assert_eq!(t.tap(10.5), Some(120.0));
        assert_eq!(t.tap(11.0), Some(120.0));
        // Uneven taps average out: three intervals over 1.52 s.
        assert_eq!(t.tap(11.52), Some(118.4));
        // A long pause starts over.
        assert_eq!(t.tap(20.0), None);
        assert_eq!(t.tap(20.6), Some(100.0));
        // Only the last few intervals count.
        let mut t = TapTempo::default();
        for k in 0..10 {
            t.tap(k as f64 * 0.75);
        }
        assert_eq!(t.tap(7.5), Some(80.0));
        assert_eq!(t.count(7.6), 5);
        assert_eq!(t.count(20.0), 0);
    }

    /// Frames of the control alone, in a right-to-left bar like the
    /// toolbar's; returns whether the pointer was on it.
    struct Bar {
        ctx: egui::Context,
        control: TempoControl,
        time: f64,
    }

    impl Bar {
        fn frame(&mut self, tempo: &mut f32, events: Vec<egui::Event>) -> bool {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 36.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut on = false;
            let (control, now) = (&mut self.control, self.time);
            let mut out = self.ctx.run_ui(input, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    on = control.show(ui, tempo, None, now);
                });
            });
            out.textures_delta.clear();
            on
        }
    }

    fn press(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn the_field_is_dragged_typed_and_tapped() {
        let mut bar = Bar {
            ctx: egui::Context::default(),
            control: TempoControl::default(),
            time: 0.0,
        };
        let mut tempo = 90.0;
        bar.frame(&mut tempo, vec![]);
        // Right to left: TAP (40) at the right edge, a gap, then the field (84).
        let field = egui::pos2(400.0 - 40.0 - 4.0 - 42.0, 18.0);
        let tap = egui::pos2(400.0 - 20.0, 18.0);
        let none = egui::Modifiers::NONE;

        // The pointer on it keeps the window from moving.
        assert!(bar.frame(&mut tempo, vec![egui::Event::PointerMoved(field)]));
        assert!(!bar.frame(
            &mut tempo,
            vec![egui::Event::PointerMoved(egui::pos2(20.0, 18.0))]
        ));

        // Dragging up raises it.
        bar.frame(
            &mut tempo,
            vec![egui::Event::PointerMoved(field), press(field, true, none)],
        );
        for k in 1..=5 {
            bar.frame(
                &mut tempo,
                vec![egui::Event::PointerMoved(field - vec2(0.0, 5.0 * k as f32))],
            );
        }
        bar.frame(
            &mut tempo,
            vec![press(field - vec2(0.0, 25.0), false, none)],
        );
        assert!((95.0..=100.0).contains(&tempo), "{tempo}");

        // A click opens typing: Enter sets it.
        bar.frame(
            &mut tempo,
            vec![egui::Event::PointerMoved(field), press(field, true, none)],
        );
        bar.frame(&mut tempo, vec![press(field, false, none)]);
        bar.frame(&mut tempo, vec![]);
        bar.frame(&mut tempo, vec![egui::Event::Text("128.5".into())]);
        bar.frame(&mut tempo, vec![key(egui::Key::Enter)]);
        bar.frame(&mut tempo, vec![]);
        assert_eq!(tempo, 128.5);

        // Esc cancels.
        bar.frame(&mut tempo, vec![press(field, true, none)]);
        bar.frame(&mut tempo, vec![press(field, false, none)]);
        bar.frame(&mut tempo, vec![]);
        bar.frame(&mut tempo, vec![egui::Event::Text("60".into())]);
        bar.frame(&mut tempo, vec![key(egui::Key::Escape)]);
        bar.frame(&mut tempo, vec![]);
        assert_eq!(tempo, 128.5);

        // Alt+click resets.
        let alt = egui::Modifiers { alt: true, ..none };
        bar.frame(
            &mut tempo,
            vec![
                egui::Event::ModifiersChanged(alt),
                egui::Event::PointerMoved(field),
                press(field, true, alt),
            ],
        );
        bar.frame(&mut tempo, vec![press(field, false, alt)]);
        bar.frame(&mut tempo, vec![egui::Event::ModifiersChanged(none)]);
        assert_eq!(tempo, DEFAULT_TEMPO);

        // Tapping every half second: 120 BPM.
        for _ in 0..3 {
            bar.frame(
                &mut tempo,
                vec![egui::Event::PointerMoved(tap), press(tap, true, none)],
            );
            bar.frame(&mut tempo, vec![press(tap, false, none)]);
            for _ in 0..8 {
                bar.frame(&mut tempo, vec![]);
            }
        }
        assert_eq!(tempo, 120.0);
    }

    #[test]
    fn typed_tempos() {
        assert_eq!(parse_tempo(" 128 "), Some(128.0));
        assert_eq!(parse_tempo("97,5"), Some(97.5));
        assert_eq!(parse_tempo("97.55"), Some(97.6));
        assert_eq!(parse_tempo("1000"), Some(TEMPO_RANGE.1));
        assert_eq!(parse_tempo("abc"), None);
        assert_eq!(parse_tempo(""), None);
    }
}
