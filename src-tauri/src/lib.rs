#![forbid(unsafe_code)]

mod commands;
mod state;

use state::AppState;

/// 创建主窗口，去掉系统自绘的标题栏：
/// - macOS：Overlay 风格（保留原生红黄绿，去掉系统标题栏，内容延伸到顶部）
/// - Windows / Linux：完全无边框，窗口按钮由前端自绘
fn build_main_window(app: &tauri::AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    let config = &app.config().app.windows[0];
    let mut builder = tauri::WebviewWindowBuilder::from_config(app, config)?;

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        builder = builder.decorations(false);
    }

    builder.build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new("untitled"))
        .setup(|app| {
            build_main_window(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_project,
            commands::add_track,
            commands::set_tempo,
            commands::clear_project,
            commands::history_depths,
            commands::undo,
            commands::redo,
            commands::save_project,
            commands::open_project,
            commands::render_preview,
            commands::export_wav,
        ])
        .run(tauri::generate_context!())
        .expect("error while running daw-party");
}
