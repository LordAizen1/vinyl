//! The right-click menu: window size and palette.
//!
//! Built once at startup and popped up on demand, because rebuilding it per
//! click would lose the tick marks. The frontend forwards its `contextmenu`
//! event to `show_menu`; everything else happens here.

use std::sync::mpsc::Sender;

use parking_lot::Mutex;
use tauri::menu::{CheckMenuItem, ContextMenu, Menu, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, Runtime, Window};

use crate::lyrics::Query;
use crate::prefs::{Prefs, Size, Theme};
use crate::state::SharedState;

/// Which settings a menu click actually moved. Both need work beyond writing
/// the value, and neither should run when the other was clicked.
struct Changed {
    size: bool,
    lyrics: bool,
}

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
const LYRICS: &str = "lyrics";
const VISIBLE: &str = "visible";
const QUIT: &str = "quit";

/// The menu, plus the check items that have to be re-ticked after a choice.
pub struct AppMenu<R: Runtime> {
    menu: Menu<R>,
    size_items: [(CheckMenuItem<R>, Size); 2],
    theme_items: [(CheckMenuItem<R>, Theme); 3],
    lyrics_item: CheckMenuItem<R>,
    visible_item: CheckMenuItem<R>,
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

    let appearance = Submenu::with_items(app, "Appearance", true, &[&auto, &light, &dark])?;

    let lyrics = check(LYRICS, "Show lyrics", prefs.lyrics)?;
    // Ticked because the widget starts visible. The tray is the only way back
    // once it is not: there is no taskbar entry to click.
    let visible = check(VISIBLE, "Show vinyl", true)?;

    let menu = Menu::with_items(
        app,
        &[
            &visible,
            &PredefinedMenuItem::separator(app)?,
            &full,
            &compact,
            &PredefinedMenuItem::separator(app)?,
            &appearance,
            &lyrics,
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
        lyrics_item: lyrics,
        visible_item: visible,
    })
}

impl<R: Runtime> AppMenu<R> {
    /// For the tray, which shows the same menu.
    pub fn menu(&self) -> &Menu<R> {
        &self.menu
    }

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
        // A plain on/off, so unlike the groups above it needs no exclusivity.
        let _ = self.lyrics_item.set_checked(prefs.lyrics);
    }

    /// Visibility is not a preference, so it is ticked from the window itself
    /// rather than from `Prefs`.
    pub fn set_visible(&self, visible: bool) {
        let _ = self.visible_item.set_checked(visible);
    }
}

/// Toggles the widget, keeping the tick in step.
///
/// Hiding a window with no taskbar entry and no title bar makes the tray the
/// only way to get it back, which is most of why the tray exists.
pub fn set_visible<R: Runtime>(app: &AppHandle<R>, visible: bool) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let outcome = if visible { window.show() } else { window.hide() };
    if let Err(error) = outcome {
        log::warn!("tray: could not toggle the widget ({error})");
        return;
    }

    if let Some(menu) = app.try_state::<AppMenu<R>>() {
        menu.set_visible(visible);
    }
}

/// The tray icon, sharing the widget's own menu.
///
/// One menu for both, so the two can never drift into offering different
/// things. Left-clicking the icon toggles the widget, which is what a tray icon
/// is expected to do and saves opening the menu for the common case.
pub fn build_tray<R: Runtime>(app: &AppHandle<R>, menu: &Menu<R>) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id("vinyl")
        .tooltip("vinyl")
        .menu(menu)
        // Windows convention: left-click is the action, right-click the menu.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button, .. } = event {
                if button == tauri::tray::MouseButton::Left {
                    let app = tray.app_handle();
                    let showing = app
                        .get_webview_window("main")
                        .and_then(|w| w.is_visible().ok())
                        .unwrap_or(true);
                    set_visible(app, !showing);
                }
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

/// Applies a menu choice: resize if the size changed, persist, re-tick, tell the
/// frontend.
pub fn handle<R: Runtime>(app: &AppHandle<R>, id: &str) {
    // Quit is a predefined item and Tauri handles it itself.
    if id == QUIT {
        return;
    }

    // Visibility is a window state, not a saved preference, so it is handled
    // before the settings are touched at all.
    if id == VISIBLE {
        let showing = app
            .get_webview_window("main")
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(true);
        set_visible(app, !showing);
        return;
    }

    let Some(state) = app.try_state::<PrefsState>() else {
        return;
    };

    let (prefs, changed) = {
        let mut current = state.0.lock();
        let before = *current;

        match id {
            SIZE_FULL => current.size = Size::Full,
            SIZE_COMPACT => current.size = Size::Compact,
            THEME_AUTO => current.theme = Theme::Auto,
            THEME_LIGHT => current.theme = Theme::Light,
            THEME_DARK => current.theme = Theme::Dark,
            LYRICS => current.lyrics = !current.lyrics,
            other => {
                log::warn!("menu: unknown item {other:?}");
                return;
            }
        }

        (
            *current,
            Changed {
                size: current.size != before.size,
                lyrics: current.lyrics != before.lyrics,
            },
        )
    };

    if changed.size {
        apply_size(app, prefs.size);
    }

    // Push the lyrics setting rather than waiting for the next track change.
    // Nothing else would: `publish` only runs when playback moves, so toggling
    // during a paused song used to leave the old words frozen on screen.
    if changed.lyrics {
        if let Some(sender) = app.try_state::<Sender<Option<Query>>>() {
            let query = prefs.lyrics.then(|| {
                app.try_state::<SharedState>()
                    .and_then(|state| crate::lyrics::query_for(&state.read(), true))
            });
            // `None` is the clear signal; turning it back on re-asks for the
            // track that is playing now, which the worker's cache usually
            // answers without another request.
            let _ = sender.send(query.flatten());
        }
    }

    persist(app, prefs);

    if let Some(menu) = app.try_state::<AppMenu<R>>() {
        menu.retick(prefs);
    }

    if let Err(error) = app.emit(CHANGED_EVENT, prefs) {
        log::warn!("menu: could not tell the frontend ({error})");
    }
}

/// Writes the settings out. Shared with the window-position saver, which has to
/// write the same file from its own thread.
pub fn persist<R: Runtime>(app: &AppHandle<R>, prefs: Prefs) {
    match app.path().app_config_dir() {
        Ok(dir) => prefs.save(&dir.join(CONFIG_FILE)),
        Err(error) => log::warn!("prefs: no config directory ({error}), not saving"),
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
        return;
    }

    // Growing keeps the top-left corner where it is, so a widget parked against
    // the right edge pushes its extra width straight off the screen. Pull it
    // back so the right edge lands on the work area instead.
    crate::window::clamp_into_work_area(app);
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
