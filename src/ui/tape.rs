//! Panel of the Tape node: a reel-to-reel deck that runs with the sound
//! (reels turning at their real ratio as the tape moves from one to the
//! other, tension arms swaying with wow and flutter, the heads glowing with
//! the signal, a tape counter, STOP and PLAY keys), two VU meters with
//! needles, the machine's switches and its knobs.

use std::f32::consts::TAU;

use egui::{Align2, Color32, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, Vec2, pos2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::tape::{
    self as tp, AGE, BIAS, CASSETTE, FLUTTER, HISS, INPUT, MACHINES, MIX, OUTPUT, SPEEDS, TAPES, WOW, data, viz,
};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(580.0, 456.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The deck.
const DECK: [f32; 4] = [PAD, 54.0, W - PAD, 282.0];
/// Reel centres and sizes: flange, hub, a full pack.
const REELS: [Pos2; 2] = [pos2(142.0, 142.0), pos2(W - 142.0, 142.0)];
const FLANGE: f32 = 76.0;
const HUB: f32 = 17.0;
const FULL: f32 = 66.0;
/// Tension arm rollers (at rest) and their pivots.
const ARMS: [(Pos2, Pos2); 2] = [(pos2(226.0, 232.0), pos2(196.0, 206.0)), (pos2(W - 226.0, 232.0), pos2(W - 196.0, 206.0))];
/// The heads (erase, record, play), the tape's line over them, the capstan.
const HEADS: [f32; 3] = [252.0, 290.0, 328.0];
const TAPE_Y: f32 = 244.0;
const CAPSTAN: Pos2 = pos2(358.0, 244.0);
/// Under the deck: VU meters, switches, knobs.
const VU_Y: [f32; 2] = [292.0, 374.0];
const SWITCH_X: f32 = 340.0;
const KNOB_Y: f32 = 386.0;
/// 0 VU: −12 dBFS RMS.
const VU_REF: f32 = 0.25;

/// Machine colours: reel flanges, the deck's badge.
const SILVER: Color32 = Color32::from_rgb(188, 192, 198);
const PACK: Color32 = Color32::from_rgb(58, 44, 36);
const DECK_COL: Color32 = Color32::from_rgb(36, 38, 42);
const VU_FACE: Color32 = Color32::from_rgb(240, 226, 186);
const VU_INK: Color32 = Color32::from_rgb(40, 34, 28);
const VU_RED: Color32 = Color32::from_rgb(200, 52, 40);

const KNOBS: [(usize, &str, &str); 8] = [
    (INPUT, "INPUT", "Record level: drive the tape"),
    (BIAS, "BIAS", "Under: brighter, grittier · over: cleaner, darker"),
    (WOW, "WOW", "Slow pitch drift of the transport"),
    (FLUTTER, "FLUTTER", "Fast pitch jitter: capstan and pinch roller"),
    (HISS, "HISS", "Tape noise"),
    (AGE, "AGE", "Worn tape: dropouts and a duller top"),
    (OUTPUT, "OUTPUT", "Playback level (with AUTO it follows INPUT)"),
    (MIX, "MIX", "Dry against tape"),
];

/// What moves on screen, kept between frames.
#[derive(Clone, Copy)]
struct Deck {
    last: f64,
    /// Reel angles (turns), the capstan's, how far the tape has wound
    /// (0: all on the left reel), the counter (seconds).
    reels: [f32; 2],
    capstan: f32,
    wound: f32,
    counter: f64,
    /// VU needles: position (0..1 of the scale) and speed.
    needle: [f32; 2],
    swing: [f32; 2],
}

pub fn readout(p: &ModuleParams) -> (String, String) {
    tp::readout(p)
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
}

/// `ports`: the input and the output are patched.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, ports: (bool, bool)) {
    let s = Panel { v, id, p: &m.params, accent };
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "IN", v.font(9.0), col(ports.0));
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), col(ports.1));
    let (main, sub) = tp::readout(s.p);
    v.text(W * 0.5, 42.0, Align2::CENTER_CENTER, format!("{main} · {sub}"), v.font(8.5), theme::TEXT_FAINT);

    let deck = s.animate();
    s.deck(&deck);
    s.meters(&deck);
    s.switches();
    s.knobs();
    let moving = s.p.viz[viz::SPEED].get() > 0.0 || deck.needle.iter().any(|n| *n > 0.002);
    if moving {
        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// Where a level sits on a VU scale (0: −20 VU, 1: +3 VU).
fn vu_pos(rms: f32) -> f32 {
    let lo = 0.1;
    let hi = 1.412_5;
    ((rms / VU_REF - lo) / (hi - lo)).clamp(-0.02, 1.04)
}

fn vu_pos_db(db: f32) -> f32 {
    vu_pos(VU_REF * 10f32.powf(db / 20.0))
}

/// A tape pack's radius when it holds `share` of the tape.
fn pack(share: f32) -> f32 {
    (HUB * HUB + share.clamp(0.0, 1.0) * (FULL * FULL - HUB * HUB)).sqrt()
}

/// The point where a line from `from` touches the circle (`c`, `r`), on the
/// lower side.
fn tangent(c: Pos2, r: f32, from: Pos2) -> Pos2 {
    let d = from - c;
    let len = d.length().max(r + 0.1);
    let base = d.y.atan2(d.x);
    let a = (r / len).acos();
    let p1 = c + vec2((base + a).cos(), (base + a).sin()) * r;
    let p2 = c + vec2((base - a).cos(), (base - a).sin()) * r;
    if p1.y > p2.y { p1 } else { p2 }
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("tape", self.id)).with(salt)
    }

    fn on(&self, i: usize) -> bool {
        tp::on(self.p, i)
    }

    fn cassette(&self) -> bool {
        tp::choice(self.p, data::MACHINE, MACHINES.len()) == CASSETTE
    }

    /// Moves the reels, the counter and the needles on by the time since the
    /// last frame.
    fn animate(&self) -> Deck {
        let (v, p) = (self.v, self.p);
        let now = v.ui.input(|i| i.time);
        let id = self.uid("deck");
        let mut d = v.ui.data(|data| data.get_temp::<Deck>(id)).unwrap_or(Deck {
            last: now,
            reels: [0.0, 0.37],
            capstan: 0.0,
            wound: 0.3 + (self.id % 5) as f32 * 0.08,
            counter: 0.0,
            needle: [0.0; 2],
            swing: [0.0; 2],
        });
        let dt = ((now - d.last) as f32).clamp(0.0, 0.1);
        d.last = now;
        // The tape's speed on screen: faster at 30 ips, slower on cassette.
        let pace = if self.cassette() { 0.8 } else { [0.6, 1.0, 1.6][tp::choice(p, data::SPEED, SPEEDS.len())] };
        let speed = 150.0 * pace * p.viz[viz::SPEED].get();
        // Eight minutes of tape at 15 ips, then it starts over.
        d.wound = (d.wound + speed * dt / (150.0 * 480.0)).fract();
        let radii = [pack(1.0 - d.wound), pack(d.wound)];
        for (a, r) in d.reels.iter_mut().zip(radii) {
            *a = (*a + speed * dt / (TAU * r)).fract();
        }
        d.capstan = (d.capstan + speed * dt / (TAU * 5.0)).fract();
        d.counter += (dt * p.viz[viz::SPEED].get()) as f64;
        // VU ballistics: a damped needle, ~300 ms to the mark.
        for k in 0..2 {
            let at = if k == 0 { viz::VU_IN } else { viz::VU_OUT };
            let rms = p.viz[at].get().max(p.viz[at + 1].get());
            let target = if rms > 1e-5 { vu_pos(rms).max(0.0) } else { 0.0 };
            let steps = (dt / 0.005).ceil().max(1.0) as usize;
            let h = dt / steps as f32;
            let (w0, zeta) = (13.0f32, 0.78f32);
            for _ in 0..steps {
                let acc = w0 * w0 * (target - d.needle[k]) - 2.0 * zeta * w0 * d.swing[k];
                d.swing[k] += acc * h;
                d.needle[k] = (d.needle[k] + d.swing[k] * h).clamp(-0.02, 1.04);
            }
        }
        v.ui.data_mut(|data| data.insert_temp(id, d));
        d
    }

    // -----------------------------------------------------------------------
    // The deck
    // -----------------------------------------------------------------------

    fn deck(&self, d: &Deck) {
        let (v, p, z) = (self.v, self.p, self.z());
        let painter = v.painter;
        let rect = v.r(DECK[0], DECK[1], DECK[2], DECK[3]);
        painter.rect_filled(rect, 6.0 * z, DECK_COL);
        // Brushed metal: faint horizontal lines.
        for k in 0..18 {
            let y = DECK[1] + 6.0 + 12.4 * k as f32;
            painter.hline(rect.x_range().shrink(6.0 * z), v.p(0.0, y).y, Stroke::new(1.0 * z, alpha(Color32::WHITE, 0.018)));
        }
        painter.rect_stroke(rect, 6.0 * z, Stroke::new(1.0 * z, alpha(Color32::WHITE, 0.08)), egui::StrokeKind::Inside);

        let r = p.viz[viz::SPEED].get();
        let wobble = p.viz[viz::WOBBLE].get();
        let radii = [pack(1.0 - d.wound), pack(d.wound)];

        // Tension arms sway with the tape's wobble.
        let rollers: [Pos2; 2] = std::array::from_fn(|k| {
            let (roller, pivot) = ARMS[k];
            let arm = roller - pivot;
            let turn = (wobble * 0.18).clamp(-0.25, 0.25) * if k == 0 { 1.0 } else { -1.0 };
            pivot + vec2(arm.x * turn.cos() - arm.y * turn.sin(), arm.x * turn.sin() + arm.y * turn.cos())
        });

        // The tape: off the left pack, round the arm, over the heads, past
        // the capstan, round the other arm, onto the right pack.
        let tape_col = Color32::from_rgb(92, 66, 48);
        let tape = Stroke::new(2.2 * z, tape_col);
        let from_left = tangent(REELS[0], radii[0], rollers[0]);
        let onto_right = tangent(REELS[1], radii[1], rollers[1]);
        let path = [
            from_left,
            rollers[0] + vec2(0.0, 6.0),
            pos2(HEADS[0] - 16.0, TAPE_Y),
            pos2(CAPSTAN.x, TAPE_Y),
            rollers[1] + vec2(0.0, 6.0),
            onto_right,
        ];
        painter.add(Shape::line(path.iter().map(|q| v.p(q.x, q.y)).collect(), tape));

        for (k, c) in REELS.iter().enumerate() {
            self.reel(*c, radii[k], d.reels[k]);
        }
        for (k, roller) in rollers.iter().enumerate() {
            let pivot = ARMS[k].1;
            painter.line_segment([v.p(pivot.x, pivot.y), v.p(roller.x, roller.y)], Stroke::new(3.0 * z, SILVER));
            painter.circle_filled(v.p(pivot.x, pivot.y), 4.0 * z, Color32::from_rgb(90, 94, 100));
            painter.circle_filled(v.p(roller.x, roller.y), 6.5 * z, SILVER);
            painter.circle_filled(v.p(roller.x, roller.y), 2.5 * z, Color32::from_rgb(70, 72, 78));
        }

        // Heads: erase, record (glows with the input), play (with the output).
        let glow = |at: usize| vu_pos(p.viz[at].get().max(p.viz[at + 1].get())).clamp(0.0, 1.0);
        for (k, (x, label)) in HEADS.iter().zip(["ERASE", "REC", "PLAY"]).enumerate() {
            let head = v.r(x - 12.0, TAPE_Y + 1.0, x + 12.0, TAPE_Y + 22.0);
            painter.rect_filled(head, 3.0 * z, Color32::from_rgb(120, 124, 132));
            painter.rect_filled(v.r(x - 9.0, TAPE_Y + 1.0, x + 9.0, TAPE_Y + 5.0), 1.5 * z, Color32::from_rgb(170, 174, 182));
            let lit = match k {
                1 => Some((Color32::from_rgb(236, 82, 60), glow(viz::VU_IN))),
                2 => Some((Color32::from_rgb(90, 214, 120), glow(viz::VU_OUT))),
                _ => None,
            };
            if let Some((c, g)) = lit.filter(|_| r > 0.0) {
                painter.circle_filled(v.p(*x, TAPE_Y + 1.0), (4.0 + 6.0 * g) * z, alpha(c, 0.15 + 0.5 * g));
            }
            v.text(*x, TAPE_Y + 30.0, Align2::CENTER_CENTER, label, v.font(6.5), theme::TEXT_FAINT);
        }
        // The tape over the heads, on top of their glow.
        painter.line_segment([v.p(HEADS[0] - 16.0, TAPE_Y), v.p(CAPSTAN.x, TAPE_Y)], tape);

        // Capstan and pinch roller.
        let cap = v.p(CAPSTAN.x, CAPSTAN.y);
        painter.circle_filled(cap, 4.5 * z, Color32::from_rgb(214, 218, 224));
        let pinch = v.p(CAPSTAN.x, CAPSTAN.y + 11.0);
        painter.circle_filled(pinch, 8.0 * z, Color32::from_rgb(22, 22, 24));
        let a = -d.capstan * TAU * 0.6;
        painter.line_segment([pinch, pinch + vec2(a.cos(), a.sin()) * 6.0 * z], Stroke::new(1.5 * z, Color32::from_rgb(70, 70, 74)));

        self.centre(d);
    }

    /// One reel: the tape pack under a silver flange with three spokes.
    fn reel(&self, c: Pos2, radius: f32, turns: f32) {
        let (v, z) = (self.v, self.z());
        let painter = v.painter;
        let centre = v.p(c.x, c.y);
        painter.circle_filled(centre, FLANGE * z, alpha(Color32::BLACK, 0.35));
        painter.circle_filled(centre, radius * z, PACK);
        // The pack's wound layers.
        for k in 1..4 {
            let rr = HUB + (radius - HUB) * k as f32 / 4.0;
            painter.circle_stroke(centre, rr * z, Stroke::new(1.0 * z, alpha(Color32::WHITE, 0.04)));
        }
        painter.circle_stroke(centre, radius * z, Stroke::new(1.0 * z, alpha(Color32::from_rgb(160, 120, 90), 0.5)));
        // Flange: rim and three spokes, turning (clockwise).
        painter.circle_stroke(centre, (FLANGE - 2.5) * z, Stroke::new(5.0 * z, SILVER));
        for k in 0..3 {
            let a = (turns + k as f32 / 3.0) * TAU;
            let dir = vec2(a.cos(), a.sin());
            let side = vec2(-dir.y, dir.x);
            let (r0, r1) = (HUB + 2.0, FLANGE - 4.0);
            let quad = vec![
                centre + (dir * r0 + side * 7.0) * z,
                centre + (dir * r1 + side * 15.0) * z,
                centre + (dir * r1 - side * 15.0) * z,
                centre + (dir * r0 - side * 7.0) * z,
            ];
            painter.add(Shape::convex_polygon(quad, alpha(SILVER, 0.82), Stroke::NONE));
            // A bolt near the rim shows the turning.
            painter.circle_filled(centre + dir * (FLANGE - 12.0) * z, 2.0 * z, Color32::from_rgb(110, 114, 120));
        }
        painter.circle_filled(centre, HUB * z, SILVER);
        painter.circle_filled(centre, 7.0 * z, Color32::from_rgb(60, 62, 68));
        for k in 0..3 {
            let a = (turns + k as f32 / 3.0 + 1.0 / 6.0) * TAU;
            let at = centre + vec2(a.cos(), a.sin()) * 7.0 * z;
            painter.circle_filled(at, 2.2 * z, SILVER);
        }
    }

    /// Between the reels: the badge, the counter and the transport keys.
    fn centre(&self, d: &Deck) {
        let (v, p, z) = (self.v, self.p, self.z());
        let painter = v.painter;
        let cx = W * 0.5;
        v.text(cx, 80.0, Align2::CENTER_CENTER, "TAPE", theme::strong(15.0 * z), alpha(Color32::WHITE, 0.82));
        let m = tp::machine(p);
        v.text(cx, 96.0, Align2::CENTER_CENTER, format!("{} · {} ips", m.name, tp::speed_name(p)), v.font(7.5), self.accent);

        // Tape counter: minutes and seconds on black wheels.
        let secs = d.counter as u64;
        let digits = format!("{:02}{:02}", (secs / 60) % 100, secs % 60);
        let box_ = v.r(cx - 30.0, 108.0, cx + 30.0, 126.0);
        painter.rect_filled(box_, 2.0 * z, Color32::from_rgb(14, 14, 16));
        for (k, ch) in digits.chars().enumerate() {
            let x = cx - 22.5 + 15.0 * k as f32;
            painter.rect_filled(v.r(x - 6.0, 110.0, x + 6.0, 124.0), 1.5 * z, Color32::from_rgb(34, 34, 38));
            v.text(x, 117.0, Align2::CENTER_CENTER, ch, v.mono(10.0), Color32::from_rgb(236, 236, 230));
        }

        // Transport keys.
        let running = self.on(data::RUNNING);
        let keys = [("■", false, "Stop: the tape slows to a halt"), ("▶", true, "Play: the tape spins back up")];
        for (k, (label, play, tip)) in keys.into_iter().enumerate() {
            let x0 = cx - 44.0 + 46.0 * k as f32;
            let rect = v.r(x0, 140.0, x0 + 42.0, 164.0);
            let lit = play == running;
            let resp = v.ui.interact(rect, self.uid(("key", k)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip);
            let face = if resp.hovered() { Color32::from_rgb(70, 72, 78) } else { Color32::from_rgb(54, 56, 62) };
            painter.rect_filled(rect.translate(vec2(0.0, 2.0) * z), 3.0 * z, Color32::from_rgb(18, 18, 20));
            painter.rect_filled(rect, 3.0 * z, face);
            let lamp = if play { Color32::from_rgb(96, 214, 120) } else { Color32::from_rgb(232, 76, 60) };
            painter.text(rect.center(), Align2::CENTER_CENTER, label, theme::font(11.0 * z), if lit { lamp } else { theme::TEXT_FAINT });
            if lit {
                painter.circle_filled(rect.center_top() + vec2(0.0, -4.0) * z, 2.0 * z, lamp);
            }
            if resp.clicked() {
                p.data[data::RUNNING].set(if play { 1.0 } else { 0.0 });
            }
        }

        // Saturation lamp.
        let sat = (-p.viz[viz::SAT].get() / 4.0).clamp(0.0, 1.0);
        let lamp = v.p(cx - 16.0, 184.0);
        painter.circle_filled(lamp, 4.0 * z, theme::mix(Color32::from_rgb(70, 30, 26), Color32::from_rgb(255, 120, 60), sat));
        if sat > 0.05 {
            painter.circle_filled(lamp, (5.0 + 5.0 * sat) * z, alpha(Color32::from_rgb(255, 120, 60), 0.25 * sat));
        }
        v.text(cx - 8.0, 184.0, Align2::LEFT_CENTER, "SAT", v.font(7.0), theme::TEXT_FAINT);
        let drop = p.viz[viz::DROP].get();
        if drop > 0.05 {
            v.text(cx, 198.0, Align2::CENTER_CENTER, "dropout", v.font(7.0), alpha(theme::METER_YELLOW, drop.min(1.0)));
        }
    }

    // -----------------------------------------------------------------------
    // VU meters
    // -----------------------------------------------------------------------

    fn meters(&self, d: &Deck) {
        for (k, label) in ["INPUT", "OUTPUT"].into_iter().enumerate() {
            let x0 = PAD + 160.0 * k as f32;
            self.vu(Rect::from_min_max(pos2(x0, VU_Y[0]), pos2(x0 + 152.0, VU_Y[1])), d.needle[k], label);
        }
    }

    /// An analog VU meter in `rect` (node units), needle at `pos`.
    fn vu(&self, rect: Rect, pos: f32, label: &str) {
        let (v, z) = (self.v, self.z());
        let painter = v.painter;
        let frame = v.r(rect.left(), rect.top(), rect.right(), rect.bottom());
        painter.rect_filled(frame, 4.0 * z, Color32::from_rgb(20, 20, 22));
        let face = frame.shrink(4.0 * z);
        painter.rect_filled(face, 3.0 * z, VU_FACE);
        // The needle's pivot sits under the face.
        let pivot = pos2(rect.center().x, rect.bottom() + 16.0);
        let radius = 74.0;
        let span = 0.75f32;
        let at = |t: f32, r: f32| {
            let a = -std::f32::consts::FRAC_PI_2 + (t - 0.5) * 2.0 * span;
            let q = pivot + vec2(a.cos(), a.sin()) * r;
            v.p(q.x, q.y)
        };
        let clip = painter.with_clip_rect(face.intersect(painter.clip_rect()));
        // Scale: black arc, red past 0 VU.
        let zero = vu_pos_db(0.0);
        let arc = |t0: f32, t1: f32, r: f32| (0..=24).map(|k| at(t0 + (t1 - t0) * k as f32 / 24.0, r)).collect::<Vec<_>>();
        clip.add(Shape::line(arc(0.0, zero, radius), Stroke::new(1.2 * z, VU_INK)));
        clip.add(Shape::line(arc(zero, 1.0, radius), Stroke::new(3.0 * z, VU_RED)));
        for (db, label) in [(-20.0, "20"), (-10.0, "10"), (-7.0, "7"), (-5.0, "5"), (-3.0, "3"), (-2.0, ""), (-1.0, ""), (0.0, "0"), (1.0, ""), (2.0, ""), (3.0, "+3")] {
            let t = vu_pos_db(db);
            let ink = if db > 0.0 { VU_RED } else { VU_INK };
            clip.line_segment([at(t, radius), at(t, radius + 7.0)], Stroke::new(1.0 * z, ink));
            if !label.is_empty() {
                clip.text(at(t, radius + 14.0), Align2::CENTER_CENTER, label, theme::font(6.5 * z), ink);
            }
        }
        clip.text(v.p(rect.center().x, rect.bottom() - 18.0), Align2::CENTER_CENTER, "VU", theme::strong(9.0 * z), VU_INK);
        clip.text(v.p(rect.left() + 10.0, rect.bottom() - 10.0), Align2::LEFT_CENTER, label, theme::font(6.5 * z), alpha(VU_INK, 0.7));
        // Needle.
        clip.line_segment([v.p(pivot.x, pivot.y), at(pos, radius + 10.0)], Stroke::new(1.3 * z, Color32::from_rgb(24, 20, 18)));
        // Peak lamp: past +3 VU.
        let hot = pos >= 0.99;
        let lamp = v.p(rect.right() - 12.0, rect.top() + 11.0);
        painter.circle_filled(lamp, 3.0 * z, if hot { Color32::from_rgb(255, 70, 50) } else { Color32::from_rgb(110, 60, 50) });
        // Warm glass.
        painter.rect_stroke(face, 3.0 * z, Stroke::new(1.0 * z, alpha(Color32::BLACK, 0.3)), egui::StrokeKind::Inside);
    }

    // -----------------------------------------------------------------------
    // Switches and knobs
    // -----------------------------------------------------------------------

    fn seg(&self, salt: &str, rect: Rect, items: &[(&'static str, &'static str)], current: usize) -> Option<usize> {
        let v = self.v;
        segmented(v, self.uid(("seg", salt)), rect, items.len(), current, self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, items[k].0, v.font(7.5), col);
            items[k].1
        })
    }

    fn switches(&self) {
        let (v, p) = (self.v, self.p);
        let x0 = SWITCH_X + 52.0;
        let x1 = W - PAD;
        let row = |k: usize| VU_Y[0] + 2.0 + 20.0 * k as f32;
        let label = |k: usize, text: &str| v.text(SWITCH_X, row(k) + 8.0, Align2::LEFT_CENTER, text, v.font(7.5), theme::TEXT_FAINT);

        label(0, "MACHINE");
        let machines: Vec<(&str, &str)> = MACHINES.iter().map(|m| (m.name, m.hint)).collect();
        if let Some(k) = self.seg("machine", v.r(x0, row(0), x1, row(0) + 16.0), &machines, tp::choice(p, data::MACHINE, MACHINES.len())) {
            p.data[data::MACHINE].set(k as f32);
        }
        label(1, "SPEED");
        if self.cassette() {
            v.text(x0 + 4.0, row(1) + 8.0, Align2::LEFT_CENTER, "1⅞ ips (cassette)", v.font(7.5), theme::TEXT_DIM);
        } else {
            let speeds = [("7½ IPS", "7½ ips: darker, lower head bump, more wow"), ("15 IPS", "15 ips: the classic"), ("30 IPS", "30 ips: open top, tight low end")];
            if let Some(k) = self.seg("speed", v.r(x0, row(1), x1, row(1) + 16.0), &speeds, tp::choice(p, data::SPEED, SPEEDS.len())) {
                p.data[data::SPEED].set(k as f32);
            }
        }
        label(2, "TAPE");
        let tapes: Vec<(&str, &str)> = TAPES.iter().map(|t| (t.0, t.1)).collect();
        if let Some(k) = self.seg("tape", v.r(x0, row(2), x1, row(2) + 16.0), &tapes, tp::choice(p, data::TAPE, TAPES.len())) {
            p.data[data::TAPE].set(k as f32);
        }
        label(3, "EQ");
        let mid = x0 + 104.0;
        if self.cassette() {
            v.text(x0 + 4.0, row(3) + 8.0, Align2::LEFT_CENTER, "120 µs", v.font(7.5), theme::TEXT_DIM);
        } else {
            let eqs = [("NAB", "American curve: fuller lows"), ("IEC", "European (CCIR) curve")];
            if let Some(k) = self.seg("eq", v.r(x0, row(3), mid, row(3) + 16.0), &eqs, tp::choice(p, data::EQ, 2)) {
                p.data[data::EQ].set(k as f32);
            }
        }
        let auto = self.on(data::AUTO);
        let resp = widgets::toggle(v.ui, v.painter, self.uid("auto"), v.r(mid + 8.0, row(3), x1, row(3) + 16.0), "AUTO GAIN", auto, self.accent, v.z)
            .on_hover_text("OUTPUT follows INPUT: drive the tape without getting louder");
        if resp.clicked() {
            p.data[data::AUTO].set(if auto { 0.0 } else { 1.0 });
        }
    }

    fn knobs(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let cw = (W - 2.0 * PAD) / KNOBS.len() as f32;
        for (k, (i, label, tip)) in KNOBS.iter().enumerate() {
            let cx = PAD + cw * (k as f32 + 0.5);
            let spec = tp::spec(*i);
            v.text(cx, KNOB_Y, Align2::CENTER_CENTER, label, v.font(7.5), theme::TEXT_FAINT);
            let mut norm = spec.to_norm(p.get(*i));
            let r = 14.0;
            let resp = widgets::knob(
                v.ui,
                v.painter,
                self.uid(("knob", i)),
                v.p(cx, KNOB_Y + 8.0 + r),
                r * z,
                &mut norm,
                spec.to_norm(spec.default),
                spec.bipolar,
                self.accent,
                z,
            )
            .on_hover_text(*tip);
            v.hit(resp.rect, ParamRef::Module(*i));
            if resp.dragged() || resp.double_clicked() {
                p.set(*i, spec.denorm(norm));
            }
            let hot = resp.dragged() || resp.hovered();
            v.text(cx, KNOB_Y + 16.0 + 2.0 * r, Align2::CENTER_CENTER, tp::format(p, *i), v.mono(7.5), if hot { theme::TEXT } else { theme::TEXT_DIM });
        }
        // What the tape is doing.
        let sat = p.viz[viz::SAT].get();
        let state = if p.viz[viz::SPEED].get() <= 0.0 {
            "stopped".to_owned()
        } else if sat < -0.3 {
            format!("tape squeezing {sat:.1} dB")
        } else {
            "clean".to_owned()
        };
        v.text(W * 0.5, SIZE.y - 10.0, Align2::CENTER_CENTER, state, v.font(7.5), theme::TEXT_FAINT);
    }
}
