//! Sibyl's preset library: sounds saved by name and category in the app's
//! preset folder (`presets/Sibyl.json`), so they are there in every project,
//! and exported to or imported from files of their own.
//!
//! A preset keeps the sound only: every knob but the master section's
//! (TUNE, OUTPUT, HQ, A-440, the mod wheel, VOICES, the controllers' numbers),
//! the effects' order and its category. Loading one never touches the
//! programs recorded in the node. Presets saved with the panel's first menu
//! (which kept every extra setting) load too: their knobs by name, their
//! effects' order from where it was.

use std::collections::BTreeMap;
use std::path::Path;

use crate::audio::modules::sibyl::{self as sb, DEF, PARAM_COUNT, PROGRAMS, data, is_global};
use crate::audio::modules::{ModuleKind, ModuleParams};
use crate::presets::{self, ModulePreset};

pub const CATEGORIES: [&str; 8] = ["ARP", "PAD", "LEAD", "KEYS", "BASS", "TEXTURE", "BOUNCE", "OTHER"];
const OTHER: usize = 7;
/// Kept among the knobs, under names no knob has.
const CATEGORY_KEY: &str = "CATEGORY";
const FX_KEYS: [&str; 5] = ["FX ORDER 1", "FX ORDER 2", "FX ORDER 3", "FX ORDER 4", "FX ORDER 5"];

/// The panel's sound as a preset.
pub fn capture(p: &ModuleParams, name: &str, category: usize) -> ModulePreset {
    let mut params: BTreeMap<String, f32> = (0..PARAM_COUNT).filter(|&i| !is_global(i)).map(|i| (DEF.params[i].name.to_owned(), p.get(i))).collect();
    for (key, e) in FX_KEYS.iter().zip(sb::fx_order(p)) {
        params.insert((*key).to_owned(), e as f32);
    }
    params.insert(CATEGORY_KEY.to_owned(), category.min(OTHER) as f32);
    ModulePreset { name: name.trim().to_owned(), params, data: Vec::new(), mode: None, toggle: None }
}

/// Sets the panel to `preset` (the master section and the programs stay).
pub fn apply(p: &ModuleParams, preset: &ModulePreset) {
    for i in (0..PARAM_COUNT).filter(|&i| !is_global(i)) {
        if let Some(&v) = preset.params.get(DEF.params[i].name) {
            p.set(i, sb::snap(i, v));
        }
    }
    let order: Option<Vec<f32>> = if FX_KEYS.iter().all(|k| preset.params.contains_key(*k)) {
        Some(FX_KEYS.iter().map(|k| preset.params[*k]).collect())
    } else if preset.data.len() >= data::FX_ORDER + 5 {
        Some(preset.data[data::FX_ORDER..data::FX_ORDER + 5].to_vec())
    } else {
        None
    };
    if let Some(order) = order {
        for (n, e) in order.into_iter().enumerate() {
            p.data[data::FX_ORDER + n].set(e.round().clamp(0.0, 4.0));
        }
    }
    sb::set_sound_name(p, &preset.name);
}

pub fn category(preset: &ModulePreset) -> usize {
    preset.params.get(CATEGORY_KEY).map_or(OTHER, |c| (c.round().max(0.0) as usize).min(OTHER))
}

/// The category a factory program belongs to (by its bank).
pub fn factory_category(k: usize) -> usize {
    match k.min(PROGRAMS - 1) / 8 {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 5,
        _ => 6,
    }
}

/// The library (none if there is no preset folder or file yet).
pub fn load() -> Vec<ModulePreset> {
    presets::dir().map(|d| presets::load(&d, ModuleKind::Sibyl)).unwrap_or_default()
}

pub fn save(list: &[ModulePreset]) -> Result<(), String> {
    let dir = presets::dir().ok_or("no folder for presets")?;
    presets::save(&dir, ModuleKind::Sibyl, list)
}

/// Presets into a file of their own (to share them).
pub fn export(path: &Path, list: &[ModulePreset]) -> Result<(), String> {
    presets::save_file(path, list)
}

/// The presets in a file (none if it can't be read).
pub fn import(path: &Path) -> Vec<ModulePreset> {
    presets::load_file(path).into_iter().filter(|p| !p.name.trim().is_empty()).collect()
}

/// Adds `incoming` to `list`, replacing presets of the same name.
pub fn merge(list: &mut Vec<ModulePreset>, incoming: Vec<ModulePreset>) {
    for preset in incoming {
        presets::store(list, preset);
    }
}
