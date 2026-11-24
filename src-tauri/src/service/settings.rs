use crate::error::AppError;
use crate::models::Settings;
use crate::Result;
use std::path::{Path, PathBuf};

/**
 * 便携模式的数据根目录。
 *
 * 判定依据是「可执行文件旁边有没有 `portable` 标记文件」，而不是
 * 「exe 所在目录可写」——后者会随 UAC、Program Files 权限漂移，
 * 同一个 exe 在不同机器上落到不同位置，用户无法预期数据在哪。
 *
 * 原版的做法更隐式：只要 exe 旁有本地配置文件就用它（platform_utils.py
 * 的 `get_config_file` 优先 local）。但那有个副作用——用户把 exe 拷到
 * 一个恰好有同名 ini 的目录旁就会莫名其妙切到便携模式。
 * 显式标记文件把「我要便携」变成用户的主动选择。
 */
pub fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    if dir.join("portable").exists() {
        Some(dir)
    } else {
        None
    }
}

/// 数据目录：便携模式用 exe 同级，否则用系统配置目录。
pub fn data_root() -> PathBuf {
    if let Some(root) = portable_root() {
        return root;
    }
    dirs::config_dir()
        .unwrap_or_default()
        .join("BlenderLauncher")
}

/// settings.json 位置。
///
/// 便携模式下是 `<exe dir>/settings.json`，否则
/// `%APPDATA%/BlenderLauncher/settings.json`。
pub fn settings_file_path() -> PathBuf {
    data_root().join("settings.json")
}

/// 缓存目录：便携模式 `<exe dir>/cache`，否则系统缓存目录。
pub fn cache_root() -> PathBuf {
    if let Some(root) = portable_root() {
        return root.join("cache");
    }
    dirs::cache_dir()
        .unwrap_or_default()
        .join("BlenderLauncher")
}

/// 默认库目录。
///
/// 便携模式必须落在 exe 同级，否则「拷走整个文件夹」带不走已下载的构建。
/// 非便携模式沿用原版语义：`%LOCALAPPDATA%/Blender Launcher`。
pub fn default_library_root() -> PathBuf {
    if let Some(root) = portable_root() {
        return root.join("builds");
    }
    dirs::data_local_dir()
        .unwrap_or_default()
        .join("Blender Launcher")
}

/// 库根目录下固定创建的子目录（原版 `library_subfolders`）。
///
/// `template` 不是构建类别，是 Blender 的启动模板目录；原版建库时一起建，
/// 少了它首次使用模板功能会报路径不存在。
pub const LIBRARY_SUBFOLDERS: [&str; 8] = [
    "stable",
    "daily",
    "experimental",
    "bforartists",
    "upbge-stable",
    "upbge-weekly",
    "custom",
    "template",
];

/// 建库目录骨架。幂等，已存在不报错。
pub fn create_library_folders(root: &Path) -> Result<()> {
    for sub in LIBRARY_SUBFOLDERS {
        std::fs::create_dir_all(root.join(sub)).map_err(AppError::from)?;
    }
    Ok(())
}

/// 加载设置；文件缺失或损坏时回退默认值（不阻断启动）。
pub fn load_settings() -> Settings {
    let path = settings_file_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!(error = %err, path = %path.display(), "corrupt settings file, using defaults");
                Settings::default()
            }
        },
        Err(_) => Settings::default(),
    }
}

/// 保存设置（原子写：先写临时文件再改名，避免半截 JSON）。
pub fn save_settings(settings: &Settings) -> Result<()> {
    let path = settings_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AppError::from)?;
    }
    let text = serde_json::to_string_pretty(settings)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(AppError::from)?;
    std::fs::rename(&tmp, &path).map_err(AppError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_roundtrip() {
        // 通过环境无关的临时文件验证序列化往返；实际路径由 load/save 内聚。
        let settings = Settings::default();
        let text = serde_json::to_string(&settings).unwrap();
        let parsed: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.install_dir, settings.install_dir);
        assert_eq!(parsed.theme, settings.theme);
        assert_eq!(
            parsed.max_concurrent_downloads,
            settings.max_concurrent_downloads
        );
    }

    #[test]
    fn test_portable_root_requires_marker_file() {
        // 当前测试二进制旁边没有 marker，应当判定为非便携。
        // 这条断言锁住判定依据：不是「目录可写」，而是标记文件存在。
        assert!(portable_root().is_none(), "测试环境不应判定为便携模式");
    }

    #[test]
    fn test_library_subfolders_match_original() {
        // 原版 settings.py 的 library_subfolders = build_library_folders + other_library_folders，
        // 共 8 项。少一项就会导致对应页面无法写入。
        assert_eq!(LIBRARY_SUBFOLDERS.len(), 8);
        for name in ["stable", "custom", "template"] {
            assert!(LIBRARY_SUBFOLDERS.contains(&name), "缺少必需子目录 {name}");
        }
    }

    #[test]
    fn test_create_library_folders_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("bl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create_library_folders(&dir).unwrap();
        // 第二次不应报错（每次启动都会跑一遍）。
        create_library_folders(&dir).unwrap();
        for sub in LIBRARY_SUBFOLDERS {
            assert!(dir.join(sub).is_dir(), "{sub} 未创建");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
