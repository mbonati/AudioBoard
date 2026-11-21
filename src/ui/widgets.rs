//! Custom-painted controls. They come from the UI Kit
//! (`audioboard_ui::widgets`); only what needs the app's DSP stays here.

use egui::{Pos2, Rect, emath, pos2};

pub use audioboard_ui::widgets::*;

use crate::audio::dsp::preview_sample;
use crate::audio::params::Waveform;

/// Polyline of `cycles` periods of a waveform fitted inside `rect`.
pub fn wave_points(
    rect: Rect,
    wave: Waveform,
    shape: f32,
    cycles: f32,
    amp: f32,
    phase: f32,
    seed: u32,
) -> Vec<Pos2> {
    let n = (rect.width() / 2.0).clamp(16.0, 240.0) as usize;
    (0..=n)
        .map(|i| {
            let x = i as f32 / n as f32;
            let y = preview_sample(wave, shape, x * cycles + phase, seed) * amp;
            pos2(
                emath::lerp(rect.left()..=rect.right(), x),
                rect.center().y - y * rect.height() * 0.5,
            )
        })
        .collect()
}
