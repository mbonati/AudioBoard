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
}

impl ModulePreset {
    /// The module's settings now, under `name`.
    pub fn capture(p: &ModuleParams, name: &str) -> Self {
        let def = p.kind.def();
        Self {
            name: name.to_owned(),
            params: def.params.iter().enumerate().map(|(i, s)| (s.name.to_owned(), p.get(i))).collect(),
            data: p.data.iter().map(|d| d.get()).collect(),
        }
    }

    /// Sets the module to this preset; knobs it doesn't name keep their
    /// value.
    pub fn apply(&self, p: &ModuleParams) {
        for (i, spec) in p.kind.def().params.iter().enumerate() {
            if let Some(&v) = self.params.get(spec.name) {
                p.set(i, v.clamp(spec.min, spec.max));
            }
        }
        for (d, &v) in p.data.iter().zip(&self.data) {
            d.set(v);
        }
    }
}

/// Where a kind's presets live.
pub fn file(dir: &Path, kind: ModuleKind) -> PathBuf {
    dir.join(format!("{}.json", kind.def().name))
}

/// The app's preset folder.
pub fn dir() -> Option<PathBuf> {
    crate::project::data_dir().map(|d| d.join("presets"))
}

/// A kind's saved presets (none if the file is missing or unreadable).
pub fn load(dir: &Path, kind: ModuleKind) -> Vec<ModulePreset> {
    std::fs::read_to_string(file(dir, kind)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save(dir: &Path, kind: ModuleKind, presets: &[ModulePreset]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(presets).map_err(|e| e.to_string())?;
    std::fs::write(file(dir, kind), json).map_err(|e| e.to_string())
}

/// Adds `preset`, replacing one of the same name.
pub fn store(presets: &mut Vec<ModulePreset>, preset: ModulePreset) {
    match presets.iter_mut().find(|p| p.name == preset.name) {
        Some(old) => *old = preset,
        None => presets.push(preset),
    }
}
