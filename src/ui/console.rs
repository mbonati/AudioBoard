//! Panel of the Console: a 12- or 24-channel mixer with AUX sends, in two
//! shapes. Horizontal: strips side by side as on a desk, inputs on the top
//! edge. Vertical: strips stacked as rows, inputs on the left. Either way
//! the master and AUX buses come last, with OUT and AUX on the right edge.

use std::sync::atomic::Ordering::Relaxed;

use egui::{Align2, Color32, Id, Pos2, Stroke, Vec2, vec2};

use super::nodes::{
    NodeView, bus_fader, channel_knob, main_out, master_mute, segmented, send_label,
};
use super::theme::{self, alpha};
use super::widgets::{hfader, hmeter, toggle, vfader, vmeter};
use crate::audio::engine::NodeId;
use crate::audio::params::{fader_gain, format_db, gain_to_db};
use crate::graph::{Console, MixerNode, ParamRef};

const PAD: f32 = 12.0;

// Horizontal: strips STRIP_W wide from STRIPS_X (a column of row labels
// before them), then the buses.
const STRIPS_X: f32 = 44.0;
const STRIP_W: f32 = 44.0;
const BUSES_W: f32 = 150.0;
const H_HEIGHT: f32 = 342.0;
/// Rows of a strip: AUX and PAN knobs, mute/solo, the fader.
const H_AUX_Y: f32 = 72.0;
const H_PAN_Y: f32 = 110.0;
const H_KEYS_Y: f32 = 138.0;
const H_FADER: [f32; 2] = [162.0, 316.0];
/// Where OUT and AUX leave, in the horizontal shape.
const H_OUT_Y: [f32; 2] = [72.0, 100.0];

// Vertical: rows ROW_H tall from ROWS_Y, then the buses.
const V_WIDTH: f32 = 420.0;
const ROWS_Y: f32 = 58.0;
const ROW_H: f32 = 26.0;
const V_FADER_X: [f32; 2] = [118.0, 270.0];

pub fn size(c: Console) -> Vec2 {
    if c.horizontal {
        vec2(buses_x(c) + BUSES_W + PAD, H_HEIGHT)
    } else {
        vec2(V_WIDTH, v_bus_y(c, true) + 30.0)
    }
}

/// The horizontal shape's bus section.
fn buses_x(c: Console) -> f32 {
    STRIPS_X + STRIP_W * c.channels as f32 + 8.0
}

/// The vertical shape's MASTER (or AUX) row.
fn v_bus_y(c: Console, aux: bool) -> f32 {
    ROWS_Y + ROW_H * c.channels as f32 + 24.0 + if aux { 38.0 } else { 0.0 }
}

pub fn input_offset(c: Console, i: usize) -> Vec2 {
    if c.horizontal {
        vec2(STRIPS_X + STRIP_W * (i as f32 + 0.5), 0.0)
    } else {
        vec2(0.0, ROWS_Y + ROW_H * (i as f32 + 0.5))
    }
}

pub fn output_offset(c: Console, i: usize) -> Vec2 {
    let y = if c.horizontal {
        H_OUT_Y[i.min(1)]
    } else {
        v_bus_y(c, i > 0)
    };
    vec2(size(c).x, y)
}

/// A name cut to `max` characters.
fn short(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        name.to_owned()
    } else {
        name.chars().take(max - 1).chain(['…']).collect()
    }
}

const CHANNEL_ITEMS: [(&str, &str); 2] = [
    ("12", "Twelve channels"),
    ("24", "Twenty-four channels (unplugs nothing)"),
];
const SHAPE_ITEMS: [(&str, &str); 2] = [
    (
        "H",
        "Horizontal: strips side by side, inputs on the top edge",
    ),
    ("V", "Vertical: strips stacked, inputs on the left edge"),
];

/// `sources[ch]`: the colour and name of what's plugged into channel `ch`;
/// `outs`: OUT and AUX are patched. Returns the shape asked for when one of
/// the switches is clicked.
pub fn draw(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &MixerNode,
    sources: &[Option<(Color32, String)>],
    outs: (bool, bool),
) -> Option<Console> {
    let c = m.console?;
    // The switches: over the buses, or over the rows.
    let switches = if c.horizontal {
        (buses_x(c), 34.0)
    } else {
        (PAD, 32.0)
    };
    let mut asked = None;
    let r = |dx0: f32, dx1: f32| {
        v.r(
            switches.0 + dx0,
            switches.1,
            switches.0 + dx1,
            switches.1 + 16.0,
        )
    };
    let seg = |salt: &str, rect, items: &[(&'static str, &'static str); 2], current| {
        segmented(
            v,
            Id::unique((salt, id)),
            rect,
            2,
            current,
            accent,
            |k, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    items[k].0,
                    v.font(8.0),
                    col,
                );
                items[k].1
            },
        )
    };
    if let Some(k) = seg(
        "console-channels",
        r(0.0, 64.0),
        &CHANNEL_ITEMS,
        usize::from(c.channels > 12),
    ) {
        asked = Some(Console {
            channels: if k == 1 { 24 } else { 12 },
            ..c
        });
    }
    if let Some(k) = seg(
        "console-shape",
        r(70.0, 134.0),
        &SHAPE_ITEMS,
        usize::from(!c.horizontal),
    ) {
        asked = Some(Console {
            horizontal: k == 0,
            ..c
        });
    }
    if c.horizontal {
        horizontal(v, id, accent, m, c, sources, outs);
    } else {
        vertical(v, id, accent, m, c, sources, outs);
    }
    asked.filter(|&a| a != c)
}

/// A channel's mute and solo buttons in two rects (node units).
fn keys(v: &NodeView, id: NodeId, m: &MixerNode, ch: usize, mute: [f32; 4], solo: [f32; 4]) {
    let p = &m.params;
    let (muted, soloed) = (p.mute[ch].load(Relaxed), p.solo[ch].load(Relaxed));
    let rect = |r: [f32; 4]| v.r(r[0], r[1], r[2], r[3]);
    if toggle(
        v.ui,
        v.painter,
        Id::unique(("mute", id, ch)),
        rect(mute),
        "M",
        muted,
        theme::mute(),
        v.z,
    )
    .clicked()
    {
        p.mute[ch].store(!muted, Relaxed);
    }
    if toggle(
        v.ui,
        v.painter,
        Id::unique(("solo", id, ch)),
        rect(solo),
        "S",
        soloed,
        theme::solo(),
        v.z,
    )
    .clicked()
    {
        p.solo[ch].store(!soloed, Relaxed);
    }
}

fn audible(m: &MixerNode, ch: usize) -> bool {
    let p = &m.params;
    let any_solo = p.solo.iter().any(|s| s.load(Relaxed));
    !p.mute[ch].load(Relaxed) && (!any_solo || p.solo[ch].load(Relaxed))
}

/// A channel fader (MIDI can map it), vertical or not, in `rect` (node units).
fn channel_fader(
    v: &NodeView,
    id: NodeId,
    m: &MixerNode,
    ch: usize,
    rect: [f32; 4],
    color: Color32,
    vertical: bool,
) {
    let mut pos = m.params.fader[ch].get();
    let (fid, r) = (
        Id::unique(("fader", id, ch)),
        v.r(rect[0], rect[1], rect[2], rect[3]),
    );
    let on = audible(m, ch);
    let resp = if vertical {
        vfader(v.ui, v.painter, fid, r, &mut pos, color, on, v.z)
    } else {
        hfader(v.ui, v.painter, fid, r, &mut pos, color, on, v.z)
    };
    v.hit(resp.rect, ParamRef::Fader(ch));
    if resp.dragged() || resp.double_clicked() {
        m.params.fader[ch].set(pos);
    }
}

fn fader_db(m: &MixerNode, ch: usize) -> String {
    format_db(gain_to_db(fader_gain(m.params.fader[ch].get())))
}

fn horizontal(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &MixerNode,
    c: Console,
    sources: &[Option<(Color32, String)>],
    outs: (bool, bool),
) {
    let z = v.z;
    for (y, label) in [(H_AUX_Y, "AUX"), (H_PAN_Y, "PAN")] {
        v.text(
            PAD,
            y,
            Align2::LEFT_CENTER,
            label,
            v.font(7.0),
            if label == "AUX" {
                theme::aux()
            } else {
                theme::text()_FAINT
            },
        );
    }
    for ch in 0..c.channels {
        let x0 = STRIPS_X + STRIP_W * ch as f32;
        let cx = x0 + STRIP_W * 0.5;
        if ch % 2 == 1 {
            v.painter.rect_filled(
                v.r(x0, 28.0, x0 + STRIP_W, H_HEIGHT - 6.0),
                3.0 * z,
                alpha(theme::inset(), 0.35),
            );
        }
        let source = sources.get(ch).and_then(|s| s.as_ref());
        let col = source.map_or(theme::text()_FAINT, |s| s.0);
        v.text(cx, 36.0, Align2::CENTER_CENTER, ch + 1, v.mono(10.0), col);
        let name = source.map_or("—".to_owned(), |s| short(&s.1, 7));
        v.text(
            cx,
            49.0,
            Align2::CENTER_CENTER,
            name,
            v.font(7.0),
            theme::text()_FAINT,
        );
        channel_knob(
            v,
            id,
            &m.params,
            ch,
            true,
            Pos2::new(cx, H_AUX_Y),
            10.0,
            theme::aux(),
        );
        v.text(
            cx,
            H_AUX_Y + 17.0,
            Align2::CENTER_CENTER,
            send_label(m.params.aux[ch].get()),
            v.mono(7.0),
            theme::text()_FAINT,
        );
        channel_knob(
            v,
            id,
            &m.params,
            ch,
            false,
            Pos2::new(cx, H_PAN_Y),
            10.0,
            col,
        );
        v.text(
            cx,
            H_PAN_Y + 17.0,
            Align2::CENTER_CENTER,
            super::nodes::pan_label(m.params.pan[ch].get()),
            v.mono(7.0),
            theme::text()_FAINT,
        );
        keys(
            v,
            id,
            m,
            ch,
            [cx - 19.0, H_KEYS_Y, cx - 1.0, H_KEYS_Y + 14.0],
            [cx + 1.0, H_KEYS_Y, cx + 19.0, H_KEYS_Y + 14.0],
        );
        let [f0, f1] = H_FADER;
        channel_fader(v, id, m, ch, [cx - 13.0, f0, cx + 1.0, f1], col, true);
        vmeter(
            v.painter,
            v.r(cx + 6.0, f0, cx + 10.0, f1),
            &m.channels[ch],
            z,
        );
        v.text(
            cx,
            f1 + 12.0,
            Align2::CENTER_CENTER,
            fader_db(m, ch),
            v.mono(7.5),
            if audible(m, ch) {
                theme::text()_DIM
            } else {
                theme::text()_FAINT
            },
        );
    }

    // The buses: MASTER and AUX columns.
    let x = buses_x(c);
    let w = size(c).x;
    main_out(v, x, H_OUT_Y[0], m, accent, outs.0);
    v.text(
        x,
        H_OUT_Y[1],
        Align2::LEFT_CENTER,
        if outs.1 {
            "Sends by cable"
        } else {
            "AUX not patched"
        },
        v.font(8.5),
        theme::text()_FAINT,
    );
    v.text(
        w - PAD,
        H_OUT_Y[0],
        Align2::RIGHT_CENTER,
        "OUT",
        v.font(9.0),
        if outs.0 {
            theme::text()_DIM
        } else {
            theme::text()_FAINT
        },
    );
    v.text(
        w - PAD,
        H_OUT_Y[1],
        Align2::RIGHT_CENTER,
        "AUX",
        v.font(9.0),
        if outs.1 {
            theme::aux()
        } else {
            theme::text()_FAINT
        },
    );
    v.painter.vline(
        v.p(x - 4.0, 0.0).x,
        v.p(0.0, 30.0).y..=v.p(0.0, H_HEIGHT - 8.0).y,
        Stroke::new(1.0 * z, theme::node_border()),
    );
    let muted = m.params.master_mute.load(Relaxed);
    for (aux, cx) in [(false, x + 40.0), (true, x + 112.0)] {
        let (label, col) = if aux {
            ("AUX", theme::aux())
        } else {
            ("MASTER", theme::text()_DIM)
        };
        v.text(
            cx,
            124.0,
            Align2::CENTER_CENTER,
            label,
            theme::strong(8.5 * z),
            col,
        );
        let [f0, f1] = H_FADER;
        bus_fader(
            v,
            id,
            &m.params,
            aux,
            v.r(cx - 16.0, f0, cx - 2.0, f1),
            true,
        );
        let meters = if aux { &m.aux } else { &m.master };
        for (i, meter) in meters.iter().enumerate() {
            let mx = cx + 4.0 + 6.0 * i as f32;
            vmeter(v.painter, v.r(mx, f0, mx + 4.0, f1), meter, z);
        }
        let value = if aux {
            m.params.aux_master.get()
        } else {
            m.params.master.get()
        };
        let off = !aux && muted;
        let text = if off {
            "muted".to_owned()
        } else {
            format_db(gain_to_db(fader_gain(value)))
        };
        v.text(
            cx,
            f1 + 12.0,
            Align2::CENTER_CENTER,
            text,
            v.mono(8.5),
            if off { theme::mute() } else { theme::text() },
        );
    }
    master_mute(
        v,
        id,
        &m.params,
        v.r(x + 18.0, H_KEYS_Y, x + 62.0, H_KEYS_Y + 14.0),
        "MUTE",
    );
}

fn vertical(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &MixerNode,
    c: Console,
    sources: &[Option<(Color32, String)>],
    outs: (bool, bool),
) {
    let z = v.z;
    let [f0, f1] = V_FADER_X;
    v.text(
        76.0,
        ROWS_Y - 5.0,
        Align2::CENTER_CENTER,
        "PAN",
        v.font(7.0),
        theme::text()_FAINT,
    );
    v.text(
        100.0,
        ROWS_Y - 5.0,
        Align2::CENTER_CENTER,
        "AUX",
        v.font(7.0),
        theme::aux(),
    );
    for ch in 0..c.channels {
        let y0 = ROWS_Y + ROW_H * ch as f32;
        let cy = y0 + ROW_H * 0.5;
        if ch % 2 == 1 {
            v.painter.rect_filled(
                v.r(4.0, y0, V_WIDTH - 4.0, y0 + ROW_H),
                3.0 * z,
                alpha(theme::inset(), 0.35),
            );
        }
        let source = sources.get(ch).and_then(|s| s.as_ref());
        let col = source.map_or(theme::text()_FAINT, |s| s.0);
        v.text(14.0, cy, Align2::CENTER_CENTER, ch + 1, v.mono(9.0), col);
        keys(
            v,
            id,
            m,
            ch,
            [26.0, cy - 7.0, 40.0, cy + 7.0],
            [42.0, cy - 7.0, 56.0, cy + 7.0],
        );
        channel_knob(v, id, &m.params, ch, false, Pos2::new(76.0, cy), 8.0, col);
        channel_knob(
            v,
            id,
            &m.params,
            ch,
            true,
            Pos2::new(100.0, cy),
            8.0,
            theme::aux(),
        );
        channel_fader(v, id, m, ch, [f0, cy - 8.0, f1, cy + 2.0], col, false);
        hmeter(
            v.painter,
            v.r(f0, cy + 5.0, f1, cy + 8.0),
            &m.channels[ch],
            z,
        );
        v.text(
            318.0,
            cy,
            Align2::RIGHT_CENTER,
            fader_db(m, ch),
            v.mono(8.5),
            if audible(m, ch) {
                theme::text()
            } else {
                theme::text()_FAINT
            },
        );
        let name = source.map_or("No input".to_owned(), |s| short(&s.1, 13));
        v.text(
            326.0,
            cy,
            Align2::LEFT_CENTER,
            name,
            v.font(7.5),
            if source.is_some() {
                theme::text()_DIM
            } else {
                theme::text()_FAINT
            },
        );
    }

    // The buses: MASTER and AUX rows.
    let sep = ROWS_Y + ROW_H * c.channels as f32 + 6.0;
    let w = V_WIDTH;
    v.painter.hline(
        v.p(PAD, 0.0).x..=v.p(w - PAD, 0.0).x,
        v.p(0.0, sep).y,
        Stroke::new(1.0 * z, theme::node_border()),
    );
    let muted = m.params.master_mute.load(Relaxed);
    for aux in [false, true] {
        let cy = v_bus_y(c, aux);
        let (label, col) = if aux {
            ("AUX", theme::aux())
        } else {
            ("MASTER", theme::text()_DIM)
        };
        v.text(
            PAD,
            cy - 5.0,
            Align2::LEFT_CENTER,
            label,
            theme::strong(9.5 * z),
            col,
        );
        bus_fader(
            v,
            id,
            &m.params,
            aux,
            v.r(f0, cy - 6.0, f1, cy + 6.0),
            false,
        );
        let meters = if aux { &m.aux } else { &m.master };
        for (i, meter) in meters.iter().enumerate() {
            let y = cy + 9.0 + 6.0 * i as f32;
            hmeter(v.painter, v.r(f0, y, f1, y + 3.0), meter, z);
        }
        let value = if aux {
            m.params.aux_master.get()
        } else {
            m.params.master.get()
        };
        let off = !aux && muted;
        let text = if off {
            "muted".to_owned()
        } else {
            format_db(gain_to_db(fader_gain(value)))
        };
        v.text(
            318.0,
            cy - 3.0,
            Align2::RIGHT_CENTER,
            text,
            v.mono(10.0),
            if off { theme::mute() } else { theme::text() },
        );
        let port = if aux {
            ("AUX", outs.1, theme::aux())
        } else {
            ("OUT", outs.0, theme::text()_DIM)
        };
        v.text(
            w - PAD,
            cy - 3.0,
            Align2::RIGHT_CENTER,
            port.0,
            v.font(9.0),
            if port.1 { port.2 } else { theme::text()_FAINT },
        );
    }
    let my = v_bus_y(c, false);
    master_mute(
        v,
        id,
        &m.params,
        v.r(70.0, my - 9.0, 108.0, my + 7.0),
        "MUTE",
    );
    main_out(v, PAD, my + 10.0, m, accent, outs.0);
    let ay = v_bus_y(c, true);
    v.text(
        PAD,
        ay + 10.0,
        Align2::LEFT_CENTER,
        if outs.1 {
            "Sends by cable"
        } else {
            "Not patched"
        },
        v.font(8.5),
        theme::text()_FAINT,
    );
}
