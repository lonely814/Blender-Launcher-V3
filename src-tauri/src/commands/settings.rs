use crate::models::Settings;
use crate::service::settings;
use crate::state::AppState;
use tauri::{Emitter, State};

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let settings = state.settings.lock().unwrap();
    Ok(settings.clone())
}

/// 更新设置并持久化到 settings.json (docs/01 §3.5)。
#[tauri::command]
pub async fn update_settings(
    new_settings: Settings,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    // 先落盘再更新内存：写失败时内存保持旧值，两端一致。
    settings::save_settings(&new_settings).map_err(|err| err.to_string())?;

    let (install_dir_changed, previous_max) = {
        let current = state.settings.lock().unwrap();
        (
            current.install_dir != new_settings.install_dir,
            current.max_concurrent_downloads,
        )
    };
    *state.settings.lock().unwrap() = new_settings.clone();

    // 换了库目录必须重扫，否则界面还显示旧目录里的构建。
    if install_dir_changed {
        if let Err(err) = crate::commands::library::rescan(&state, &app_handle).await {
            tracing::warn!(error = %err, "rescan after install_dir change failed");
        }
    }

    // 并发数在启动时用于构造 Semaphore（Arc 内部不可替换），
    // 所以改动只在下次启动生效——与原版一致，此处仅记日志。
    if new_settings.max_concurrent_downloads != previous_max {
        tracing::info!(
            value = new_settings.max_concurrent_downloads,
            "max_concurrent_downloads takes effect on next launch"
        );
    }

    // 快捷启动列表可能变了，托盘菜单要跟着重建。
    if let Err(err) = crate::commands::tray::rebuild_tray(&app_handle) {
        tracing::warn!(error = %err, "failed to rebuild tray menu");
    }

    // settings_changed 事件 (docs/01 §4)。
    let _ = app_handle.emit("settings_changed", &new_settings);
    Ok(())
}

/// 首启动向导完成后置位（原版 onboarding 向导）。
#[tauri::command]
pub async fn set_first_run_done(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let snapshot = {
        let mut settings = state.settings.lock().unwrap();
        settings.first_run_done = true;
        settings.clone()
    };
    settings::save_settings(&snapshot).map_err(|err| err.to_string())?;
    let _ = app_handle.emit("settings_changed", &snapshot);
    Ok(())
}

/// 原生目录选择框（前端用 @tauri-apps/plugin-dialog 亦可，这里给 Rust 侧兜底）。
#[tauri::command]
pub async fn pick_directory(app_handle: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = app_handle
        .dialog()
        .file()
        .blocking_pick_folder()
        .map(|path| path.to_string());

    Ok(picked)
}
