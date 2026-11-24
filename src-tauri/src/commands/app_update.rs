//! 本程序自身的版本检查（**不是** Blender 构建的更新检测）。
//!
//! 只查 GitHub Releases，不自动下载：替换正在运行的 exe 需要处理
//! 文件占用、回滚、权限等一堆问题，而这些已有的发布流程（手动下 exe）
//! 完全够用。这里只回答一个问题——「我是不是落后了」。
use crate::service::http::build_client;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::State;

const RELEASES_URL: &str =
    "https://api.github.com/repos/lonely814/Blender-Launcher-V3/releases/latest";
const TIMEOUT: Duration = Duration::from_secs(20);

/// 检查结果。`update_available` 为假时不带 `latest` 以外的字段，
/// 前端据此决定要不要显示提示条。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppUpdate {
    pub current: String,
    pub latest: String,
    pub update_available: bool,
    /// 发布页地址，供「去看看」按钮打开。
    pub html_url: String,
    /// 发布说明（GitHub 的 body）。可能很长，前端自行截断。
    #[serde(default)]
    pub notes: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
}

/// 查最新 release 并与当前版本比较。
///
/// 无发布、网络失败、JSON 结构变化都返回错误原文——这个功能是用户
/// 主动点的，静默失败会让人以为「已经是最新版」。
#[tauri::command]
pub async fn check_app_update(
    state: State<'_, AppState>,
) -> std::result::Result<AppUpdate, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let proxy = state.settings.lock().unwrap().proxy.clone();

    let client = build_client(&proxy, "blender-launcher-v3", TIMEOUT).map_err(String::from)?;
    let response = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|err| format!("检查更新失败: {err}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "检查更新失败: HTTP {}（GitHub 对匿名请求限流，稍后再试）",
            response.status()
        ));
    }

    // 不依赖 reqwest 的 `json` feature（本项目没开），自己解析。
    // 顺带把响应原文留在报错里，GitHub 改格式时能直接看出原因。
    let body = response
        .text()
        .await
        .map_err(|err| format!("读取发布信息失败: {err}"))?;
    let release: Release =
        serde_json::from_str(&body).map_err(|err| format!("解析发布信息失败: {err}"))?;

    let latest = release.tag_name.trim_start_matches('v').to_string();
    Ok(AppUpdate {
        update_available: is_newer(&latest, &current),
        current,
        latest,
        html_url: release.html_url,
        notes: release.body,
    })
}

/// 语义化版本比较；解析不了就当作「没有新版本」。
///
/// 宁可漏报也不要误报：误报会把用户推去下一个其实更旧的包。
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |value: &str| semver::Version::parse(value.trim().trim_start_matches('v')).ok();
    match (parse(candidate), parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// 供测试与前端共用的版本比较（命令之外没有别的调用点，故不导出）。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_semver() {
        assert!(is_newer("3.1.1", "3.1.0"));
        assert!(is_newer("3.2.0", "3.1.9"));
        assert!(!is_newer("3.1.0", "3.1.0"));
        assert!(!is_newer("3.0.9", "3.1.0"));
        // 预发布版本低于正式版：3.2.0-rc1 < 3.2.0。
        assert!(!is_newer("3.2.0-rc1", "3.2.0"));
    }

    #[test]
    fn test_is_newer_ignores_unparsable() {
        // 解析失败按「没有新版本」处理：误报会推用户去装更旧的包。
        assert!(!is_newer("nightly", "3.1.0"));
        assert!(!is_newer("3.1.1", ""));
    }

    #[test]
    fn test_release_json_shape() {
        // 取自 GitHub releases/latest 的真实字段（裁剪过）。
        let raw = r###"{
            "tag_name": "v3.1.0",
            "html_url": "https://github.com/lonely814/Blender-Launcher-V3/releases/tag/v3.1.0",
            "body": "## 3.1.0",
            "assets": []
        }"###;
        let release: Release = serde_json::from_str(raw).unwrap();
        assert_eq!(release.tag_name, "v3.1.0");
        assert_eq!(release.tag_name.trim_start_matches('v'), "3.1.0");
        // 缺 body 也不能让整条解析失败。
        let minimal: Release = serde_json::from_str(r#"{"tag_name":"v1.0.0"}"#).unwrap();
        assert_eq!(minimal.html_url, "");
    }
}
