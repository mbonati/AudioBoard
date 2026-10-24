//! Panel of the Tape node, in two views picked in its header: COMPACT (the
//! deck and five knobs) and FULL (plus the switches and every knob). The
//! deck is only sketched: two reels turning as the tape winds from one to
//! the other, the tape line swaying with wow and flutter over a head that
//! glows with the signal, two needles on bare arcs for the VU meters, and
//! the STOP and PLAY keys.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use egui::{Align2, Color32, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, Vec2, pos2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::tape::{
    self as tp, AGE, BIAS, CASSETTE, FLUTTER, HISS, INPUT, MACHINES, MIX, OUTPUT, SATURATION, SPEEDS, TAPES, WOW, data, viz,
};
use crate::graph::{ModuleNode, ParamRef};

const PAD: f32 = 12.0;
/// 0 VU: −12 dBFS RMS.
const VU_REF: f32 = 0.25;
/// Tape packs, as a share of the reel: empty (the hub) and full.
const HUB: f32 = 0.22;
const FULL_PACK: f32 = 0.86;
const SAT_COL: Color32 = Color32::from_rgb(255, 128, 64);

/// Where things sit in a view.
struct Layout {
    size: Vec2,
    /// The deck's band, and in it the reels, the meters, the tape's line,
    /// the keys.
    deck: [f32; 2],
    reels: [Pos2; 2],
    reel_r: f32,
    meters: [Pos2; 2],
    meter_r: f32,
    tape_y: f32,
    keys: Pos2,
    /// The knobs' row, and which knobs.
    knob_y: f32,
    knob_r: f32,
    knobs: &'static [(usize, &'static str, &'static str)],
}

const ALL_KNOBS: [(usize, &str, &str); 9] = [
    (INPUT, "INPUT", "Record level: drive the tape"),
    (SATURATION, "SATURATION", "How hard into the tape, at the same level: harmonics, warmth, rounded and compressed peaks"),
    (BIAS, "BIAS", "Under: brighter, grittier · over: cleaner, darker"),
    (WOW, "WOW", "Slow pitch drift of the transport"),
    (FLUTTER, "FLUTTER", "Fast pitch jitter: capstan and pinch roller"),
    (HISS, "HISS", "Tape noise"),
    (AGE, "AGE", "Worn tape: dropouts and a duller top"),
    (OUTPUT, "OUTPUT", "Playback level (with AUTO GAIN it follows INPUT)"),
    (MIX, "MIX", "Dry against tape"),
];
const FEW_KNOBS: [(usize, &str, &str); 5] = [ALL_KNOBS[0], ALL_KNOBS[1], ALL_KNOBS[3], ALL_KNOBS[4], ALL_KNOBS[8]];

const COMPACT: Layout = Layout {
    size: vec2(380.0, 206.0),
    deck: [54.0, 148.0],
    reels: [pos2(62.0, 96.0), pos2(318.0, 96.0)],
    reel_r: 34.0,
    meters: [pos2(160.0, 116.0), pos2(220.0, 116.0)],
    meter_r: 24.0,
    tape_y: 140.0,
    keys: pos2(190.0, 68.0),
    knob_y: 156.0,
    knob_r: 12.0,
    knobs: &FEW_KNOBS,
};

const FULL: Layout = Layout {
    size: vec2(580.0, 338.0),
    deck: [54.0, 208.0],
    reels: [pos2(108.0, 124.0), pos2(472.0, 124.0)],
    reel_r: 54.0,
    meters: [pos2(246.0, 156.0), pos2(334.0, 156.0)],
    meter_r: 34.0,
    tape_y: 196.0,
    keys: pos2(290.0, 72.0),
    knob_y: 262.0,
    knob_r: 14.0,
    knobs: &ALL_KNOBS,
};

pub const SIZE: Vec2 = FULL.size;

const DETAILS: [(&str, &str); 2] = [("COMPACT", "The deck and the main knobs"), ("FULL", "Every switch and knob")];

fn layout(p: &ModuleParams) -> &'static Layout {
    if tp::choice(p, data::DETAIL, 2) == 0 { &COMPACT } else { &FULL }
}

pub fn size(p: &ModuleParams) -> Vec2 {
    layout(p).size
}

pub fn readout(p: &ModuleParams) -> (String, String) {
    tp::readout(p)
}

/// What moves on screen, kept between frames.
#[derive(Clone, Copy)]
struct Deck {
    last: f64,
    /// Reel angles (turns), how far the tape has wound (0: all on the
    /// left), the tape's time (seconds).
    reels: [f32; 2],
    wound: f32,
    counter: f64,
    /// VU needles: position (0..1 of the scale) and speed.
    needle: [f32; 2],
    swing: [f32; 2],
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    l: &'static Layout,
}

/// `ports`: the input and the output are patched.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, ports: (bool, bool)) {
    let p = &m.params;
    let s = Panel { v, id, p, accent, l: layout(p) };
    let w = s.l.size.x;
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "IN", v.font(9.0), col(ports.0));
    v.text(w - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), col(ports.1));
    let m = tp::machine(p);
    v.text(PAD + 22.0, 42.0, Align2::LEFT_CENTER, format!("{} · {} ips", m.name, tp::speed_name(p)), v.font(8.0), theme::TEXT_FAINT);
    let switch = v.r(w - 44.0 - 120.0, 34.0, w - 44.0, 50.0);
    if let Some(k) = s.seg("detail", switch, &DETAILS, tp::choice(p, data::DETAIL, 2)) {
        p.data[data::DETAIL].set(k as f32);
    }

    let deck = s.animate();
    s.deck(&deck);
    if std::ptr::eq(s.l, &FULL) {
        s.switches();
    }
    s.knobs();
    let moving = p.viz[viz::SPEED].get() > 0.0 || deck.needle.iter().any(|n| *n > 0.002);
    if moving {
        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// Where a level sits on a VU scale (0: −20 VU, 1: +3 VU).
fn vu_pos(rms: f32) -> f32 {
    let (lo, hi) = (0.1, 1.412_5);
    ((rms / VU_REF - lo) / (hi - lo)).clamp(-0.02, 1.04)
}

/// A tape pack's radius (share of the reel) when it holds `share` of the
/// tape.
fn pack(share: f32) -> f32 {
    (HUB * HUB + share.clamp(0.0, 1.0) * (FULL_PACK * FULL_PACK - HUB * HUB)).sqrt()
}

/// Where a line from `from` touches the circle (`c`, `r`), on its lower
/// side.
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

    /// A level from the meters (0..1 on the VU scale).
    fn level(&self, at: usize) -> f32 {
        vu_pos(self.p.viz[at].get().max(self.p.viz[at + 1].get())).clamp(0.0, 1.0)
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
            wound: 0.3 + (self.id % 5) as f32 * 0.08,
            counter: 0.0,
            needle: [0.0; 2],
            swing: [0.0; 2],
        });
        let dt = ((now - d.last) as f32).clamp(0.0, 0.1);
        d.last = now;
        let running = p.viz[viz::SPEED].get();
        // Faster at 30 ips, slower on cassette; eight minutes of tape at 15.
        let pace = if self.cassette() { 0.8 } else { [0.6, 1.0, 1.6][tp::choice(p, data::SPEED, SPEEDS.len())] };
        let speed = 0.5 * pace * running;
        d.wound = (d.wound + speed * dt / 240.0).fract();
        // The same tape length moves on both: the fuller reel turns slower.
        for (a, share) in d.reels.iter_mut().zip([1.0 - d.wound, d.wound]) {
            *a = (*a + speed * dt * HUB * 2.0 / pack(share)).fract();
        }
        d.counter += (dt * running) as f64;
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
    // The deck, sketched
    // -----------------------------------------------------------------------

    fn deck(&self, d: &Deck) {
        let (v, p, z, l) = (self.v, self.p, self.z(), self.l);
        let painter = v.painter;
        let w = l.size.x;
        painter.rect_filled(v.r(PAD, l.deck[0], w - PAD, l.deck[1]), 8.0 * z, alpha(theme::INSET, 0.35));

        let packs = [pack(1.0 - d.wound) * l.reel_r, pack(d.wound) * l.reel_r];
        let out = self.level(viz::VU_OUT);
        let sat = (-p.viz[viz::SAT].get() / 4.0).clamp(0.0, 1.0);
        let drop = p.viz[viz::DROP].get().clamp(0.0, 1.0);
        let running = p.viz[viz::SPEED].get() > 0.0;

        // The tape: off the left pack, along the line past the head, onto
        // the right pack; it sags and sways with wow and flutter, and
        // fades in a dropout.
        let (a, b) = (pos2(l.keys.x - l.reel_r, l.tape_y), pos2(l.keys.x + l.reel_r, l.tape_y));
        let sway = (p.viz[viz::WOBBLE].get() * 2.5).clamp(-4.0, 4.0);
        let mut line = vec![tangent(l.reels[0], packs[0], a)];
        line.extend((0..=16).map(|k| {
            let t = k as f32 / 16.0;
            pos2(a.x + (b.x - a.x) * t, a.y + (PI * t).sin() * sway)
        }));
        line.push(tangent(l.reels[1], packs[1], b));
        let tape = Stroke::new(1.4 * z, alpha(theme::TEXT_DIM, 0.75 * (1.0 - 0.7 * drop)));
        painter.add(Shape::line(line.iter().map(|q| v.p(q.x, q.y)).collect(), tape));

        for ((c, pack), turns) in l.reels.iter().zip(packs).zip(d.reels) {
            self.reel(*c, l.reel_r, pack, turns);
        }

        // The head: a short bar under the tape, glowing with the output
        // (orange when the tape squeezes).
        let glow = if running { out } else { 0.0 };
        let colour = theme::mix(theme::mix(theme::TEXT_FAINT, self.accent, glow), SAT_COL, sat);
        let head = v.p(l.keys.x, l.tape_y + 4.0);
        if glow > 0.05 {
            painter.circle_filled(head, (5.0 + 7.0 * glow) * z, alpha(colour, 0.18 * glow));
        }
        painter.rect_filled(Rect::from_center_size(head, vec2(16.0, 5.0) * z), 2.0 * z, colour);

        for (k, label) in ["IN", "OUT"].into_iter().enumerate() {
            self.vu(l.meters[k], l.meter_r, d.needle[k], label);
        }
        self.keys(d);
    }

    /// A reel: the pack as a soft disc, three arcs of flange turning
    /// round it, a hub with a mark.
    fn reel(&self, c: Pos2, r: f32, pack: f32, turns: f32) {
        let (v, z) = (self.v, self.z());
        let painter = v.painter;
        let centre = v.p(c.x, c.y);
        painter.circle_filled(centre, pack * z, alpha(self.accent, 0.12));
        painter.circle_stroke(centre, pack * z, Stroke::new(1.0 * z, alpha(self.accent, 0.5)));
        let gap = 0.5;
        let sweep = TAU / 3.0 - gap;
        for k in 0..3 {
            let a0 = (turns + k as f32 / 3.0) * TAU + gap * 0.5;
            let arc: Vec<Pos2> = (0..=12)
                .map(|j| {
                    let a = a0 + sweep * j as f32 / 12.0;
                    centre + vec2(a.cos(), a.sin()) * r * z
                })
                .collect();
            painter.add(Shape::line(arc, Stroke::new(1.4 * z, alpha(theme::TEXT_DIM, 0.8))));
        }
        painter.circle_filled(centre, r * 0.09 * z, theme::TEXT_DIM);
        let a = turns * TAU;
        painter.circle_filled(centre + vec2(a.cos(), a.sin()) * r * HUB * 0.9 * z, 1.6 * z, self.accent);
    }

    /// A VU meter, barely there: a bare arc (red past 0 VU) and a needle.
    fn vu(&self, c: Pos2, r: f32, pos: f32, label: &str) {
        let (v, z) = (self.v, self.z());
        let painter = v.painter;
        let span = 1.0;
        let at = |t: f32, rr: f32| {
            let a = -FRAC_PI_2 + (t - 0.5) * 2.0 * span;
            v.p(c.x + a.cos() * rr, c.y + a.sin() * rr)
        };
        let zero = vu_pos(VU_REF);
        let arc = |t0: f32, t1: f32| (0..=16).map(|k| at(t0 + (t1 - t0) * k as f32 / 16.0, r)).collect::<Vec<_>>();
        painter.add(Shape::line(arc(0.0, zero), Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.9))));
        painter.add(Shape::line(arc(zero, 1.0), Stroke::new(2.0 * z, alpha(theme::METER_RED, 0.8))));
        painter.line_segment([at(zero, r + 2.0), at(zero, r + 5.0)], Stroke::new(1.0 * z, theme::TEXT_FAINT));
        let hot = pos >= 0.99;
        let needle = if hot { theme::METER_RED } else { self.accent };
        painter.line_segment([v.p(c.x, c.y), at(pos, r + 3.0)], Stroke::new(1.5 * z, needle));
        painter.circle_filled(v.p(c.x, c.y), 2.5 * z, theme::TEXT_DIM);
        v.text(c.x, c.y + 9.0, Align2::CENTER_CENTER, label, v.font(6.5), theme::TEXT_FAINT);
    }

    /// STOP and PLAY, as two bare glyphs; and in the full view the tape's
    /// time under them.
    fn keys(&self, d: &Deck) {
        let (v, p, z, l) = (self.v, self.p, self.z(), self.l);
        let painter = v.painter;
        let running = self.on(data::RUNNING);
        let keys = [("■", false, "Stop: the tape slows to a halt"), ("▶", true, "Play: the tape spins back up")];
        for (k, (glyph, play, tip)) in keys.into_iter().enumerate() {
            let c = pos2(l.keys.x + if play { 14.0 } else { -14.0 }, l.keys.y);
            let rect = v.r(c.x - 10.0, c.y - 9.0, c.x + 10.0, c.y + 9.0);
            let resp = v.ui.interact(rect, self.uid(("key", k)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip);
            if resp.hovered() {
                painter.circle_filled(rect.center(), 9.0 * z, theme::CONTROL_HI);
            }
            let lit = play == running;
            painter.text(rect.center(), Align2::CENTER_CENTER, glyph, theme::font(10.0 * z), if lit { self.accent } else { theme::TEXT_FAINT });
            if resp.clicked() {
                p.data[data::RUNNING].set(if play { 1.0 } else { 0.0 });
            }
        }
        if std::ptr::eq(l, &FULL) {
            let secs = d.counter as u64;
            v.text(l.keys.x, l.keys.y + 20.0, Align2::CENTER_CENTER, format!("{:02}:{:02}", (secs / 60) % 100, secs % 60), v.mono(8.0), theme::TEXT_FAINT);
        }
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

    /// Two columns, two rows: MACHINE and SPEED, TAPE and EQ with AUTO GAIN.
    fn switches(&self) {
        let (v, p, l) = (self.v, self.p, self.l);
        let w = l.size.x;
        let y = |row: usize| l.deck[1] + 8.0 + 22.0 * row as f32;
        let cols = [(PAD, w * 0.5 - 8.0), (w * 0.5 + 8.0, w - PAD)];
        let slot = |col: usize, row: usize, label: &str| {
            let (x0, x1) = cols[col];
            v.text(x0, y(row) + 8.0, Align2::LEFT_CENTER, label, v.font(7.5), theme::TEXT_FAINT);
            (x0 + 54.0, x1, y(row))
        };
        let fixed = |x: f32, y: f32, text: &str| v.text(x + 4.0, y + 8.0, Align2::LEFT_CENTER, text, v.font(7.5), theme::TEXT_DIM);

        let (x0, x1, y0) = slot(0, 0, "MACHINE");
        let machines: Vec<(&'static str, &'static str)> = MACHINES.iter().map(|m| (m.name, m.hint)).collect();
        if let Some(k) = self.seg("machine", v.r(x0, y0, x1, y0 + 16.0), &machines, tp::choice(p, data::MACHINE, MACHINES.len())) {
            p.data[data::MACHINE].set(k as f32);
        }
        let (x0, x1, y0) = slot(0, 1, "SPEED");
        if self.cassette() {
            fixed(x0, y0, "1⅞ ips (cassette)");
        } else {
            let speeds = [("7½ IPS", "7½ ips: darker, lower head bump, more wow"), ("15 IPS", "15 ips: the classic"), ("30 IPS", "30 ips: open top, tight low end")];
            if let Some(k) = self.seg("speed", v.r(x0, y0, x1, y0 + 16.0), &speeds, tp::choice(p, data::SPEED, SPEEDS.len())) {
                p.data[data::SPEED].set(k as f32);
            }
        }
        let (x0, x1, y0) = slot(1, 0, "TAPE");
        let tapes: Vec<(&'static str, &'static str)> = TAPES.iter().map(|t| (t.0, t.1)).collect();
        if let Some(k) = self.seg("tape", v.r(x0, y0, x1, y0 + 16.0), &tapes, tp::choice(p, data::TAPE, TAPES.len())) {
            p.data[data::TAPE].set(k as f32);
        }
        let (x0, x1, y0) = slot(1, 1, "EQ");
        let mid = x0 + (x1 - x0) * 0.45;
        if self.cassette() {
            fixed(x0, y0, "120 µs");
        } else {
            let eqs = [("NAB", "American curve: fuller lows"), ("IEC", "European (CCIR) curve")];
            if let Some(k) = self.seg("eq", v.r(x0, y0, mid, y0 + 16.0), &eqs, tp::choice(p, data::EQ, 2)) {
                p.data[data::EQ].set(k as f32);
            }
        }
        let auto = self.on(data::AUTO);
        let resp = widgets::toggle(v.ui, v.painter, self.uid("auto"), v.r(mid + 8.0, y0, x1, y0 + 16.0), "AUTO GAIN", auto, self.accent, v.z)
            .on_hover_text("OUTPUT follows INPUT: drive the tape without getting louder");
        if resp.clicked() {
            p.data[data::AUTO].set(if auto { 0.0 } else { 1.0 });
        }
    }

    fn knobs(&self) {
        let (v, p, z, l) = (self.v, self.p, self.z(), self.l);
        let w = l.size.x;
        let cw = (w - 2.0 * PAD) / l.knobs.len() as f32;
        let r = l.knob_r;
        for (k, (i, label, tip)) in l.knobs.iter().enumerate() {
            let cx = PAD + cw * (k as f32 + 0.5);
            let spec = tp::spec(*i);
            v.text(cx, l.knob_y, Align2::CENTER_CENTER, label, v.font(7.5), theme::TEXT_FAINT);
            let mut norm = spec.to_norm(p.get(*i));
            let resp = widgets::knob(
                v.ui,
                v.painter,
                self.uid(("knob", i)),
                v.p(cx, l.knob_y + 8.0 + r),
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
            v.text(cx, l.knob_y + 16.0 + 2.0 * r, Align2::CENTER_CENTER, tp::format(p, *i), v.mono(7.5), if hot { theme::TEXT } else { theme::TEXT_DIM });
        }
        if std::ptr::eq(l, &FULL) {
            let sat = p.viz[viz::SAT].get();
            let state = if p.viz[viz::SPEED].get() <= 0.0 {
                "stopped".to_owned()
            } else if sat < -0.3 {
                format!("tape squeezing {sat:.1} dB")
            } else {
                "clean".to_owned()
            };
            v.text(w * 0.5, l.size.y - 10.0, Align2::CENTER_CENTER, state, v.font(7.5), theme::TEXT_FAINT);
        }
    }
}
