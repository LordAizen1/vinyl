//! What the user has chosen from the context menu, and where it is kept.
//!
//! The shape is deliberately plain, with a `Default` and `#[serde(default)]`, so
//! a config written by a build that knew fewer fields still loads rather than
//! throwing the lot away.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::window::Placement;

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
    /// These are window sizes, not chassis sizes: `styles.css` keeps a 16px
    /// transparent gutter on the body so the chassis has somewhere to cast its
    /// shadow. Without it the shadow is clipped by the square window edge and
    /// the clip reads as four sharp corners against the desktop. So the visible
    /// widget is 32px smaller than these figures in each direction.
    ///
    /// Neither height is free. The deck's panel is sized in cqw, a share of
    /// *width*, so its height does not track the window's: only one height per
    /// width leaves the margin above and below the deck equal to the 16px at its
    /// left, and only one keeps the 715x700 viewBox filling its panel without
    /// letterboxing. Both were measured, not derived, and the tests below pin
    /// them.
    ///
    /// Both must stay above the window's `minWidth`/`minHeight` in
    /// `tauri.conf.json`, or the resize is silently clamped.
    pub fn dimensions(self) -> (f64, f64) {
        match self {
            Size::Full => (460.0, 273.0),
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub size: Size,
    pub theme: Theme,
    /// Whether to look lyrics up. This is the only setting that governs network
    /// access: with it off, the app makes no outbound requests at all.
    pub lyrics: bool,
    /// Where the window was left. `None` on a first run, which means "let the
    /// window manager decide" rather than any particular corner.
    pub placement: Option<Placement>,
    /// Locked means the widget ignores the mouse entirely: clicks, drags and
    /// right-clicks all pass through to whatever is beneath it, which on the
    /// desktop is the desktop. It just sits there.
    ///
    /// On by default. A widget is something you glance at, and one that
    /// swallows clicks in the middle of your wallpaper is a nuisance. The tray
    /// is how you unlock it, and is the only way in: a locked widget cannot be
    /// right-clicked, by definition.
    pub locked: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            size: Size::Full,
            theme: Theme::Auto,
            lyrics: true,
            placement: None,
            locked: true,
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
        assert!(prefs.lyrics, "an older config should keep the default");
        assert!(prefs.placement.is_none());
        assert!(prefs.locked, "an older config should keep the default");
    }

    #[test]
    fn round_trips() {
        let prefs = Prefs {
            size: Size::Compact,
            theme: Theme::Dark,
            lyrics: false,
            placement: Some(Placement { x: 12.0, y: 34.0 }),
            locked: false,
        };
        let text = serde_json::to_string(&prefs).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&text).unwrap(), prefs);
    }

    /// The layout model, mirroring `styles.css`.
    ///
    /// Two traps live in here. The body carries a 16px gutter for the chassis
    /// shadow, so the chassis is 32px smaller than the window. And the chassis's
    /// own `padding: 0.62cqw` cannot query the element it sits on, so it
    /// resolves against the viewport — the window width — while every descendant
    /// resolves against the chassis's *content* box. Getting either wrong puts
    /// the answer out by several pixels, which is what these tests catch.
    const GUTTER: f64 = 16.0;
    const DECK_GAP: f64 = 16.0;

    /// Returns (face width, face height, one cqw).
    fn face(width: f64, height: f64) -> (f64, f64, f64) {
        let padding = 0.0062 * width;
        let face_w = width - 2.0 * GUTTER - 2.0 * padding;
        let face_h = height - 2.0 * GUTTER - 2.0 * padding;
        (face_w, face_h, face_w / 100.0)
    }

    /// The deck letterboxes inside its own panel at any other ratio.
    #[test]
    fn compact_keeps_the_ratio_the_deck_needs() {
        let (width, height) = Size::Compact.dimensions();
        let (face_w, face_h, _) = face(width, height);
        // Compact insets the plate by one gap on all four sides.
        let plate_w = face_w - 2.0 * DECK_GAP;
        let plate_h = face_h - 2.0 * DECK_GAP;
        assert!(
            (plate_w / plate_h - 715.0 / 700.0).abs() < 0.005,
            "plate is {plate_w}x{plate_h}, which does not match the viewBox"
        );
    }

    /// The margin above and below the deck must match the gap at its left, and
    /// for a given width only one height delivers that. Guards against anyone
    /// rounding the height to something tidier.
    #[test]
    fn full_puts_an_equal_margin_on_every_side_of_the_deck() {
        let (width, height) = Size::Full.dimensions();
        let (_, face_h, cqw) = face(width, height);
        // .plate is 49.2cqw wide and 48.17cqw tall in the full layout.
        let margin = (face_h - 48.17 * cqw) / 2.0;
        assert!(
            (margin - DECK_GAP).abs() < 0.5,
            "deck margin is {margin}, but its left margin is {DECK_GAP}"
        );
    }

    /// The point of --deck-gap: the two sizes must read as one design, and that
    /// means the same margin in pixels rather than the same figure in cqw.
    #[test]
    fn both_sizes_sit_on_the_same_margin() {
        for size in [Size::Full, Size::Compact] {
            let (width, height) = size.dimensions();
            let (_, face_h, cqw) = face(width, height);
            let deck_h = match size {
                Size::Full => 48.17 * cqw,
                Size::Compact => face_h - 2.0 * DECK_GAP,
            };
            let margin = (face_h - deck_h) / 2.0;
            assert!(
                (margin - DECK_GAP).abs() < 0.5,
                "{size:?} sits on a {margin}px margin, not {DECK_GAP}px"
            );
        }
    }

    /// Both presets have to clear the configured minimum, or the resize is
    /// clamped and the layout silently comes out wrong.
    #[test]
    fn both_presets_clear_the_window_minimum() {
        for size in [Size::Full, Size::Compact] {
            let (width, height) = size.dimensions();
            assert!(width >= 260.0, "{size:?} is narrower than minWidth");
            assert!(height >= 240.0, "{size:?} is shorter than minHeight");
        }
    }
}
