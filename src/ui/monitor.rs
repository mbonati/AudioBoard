//! Bottom dock: oscilloscope and spectrum of the master output.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

use super::theme::{self, alpha};
use crate::audio::params::EngineShared;

const FFT_SIZE: usize = 2048;
const SCOPE_WINDOW: usize = 1024;
/// Room above the trace for the screen's label.
const LABEL_H: f32 = 18.0;

pub struct Monitor {
    samples: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    window: Vec<f32>,
    /// Smoothed spectrum in dB per display column.
    columns: Vec<f32>,
}

impl Monitor {
    pub fn new() -> Self {
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (FFT_SIZE - 1) as f32).cos())
            .collect();
        Self {
            samples: Vec::with_capacity(4096),
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            window,
            columns: Vec::new(),
        }
    }

    pub fn scope(&mut self, ui: &mut Ui, shared: &EngineShared, sample_rate: f32, size: Vec2) {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let p = ui.painter_at(rect);
        let detail = format!("±1.0 · {:.0} ms", SCOPE_WINDOW as f32 / sample_rate * 1000.0);
        let inner = screen(&p, rect, "Oscilloscope", &detail);
        p.hline(inner.x_range(), inner.center().y, Stroke::new(1.0, theme::GRID_MAJOR));

        shared.scope.snapshot(&mut self.samples, 4096);
        let s = &self.samples;
        // Rising zero-crossing trigger for a stable trace.
        let latest = s.len() - SCOPE_WINDOW;
        let start = (latest.saturating_sub(SCOPE_WINDOW)..latest)
            .rev()
            .find(|&i| i > 0 && s[i - 1] < 0.0 && s[i] >= 0.0)
            .unwrap_or(latest);
        let trace = &s[start..start + SCOPE_WINDOW];

        let pts: Vec<Pos2> = trace
            .iter()
            .enumerate()
            .map(|(i, v)| {
                pos2(
                    inner.left() + inner.width() * i as f32 / (SCOPE_WINDOW - 1) as f32,
                    inner.center().y - v.clamp(-1.0, 1.0) * inner.height() * 0.5,
                )
            })
            .collect();
        p.add(Shape::line(pts, Stroke::new(1.2, theme::ACCENT)));
    }

    pub fn spectrum(&mut self, ui: &mut Ui, sample_rate: f32, dt: f32, size: Vec2) {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let p = ui.painter_at(rect);
        let inner = screen(&p, rect, "Spectrum", "20 Hz – 20 kHz · −96 dB");

        let s = &self.samples;
        if s.len() < FFT_SIZE {
            return;
        }
        let tail = &s[s.len() - FFT_SIZE..];
        for ((re, s), w) in self.re.iter_mut().zip(tail).zip(&self.window) {
            *re = s * w;
        }
        self.im.fill(0.0);
        fft(&mut self.re, &mut self.im);

        let cols = (inner.width() / 3.0) as usize;
        self.columns.resize(cols, -120.0);

        let (f_lo, f_hi) = (20.0f32, (sample_rate * 0.5).min(20_000.0));
        let bin_hz = sample_rate / FFT_SIZE as f32;
        let norm = 2.0 / (FFT_SIZE as f32 * 0.5); // Hann coherent gain = 0.5
        let x_of = |f: f32| inner.left() + inner.width() * (f / f_lo).ln() / (f_hi / f_lo).ln();
        let (db_lo, db_hi) = (-96.0f32, 0.0f32);
        let y_of = |db: f32| inner.bottom() - inner.height() * ((db - db_lo) / (db_hi - db_lo)).clamp(0.0, 1.0);

        // Frequency grid.
        for f in [100.0, 1000.0, 10_000.0] {
            let x = x_of(f);
            p.vline(x, inner.y_range(), Stroke::new(1.0, theme::GRID_MAJOR));
            let label = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
            p.text(pos2(x + 3.0, inner.bottom()), Align2::LEFT_BOTTOM, label, FontId::monospace(8.5), theme::TEXT_FAINT);
        }

        let fall = 60.0 * dt; // dB per frame of release
        let mut pts = Vec::with_capacity(cols);
        for c in 0..cols {
            let f0 = f_lo * (f_hi / f_lo).powf(c as f32 / cols as f32);
            let f1 = f_lo * (f_hi / f_lo).powf((c + 1) as f32 / cols as f32);
            let b0 = (f0 / bin_hz).floor() as usize;
            let b1 = ((f1 / bin_hz).ceil() as usize).clamp(b0 + 1, FFT_SIZE / 2);
            let mag = (b0..b1)
                .map(|b| (self.re[b] * self.re[b] + self.im[b] * self.im[b]).sqrt())
                .fold(0.0, f32::max)
                * norm;
            let db = 20.0 * mag.max(1e-7).log10();
            let col = &mut self.columns[c];
            *col = if db > *col { db } else { (*col - fall).max(db) };
            pts.push(pos2(inner.left() + inner.width() * (c as f32 + 0.5) / cols as f32, y_of(*col)));
        }

        let color = theme::ACCENT;
        let mut mesh = Mesh::default();
        for (i, pt) in pts.iter().enumerate() {
            mesh.vertices.push(Vertex { pos: *pt, uv: WHITE_UV, color: alpha(color, 0.22) });
            mesh.vertices.push(Vertex {
                pos: pos2(pt.x, inner.bottom()),
                uv: WHITE_UV,
                color: alpha(color, 0.02),
            });
            if i > 0 {
                let k = (i * 2) as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        p.add(Shape::mesh(mesh));
        p.add(Shape::line(pts, Stroke::new(1.2, color)));
    }
}

/// Inset analyzer background with its title; returns the plotting area.
fn screen(p: &Painter, rect: Rect, title: &str, detail: &str) -> Rect {
    p.rect_filled(rect, 4.0, theme::INSET);
    p.rect_stroke(rect, 4.0, Stroke::new(1.0, theme::PANEL_BORDER), egui::StrokeKind::Inside);
    let t = p.text(
        rect.left_top() + vec2(8.0, 9.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(10.5),
        theme::TEXT_DIM,
    );
    p.text(
        pos2(t.right() + 8.0, t.center().y),
        Align2::LEFT_CENTER,
        detail,
        FontId::monospace(9.0),
        theme::TEXT_FAINT,
    );
    Rect::from_min_max(rect.min + vec2(6.0, LABEL_H), rect.max - vec2(6.0, 6.0))
}

/// In-place iterative radix-2 FFT.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let (cr32, ci32) = (cr as f32, ci as f32);
                let tr = re[b] * cr32 - im[b] * ci32;
                let ti = re[b] * ci32 + im[b] * cr32;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
}
