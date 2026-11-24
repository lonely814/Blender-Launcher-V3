use crate::commands::launcher;
use crate::models::Build;
use crate::state::AppState;
use std::path::PathBuf;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

/// 快捷启动列表 = 构建的 install_path 集合（原版 quick_launch_paths）。
/// 原版存 QSettings，这里存 settings.json，字段 `quick_launch_paths`。
#[tauri::command]
pub async fn get_quick_launch(state: State<'_, AppState>) -> Result<Vec<Build>, String> {
    let paths = state.settings.lock().unwrap().quick_launch_paths.clone();
    let builds = state.builds.lock().unwrap();
    Ok(paths
        .iter()
        .filter_map(|path| {
            builds
                .iter()
                .find(|build| build.install_path.as_deref() == Some(PathBuf::from(path).as_path()))
                .cloned()
        })
        .collect())
}

/// 加入/移出快捷启动（原版 Add to / Remove from Quick Launch）。
#[tauri::command]
pub async fn toggle_quick_launch(
    build_id: String,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let path = {
        let builds = state.builds.lock().unwrap();
        builds
            .iter()
            .find(|build| build.id == build_id)
            .and_then(|build| build.install_path.clone())
            .ok_or_else(|| "Build is not installed".to_string())?
            .to_string_lossy()
            .into_owned()
    };

    let enabled = {
        let mut settings = state.settings.lock().unwrap();
        let Some(index) = settings.quick_launch_paths.iter().position(|p| p == &path) else {
            settings.quick_launch_paths.push(path);
            let snapshot = settings.clone();
            drop(settings);
            crate::service::settings::save_settings(&snapshot).map_err(|err| err.to_string())?;
            rebuild_tray(&app_handle)?;
            return Ok(true);
        };
        settings.quick_launch_paths.remove(index);
        let snapshot = settings.clone();
        drop(settings);
        crate::service::settings::save_settings(&snapshot).map_err(|err| err.to_string())?;
        rebuild_tray(&app_handle)?;
        false
    };
    let _ = enabled;
    Ok(false)
}

/// 快捷键触发：启动快捷启动列表里的第一个（原版 quick_launch）。
#[tauri::command]
pub async fn quick_launch(
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let paths = state.settings.lock().unwrap().quick_launch_paths.clone();
    let build_id = {
        let builds = state.builds.lock().unwrap();
        paths
            .iter()
            .find_map(|path| {
                builds
                    .iter()
                    .find(|build| build.install_path.as_deref() == Some(PathBuf::from(path).as_path()))
                    .map(|build| build.id.clone())
            })
            .ok_or_else(|| "Quick launch list is empty".to_string())?
    };
    launcher::launch_impl(&state, &app_handle, build_id, None, None, None, false)
        .await
        .map(|_| ())
}

/// 显示/隐藏主窗口（托盘左键、Alt+F11）。
pub fn toggle_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        match window.is_visible() {
            Ok(true) => {
                let _ = window.hide();
            }
            _ => {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }
    }
}

/// 重建托盘菜单（快捷启动项随列表变化）。
pub fn rebuild_tray(app: &AppHandle) -> Result<(), String> {
    use tauri::menu::IsMenuItem;

    let state = app.state::<AppState>();
    let paths = state.settings.lock().unwrap().quick_launch_paths.clone();
    let builds = state.builds.lock().unwrap().clone();

    let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
    for path in &paths {
        if let Some(build) = builds
            .iter()
            .find(|build| build.install_path.as_deref() == Some(PathBuf::from(path).as_path()))
        {
            let label = format!("Blender {} ({:?})", build.version, build.branch);
            let item = MenuItem::with_id(
                app,
                format!("quick:{}", build.id),
                label.to_lowercase(),
                true,
                None::<&str>,
            )
            .map_err(|err| err.to_string())?;
            items.push(Box::new(item));
        }
    }
    if !items.is_empty() {
        items.push(Box::new(
            PredefinedMenuItem::separator(app).map_err(|err| err.to_string())?,
        ));
    }

    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)
        .map_err(|err| err.to_string())?;
    let favorites = MenuItem::with_id(app, "favorites", "Favorites", true, None::<&str>)
        .map_err(|err| err.to_string())?;
    let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)
        .map_err(|err| err.to_string())?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)
        .map_err(|err| err.to_string())?;

    items.extend([
        Box::new(favorites) as Box<dyn IsMenuItem<tauri::Wry>>,
        Box::new(show),
        Box::new(hide),
        Box::new(quit),
    ]);
    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|item| item.as_ref()).collect();
    let menu = Menu::with_items(app, &refs).map_err(|err| err.to_string())?;

    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_menu(Some(menu)).map_err(|err| err.to_string())?;
    }
    Ok(())
}

/// 创建托盘图标（启动时调用一次）。
pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    // 无内置图标（资源损坏）时跳过托盘，不影响主窗口启动。
    let Some(icon) = app.default_window_icon().cloned() else {
        tracing::warn!("no window icon available; tray icon skipped");
        return Ok(());
    };

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Blender Launcher")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => toggle_window(app),
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "favorites" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                let _ = app.emit("show_favorites", ());
            }
            "quit" => app.exit(0),
            id if id.starts_with("quick:") => {
                let build_id = id.trim_start_matches("quick:").to_string();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    let _ = launcher::launch_impl(
                        &state, &app, build_id, None, None, None, false,
                    )
                    .await;
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;

    rebuild_tray(app).map_err(|err| tauri::Error::Anyhow(anyhow::anyhow!(err)))?;
    Ok(())
}

/// 全局快捷启动热键（原版 pynput 的 alt+f11 → 启动快捷启动列表第一个）。
/// 注册失败（热键被占用/语法非法）只记日志，不阻断启动。
pub fn register_quick_launch_hotkey(app: &AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let state = app.state::<AppState>();
    let key_seq = state.settings.lock().unwrap().quick_launch_key_seq.clone();
    let Some(shortcut) = parse_shortcut(&key_seq) else {
        tracing::warn!(key_seq, "unsupported quick launch hotkey");
        return;
    };

    let handle = app.clone();
    if let Err(err) = app.global_shortcut().on_shortcut(shortcut, move |_, _, _| {
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            let state = handle.state::<AppState>();
            let _ = quick_launch(handle.clone(), state).await;
        });
    }) {
        tracing::warn!(error = %err, "failed to register quick launch hotkey");
    }
}

/// 解析 `Alt+F11` 这类组合键（原版用 pynput 的 `<alt>+<f11>` 语法）。
fn parse_shortcut(text: &str) -> Option<tauri_plugin_global_shortcut::Shortcut> {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

    let mut modifiers = Modifiers::empty();
    let mut code: Option<Code> = None;
    for part in text.split('+') {
        match part.trim().to_lowercase().as_str() {
            "alt" => modifiers |= Modifiers::ALT,
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "meta" | "super" | "win" => modifiers |= Modifiers::SUPER,
            key => code = Some(parse_key(key)?),
        }
    }
    Some(Shortcut::new(Some(modifiers), code?))
}

fn parse_key(key: &str) -> Option<tauri_plugin_global_shortcut::Code> {
    use tauri_plugin_global_shortcut::Code;
    let code = match key {
        "f1" => Code::F1,
        "f2" => Code::F2,
        "f3" => Code::F3,
        "f4" => Code::F4,
        "f5" => Code::F5,
        "f6" => Code::F6,
        "f7" => Code::F7,
        "f8" => Code::F8,
        "f9" => Code::F9,
        "f10" => Code::F10,
        "f11" => Code::F11,
        "f12" => Code::F12,
        "space" => Code::Space,
        single if single.len() == 1 => {
            let ch = single.chars().next()?;
            // Code 枚举没有运行时构造，覆盖常用字母/数字即可。
            match ch {
                'a' => Code::KeyA,
                'b' => Code::KeyB,
                'c' => Code::KeyC,
                'd' => Code::KeyD,
                'e' => Code::KeyE,
                'f' => Code::KeyF,
                'g' => Code::KeyG,
                'h' => Code::KeyH,
                'i' => Code::KeyI,
                'j' => Code::KeyJ,
                'k' => Code::KeyK,
                'l' => Code::KeyL,
                'm' => Code::KeyM,
                'n' => Code::KeyN,
                'o' => Code::KeyO,
                'p' => Code::KeyP,
                'q' => Code::KeyQ,
                'r' => Code::KeyR,
                's' => Code::KeyS,
                't' => Code::KeyT,
                'u' => Code::KeyU,
                'v' => Code::KeyV,
                'w' => Code::KeyW,
                'x' => Code::KeyX,
                'y' => Code::KeyY,
                'z' => Code::KeyZ,
                '0' => Code::Digit0,
                '1' => Code::Digit1,
                '2' => Code::Digit2,
                '3' => Code::Digit3,
                '4' => Code::Digit4,
                '5' => Code::Digit5,
                '6' => Code::Digit6,
                '7' => Code::Digit7,
                '8' => Code::Digit8,
                '9' => Code::Digit9,
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(code)
}
