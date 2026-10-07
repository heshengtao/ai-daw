#![forbid(unsafe_code)]

mod commands;
mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new("untitled"))
        .invoke_handler(tauri::generate_handler![
            commands::get_project,
            commands::add_track,
            commands::set_tempo,
            commands::clear_project,
            commands::load_demo,
            commands::render_preview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running daw-party");
}
