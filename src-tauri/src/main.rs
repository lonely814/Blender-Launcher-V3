// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use blender_launcher_lib::commands;
use blender_launcher_lib::service::settings::{
    create_library_folders, load_settings, portable_root,
};
use blender_launcher_lib::state::AppState;
use tauri::Manager;


fn main() {
    tracing_subscriber::fmt::init();

    // 启动时从 settings.json 加载设置（缺失/损坏回退默认值）。
    let settings = load_settings();

    // 便携模式：exe 旁有 `portable` 标记文件时，配置/缓存/库全部落在 exe 同级。
    // 打一行日志，用户能从控制台一眼确认数据会被写到哪。
    if let Some(root) = portable_root() {
        tracing::info!(path = %root.display(), "portable mode");
    }

    // 建库骨架：原版启动即创建全部 8 个子目录（含 template）。
    // 幂等，失败只记警告——目录不可写时应用仍应能打开，
    // 用户可以在设置里改到别的路径。
    if let Err(err) = create_library_folders(&settings.install_dir) {
        tracing::warn!(error = %err, path = %settings.install_dir.display(), "failed to create library folders");
    }

    // 单实例：已有实例在跑时，只请求它露脸然后退出。
    // 放在最前面，避免第二个进程去扫库、抢热键、写同一个 SQLite。
    let Some(guard) =
        blender_launcher_lib::single_instance::InstanceGuard::acquire(&settings.cache_dir)
    else {
        blender_launcher_lib::single_instance::InstanceGuard::request_focus(&settings.cache_dir);
        tracing::info!("another instance is running; asked it to show and exiting");
        return;
    };

    let mut state = AppState::with_settings(settings.clone());

    // SQLite 缓存：恢复上次爬取的版本列表（含 is_installed 状态）。
    // 打开失败不阻断启动，仅记日志并退化为纯内存模式。
    match blender_launcher_lib::db::Db::open(&blender_launcher_lib::db::default_db_path(
        &settings.cache_dir,
    )) {
        Ok(db) => {
            match db.load_builds() {
                Ok((builds, last_refresh)) => {
                    let count = builds.len();
                    *state.builds.lock().unwrap() = builds;
                    *state.builds_last_refresh.lock().unwrap() = last_refresh;
                    tracing::info!(count, "builds cache restored from database");
                }
                Err(err) => tracing::warn!(error = %err, "failed to load builds cache"),
            }
            state.db = Some(std::sync::Arc::new(db));
        }
        Err(err) => {
            tracing::warn!(error = %err, "failed to open database, running in memory-only mode")
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(state)
        .setup(|app| {
            let handle = app.handle().clone();
            // 托盘：初始化失败不阻断启动（无托盘环境很常见）。
            let prefs = {
                let state = app.state::<AppState>();
                let settings = state.settings.lock().unwrap();
                (settings.show_tray_icon, settings.launch_minimized_to_tray)
            };

            // show_tray_icon=false 时不建托盘（原版语义：托盘是可选常驻入口）。
            if prefs.0 {
                if let Err(err) = commands::tray::setup_tray(&handle) {
                    tracing::warn!(error = %err, "failed to set up tray icon");
                }
                commands::tray::register_quick_launch_hotkey(&handle);
            }

            // launch_minimized_to_tray：启动即隐藏，只能从托盘唤出。
            // 没有托盘时强制显示，否则窗口再也打不开。
            if prefs.1 && prefs.0 {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            // 单实例守卫移交后台：轮询「另一个实例请求露脸」的标记。
            blender_launcher_lib::single_instance::start_watchdog(handle, guard);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::builds::get_builds,
            commands::builds::refresh_builds,
            commands::builds::get_last_check,
            commands::builds::get_build_details,
            commands::builds::search_builds,
            commands::projects::get_recent_projects,
            commands::projects::forget_recent_project,
            commands::downloads::start_download,
            commands::downloads::pause_download,
            commands::downloads::resume_download,
            commands::downloads::cancel_download,
            commands::downloads::get_active_downloads,
            commands::launcher::launch_blender,
            commands::launcher::get_running_instances,
            commands::launcher::kill_instance,
            commands::library::scan_library,
            commands::library::remove_build,
            commands::library::get_library_stats,
            commands::library::cleanup_library,
            commands::library::get_favorites,
            commands::library::set_favorite,
            commands::library::open_in_explorer,
            commands::build_entry::list_executables,
            commands::build_entry::detect_build_info,
            commands::build_entry::save_custom_build,
            commands::build_entry::move_build_to_category,
            commands::build_entry::create_shortcut,
            commands::build_entry::create_symlink,
            commands::build_entry::toggle_portable,
            commands::build_entry::is_portable,
            commands::build_entry::register_file_association,
            commands::build_entry::install_template,
            commands::build_entry::get_build_hash,
            commands::winget::register_with_winget,
            commands::winget::is_winget_registered,
            commands::tray::get_quick_launch,
            commands::tray::toggle_quick_launch,
            commands::tray::quick_launch,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::set_first_run_done,
            commands::settings::pick_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
