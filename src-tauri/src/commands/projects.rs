use crate::models::RecentProject;
use crate::state::AppState;
use std::path::Path;
use tauri::State;

/// 获取最近打开的项目 (docs/01 §3.3)。
///
/// 顺手清理磁盘上已不存在的条目，避免列表堆积失效路径。
#[tauri::command]
pub async fn get_recent_projects(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<RecentProject>, String> {
    let Some(db) = state.db.as_ref().map(std::sync::Arc::clone) else {
        return Ok(Vec::new()); // 内存模式：最近项目不可用
    };

    let limit = limit.unwrap_or(20);
    let projects = tokio::task::spawn_blocking(move || db.recent_projects(limit))
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())?;

    let mut alive = Vec::with_capacity(projects.len());
    for project in projects {
        if Path::new(&project.blend_file).exists() {
            alive.push(project);
        } else if let Some(db) = state.db.as_ref() {
            let file = project.blend_file.clone();
            let db = std::sync::Arc::clone(db);
            tokio::task::spawn_blocking(move || db.forget_recent_project(&file))
                .await
                .ok();
        }
    }
    Ok(alive)
}

/// 清除一条最近项目记录（文件被移动或用户手动移除时）。
#[tauri::command]
pub async fn forget_recent_project(
    blend_file: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let Some(db) = state.db.as_ref().map(std::sync::Arc::clone) else {
        return Ok(());
    };
    tokio::task::spawn_blocking(move || db.forget_recent_project(&blend_file))
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())
}
