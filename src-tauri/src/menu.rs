//! The right-click menu: window size and palette.
//!
//! Built once at startup and popped up on demand, because rebuilding it per
//! click would lose the tick marks. The frontend forwards its `contextmenu`
//! event to `show_menu`; everything else happens here.

use parking_lot::Mutex;
use tauri::menu::{CheckMenuItem, ContextMenu, Menu, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, Runtime, Window};

use crate::prefs::{Prefs, Size, Theme};

/// Where the choice is written. Tauri resolves this to
/// `%APPDATA%\dev.lordaizen.vinyl\config.json` on Windows.
const CONFIG_FILE: &str = "config.json";

/// Emitted whenever a menu choice lands, so the frontend can restyle without
/// polling. The payload is the whole `Prefs`, not a delta: it is two fields, and
/// sending the state avoids the frontend having to track which one changed.
pub const CHANGED_EVENT: &str = "prefs-changed";

const SIZE_FULL: &str = "size:full";
const SIZE_COMPACT: &str = "size:compact";
const THEME_AUTO: &str = "theme:auto";
const THEME_LIGHT: &str = "theme:light";
const THEME_DARK: &str = "theme:dark";
const QUIT: &str = "quit";

/// The menu, plus the check items that have to be re-ticked after a choice.
pub struct AppMenu<R: Runtime> {
    menu: Menu<R>,
    size_items: [(CheckMenuItem<R>, Size); 2],
    theme_items: [(CheckMenuItem<R>, Theme); 3],
}

/// The live preferences. A `Mutex` rather than a channel: menu clicks are rare
/// and the handler is already off the render path.
pub struct PrefsState(pub Mutex<Prefs>);

pub fn build<R: Runtime>(app: &AppHandle<R>, prefs: Prefs) -> tauri::Result<AppMenu<R>> {
    let check = |id: &str, label: &str, on: bool| {
        CheckMenuItem::with_id(app, id, label, true, on, None::<&str>)
    };

    let full = check(SIZE_FULL, "Full size", prefs.size == Size::Full)?;
    let compact = check(SIZE_COMPACT, "Compact", prefs.size == Size::Compact)?;

    let auto = check(THEME_AUTO, "Match Windows", prefs.theme == Theme::Auto)?;
    let light = check(THEME_LIGHT, "Light", prefs.theme == Theme::Light)?;
    let dark = check(THEME_DARK, "Dark", prefs.theme == Theme::Dark)?;

    let appearance = Submenu::with_items(
        app,
        "Appearance",
        true,
        &[&auto, &light, &dark],
    )?;

    let menu = Menu::with_items(
        app,
        &[
            &full,
            &compact,
            &PredefinedMenuItem::separator(app)?,
            &appearance,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("Quit vinyl"))?,
        ],
    )?;

    Ok(AppMenu {
        menu,
        size_items: [(full, Size::Full), (compact, Size::Compact)],
        theme_items: [
            (auto, Theme::Auto),
            (light, Theme::Light),
            (dark, Theme::Dark),
        ],
    })
}

impl<R: Runtime> AppMenu<R> {
    pub fn popup(&self, window: Window<R>) -> tauri::Result<()> {
        self.menu.popup(window)
    }

    /// Only one item per group may be ticked. Windows draws a `CheckMenuItem` as
    /// a tick rather than a radio dot, so the exclusivity has to be enforced
    /// here; leaving the old one ticked would show two "current" sizes.
    fn retick(&self, prefs: Prefs) {
        for (item, size) in &self.size_items {
            let _ = item.set_checked(*size == prefs.size);
        }
        for (item, theme) in &self.theme_items {
            let _ = item.set_checked(*theme == prefs.theme);
        }
    }
}

/// Applies a menu choice: resize if the size changed, persist, re-tick, tell the
/// frontend.
pub fn handle<R: Runtime>(app: &AppHandle<R>, id: &str) {
    // Quit is a predefined item and Tauri handles it itself.
    if id == QUIT {
        return;
    }

    let Some(state) = app.try_state::<PrefsState>() else {
        return;
    };

    let (prefs, size_changed) = {
        let mut current = state.0.lock();
        let before = *current;

        match id {
            SIZE_FULL => current.size = Size::Full,
            SIZE_COMPACT => current.size = Size::Compact,
            THEME_AUTO => current.theme = Theme::Auto,
            THEME_LIGHT => current.theme = Theme::Light,
            THEME_DARK => current.theme = Theme::Dark,
            other => {
                log::warn!("menu: unknown item {other:?}");
                return;
            }
        }

        (*current, current.size != before.size)
    };

    if size_changed {
        apply_size(app, prefs.size);
    }

    if let Ok(dir) = app.path().app_config_dir() {
        prefs.save(&dir.join(CONFIG_FILE));
    }

    if let Some(menu) = app.try_state::<AppMenu<R>>() {
        menu.retick(prefs);
    }

    if let Err(error) = app.emit(CHANGED_EVENT, prefs) {
        log::warn!("menu: could not tell the frontend ({error})");
    }
}

/// Resizes the main window to a preset.
///
/// Logical, not physical: the widget is laid out in CSS pixels, so on a 150%
/// display a physical size would come out two thirds the intended one.
pub fn apply_size<R: Runtime>(app: &AppHandle<R>, size: Size) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let (width, height) = size.dimensions();
    if let Err(error) = window.set_size(LogicalSize::new(width, height)) {
        log::warn!("menu: could not resize to {size:?} ({error})");
    }
}

/// Reads the config for this app.
pub fn load<R: Runtime>(app: &AppHandle<R>) -> Prefs {
    match app.path().app_config_dir() {
        Ok(dir) => Prefs::load(&dir.join(CONFIG_FILE)),
        Err(error) => {
            log::warn!("prefs: no config directory ({error}); using defaults");
            Prefs::default()
        }
    }
}
