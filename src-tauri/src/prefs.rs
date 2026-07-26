//! What the user has chosen from the context menu, and where it is kept.
//!
//! PLAN.md Phase 4 will add window position here. The shape is deliberately a
//! plain struct of `Copy` enums with a `Default`, so a config written by a build
//! that knew fewer fields still loads rather than throwing the lot away.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The two window presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Size {
    /// Deck, screen, progress readout and transport.
    Full,
    /// The deck alone: a display, with no controls but the deck itself.
    Compact,
}

impl Size {
    /// Logical size, in pixels.
    ///
    /// Neither height is round, and neither is free to change.
    ///
    /// Full: the deck panel is 45.6cqw tall inside a 0.62cqw chassis padding,
    /// and cqw is a share of *width*, so the panel's height does not track the
    /// window's. Only at 275 does the margin above and below the deck come out
    /// equal to the 6.2cqw at its left. Taller and the deck floats in dead
    /// space; shorter and it crowds the chassis.
    ///
    /// Compact: the deck's 715x700 viewBox only fills its plate at one ratio;
    /// any other letterboxes the deck inside its own panel.
    ///
    /// Both must stay above the window's `minWidth`/`minHeight` in
    /// `tauri.conf.json`, or the resize is silently clamped.
    pub fn dimensions(self) -> (f64, f64) {
        match self {
            Size::Full => (470.0, 275.0),
            Size::Compact => (280.0, 275.0),
        }
    }
}

/// Which palette to draw in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    /// Follow the Windows app-mode setting. What the widget has always done,
    /// and still the default.
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub size: Size,
    pub theme: Theme,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            size: Size::Full,
            theme: Theme::Auto,
        }
    }
}

impl Prefs {
    /// Reads the config, falling back to defaults on anything unreadable.
    ///
    /// Every failure is logged and swallowed: a corrupt or half-written config
    /// must never stop the widget starting. A missing file is the normal first
    /// run and is not worth a warning.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                log::warn!(
                    "prefs: {} did not parse ({error}); using defaults",
                    path.display()
                );
                Self::default()
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!(
                    "prefs: could not read {} ({error}); using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Writes the config, creating the directory if this is the first run.
    ///
    /// Also best-effort: failing to persist a menu choice is worth a log line,
    /// not a broken widget. The choice still applies for this session.
    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                log::warn!("prefs: could not create {} ({error})", parent.display());
                return;
            }
        }

        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(error) = fs::write(path, text) {
                    log::warn!("prefs: could not write {} ({error})", path.display());
                }
            }
            Err(error) => log::warn!("prefs: could not serialise ({error})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let prefs = Prefs::load(Path::new("does-not-exist.json"));
        assert_eq!(prefs, Prefs::default());
    }

    #[test]
    fn partial_config_keeps_the_defaults_for_what_is_absent() {
        let prefs: Prefs = serde_json::from_str(r#"{"size":"compact"}"#).unwrap();
        assert_eq!(prefs.size, Size::Compact);
        assert_eq!(prefs.theme, Theme::Auto);
    }

    #[test]
    fn round_trips() {
        let prefs = Prefs {
            size: Size::Compact,
            theme: Theme::Dark,
        };
        let text = serde_json::to_string(&prefs).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&text).unwrap(), prefs);
    }

    /// The chassis's own `padding: 0.62cqw` cannot query the element it sits on,
    /// so it resolves against the viewport — the window width — while every
    /// descendant's cqw resolves against the chassis's *content* box. Getting
    /// this backwards puts the answer out by ~3px, which is what these two tests
    /// exist to catch.
    fn face(width: f64, height: f64) -> (f64, f64, f64) {
        let padding = 0.0062 * width;
        let cqw = (width - 2.0 * padding) / 100.0;
        (width - 2.0 * padding, height - 2.0 * padding, cqw)
    }

    /// The deck letterboxes inside its own panel at any other ratio.
    #[test]
    fn compact_keeps_the_ratio_the_deck_needs() {
        let (width, height) = Size::Compact.dimensions();
        let (face_w, face_h, cqw) = face(width, height);
        // Compact insets the plate 6.2cqw on all four sides.
        let plate_w = face_w - 12.4 * cqw;
        let plate_h = face_h - 12.4 * cqw;
        assert!(
            (plate_w / plate_h - 715.0 / 700.0).abs() < 0.005,
            "plate is {plate_w}x{plate_h}, which does not match the viewBox"
        );
    }

    /// The margin above and below the deck must match the 6.2cqw at its left,
    /// and for a given width only one height delivers that. Guards against
    /// anyone rounding the height to something tidier.
    #[test]
    fn full_puts_an_equal_margin_on_every_side_of_the_deck() {
        let (width, height) = Size::Full.dimensions();
        let (_, face_h, cqw) = face(width, height);
        let margin = (face_h - 45.6 * cqw) / 2.0;
        assert!(
            (margin - 6.2 * cqw).abs() < 0.5,
            "deck margin is {margin}, but its left margin is {}",
            6.2 * cqw
        );
    }
}
