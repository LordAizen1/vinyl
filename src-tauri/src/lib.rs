mod smtc;
mod state;

use state::{PlaybackState, SharedState};

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

    tauri::Builder::default()
        .manage(shared.clone())
        .setup(move |app| {
            smtc::spawn(app.handle().clone(), shared.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_state])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
