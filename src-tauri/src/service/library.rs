use crate::error::AppError;
use crate::models::{Architecture, Build, BuildBranch, BuildFork, Platform};
use crate::service::scraper::{build_id, parse_builder_stem, parse_release_stem, Scraper};
use crate::Result;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// 合成 URL 前缀。
/// - `local://`  → `<library>/custom/` 下用户手动添加的外来安装（原版 Custom 页）
/// - `library://` → 本程序下载安装、但已不在远程列表里的构建（仍属其类别页）
pub const LOCAL_URL_PREFIX: &str = "local://";
pub const LIBRARY_URL_PREFIX: &str = "library://";

/// 扫描结果：有效版本 + 损坏目录（docs/00 §3.4）。
pub struct ScanResult {
    pub builds: Vec<Build>,
    pub corrupt_dirs: Vec<String>,
}

/// 原版的库目录结构：`<library>/<类别>/<blender-xxx>/`。
/// 类别 → (fork, branch)，与原版 `build_library_folders` 一致。
const CATEGORY_DIRS: [(&str, BuildFork, BuildBranch); 8] = [
    ("stable", BuildFork::Official, BuildBranch::Stable),
    ("daily", BuildFork::Official, BuildBranch::Daily),
    ("experimental", BuildFork::Official, BuildBranch::Experimental),
    ("patch", BuildFork::Official, BuildBranch::Patch),
    ("bforartists", BuildFork::Bforartists, BuildBranch::Stable),
    ("upbge-stable", BuildFork::Upbge, BuildBranch::Stable),
    ("upbge-weekly", BuildFork::Upbge, BuildBranch::Daily),
    // custom 下是用户手动添加的外来安装，版本信息只能来自 .blinfo / 目录名。
    ("custom", BuildFork::Official, BuildBranch::Stable),
];

/// 目录名是否是解压出来的 Blender 版本目录。
fn looks_like_build_dir(dir_name: &str) -> bool {
    let lower = dir_name.to_lowercase();
    lower.starts_with("blender-")
        || lower.starts_with("bforartists-")
        || lower.starts_with("upbge-")
        || lower.starts_with("blender")
        || lower.starts_with("bforartists")
        || lower.starts_with("upbge")
}

/// 原版 `.blinfo` 里的一节（file_version 1.5）。
#[derive(serde::Deserialize)]
pub(crate) struct BlinfoEntry {
    #[serde(default)]
    pub(crate) branch: String,
    pub(crate) subversion: String,
    #[serde(default)]
    pub(crate) build_hash: Option<String>,
    #[serde(default)]
    commit_time: Option<String>,
    #[serde(default)]
    pub(crate) custom_name: String,
    #[serde(default)]
    pub(crate) is_favorite: bool,
}

#[derive(serde::Deserialize)]
struct Blinfo {
    #[serde(default)]
    blinfo: Vec<BlinfoEntry>,
}

pub struct LibraryManager;

impl LibraryManager {
    /// 扫描安装目录，返回本地已安装的 Build 列表（docs/01 §3.4 `scan_library`）。
    ///
    /// 识别规则：一级子目录且目录内含当前平台的 blender 可执行文件。
    /// 目录名等于下载归档的 stem（zip/tar.xz 顶层目录），据此解析
    /// version/platform/architecture 并与远程条目精确匹配。
    pub fn scan(install_dir: &Path) -> ScanResult {
        let mut builds = Vec::new();
        let mut corrupt_dirs = Vec::new();
        let Ok(entries) = std::fs::read_dir(install_dir) else {
            return ScanResult {
                builds,
                corrupt_dirs,
            };
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };

            // 原版结构：一级是类别目录，构建在二级。
            if let Some((dir, fork, branch)) = CATEGORY_DIRS
                .iter()
                .find(|(dir, _, _)| dir.eq_ignore_ascii_case(name))
            {
                // custom 目录是用户手动添加的外来安装，单独标记。
                let is_custom = dir.eq_ignore_ascii_case("custom");
                Self::scan_category(&path, *fork, *branch, is_custom, &mut builds, &mut corrupt_dirs);
                continue;
            }

            // 兼容：构建目录直接放在库根下（本程序早期版本 / 手动解压）。
            if looks_like_build_dir(name) {
                if !executable_exists(&path) {
                    corrupt_dirs.push(name.to_string());
                } else {
                    builds.push(local_build(name, &path, None, None, false));
                }
            }
        }

        ScanResult {
            builds,
            corrupt_dirs,
        }
    }

    /// 扫描一个类别目录（如 `stable/`）下的所有构建目录。
    #[allow(clippy::too_many_arguments)]
    fn scan_category(
        category: &Path,
        fork: BuildFork,
        branch: BuildBranch,
        is_custom: bool,
        builds: &mut Vec<Build>,
        corrupt_dirs: &mut Vec<String>,
    ) {
        let Ok(entries) = std::fs::read_dir(category) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !executable_exists(&path) {
                // 只有「像构建目录」的才报损坏，避免把用户的杂项目录算进来。
                if looks_like_build_dir(name) {
                    corrupt_dirs.push(name.to_string());
                }
                continue;
            }
            builds.push(local_build(name, &path, Some(fork), Some(branch), is_custom));
        }
    }

    /// 删除已安装版本（docs/01 §3.4 `remove_build`）。
    ///
    /// 破坏性操作：调用方（命令层）负责先定位并确认 build 存在。
    /// `delete_user_data` 同时删除该版本的用户配置目录。
    /// `to_trash` 为原版默认行为（Send to Trash）；只有用户显式选择永久删除
    /// 或 Shift 反转时才用 `remove_dir_all`。
    pub fn remove(
        install_path: &Path,
        version: &str,
        delete_user_data: bool,
        to_trash: bool,
    ) -> Result<()> {
        if install_path.exists() {
            if to_trash {
                trash::delete(install_path)
                    .map_err(|err| AppError::FileSystem(std::io::Error::other(err.to_string())))?;
            } else {
                std::fs::remove_dir_all(install_path).map_err(AppError::from)?;
            }
        }
        if delete_user_data {
            if let Some(user_dir) = user_data_dir(version) {
                if user_dir.exists() {
                    std::fs::remove_dir_all(&user_dir).map_err(AppError::from)?;
                }
            }
        }
        Ok(())
    }

    /// 目录树总字节数（符号链接不跟随）。
    pub fn dir_size(path: &Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(path) else {
            return 0;
        };
        let mut total = 0;
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let entry_path = entry.path();
            if file_type.is_dir() {
                total += Self::dir_size(&entry_path);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
        total
    }
}

/// 当前平台的 blender 可执行文件是否存在。
fn executable_exists(dir: &Path) -> bool {
    // 原版同时接受 blender 与 bforartists 的可执行文件名。
    #[cfg(target_os = "windows")]
    let candidates = ["blender.exe", "bforartists.exe"];
    #[cfg(target_os = "macos")]
    let candidates = [
        "Blender.app/Contents/MacOS/Blender",
        "Bforartists.app/Contents/MacOS/Bforartists",
    ];
    #[cfg(target_os = "linux")]
    let candidates = ["blender", "bforartists"];

    candidates.iter().any(|name| dir.join(name).exists())
}

/// 目录名 → Build。
///
/// 优先级（与原版一致）：`.blinfo` > 归档命名解析 > 目录名兜底。
/// `fork`/`branch` 由所在类别目录决定；扁平目录则从命名推断。
fn local_build(
    dir_name: &str,
    path: &Path,
    fork: Option<BuildFork>,
    branch: Option<BuildBranch>,
    is_custom: bool,
) -> Build {
    let dir_date = dir_modified_at(path).unwrap_or_else(Utc::now);

    // 1) 原版 .blinfo：最可靠的来源（含真实子版本、提交时间、自定义名）。
    if let Some(entry) = read_blinfo(path) {
        let platform = Scraper::detect_platform();
        let architecture = Scraper::detect_architecture();
        let release_date = entry
            .commit_time
            .as_deref()
            .and_then(parse_commit_time)
            .unwrap_or(dir_date);
        let version = if entry.custom_name.is_empty() {
            entry.subversion.clone()
        } else {
            entry.custom_name.clone()
        };
        return build_local(
            dir_name,
            path,
            version,
            platform,
            architecture,
            resolve_branch(&entry, branch),
            resolve_fork(&entry, fork),
            release_date,
            is_custom,
        );
    }

    // 2) 归档文件名/目录名解析。
    if let Some((version, platform, architecture)) = parse_builder_stem(dir_name) {
        let platform = if fork.is_some() { Scraper::detect_platform() } else { platform };
        return build_local(
            dir_name,
            path,
            version,
            platform,
            architecture,
            branch.unwrap_or(BuildBranch::Daily),
            fork.unwrap_or(BuildFork::Official),
            dir_date,
            is_custom,
        );
    }
    if let Some((version, platform, architecture)) = parse_release_stem(dir_name) {
        let platform = if fork.is_some() { Scraper::detect_platform() } else { platform };
        return build_local(
            dir_name,
            path,
            version,
            platform,
            architecture,
            branch.unwrap_or(BuildBranch::Stable),
            fork.unwrap_or(BuildFork::Official),
            dir_date,
            is_custom,
        );
    }

    // 3) 兜底：目录名当版本（外来安装，用户在 Build Entry Creator 里可修正）。
    build_local(
        dir_name,
        path,
        dir_name.to_string(),
        Scraper::detect_platform(),
        Scraper::detect_architecture(),
        branch.unwrap_or(BuildBranch::Stable),
        fork.unwrap_or(BuildFork::Official),
        dir_date,
        is_custom,
    )
}

/// 读构建目录内的 `.blinfo`（原版元数据文件）。
pub(crate) fn read_blinfo(path: &Path) -> Option<BlinfoEntry> {
    let text = std::fs::read_to_string(path.join(".blinfo")).ok()?;
    let parsed: Blinfo = serde_json::from_str(&text).ok()?;
    parsed.blinfo.into_iter().next()
}

/// 解析原版可能写入的几种时间格式。
fn parse_commit_time(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Some(value.with_timezone(&Utc));
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(text, "%d-%b-%y-%H:%M") {
        return Some(naive.and_utc());
    }
    DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

/// `.blinfo` 的 branch 决定条目归属（原版 known_branches 白名单）。
/// 目录层级已知时以目录为准，但 custom/ 下的条目由 .blinfo 决定
/// （原版把 custom 条目的 branch 写为目录名）。
fn resolve_branch(entry: &BlinfoEntry, category: Option<BuildBranch>) -> BuildBranch {
    let recorded = entry.branch.trim().to_lowercase();
    match (category, recorded.as_str()) {
        // custom/ 或扁平目录：.blinfo 说什么就是什么。
        (None, "daily") => BuildBranch::Daily,
        (None, "experimental") => BuildBranch::Experimental,
        (None, "patch") => BuildBranch::Patch,
        (None, _) => BuildBranch::Stable,
        // 类别目录已知：lts 归 stable，其余以目录为准。
        (Some(branch), _) => branch,
    }
}

/// bforartists / upbge 只能由类别目录判断；.blinfo 的 branch 是构建分支。
fn resolve_fork(entry: &BlinfoEntry, category: Option<BuildFork>) -> BuildFork {
    if let Some(fork) = category {
        return fork;
    }
    let recorded = entry.branch.to_lowercase();
    if recorded.contains("bforartists") {
        BuildFork::Bforartists
    } else if recorded.contains("upbge") {
        BuildFork::Upbge
    } else {
        BuildFork::Official
    }
}

fn build_local(
    dir_name: &str,
    path: &Path,
    version: String,
    platform: Platform,
    architecture: Architecture,
    branch: BuildBranch,
    fork: BuildFork,
    release_date: DateTime<Utc>,
    is_custom: bool,
) -> Build {
    let prefix = if is_custom { LOCAL_URL_PREFIX } else { LIBRARY_URL_PREFIX };
    Build {
        id: build_id(&format!("{prefix}{dir_name}"), platform),
        version,
        branch,
        fork,
        platform,
        architecture,
        url: format!("{prefix}{dir_name}"),
        // 扫描时不递归统计体积（数 GB 会卡住启动）；统计页按需计算。
        size_bytes: 0,
        release_date,
        release_notes_url: None,
        breaking_changes: None,
        is_installed: true,
        install_path: Some(path.to_path_buf()),
        update_build_id: None,
    }
}

/// 平台无关的用户配置目录：Windows/macOS 是
/// `<config>/Blender Foundation/Blender/<major.minor>`，Linux 是
/// `<config>/blender/<major.minor>`。
fn user_data_dir(version: &str) -> Option<PathBuf> {
    let major_minor = version.split('.').take(2).collect::<Vec<_>>().join(".");
    if major_minor.is_empty() {
        return None;
    }
    let config = dirs::config_dir()?;
    #[cfg(target_os = "linux")]
    let base = config.join("blender");
    #[cfg(not(target_os = "linux"))]
    let base = config.join("Blender Foundation").join("Blender");
    Some(base.join(major_minor))
}

fn dir_modified_at(path: &Path) -> Option<DateTime<Utc>> {
    let modified = path.metadata().ok()?.modified().ok()?;
    Some(DateTime::from(modified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::scraper::archive_stem;

    fn make_build(
        id: &str,
        branch: BuildBranch,
        fork: BuildFork,
        url: &str,
        installed: bool,
    ) -> Build {
        Build {
            id: id.to_string(),
            version: "4.5.0".to_string(),
            branch,
            fork,
            platform: Platform::Windows,
            architecture: Architecture::X64,
            url: url.to_string(),
            size_bytes: 100,
            release_date: Utc::now(),
            release_notes_url: None,
            breaking_changes: None,
            is_installed: installed,
            install_path: None,
            update_build_id: None,
        }
    }

    #[test]
    fn test_blinfo_branch_and_fork_resolution() {
        // custom/ 无类别目录：.blinfo 的 branch 决定分组。
        let entry = BlinfoEntry {
            branch: "Daily".into(),
            subversion: "5.0.0".into(),
            build_hash: None,
            commit_time: None,
            custom_name: String::new(),
            is_favorite: false,
        };
        assert_eq!(resolve_branch(&entry, None), BuildBranch::Daily);
        assert_eq!(resolve_fork(&entry, None), BuildFork::Official);

        // 类别目录已知时以目录为准（原版 lts 归 stable）。
        assert_eq!(
            resolve_branch(&entry, Some(BuildBranch::Stable)),
            BuildBranch::Stable
        );
    }

    #[test]
    fn test_blinfo_fork_detection() {
        let entry = BlinfoEntry {
            branch: "bforartists".into(),
            subversion: "4.5.0".into(),
            build_hash: None,
            commit_time: None,
            custom_name: String::new(),
            is_favorite: false,
        };
        assert_eq!(resolve_fork(&entry, None), BuildFork::Bforartists);
        assert_eq!(
            resolve_fork(&entry, Some(BuildFork::Upbge)),
            BuildFork::Upbge,
            "category dir wins over .blinfo"
        );
    }

    #[test]
    fn test_user_data_dir_major_minor() {
        // 路径包含版本号即可断言；dirs::config_dir 依机器而定。
        if let Some(dir) = user_data_dir("4.5.17") {
            let text = dir.to_string_lossy();
            assert!(text.ends_with("4.5"), "got {text}");
        }
    }

    #[test]
    fn test_local_scan_matching_rules() {
        // 归档 stem 与目录名一致即视为精确匹配（docs/01 §3.4）。
        assert_eq!(
            archive_stem(
                "https://download.blender.org/release/Blender4.5/blender-4.5.0-windows-x64.zip"
            )
            .as_deref(),
            Some("blender-4.5.0-windows-x64")
        );
        assert_eq!(
            archive_stem("https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-linux.x86_64-release.tar.xz")
                .as_deref(),
            Some("blender-5.4.0-alpha+main.1205aee3644f-linux.x86_64-release")
        );
        assert_eq!(
            archive_stem("https://cdn.example.com/blender-4.5.0.sha256"),
            None
        );
    }

    #[test]
    fn test_looks_like_build_dir() {
        assert!(looks_like_build_dir("blender-4.5.0-windows-x64"));
        assert!(looks_like_build_dir("Bforartists-5.2.3-Windows"));
        assert!(looks_like_build_dir(
            "upbge-0.53-alpha-windows-x86_64-2026-11-15"
        ));
        assert!(!looks_like_build_dir("my custom folder"));
        assert!(!looks_like_build_dir("template"));
    }

    #[test]
    fn test_scan_reports_corrupt_dirs() {
        let root = std::env::temp_dir().join(format!("bl-scan-{}", uuid::Uuid::new_v4()));
        // 有效版本：目录名合规 + 平台可执行文件。
        let good = root.join("blender-4.5.0-windows-x64");
        std::fs::create_dir_all(&good).unwrap();
        #[cfg(target_os = "windows")]
        std::fs::write(good.join("blender.exe"), b"x").unwrap();
        #[cfg(target_os = "linux")]
        std::fs::write(good.join("blender"), b"x").unwrap();
        #[cfg(target_os = "macos")]
        {
            let mac = good.join("Blender.app/Contents/MacOS");
            std::fs::create_dir_all(&mac).unwrap();
            std::fs::write(mac.join("Blender"), b"x").unwrap();
        }
        // 损坏：像版本目录但没有可执行文件。
        std::fs::create_dir_all(root.join("blender-4.4.0-windows-x64")).unwrap();
        // 忽略：与版本无关的自建目录。
        std::fs::create_dir_all(root.join("my stuff")).unwrap();

        let scan = LibraryManager::scan(&root);
        assert_eq!(
            scan.builds.len(),
            1,
            "only the healthy dir counts as a build"
        );
        assert_eq!(scan.builds[0].version, "4.5.0");
        assert_eq!(
            scan.corrupt_dirs,
            vec!["blender-4.4.0-windows-x64".to_string()]
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn test_local_build_stem_parsing() {
        // builder 风格目录名 → Daily；release 风格 → Stable。
        let stem = "blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release";
        let (version, platform, arch) = parse_builder_stem(stem).unwrap();
        assert_eq!(version, "5.4.0-alpha+main.1205aee3644f");
        assert_eq!(platform, Platform::Windows);
        assert_eq!(arch, Architecture::X64);

        let (version, platform, arch) = parse_release_stem("blender-4.5.0-windows-x64").unwrap();
        assert_eq!(version, "4.5.0");
        assert_eq!(platform, Platform::Windows);
        assert_eq!(arch, Architecture::X64);
    }
}
