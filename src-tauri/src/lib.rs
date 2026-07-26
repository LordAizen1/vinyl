mod art;
mod smtc;
mod state;

use std::sync::Arc;

use std::sync::mpsc::Sender;

use art::ArtCache;
use smtc::{Command, Signal};
use state::{PlaybackState, SharedState};
use tauri::{http, Manager};

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

            log::info!(
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
                    .body(bytes.to_vec())
                    .unwrap_or_default(),
                None => http::Response::builder()
                    .status(http::StatusCode::NOT_FOUND)
                    .body(Vec::new())
                    .unwrap_or_default(),
            }
        })
        .setup(move |app| {
            let sender = smtc::spawn(app.handle().clone(), worker_state, worker_cache);
            app.manage(sender);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_state, transport])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
