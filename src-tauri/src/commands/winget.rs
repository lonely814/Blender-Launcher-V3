use crate::state::AppState;
use tauri::State;

/// 注册表路径：HKCU 下，无需管理员权限。
#[cfg(target_os = "windows")]
const ARP_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\BlenderLauncherV2";

/// 把本程序登记到 Windows「应用和功能」列表（ARP）。
///
/// **刻意不写 `WinGetPackageIdentifier`。** 原版写的是作者自己的
/// `VictorIX.BlenderLauncher`，本实现如果照抄，`winget upgrade` 会去拉
/// 原版的安装包并覆盖用户现有安装 —— 这是供应链风险，不是功能。
/// 只有等本项目的清单正式提交到 microsoft/winget-pkgs 之后，
/// 才能写这个字段，届时值应替换成本项目自己的 PackageIdentifier。
///
/// 因此本命令的作用是：让程序出现在「应用和功能」里、可被正常卸载，
/// 而不是让 `winget upgrade` 接管更新。
#[tauri::command]
pub async fn register_with_winget(state: State<'_, AppState>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let exe = exe.display().to_string();
    let version = env!("CARGO_PKG_VERSION");

    #[cfg(target_os = "windows")]
    {
        // 安装位置 = 可执行文件所在目录。
        let install_location = std::path::Path::new(&exe)
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_default();

        let values: [(&str, &str, &str); 6] = [
            ("DisplayName", "Blender Launcher", "REG_SZ"),
            ("DisplayVersion", version, "REG_SZ"),
            ("Publisher", "Blender Launcher", "REG_SZ"),
            ("InstallLocation", &install_location, "REG_SZ"),
            ("DisplayIcon", &exe, "REG_SZ"),
            // 卸载走 NSIS 安装器自带的卸载程序；这里只登记，不接管卸载。
            ("UninstallString", &format!("\"{exe}\" uninstall"), "REG_SZ"),
        ];

        for (name, value, kind) in values {
            let output = std::process::Command::new("reg")
                .args(["add", ARP_KEY, "/v", name, "/t", kind, "/d", value, "/f"])
                .output()
                .map_err(|err| err.to_string())?;
            if !output.status.success() {
                return Err(format!(
                    "reg add {name} failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }

        // 原版从旧版本升级时确保 NoRemove 不残留（否则用户卸不掉）。
        let _ = std::process::Command::new("reg")
            .args(["delete", ARP_KEY, "/v", "NoRemove", "/f"])
            .output();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (exe, version);
        return Err("WinGet / ARP registration is only available on Windows".into());
    }

    // 记一笔，供设置页回显「已登记」。注意：这不代表 winget 可用。
    {
        let mut settings = state.settings.lock().unwrap();
        settings.auto_register_winget = true;
        let snapshot = settings.clone();
        drop(settings);
        crate::service::settings::save_settings(&snapshot).map_err(|err| err.to_string())?;
    }

    Ok(())
}

/// 是否已登记到 ARP（只查键是否存在，不校验内容）。
#[tauri::command]
pub async fn is_winget_registered() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("reg")
            .args(["query", ARP_KEY])
            .output()
            .map_err(|err| err.to_string())?;
        return Ok(output.status.success());
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(false)
    }
}
