use crate::error::AppError;
use crate::models::Settings;
use crate::Result;
use std::path::PathBuf;

/// settings.json 位置：%APPDATA%/BlenderLauncher/settings.json。
/// docs/00 §1.2 允许 YAML/JSON；JSON 无需额外依赖（serde_json 已有）。
pub fn settings_file_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_default()
        .join("BlenderLauncher")
        .join("settings.json")
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
}
