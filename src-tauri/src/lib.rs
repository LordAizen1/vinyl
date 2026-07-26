mod art;
mod smtc;
mod state;

use std::sync::Arc;

use art::ArtCache;
use state::{PlaybackState, SharedState};
use tauri::{http, Manager};

/// The frontend's initial read. Every update after this arrives as a
/// `playback-changed` event.
#[tauri::command]
fn get_state(state: tauri::State<'_, SharedState>) -> PlaybackState {
    state.read().clone()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

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
            smtc::spawn(app.handle().clone(), worker_state, worker_cache);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_state])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
