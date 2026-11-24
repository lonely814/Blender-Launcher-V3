use crate::models::{BuildBranch, BuildFork};
use serde::Serialize;
use std::collections::HashMap;

/// 库统计信息 (docs/01 §3.4 `get_library_stats`)。
#[derive(Debug, Clone, Serialize)]
pub struct LibraryStats {
    pub total_builds: usize,
    pub total_size_bytes: u64,
    pub by_branch: HashMap<BuildBranch, usize>,
    pub by_fork: HashMap<BuildFork, usize>,
    /// 只存在于本地、匹配不到任何远程条目的版本 id。
    pub orphaned_builds: Vec<String>,
    /// 看起来是版本目录但缺少可执行文件的目录名（docs/00 §3.4 损坏检测）。
    pub corrupt_dirs: Vec<String>,
}

/// 清理结果 (docs/01 §3.4 `cleanup_library`)。
#[derive(Debug, Clone, Serialize)]
pub struct CleanupResult {
    pub deleted_files: usize,
    pub freed_bytes: u64,
}

/// 最近打开的项目 (docs/01 §3.3 `get_recent_projects`)。
/// 直接序列化，`last_opened` 为 Unix 秒，前端自行格式化。
#[derive(Debug, Clone, Serialize)]
pub struct RecentProject {
    pub blend_file: String,
    pub build_id: String,
    pub last_opened: i64,
}
