//! Open Recent: the projects opened or saved last, newest first, kept
//! between launches in the app's settings. The File menu lists them in a
//! submenu; one that has gone missing is shown greyed out until the list is
//! cleared or it comes back.

use std::path::{Path, PathBuf};

use egui::{RichText, Ui, vec2};

use super::theme;

/// Projects the menu remembers.
pub const RECENT_MAX: usize = 10;

/// Puts `path` at the top of `list` (once), keeping at most [`RECENT_MAX`].
pub fn remember(list: &mut Vec<PathBuf>, path: &Path) {
    let key = same_file_key(path);
    list.retain(|p| same_file_key(p) != key);
    list.insert(0, path.to_path_buf());
    list.truncate(RECENT_MAX);
}

/// Drops `path` from `list`.
pub fn forget(list: &mut Vec<PathBuf>, path: &Path) {
    let key = same_file_key(path);
    list.retain(|p| same_file_key(p) != key);
}

/// Two spellings of one file ("~/a/../a/x" and "~/a/x") compare equal while
/// it exists; a missing one compares as written.
fn same_file_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// "~/Music/Projects" for "/Users/me/Music/Projects", shortened from the
/// left to about `max` characters.
pub fn folder_hint(path: &Path, max: usize) -> String {
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let text = match home.as_deref().and_then(|h| folder.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => folder.display().to_string(),
    };
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        text
    } else {
        let tail: String = chars[chars.len() - max.saturating_sub(1)..]
            .iter()
            .collect();
        format!("…{tail}")
    }
}

/// What the submenu asks for.
pub enum RecentPick {
    Open(PathBuf),
    Clear,
}

/// The entries of the Open Recent submenu. Returns the pick (and closes).
pub fn menu(ui: &mut Ui, recent: &[PathBuf]) -> Option<RecentPick> {
    let mut picked = None;
    if recent.is_empty() {
        ui.add_enabled(
            false,
            egui::Button::new(RichText::new("No recent projects").size(12.5))
                .min_size(vec2(260.0, 22.0)),
        );
        return None;
    }
    for path in recent {
        let name = path.file_stem().map_or_else(
            || path.display().to_string(),
            |s| s.to_string_lossy().into_owned(),
        );
        let exists = path.exists();
        let hint = if exists {
            folder_hint(path, 30)
        } else {
            "missing".to_owned()
        };
        let button = egui::Button::new(RichText::new(name).size(12.5).color(if exists {
            theme::text()
        } else {
            theme::text()_FAINT
        }))
        .right_text(RichText::new(hint).size(10.5).color(theme::text()_FAINT))
        .min_size(vec2(260.0, 22.0));
        let resp = ui.add_enabled(exists, button);
        let resp = if exists {
            resp.on_hover_text(path.display().to_string())
        } else {
            resp.on_disabled_hover_text(format!("{} is no longer there", path.display()))
        };
        if resp.clicked() {
            picked = Some(RecentPick::Open(path.clone()));
            ui.close();
        }
    }
    ui.separator();
    if super::canvas::menu_item(ui, "Clear menu", "") {
        picked = Some(RecentPick::Clear);
        ui.close();
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_once_and_capped() {
        let mut list = Vec::new();
        for k in 0..12 {
            remember(&mut list, Path::new(&format!("/nowhere/p{k}.audioboard")));
        }
        assert_eq!(list.len(), RECENT_MAX);
        assert_eq!(list[0], Path::new("/nowhere/p11.audioboard"));
        remember(&mut list, Path::new("/nowhere/p5.audioboard"));
        assert_eq!(list[0], Path::new("/nowhere/p5.audioboard"));
        assert_eq!(
            list.iter().filter(|p| p.ends_with("p5.audioboard")).count(),
            1
        );
        assert_eq!(list.len(), RECENT_MAX);
        forget(&mut list, Path::new("/nowhere/p5.audioboard"));
        assert!(list.iter().all(|p| !p.ends_with("p5.audioboard")));
    }

    #[test]
    fn folder_hints_are_short() {
        let home = std::env::var("HOME").unwrap_or_default();
        assert_eq!(
            folder_hint(Path::new(&format!("{home}/Music/a.audioboard")), 40),
            "~/Music"
        );
        assert_eq!(folder_hint(Path::new("/tmp/a.audioboard"), 40), "/tmp");
        let long = folder_hint(
            Path::new("/a/very/long/folder/name/for/projects/x.audioboard"),
            12,
        );
        assert!(
            long.starts_with('…') && long.chars().count() == 12,
            "{long}"
        );
    }
}
