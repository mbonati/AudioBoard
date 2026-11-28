// Optimized builds on Windows open no console window beside the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod board;
mod graph;
mod presets;
mod project;
mod song;
mod tracks;
mod ui;

#[cfg(test)]
mod perf;
#[cfg(test)]
mod tests;

pub const APP_TITLE: &str = concat!("AudioBoard ", env!("CARGO_PKG_VERSION"));

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_TITLE)
            .with_inner_size([1520.0, 940.0])
            .with_min_inner_size([1040.0, 680.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(ui::app::AudioBoardApp::new(cc)))),
    )
}
