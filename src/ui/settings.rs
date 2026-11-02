//! The Settings window (Cmd+,): the audio system, the output (device, the
//! channel pair the board plays on, sample rate, buffer size) and the input
//! the Audio In nodes listen to, with a meter per input channel.

use egui::{Align2, Color32, ComboBox, Rect, RichText, Sense, Stroke, Ui, vec2};

use super::theme::{self, alpha};
use crate::audio::AudioHandle;
use crate::audio::device::{self, AudioConfig, BUFFER_SIZES, DeviceList, InputChoice};
use crate::audio::input::MAX_CHANNELS;

/// Read the devices again this often while the window is open: they come
/// and go (a USB interface plugged in).
const RELIST_SECONDS: f64 = 3.0;

const COMBO_W: f32 = 300.0;
/// The labels' column, the same in both sections.
const LABEL_W: f32 = 90.0;

#[derive(Default)]
pub struct SettingsWindow {
    pub open: bool,
    list: Option<DeviceList>,
    listed_at: f64,
    /// Input meters (falling slowly).
    levels: [f32; MAX_CHANNELS],
}

impl SettingsWindow {
    /// Draws the window; returns the setup the user picked, if it changed.
    pub fn show(&mut self, ctx: &egui::Context, audio: &AudioHandle, now: f64, dt: f32) -> Option<AudioConfig> {
        if !self.open {
            self.list = None;
            return None;
        }
        if self.list.is_none() || now - self.listed_at > RELIST_SECONDS {
            self.list = Some(device::list(&audio.config));
            self.listed_at = now;
        }
        let peaks = audio.shared.input.take_peaks();
        let fall = (-dt / 0.3).exp();
        for (level, peak) in self.levels.iter_mut().zip(peaks) {
            *level = peak.max(*level * fall);
        }
        ctx.request_repaint();

        let mut open = true;
        let mut picked = None;
        let mut relist = false;
        let list = self.list.as_ref()?;
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                let mut config = audio.config.clone();
                output_section(ui, list, audio, &mut config);
                ui.add_space(10.0);
                input_section(ui, list, audio, &mut config, &self.levels);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    relist = ui.button("Refresh devices").clicked();
                    ui.label(RichText::new("Changes apply at once and are kept.").size(10.5).color(theme::TEXT_FAINT));
                });
                if config != audio.config {
                    picked = Some(config);
                }
            });
        self.open = open;
        if relist {
            self.list = None;
        }
        picked
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).strong().color(theme::TEXT_DIM));
    ui.add_space(2.0);
}

fn note(ui: &mut Ui, text: impl Into<String>, color: Color32) {
    ui.label(RichText::new(text).size(10.5).color(color));
}

fn rate_label(rate: u32) -> String {
    if rate.is_multiple_of(1000) { format!("{} kHz", rate / 1000) } else { format!("{:.1} kHz", rate as f32 / 1000.0) }
}

fn output_section(ui: &mut Ui, list: &DeviceList, audio: &AudioHandle, config: &mut AudioConfig) {
    heading(ui, "OUTPUT");
    let info = &audio.info;
    let device = list.output(config.output.as_deref());
    egui::Grid::new("output").num_columns(2).min_col_width(LABEL_W).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        if list.hosts.len() > 1 {
            ui.label("Audio system");
            let current = config.host.clone().unwrap_or_else(|| list.host.clone());
            ComboBox::from_id_salt("host").width(COMBO_W).selected_text(&current).show_ui(ui, |ui| {
                for host in &list.hosts {
                    if ui.selectable_label(*host == current, host).clicked() && *host != current {
                        // Another system: its own devices, from their defaults.
                        *config = AudioConfig { host: Some(host.clone()), buffer_size: config.buffer_size, ..Default::default() };
                    }
                }
            });
            ui.end_row();
        }

        ui.label("Device");
        let default_name = list.output(None).map_or("none", |d| d.name.as_str());
        let shown = match &config.output {
            None => format!("System default ({default_name})"),
            Some(id) => list.outputs.iter().find(|d| d.id == *id).map_or_else(|| "Missing device".to_owned(), |d| d.name.clone()),
        };
        ComboBox::from_id_salt("output").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
            if ui.selectable_label(config.output.is_none(), format!("System default ({default_name})")).clicked() {
                config.output = None;
                config.output_channel = 0;
            }
            for d in &list.outputs {
                let on = config.output.as_deref() == Some(d.id.as_str());
                if ui.selectable_label(on, format!("{}  ·  {} ch", d.name, d.channels)).clicked() && !on {
                    config.output = Some(d.id.clone());
                    config.output_channel = 0;
                }
            }
        });
        ui.end_row();

        let channels = device.map_or(info.channels, |d| d.channels);
        if channels > 2 {
            ui.label("Channels");
            let pairs = device::pairs(channels);
            let shown = pairs.iter().find(|(c, _)| *c == config.output_channel).map_or("1–2", |(_, l)| l.as_str());
            ComboBox::from_id_salt("out_pair").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
                for (c, label) in pairs.iter().filter(|(c, _)| c + 1 < channels) {
                    ui.selectable_value(&mut config.output_channel, *c, label);
                }
            });
            ui.end_row();
        }

        ui.label("Sample rate");
        let shown = match config.sample_rate {
            None => format!("Device's own ({})", rate_label(info.sample_rate)),
            Some(rate) => rate_label(rate),
        };
        ComboBox::from_id_salt("rate").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut config.sample_rate, None, "Device's own");
            for &rate in device.map_or(&[][..], |d| &d.rates) {
                ui.selectable_value(&mut config.sample_rate, Some(rate), rate_label(rate));
            }
        });
        ui.end_row();

        ui.label("Buffer size");
        let ms = |frames: u32| frames as f32 / info.sample_rate.max(1) as f32 * 1000.0;
        let shown = match config.buffer_size {
            None => "Device's own".to_owned(),
            Some(size) => format!("{size} samples · {:.1} ms", ms(size)),
        };
        let range = device.and_then(|d| d.buffer_range).or(info.buffer_range);
        ComboBox::from_id_salt("buffer").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut config.buffer_size, None, "Device's own");
            for size in BUFFER_SIZES {
                if range.is_none_or(|(lo, hi)| (lo..=hi).contains(&size)) {
                    ui.selectable_value(&mut config.buffer_size, Some(size), format!("{size} samples · {:.1} ms", ms(size)));
                }
            }
        });
        ui.end_row();
    });
    ui.add_space(4.0);
    match &info.error {
        Some(e) => note(ui, e, theme::MUTE),
        None => {
            let frames = audio.shared.frames.load(std::sync::atomic::Ordering::Relaxed);
            let pair = device::pairs(info.channels).into_iter().find(|(c, _)| *c == info.output_channel).map_or_else(String::new, |(_, l)| format!(" · out {l}"));
            note(
                ui,
                format!("Playing on {} · {} · {frames} samples{pair}", info.device, rate_label(info.sample_rate)),
                theme::TEXT_FAINT,
            );
        }
    }
    if config.sample_rate.is_some_and(|r| r != info.sample_rate) {
        note(ui, "A new sample rate rebuilds every node (the patch stays as it is).", theme::TEXT_FAINT);
    }
}

fn input_section(ui: &mut Ui, list: &DeviceList, audio: &AudioHandle, config: &mut AudioConfig, levels: &[f32; MAX_CHANNELS]) {
    heading(ui, "INPUT");
    let info = &audio.info;
    egui::Grid::new("input").num_columns(2).min_col_width(LABEL_W).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        ui.label("Device");
        let default_name = list.input(&InputChoice::SystemDefault).map_or("none", |d| d.name.as_str());
        let shown = match &config.input {
            InputChoice::Off => "Off".to_owned(),
            InputChoice::SystemDefault => format!("System default ({default_name})"),
            InputChoice::Device(id) => list.inputs.iter().find(|d| d.id == *id).map_or_else(|| "Missing device".to_owned(), |d| d.name.clone()),
        };
        ComboBox::from_id_salt("input").width(COMBO_W).selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut config.input, InputChoice::Off, "Off");
            ui.selectable_value(&mut config.input, InputChoice::SystemDefault, format!("System default ({default_name})"));
            for d in &list.inputs {
                let rate_ok = d.rates.contains(&info.sample_rate);
                let label = format!("{}  ·  {} ch{}", d.name, d.channels, if rate_ok { "" } else { "  ·  other rate" });
                ui.selectable_value(&mut config.input, InputChoice::Device(d.id.clone()), label);
            }
        });
        ui.end_row();
    });
    ui.add_space(4.0);
    if let Some(e) = &info.input_error {
        note(ui, e, theme::MUTE);
        return;
    }
    let Some(name) = &info.input else {
        if config.input != InputChoice::Off {
            note(ui, "Opens while an Audio In node is on the board.", theme::TEXT_FAINT);
        }
        return;
    };
    let channels = (info.input_channels as usize).min(MAX_CHANNELS);
    note(ui, format!("Listening to {name} · {channels} ch · Audio In nodes pick their channels"), theme::TEXT_FAINT);
    ui.add_space(4.0);
    for (c, &level) in levels.iter().enumerate().take(channels) {
        meter(ui, c + 1, level);
    }
    let lost = audio.shared.input.dropouts.load(std::sync::atomic::Ordering::Relaxed);
    if lost > 0 {
        note(ui, format!("{lost} input dropouts (a larger buffer, or the same device for input and output, helps)"), theme::SOLO);
    }
}

/// One input channel's level, −60 … 0 dBFS.
fn meter(ui: &mut Ui, channel: usize, level: f32) {
    let width = COMBO_W + 90.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 12.0), Sense::hover());
    let p = ui.painter();
    p.text(rect.left_center() + vec2(8.0, 0.0), Align2::CENTER_CENTER, channel.to_string(), theme::mono(9.0), theme::TEXT_DIM);
    let bar = Rect::from_min_max(rect.left_top() + vec2(22.0, 3.0), rect.right_bottom() - vec2(0.0, 3.0));
    p.rect_filled(bar, 2.0, alpha(theme::TEXT, 0.06));
    let x = (1.0 + 20.0 * level.max(1e-6).log10() / 60.0).clamp(0.0, 1.0);
    let color = if level >= 0.89 { theme::METER_RED } else if level >= 0.5 { theme::METER_YELLOW } else { theme::METER_GREEN };
    p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * x, bar.height())), 2.0, color);
    for db in [-18.0, -6.0] {
        let x = bar.left() + bar.width() * (1.0 + db / 60.0);
        p.vline(x, bar.y_range(), Stroke::new(1.0, alpha(theme::TEXT, 0.18)));
    }
}
