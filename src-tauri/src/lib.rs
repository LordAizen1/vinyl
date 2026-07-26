mod art;
mod lyrics;
mod menu;
mod prefs;
mod smtc;
mod state;
mod window;

use std::sync::Arc;

use std::sync::mpsc::Sender;

use art::ArtCache;
use lyrics::{Lyrics, SharedLyrics};
use menu::{AppMenu, PrefsState};
use parking_lot::Mutex;
use prefs::Prefs;
use smtc::{Command, Signal};
use state::{PlaybackState, SharedState};
use tauri::{http, Manager, Wry};

/// The frontend's initial read. Every update after this arrives as a
/// `playback-changed` event.
#[tauri::command]
fn get_state(state: tauri::State<'_, SharedState>) -> PlaybackState {
    state.read().clone()
}

/// Sends a transport action to the session the widget is showing.
///
/// Returns as soon as the command is queued. The frontend updates optimistically
/// and the real SMTC event reconciles it a moment later, so a slow source never
/// makes the button feel unresponsive.
#[tauri::command]
fn transport(action: &str, sender: tauri::State<'_, Sender<Signal>>) -> Result<(), String> {
    let command = match action {
        "toggle" => Command::Toggle,
        "next" => Command::Next,
        "previous" => Command::Previous,
        "shuffle" => Command::ToggleShuffle,
        "repeat" => Command::CycleRepeat,
        other => return Err(format!("unknown transport action: {other}")),
    };

    sender
        .send(Signal::Run(command))
        .map_err(|_| "the SMTC worker is not running".to_owned())
}

/// The frontend's initial read of the menu choices, so it can style itself on
/// load without waiting for a `prefs-changed` it may never get.
#[tauri::command]
fn get_prefs(state: tauri::State<'_, PrefsState>) -> Prefs {
    *state.0.lock()
}

/// The frontend's initial read of the lyrics.
///
/// Needed because Rust reaches a track well before the webview finishes
/// loading: without this the first `lyrics-changed` fires with nobody
/// listening, and the song playing at launch never gets its words.
#[tauri::command]
fn get_lyrics(state: tauri::State<'_, SharedLyrics>) -> Lyrics {
    state.read().clone()
}

/// Pops the context menu at the cursor. The webview owns the `contextmenu`
/// event, so the frontend has to ask for this rather than Tauri intercepting it.
#[tauri::command]
fn show_menu(window: tauri::Window, menu: tauri::State<'_, AppMenu<Wry>>) -> Result<(), String> {
    menu.popup(window).map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Default to info so a dev run says something useful without needing
    // RUST_LOG set. Override with RUST_LOG as usual.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let shared = state::shared();
    let cache = Arc::new(ArtCache::default());

    let worker_state = shared.clone();
    let worker_cache = cache.clone();

    tauri::Builder::default()
        .manage(shared)
        .manage(cache)
        // Album art is served over its own scheme so the bytes never cross the
        // Tauri bridge as base64 on a hot path. The frontend just points an
        // <image> at the URL for a given art_id.
        .register_uri_scheme_protocol("art", |ctx, request| {
            let id = request.uri().path().trim_start_matches('/').to_owned();

            let Some(cache) = ctx.app_handle().try_state::<Arc<ArtCache>>() else {
                return http::Response::builder()
                    .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Vec::new())
                    .unwrap_or_default();
            };

            log::debug!(
                "art: request for {id:?} -> {}",
                if cache.get(&id).is_some() {
                    "hit"
                } else {
                    "MISS"
                }
            );

            match cache.get(&id) {
                Some(bytes) => http::Response::builder()
                    .status(http::StatusCode::OK)
                    .header(http::header::CONTENT_TYPE, art::content_type(&bytes))
                    // art_id is a hash of the bytes, so a given URL can never
                    // change meaning. Cache it hard.
                    .header(
                        http::header::CACHE_CONTROL,
                        "public, max-age=31536000, immutable",
                    )
                    // The frontend samples the cover on a canvas to tint the
                    // screen. This scheme is a different origin to the webview,
                    // so without this the canvas is tainted and getImageData
                    // throws a SecurityError.
                    .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                    .body(bytes.to_vec())
                    .unwrap_or_default(),
                None => http::Response::builder()
                    .status(http::StatusCode::NOT_FOUND)
                    .body(Vec::new())
                    .unwrap_or_default(),
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();

            // Before the window is shown, so a compact config does not flash at
            // the full size first.
            let prefs = menu::load(&handle);
            if prefs.size != prefs::Size::Full {
                menu::apply_size(&handle, prefs.size);
            }

            app.manage(PrefsState(Mutex::new(prefs)));
            app.manage(menu::build(&handle, prefs)?);

            // Its own thread: a lookup blocks for up to 8s and must never sit
            // in front of the SMTC worker, which owns the session events.
            let current_lyrics = lyrics::shared();
            app.manage(current_lyrics.clone());
            app.manage(lyrics::spawn(handle.clone(), current_lyrics));

            // Before anything is shown, so it does not appear centred and then
            // jump to where it was left.
            window::restore(&handle, prefs.placement, prefs.size.dimensions());
            window::keep_on_desktop(handle.clone());
            app.manage(window::spawn_saver(handle.clone()));

            let sender = smtc::spawn(handle, worker_state, worker_cache);
            app.manage(sender);
            Ok(())
        })
        .on_menu_event(|app, event| menu::handle(app, event.id().as_ref()))
        // Dragging emits a stream of Moved events, so the write is debounced
        // rather than run per pixel: see menu::remember_placement.
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Moved(_)) {
                let app = window.app_handle();
                // Clamp first, then save, so what lands in config.json is the
                // corrected position rather than the one that went off-screen.
                // The clamp re-enters this handler with its own Moved event;
                // the second pass is already in bounds and does nothing.
                window::clamp_into_work_area(app);
                if let Some(saver) = app.try_state::<window::Saver>() {
                    let _ = saver.0.send(());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state, transport, get_prefs, get_lyrics, show_menu
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
