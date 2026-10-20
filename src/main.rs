mod audio;
mod graph;
mod project;
mod ui;

#[cfg(test)]
mod tests;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AudioBoard")
            .with_inner_size([1520.0, 940.0])
            .with_min_inner_size([1040.0, 680.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    eframe::run_native(
        "AudioBoard",
        options,
        Box::new(|cc| Ok(Box::new(ui::app::AudioBoardApp::new(cc)))),
    )
}
