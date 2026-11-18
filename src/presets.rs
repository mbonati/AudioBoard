//! Presets of a single module: its knobs (by name) and its extra settings,
//! kept in a file per module kind under the app's data folder
//! (`presets/Nullpunkt.json`), so they are there in every project.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audio::modules::{ModuleKind, ModuleParams};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ModulePreset {
    pub name: String,
    pub params: BTreeMap<String, f32>,
    #[serde(default)]
    pub data: Vec<f32>,
    /// The mode by name, and the toggle, for kinds that have them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toggle: Option<bool>,
}

impl ModulePreset {
    /// The module's settings now, under `name`.
    pub fn capture(p: &ModuleParams, name: &str) -> Self {
        let def = p.kind.def();
        Self {
            name: name.to_owned(),
            params: def.params.iter().enumerate().map(|(i, s)| (s.name.to_owned(), p.get(i))).collect(),
            data: p.data.iter().map(|d| d.get()).collect(),
            mode: def.modes.get(p.mode()).map(|m| (*m).to_owned()),
            toggle: def.toggle.map(|_| p.toggled()),
        }
    }

    /// Sets the module to this preset; knobs it doesn't name keep their
    /// value.
    pub fn apply(&self, p: &ModuleParams) {
        let def = p.kind.def();
        for (i, spec) in def.params.iter().enumerate() {
            if let Some(&v) = self.params.get(spec.name) {
                p.set(i, v.clamp(spec.min, spec.max));
            }
        }
        for (d, &v) in p.data.iter().zip(&self.data) {
            d.set(v);
        }
        if let Some(k) = self.mode.as_ref().and_then(|m| def.modes.iter().position(|n| n == m)) {
            p.set_mode(k);
        }
        if let Some(on) = self.toggle.filter(|_| def.toggle.is_some()) {
            p.set_toggle(on);
        }
    }
}

/// Where a kind's presets live.
pub fn file(dir: &Path, kind: ModuleKind) -> PathBuf {
    // Built-in modules by name, plugin nodes by their stable id.
    dir.join(format!("{}.json", kind.key()))
}

/// The app's preset folder.
pub fn dir() -> Option<PathBuf> {
    crate::project::data_dir().map(|d| d.join("presets"))
}

/// A kind's saved presets (none if the file is missing or unreadable).
pub fn load(dir: &Path, kind: ModuleKind) -> Vec<ModulePreset> {
    load_file(&file(dir, kind))
}

pub fn save(dir: &Path, kind: ModuleKind, presets: &[ModulePreset]) -> Result<(), String> {
    save_file(&file(dir, kind), presets)
}

/// Presets kept in a file of their own (Orrery's patterns: `data` holds a
/// pattern, not the whole module); none if it is missing or unreadable.
pub fn load_file(path: &Path) -> Vec<ModulePreset> {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save_file(path: &Path, presets: &[ModulePreset]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(presets).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// Adds `preset`, replacing one of the same name.
pub fn store(presets: &mut Vec<ModulePreset>, preset: ModulePreset) {
    match presets.iter_mut().find(|p| p.name == preset.name) {
        Some(old) => *old = preset,
        None => presets.push(preset),
    }
}
