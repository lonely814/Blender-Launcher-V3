use crate::models::{Build, BuildBranch, BuildFork, Settings};
use crate::service::Scraper;
use crate::state::AppState;
use chrono::Utc;
use tauri::{Emitter, State};

/// 官方四源并发 + 按 settings 可选的 fork 源（docs/01 §2.4 开关）。
/// 单源失败只记日志，不阻塞其它来源。
async fn scrape_sources(settings: &Settings) -> Vec<Build> {
    let scraper = Scraper::with_proxy(&settings.proxy);
    let (official, bforartists, upbge) = tokio::join!(
        scraper.scrape_all(),
        async {
            if settings.scrape_bforartists {
                scraper
                    .fetch_builds(BuildFork::Bforartists, BuildBranch::Stable)
                    .await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if settings.scrape_upbge {
                scraper
                    .fetch_builds(BuildFork::Upbge, BuildBranch::Daily)
                    .await
            } else {
                Ok(Vec::new())
            }
        },
    );

    let mut builds = Vec::new();
    for source in [official, bforartists, upbge] {
        match source {
            Ok(list) => builds.extend(list),
            Err(err) => tracing::warn!(error = %err, "build source scrape failed"),
        }
    }

    // 只保留能在本机跑的包。
    //
    // 之前不过滤：下载页把 windows-arm64 的包也列出来，用户在 x64 机器上
    // 点了就真下 700 MB 的错包（2026-11-25 实测踩到）。构建来源页面本身
    // 就同时列了各架构，按本机架构筛是这一层该做的事。
    let platform = Scraper::detect_platform();
    let architecture = Scraper::detect_architecture();
    let before = builds.len();
    builds.retain(|build| build.platform == platform && build.architecture == architecture);
    if builds.len() != before {
        tracing::info!(
            kept = builds.len(),
            dropped = before - builds.len(),
            ?platform,
            ?architecture,
            "filtered builds by host platform"
        );
    }
    builds
}

#[derive(serde::Deserialize)]
pub struct BuildFilter {
    pub branch: Option<Vec<BuildBranch>>,
    pub fork: Option<Vec<BuildFork>>,
    pub installed_only: Option<bool>,
}

/// Scrapes remote builds into the cache when it is empty or older than the
/// TTL. A failed scrape still records the attempt, so a dead network does
/// not retry on every `get_builds` call.
async fn ensure_builds_cache(state: &AppState) {
    // 自动检查关闭时只要缓存非空就直接用，不再按 TTL 重爬。
    let (auto_check, interval_hours) = {
        let settings = state.settings.lock().unwrap();
        (
            settings.auto_check_updates,
            settings.update_check_interval_hours.max(1),
        )
    };
    let stale = {
        let last = state.builds_last_refresh.lock().unwrap();
        match *last {
            Some(at) => {
                if !auto_check {
                    false
                } else {
                    Utc::now() - at >= chrono::Duration::hours(interval_hours as i64)
                }
            }
            // 从未检查过：即使关了自动检查也要至少拉一次，
            // 否则列表永远是空的。
            None => true,
        }
    };
    if !stale {
        return;
    }
    {
        let mut refreshing = state.builds_refreshing.lock().unwrap();
        if *refreshing {
            return; // another scrape is already running; serve the cache
        }
        *refreshing = true;
    }

    let result = {
        let settings = state.settings.lock().unwrap().clone();
        scrape_sources(&settings).await
    };
    {
        let mut refreshing = state.builds_refreshing.lock().unwrap();
        *refreshing = false;
    }
    if result.is_empty() && !state.builds.lock().unwrap().is_empty() {
        // 全部来源失败时保留旧缓存：一次网络故障不应清空列表，
        // 也不应把空列表写进 SQLite（docs/01 §6.1 缓存语义）。
        tracing::warn!("scrape returned no builds; keeping existing cache");
    } else {
        // 保留本地合成条目，避免整表替换把已安装版本丢掉。
        let locals: Vec<Build> = state
            .builds
            .lock()
            .unwrap()
            .iter()
            .filter(|build| {
                build.url.starts_with(crate::service::library::LOCAL_URL_PREFIX)
                    || build.url.starts_with(crate::service::library::LIBRARY_URL_PREFIX)
            })
            .cloned()
            .collect();
        {
            let mut builds = state.builds.lock().unwrap();
            *builds = result;
            builds.extend(locals);
        }
        crate::commands::library::mark_updates(state);
        state.persist_builds();
    }
    *state.builds_last_refresh.lock().unwrap() = Some(Utc::now());
}

/// 获取所有版本 (缓存 + 按需刷新)
#[tauri::command]
pub async fn get_builds(
    filter: Option<BuildFilter>,
    state: State<'_, AppState>,
) -> Result<Vec<Build>, String> {
    ensure_builds_cache(&state).await;

    let builds = state.builds.lock().unwrap().clone();

    let mut filtered: Vec<Build> = builds
        .into_iter()
        .filter(|b| {
            if let Some(ref f) = filter {
                if let Some(ref branches) = f.branch {
                    if !branches.contains(&b.branch) {
                        return false;
                    }
                }
                if let Some(ref forks) = f.fork {
                    if !forks.contains(&b.fork) {
                        return false;
                    }
                }
                if let Some(installed) = f.installed_only {
                    if installed != b.is_installed {
                        return false;
                    }
                }
            }
            true
        })
        .collect();

    // docs/01 §3.1: 返回排序后的列表（最新发布在前）。
    filtered.sort_by_key(|build| std::cmp::Reverse(build.release_date));
    tracing::info!(returned = filtered.len(), "get_builds");
    Ok(filtered)
}

/// 获取单个版本详情（docs/01 §3.1 `get_build_details`）。
///
/// 返回 Build 加上 release notes 原文（稳定版有；其余为 None）。
/// 前端负责渲染，避免把 HTML 解析逻辑塞进 Rust。
#[tauri::command]
pub async fn get_build_details(
    build_id: String,
    state: State<'_, AppState>,
) -> Result<BuildDetails, String> {
    let build = {
        let builds = state.builds.lock().unwrap();
        builds
            .iter()
            .find(|b| b.id == build_id)
            .cloned()
            .ok_or_else(|| format!("Build not found: {build_id}"))?
    };

    let release_notes_html = Scraper::new()
        .fetch_release_notes(&build)
        .await
        .map_err(|err| err.to_string())?;

    Ok(BuildDetails {
        release_notes_html,
        build,
    })
}

/// `get_build_details` 的返回体（docs/01 §3.1 BuildDetails）。
#[derive(serde::Serialize)]
pub struct BuildDetails {
    #[serde(flatten)]
    pub build: Build,
    pub release_notes_html: Option<String>,
}

/// 模糊搜索版本（docs/01 §3.1 `search_builds`）。
///
/// 无第三方 fuzzy 库：版本号/平台/分支都是短字符串，评分用
/// 前缀 > 子串 > 子序列三级，足够满足「输入 4.5 或 daily 快速缩小」的场景。
#[tauri::command]
pub async fn search_builds(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<Build>, String> {
    let limit = limit.unwrap_or(20);
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }

    let builds = state.builds.lock().unwrap().clone();
    let mut scored: Vec<(i32, Build)> = builds
        .into_iter()
        .filter_map(|build| {
            let haystack = format!(
                "{} {:?} {:?} {:?}",
                build.version, build.branch, build.fork, build.platform
            )
            .to_lowercase();
            fuzzy_score(&haystack, &needle).map(|score| (score, build))
        })
        .collect();

    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    Ok(scored
        .into_iter()
        .take(limit)
        .map(|(_, build)| build)
        .collect())
}

/// 前缀 300 / 子串 200 / 子序列 100，未匹配返回 None。
fn fuzzy_score(haystack: &str, needle: &str) -> Option<i32> {
    if haystack.starts_with(needle) {
        return Some(300);
    }
    if haystack.contains(needle) {
        return Some(200);
    }
    // 子序列匹配：字符按序出现即可（"45" 命中 "4.5.17"）。
    let mut chars = haystack.chars();
    let matched = needle
        .chars()
        .all(|needle_char| chars.any(|haystack_char| haystack_char == needle_char));
    matched.then_some(100)
}

/// 上次检查时间（供状态栏 "Last check at ..." 显示，重启后仍保留）。
#[tauri::command]
pub async fn get_last_check(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state
        .builds_last_refresh
        .lock()
        .unwrap()
        .map(|at| at.to_rfc3339()))
}

/// 强制刷新版本列表 (重新爬取)，完成后发 `builds_updated` 事件。
#[tauri::command]
pub async fn refresh_builds(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use tauri::Manager;
    let handle = app_handle.clone();

    // docs/01 §3.1: 立即返回，后台爬取，完成后推送事件。
    let settings = state.settings.lock().unwrap().clone();
    tokio::spawn(async move {
        let builds_list = scrape_sources(&settings).await;
        if builds_list.is_empty() {
            // 全部来源失败时保留旧缓存，不发 builds_updated（列表未变化）。
            tracing::warn!("refresh returned no builds; keeping existing cache");
            let state = handle.state::<AppState>();
            *state.builds_last_refresh.lock().unwrap() = Some(Utc::now());
            return;
        }

        let state = handle.state::<AppState>();
        // 新列表只含远程条目：先保留本地合成条目，再用磁盘扫描恢复
        // is_installed / install_path，否则刷新一次已安装版本就全消失。
        let locals: Vec<Build> = state
            .builds
            .lock()
            .unwrap()
            .iter()
            .filter(|build| {
                build.url.starts_with(crate::service::library::LOCAL_URL_PREFIX)
                    || build.url.starts_with(crate::service::library::LIBRARY_URL_PREFIX)
            })
            .cloned()
            .collect();
        {
            let mut builds = state.builds.lock().unwrap();
            *builds = builds_list;
            builds.extend(locals);
        }
        if let Err(err) = crate::commands::library::rescan(&state, &handle).await {
            tracing::warn!(error = %err, "rescan after refresh failed");
        }
        state.persist_builds();

        if let Err(err) = handle.emit("builds_updated", ()) {
            tracing::warn!(error = %err, "failed to emit builds_updated");
        }
        *state.builds_last_refresh.lock().unwrap() = Some(Utc::now());
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Architecture, BuildFork, Platform};
    use chrono::Utc;

    fn build(version: &str, branch: BuildBranch) -> Build {
        Build {
            id: "x".into(),
            version: version.into(),
            branch,
            fork: BuildFork::Official,
            platform: Platform::Windows,
            architecture: Architecture::X64,
            url: "https://example.com/a.zip".into(),
            size_bytes: 1,
            release_date: Utc::now(),
            release_notes_url: None,
            breaking_changes: None,
            is_installed: false,
            install_path: None,
            update_build_id: None,
        }
    }

    #[test]
    fn test_release_notes_url() {
        assert_eq!(
            crate::service::scraper::release_notes_url(&build("4.5.17", BuildBranch::Stable)),
            Some("https://developer.blender.org/docs/release_notes/4.5/".to_string())
        );
        // 变体版本没有公开 notes
        assert!(crate::service::scraper::release_notes_url(&build(
            "5.4.0-alpha+main.1205aee3644f",
            BuildBranch::Daily
        ))
        .is_none());
        // 非稳定分支
        assert!(
            crate::service::scraper::release_notes_url(&build("5.3.0", BuildBranch::Patch))
                .is_none()
        );
    }

    #[test]
    fn test_fuzzy_score_ordering() {
        assert_eq!(fuzzy_score("4.5.17 stable", "4.5"), Some(300));
        assert_eq!(fuzzy_score("blender 4.5.17", "4.5"), Some(200));
        assert_eq!(fuzzy_score("4.5.17", "45"), Some(100));
        assert_eq!(fuzzy_score("4.5.17", "zz"), None);
    }
}
