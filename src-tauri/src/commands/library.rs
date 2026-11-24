use crate::error::AppError;
use crate::models::{Build, CleanupResult, LibraryStats};
use crate::service::library::{LibraryManager, LIBRARY_URL_PREFIX, LOCAL_URL_PREFIX};
use crate::service::scraper::archive_stem;
use crate::state::AppState;
use std::collections::HashMap;
use tauri::{Emitter, State};

/// 扫描库文件夹，检测已安装版本并合并进缓存 (docs/01 §3.4)。
#[tauri::command]
pub async fn scan_library(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    rescan(&state, &app_handle).await
}

/// `scan_library` 的可复用实现（其他命令内部也要重扫）。
pub(crate) async fn rescan(state: &AppState, app_handle: &tauri::AppHandle) -> Result<(), String> {
    let install_dir = state.settings.lock().unwrap().install_dir.clone();
    let scan = LibraryManager::scan(&install_dir);
    *state.library_corrupt_dirs.lock().unwrap() = scan.corrupt_dirs;

    let mut builds = state.builds.lock().unwrap().clone();

    // 每次扫描干净重建本地状态：清掉上次的合成条目，并把远程条目
    // 的安装标记归零，避免目录已被删除后仍显示为「已安装」。
    builds.retain(|build| {
        !build.url.starts_with(LOCAL_URL_PREFIX) && !build.url.starts_with(LIBRARY_URL_PREFIX)
    });
    for build in builds.iter_mut() {
        build.is_installed = false;
        build.install_path = None;
        build.update_build_id = None;
    }

    for local in scan.builds {
        let Some(dir_name) = local
            .install_path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
        else {
            continue;
        };

        // 优先按「归档 stem == 目录名」精确匹配（原版 renamer 会把目录改成
        // blender-<版本>，与 remote 的归档名未必一致，所以再按版本+fork+平台兜底）。
        let is_extractable = |url: &str| {
            url.ends_with(".zip")
                || url.ends_with(".tar.xz")
                || url.ends_with(".tar.gz")
                || url.ends_with(".dmg")
        };
        let match_index = builds
            .iter()
            // 幂等：同一安装路径已存在则复用，避免重复扫描产生重复条目。
            .position(|existing| {
                existing.is_installed && existing.install_path.as_ref() == local.install_path.as_ref()
            })
            .or_else(|| {
                builds.iter().position(|remote| {
                    archive_stem(&remote.url).as_deref() == Some(dir_name)
                        && is_extractable(&remote.url)
                })
            })
            .or_else(|| {
                builds
                    .iter()
                    .position(|remote| archive_stem(&remote.url).as_deref() == Some(dir_name))
            })
            .or_else(|| {
                // 版本号 + fork + 平台相同即视为同一构建（.blinfo/目录名解析出的版本）。
                builds.iter().position(|remote| {
                    remote.version == local.version
                        && remote.fork == local.fork
                        && remote.platform == local.platform
                        && !remote.is_installed
                })
            });

        match match_index {
            Some(index) => {
                let remote = &mut builds[index];
                remote.is_installed = true;
                remote.install_path = local.install_path.clone();
            }
            None => builds.push(local), // 手动安装或已下架版本 → 孤立条目
        }
    }

    *state.builds.lock().unwrap() = builds;
    mark_updates(&state);
    sync_blinfo_favorites(&state);
    state.persist_builds();
    let _ = app_handle.emit("library_scanned", ());
    Ok(())
}

/// 把 `.blinfo` 里 `is_favorite: true` 的构建并入收藏表。
/// 原版把收藏写在 `.blinfo`，本实现用 SQLite；两边共存时取并集，
/// 这样从原版迁移过来的库不会丢收藏。
fn sync_blinfo_favorites(state: &AppState) {
    let Some(db) = state.db.as_ref().map(std::sync::Arc::clone) else {
        return;
    };
    let favorites: Vec<String> = state
        .builds
        .lock()
        .unwrap()
        .iter()
        .filter(|build| {
            build.is_installed
                && build
                    .install_path
                    .as_deref()
                    .and_then(crate::service::library::read_blinfo)
                    .is_some_and(|entry| entry.is_favorite)
        })
        .map(|build| build.id.clone())
        .collect();
    if favorites.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for id in favorites {
            if let Err(err) = db.set_favorite(&id, true) {
                tracing::warn!(error = %err, "failed to sync .blinfo favorite");
            }
        }
    });
}

/// 给每个已安装条目找同 fork/branch/平台/架构 的最新远程版本，
/// 写进 `update_build_id`（前端据此把 Launch 变 Update）。
/// 本地 Custom 条目（`local://`）不参与——外来安装无法知道上游版本。
pub(crate) fn mark_updates(state: &AppState) {
    use crate::commands::build_entry::{comparable, is_newer};

    let mut builds = state.builds.lock().unwrap();
    let remote: Vec<Build> = builds
        .iter()
        .filter(|build| !build.is_installed)
        .cloned()
        .collect();

    for build in builds.iter_mut() {
        build.update_build_id = None;
        if !build.is_installed || build.url.starts_with(LOCAL_URL_PREFIX) {
            continue;
        }
        let mut best: Option<&Build> = None;
        for candidate in remote.iter().filter(|c| comparable(c, build)) {
            if !is_newer(candidate, build) {
                continue;
            }
            if best.is_none_or(|current| is_newer(candidate, current)) {
                best = Some(candidate);
            }
        }
        build.update_build_id = best.map(|candidate| candidate.id.clone());
    }
}

/// 删除已安装版本 (docs/01 §3.4)。
#[tauri::command]
pub async fn remove_build(
    build_id: String,
    delete_user_data: bool,
    to_trash: Option<bool>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (install_path, version, is_local_only) = {
        let builds = state.builds.lock().unwrap();
        let build = builds
            .iter()
            .find(|b| b.id == build_id)
            .ok_or_else(|| format!("Build not found: {build_id}"))?;
        let Some(path) = build.install_path.clone() else {
            return Err(format!("Build {build_id} is not installed"));
        };
        (
            path,
            build.version.clone(),
            build.url.starts_with(LOCAL_URL_PREFIX),
        )
    };

    // 破坏性操作：删除 install_path（及可选的用户配置目录）。
    // 原版默认 Send to Trash；调用方显式传 false 才永久删除。
    LibraryManager::remove(&install_path, &version, delete_user_data, to_trash.unwrap_or(true))
        .map_err(|err| err.to_string())?;

    let mut builds = state.builds.lock().unwrap();
    if is_local_only {
        builds.retain(|b| b.id != build_id);
    } else if let Some(build) = builds.iter_mut().find(|b| b.id == build_id) {
        build.is_installed = false;
        build.install_path = None;
    }
    drop(builds);
    state.persist_builds();
    Ok(())
}

/// 获取库统计信息 (docs/01 §3.4)。
#[tauri::command]
pub async fn get_library_stats(state: State<'_, AppState>) -> Result<LibraryStats, String> {
    let builds = state.builds.lock().unwrap();
    let installed: Vec<&Build> = builds.iter().filter(|b| b.is_installed).collect();

    let mut stats = LibraryStats {
        total_builds: installed.len(),
        total_size_bytes: 0,
        by_branch: HashMap::new(),
        by_fork: HashMap::new(),
        orphaned_builds: Vec::new(),
        corrupt_dirs: Vec::new(),
    };

    for build in installed {
        if let Some(path) = build.install_path.as_ref() {
            stats.total_size_bytes += LibraryManager::dir_size(path);
        }
        *stats.by_branch.entry(build.branch).or_default() += 1;
        *stats.by_fork.entry(build.fork).or_default() += 1;
        if build.url.starts_with(LOCAL_URL_PREFIX) {
            stats.orphaned_builds.push(build.id.clone());
        }
    }
    stats.corrupt_dirs = state.library_corrupt_dirs.lock().unwrap().clone();
    Ok(stats)
}

/// 清理临时文件（*.part 下载残留）。孤立版本不在此删除，
/// 由 remove_build 显式处理，避免误删手动安装的版本。
#[tauri::command]
pub async fn cleanup_library(state: State<'_, AppState>) -> Result<CleanupResult, String> {
    let install_dir = state.settings.lock().unwrap().install_dir.clone();
    let mut result = CleanupResult {
        deleted_files: 0,
        freed_bytes: 0,
    };

    let Ok(entries) = std::fs::read_dir(&install_dir) else {
        return Ok(result);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_part = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".part"));
        if !is_part {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        std::fs::remove_file(&path)
            .map_err(AppError::from)
            .map_err(|err| err.to_string())?;
        result.deleted_files += 1;
        result.freed_bytes += size;
    }
    Ok(result)
}

/// 收藏的版本 id 列表（原版 "Add To Favorites"，用于托盘与 User 页 Favorites 分组）。
#[tauri::command]
pub async fn get_favorites(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let Some(db) = state.db.as_ref().map(std::sync::Arc::clone) else {
        return Ok(Vec::new());
    };
    tokio::task::spawn_blocking(move || db.favorite_ids())
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())
}

/// 加/取消收藏。
#[tauri::command]
pub async fn set_favorite(
    build_id: String,
    favorite: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let Some(db) = state.db.as_ref().map(std::sync::Arc::clone) else {
        return Ok(());
    };
    tokio::task::spawn_blocking(move || db.set_favorite(&build_id, favorite))
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())
}

/// 在系统文件管理器中打开目录（右键 "Show Folder"）。
#[tauri::command]
pub async fn open_in_explorer(path: String) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.exists() {
        return Err(format!("Path does not exist: {path}"));
    }
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(target).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(target).spawn();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(target).spawn();

    result.map(|_| ()).map_err(|err| err.to_string())
}
