//! A panel's PRESETS ▾ menu: save the module's settings under a name, load
//! a built-in or a saved preset, delete a saved one. The saved ones are
//! read from the app's data folder ([`crate::presets`]) once a session.

use std::sync::Arc;

use egui::Id;

use super::theme;
use crate::audio::modules::ModuleParams;
use crate::presets::{self, ModulePreset};

/// Where the session keeps a kind's saved presets.
fn saved_id(p: &ModuleParams) -> Id {
    Id::unique(("module-presets", p.kind.def().name))
}

/// The saved presets, read once and kept for the session.
fn saved(ui: &egui::Ui, p: &ModuleParams) -> Arc<Vec<ModulePreset>> {
    let id = saved_id(p);
    if let Some(list) = ui.data(|d| d.get_temp::<Arc<Vec<ModulePreset>>>(id)) {
        return list;
    }
    let list = Arc::new(
        presets::dir()
            .map(|dir| presets::load(&dir, p.kind))
            .unwrap_or_default(),
    );
    ui.data_mut(|d| d.insert_temp(id, list.clone()));
    list
}

/// Writes the saved presets back, and keeps them for the session.
fn keep(ui: &egui::Ui, p: &ModuleParams, list: Vec<ModulePreset>) -> Result<(), String> {
    let dir = presets::dir().ok_or("no folder for presets")?;
    presets::save(&dir, p.kind, &list)?;
    ui.data_mut(|d| d.insert_temp(saved_id(p), Arc::new(list)));
    Ok(())
}

/// The menu under `button`, its state kept under `uid`. `noun` is what a
/// preset holds ("scene", "sound"); `factory` names the built-in presets,
/// which `load` sets.
pub fn show(
    button: &egui::Response,
    p: &ModuleParams,
    uid: Id,
    noun: &str,
    factory: &[&str],
    load: impl Fn(usize),
) {
    let name_id = uid.with("preset-name");
    let error_id = uid.with("preset-error");
    egui::Popup::menu(button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(200.0);
            ui.label(format!("Save this {noun} as"));
            let mut name: String = ui
                .data(|d| d.get_temp(name_id))
                .unwrap_or_else(|| format!("My {noun}"));
            let mut save = false;
            ui.horizontal(|ui| {
                let field = ui.text_edit_singleline(&mut name);
                save = ui.button("Save").clicked()
                    || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            });
            ui.data_mut(|d| d.insert_temp(name_id, name.clone()));
            let list = saved(ui, p);
            if save && !name.trim().is_empty() {
                let mut next = (*list).clone();
                presets::store(&mut next, ModulePreset::capture(p, name.trim()));
                let result = keep(ui, p, next);
                ui.data_mut(|d| d.insert_temp(error_id, result.err().unwrap_or_default()));
            }
            if let Some(error) = ui
                .data(|d| d.get_temp::<String>(error_id))
                .filter(|e| !e.is_empty())
            {
                ui.colored_label(theme::MUTE, format!("Could not save: {error}"));
            }
            ui.separator();
            ui.weak("Built in");
            for (k, name) in factory.iter().enumerate() {
                if ui.button(*name).clicked() {
                    load(k);
                    ui.close();
                }
            }
            let list = saved(ui, p);
            if !list.is_empty() {
                ui.separator();
                ui.weak("Saved");
                let mut remove = None;
                for (k, preset) in list.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(&preset.name).clicked() {
                            preset.apply(p);
                            ui.close();
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text("Delete this preset")
                            .clicked()
                        {
                            remove = Some(k);
                        }
                    });
                }
                if let Some(k) = remove {
                    let mut next = (*list).clone();
                    next.remove(k);
                    let result = keep(ui, p, next);
                    ui.data_mut(|d| d.insert_temp(error_id, result.err().unwrap_or_default()));
                }
            }
        });
}
