use crate::models::Build;
use crate::service::library::{LIBRARY_URL_PREFIX, LOCAL_URL_PREFIX};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{Emitter, State};

/// 原版 `.blinfo` 的 file_version（modules/build_info.py:167）。
const BLINFO_FILE_VERSION: &str = "1.5";

/// 构建目录内可执行文件候选（与 launcher / library 保持一致）。
#[cfg(target_os = "windows")]
const EXE_CANDIDATES: [&str; 2] = ["blender.exe", "bforartists.exe"];
#[cfg(target_os = "macos")]
const EXE_CANDIDATES: [&str; 2] = [
    "Blender.app/Contents/MacOS/Blender",
    "Bforartists.app/Contents/MacOS/Bforartists",
];
#[cfg(target_os = "linux")]
const EXE_CANDIDATES: [&str; 2] = ["blender", "bforartists"];

/// `detect_build_info` / `save_custom_build` 的载荷（对应原版 BuildInfo）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildInfoInput {
    pub path: PathBuf,
    /// 目录内可执行文件相对路径，空则用候选列表自动找。
    #[serde(default)]
    pub executable: String,
    #[serde(default)]
    pub subversion: String,
    #[serde(default)]
    pub build_hash: Option<String>,
    /// RFC3339；空则用当前时间。
    #[serde(default)]
    pub commit_time: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub custom_name: String,
}

/// 目录内可直接执行的文件（原版 CustomBuildDialogWindow 的补全列表）。
#[tauri::command]
pub async fn list_executables(path: String) -> Result<Vec<String>, String> {
    let dir = PathBuf::from(&path);
    let entries = std::fs::read_dir(&dir).map_err(|err| format!("{path}: {err}"))?;
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        #[cfg(target_os = "windows")]
        let ok = name.to_lowercase().ends_with(".exe")
            || name.to_lowercase().ends_with(".bat")
            || name.to_lowercase().ends_with(".cmd");
        #[cfg(not(target_os = "windows"))]
        let ok = {
            use std::os::unix::fs::PermissionsExt;
            entry
                .metadata()
                .map(|meta| meta.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        };
        if ok {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// 运行 `blender -v` 解析版本/哈希/提交时间（原版 fill_blender_info）。
/// 失败时返回空字段，由用户在对话框里手填。
#[tauri::command]
pub async fn detect_build_info(
    path: String,
    executable: Option<String>,
) -> Result<BuildInfoInput, String> {
    let dir = PathBuf::from(&path);
    let exe = match executable.filter(|name| !name.is_empty()) {
        Some(name) => dir.join(name),
        None => find_executable(&dir).ok_or_else(|| {
            format!(
                "No executable found in {path} (tried {EXE_CANDIDATES:?}); pick one manually."
            )
        })?,
    };

    let output = tokio::process::Command::new(&exe)
        .arg("-v")
        .output()
        .await
        .map_err(|err| format!("failed to run {}: {err}", exe.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);

    Ok(BuildInfoInput {
        path: dir,
        executable: exe
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string(),
        subversion: field(&text, "Blender (", "").unwrap_or_default(),
        build_hash: field(&text, "build hash: ", ""),
        commit_time: commit_time(&text),
        branch: field(&text, "build branch: ", ""),
        custom_name: String::new(),
    })
}

/// 读某个已安装构建的 `.blinfo` 完整字段，供「编辑构建」对话框预填。
///
/// 不能让前端自己拼：`build_hash` 与 `commit_time` 不在 `Build` 里，
/// 对话框拿不到就只能留空，保存时 `write_blinfo` 会把 commit_time
/// 重置成当前时间、build_hash 清空——用户只改了自定义名，却把
/// 构建哈希冲掉了。原有字段一律从这里取。
#[tauri::command]
pub async fn get_build_info(build_id: String, state: State<'_, AppState>) -> Result<BuildInfoInput, String> {
    let (build, install_path) = installed_build(&state, &build_id)?;
    // 下载安装的构建没有 .blinfo，缺失时用 Build 的字段合成，
    // 保证 subversion 非空——否则对话框的 versionValid 判空失败，
    // Save 按钮被禁用，用户必须先手填版本号才能保存。
    let entry = read_blinfo_json(&install_path).unwrap_or_else(|| synthesized_blinfo(&build));
    Ok(BuildInfoInput {
        path: install_path.clone(),
        executable: entry.custom_executable.unwrap_or_default(),
        subversion: entry.subversion,
        build_hash: entry.build_hash,
        commit_time: (!entry.commit_time.is_empty()).then_some(entry.commit_time),
        branch: (!entry.branch.is_empty()).then_some(entry.branch),
        custom_name: entry.custom_name,
    })
}

/// 写 `.blinfo` 并返回扫描后的 Build，供前端立即显示（原版 init_unrecognized）。
#[tauri::command]
pub async fn save_custom_build(
    input: BuildInfoInput,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Build, String> {
    let dir = input.path.clone();
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", dir.display()));
    }
    let executable = if input.executable.is_empty() {
        find_executable(&dir)
            .ok_or_else(|| format!("No executable found in {}", dir.display()))?
            .to_string_lossy()
            .into_owned()
    } else {
        let candidate = dir.join(&input.executable);
        if !candidate.is_file() {
            return Err(format!("Executable not found: {}", candidate.display()));
        }
        input.executable.clone()
    };

    write_blinfo(&dir, &input, &executable).map_err(|err| err.to_string())?;

    // 重新扫描，让新条目立刻出现在列表里。
    crate::commands::library::rescan(&state, &app_handle)
        .await
        .map_err(|err| err.to_string())?;
    let _ = app_handle.emit("library_scanned", ());

    let builds = state.builds.lock().unwrap();
    builds
        .iter()
        .find(|build| build.install_path.as_deref() == Some(dir.as_path()))
        .cloned()
        .ok_or_else(|| format!("Build at {} not picked up by scan", dir.display()))
}

/// 把库里的构建移动到另一个类别目录（原版 renamer 的 manual branch change）。
#[tauri::command]
pub async fn move_build_to_category(
    build_id: String,
    branch: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (install_path, version) = {
        let builds = state.builds.lock().unwrap();
        let build = builds
            .iter()
            .find(|build| build.id == build_id)
            .ok_or_else(|| format!("Build not found: {build_id}"))?;
        (
            build
                .install_path
                .clone()
                .ok_or_else(|| "Build is not installed".to_string())?,
            build.version.clone(),
        )
    };

    let Some(parent) = install_path.parent() else {
        return Err("Build path has no parent".into());
    };
    let target_dir = parent.join(&branch);
    std::fs::create_dir_all(&target_dir).map_err(|err| err.to_string())?;
    let Some(name) = install_path.file_name() else {
        return Err("Build path has no file name".into());
    };
    let target = target_dir.join(name);
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    std::fs::rename(&install_path, &target).map_err(|err| err.to_string())?;

    // .blinfo 的 branch 也要跟着改，否则重扫会回到旧分组。
    if let Some(mut info) = read_blinfo_json(&target) {
        info.branch = branch.clone();
        let _ = write_blinfo_raw(&target, &info);
    }
    tracing::info!(build = %build_id, version, branch, "moved build to category");

    crate::commands::library::rescan(&state, &app_handle)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}

/// 创建桌面快捷方式（Windows 用 WScript.Shell，Linux 写 .desktop）。
#[tauri::command]
pub async fn create_shortcut(build_id: String, destination: String, state: State<'_, AppState>) -> Result<(), String> {
    let (install_path, version) = installed_path(&state, &build_id)?;
    #[cfg(target_os = "windows")]
    let _ = &version;
    let exe = find_executable(&install_path)
        .ok_or_else(|| format!("Executable not found in {}", install_path.display()))?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let name = format!("Blender {version}");

    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');\
             $s.TargetPath='{}';$s.WorkingDirectory='{}';$s.IconLocation='{}';$s.Save()",
            destination.replace('\'', "''"),
            exe.display().to_string().replace('\'', "''"),
            install_path.display().to_string().replace('\'', "''"),
            exe.display().to_string().replace('\'', "''"),
        );
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|err| err.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let entry = format!(
            "[Desktop Entry]\nName={name}\nComment=3D modeling, animation, rendering and post-production\n\
             Keywords=3d;cg;modeling;animation;cycles;python;\nIcon=blender\nTerminal=false\n\
             Type=Application\nCategories=Graphics;3DGraphics;\nMimeType=application/x-blender;\n\
             Exec={} %f\n",
            exe.display()
        );
        std::fs::write(&destination, entry).map_err(|err| err.to_string())?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(())
}

/// 在库根目录建 `bl_symlink` 指向该构建（原版 create_symlink）。
#[tauri::command]
pub async fn create_symlink(build_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let (install_path, _) = installed_path(&state, &build_id)?;
    let library = state.settings.lock().unwrap().install_dir.clone();
    let link = library.join("bl_symlink");
    if link.exists() || link.is_symlink() {
        if link.is_dir() {
            let _ = std::fs::remove_dir(&link);
        } else {
            std::fs::remove_file(&link).map_err(|err| err.to_string())?;
        }
    }

    #[cfg(target_os = "windows")]
    {
        // 目录符号链接需开发者模式/管理员；失败时退到 junction。
        if std::os::windows::fs::symlink_dir(&install_path, &link).is_err() {
            let output = std::process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &link.display().to_string(),
                    &install_path.display().to_string(),
                ])
                .output()
                .map_err(|err| err.to_string())?;
            if !output.status.success() {
                return Err(String::from_utf8_lossy(&output.stdout).into_owned());
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    std::os::unix::fs::symlink(&install_path, &link).map_err(|err| err.to_string())?;

    Ok(link.display().to_string())
}

/// Make Portable：在构建目录内建 `portable/`，存在则改名为 `_portable` 关闭。
/// 返回是否处于便携模式。Blender ≥4.2 用 `<dir>/portable`，更早用
/// `<dir>/<major.minor>/config`（原版 make_portable_path）。
#[tauri::command]
pub async fn toggle_portable(build_id: String, state: State<'_, AppState>) -> Result<bool, String> {
    let (install_path, version) = installed_path(&state, &build_id)?;
    let config_path = portable_path(&install_path, &version);
    let disabled = config_path
        .parent()
        .unwrap_or(&install_path)
        .join(format!("_{}", config_path.file_name().and_then(|n| n.to_str()).unwrap_or("portable")));

    if config_path.is_dir() {
        std::fs::rename(&config_path, &disabled).map_err(|err| err.to_string())?;
        Ok(false)
    } else if disabled.is_dir() {
        std::fs::rename(&disabled, &config_path).map_err(|err| err.to_string())?;
        Ok(true)
    } else {
        std::fs::create_dir_all(&config_path).map_err(|err| err.to_string())?;
        Ok(true)
    }
}

/// 当前是否便携模式。
#[tauri::command]
pub async fn is_portable(build_id: String, state: State<'_, AppState>) -> Result<bool, String> {
    let (install_path, version) = installed_path(&state, &build_id)?;
    Ok(portable_path(&install_path, &version).is_dir())
}

/// 冻结/解冻更新（原版 `freeze_update`）：写入 `.blinfo` 的 `is_frozen`。
/// 冻结后不再提示该构建有更新。返回冻结后的状态。
#[tauri::command]
pub async fn set_build_frozen(
    build_id: String,
    frozen: bool,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let (build, install_path) = installed_build(&state, &build_id)?;
    // 下载安装的构建没有 .blinfo；缺失时按 Build 字段合成一条再写，
    // 这样「冻结更新」对所有已安装构建都可用，不只是手动添加的那些。
    let mut entry = read_blinfo_json(&install_path).unwrap_or_else(|| synthesized_blinfo(&build));
    entry.is_frozen = frozen;
    write_blinfo_raw(&install_path, &entry).map_err(|err| err.to_string())?;

    // 冻结状态影响 update_build_id，重扫一次让行内更新图标立即消失。
    crate::commands::library::rescan(&state, &app_handle)
        .await
        .map_err(|err| err.to_string())?;
    Ok(frozen)
}

/// 原版「显示配置文件夹」：优先打开构建目录内的 `portable/`，
/// 否则回退到系统的 `<config>/Blender Foundation/Blender/<major.minor>`。
///
/// 返回实际打开的路径；两处都不存在时返回错误，由前端提示。
#[tauri::command]
pub async fn open_config_folder(build_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let (install_path, version) = installed_path(&state, &build_id)?;
    let portable = portable_path(&install_path, &version);
    if portable.is_dir() {
        open_folder(&portable)?;
        return Ok(portable.display().to_string());
    }
    let user_dir = crate::service::library::user_data_dir(&version)
        .ok_or_else(|| "Unable to determine the user config folder".to_string())?;
    if !user_dir.is_dir() {
        return Err(format!("配置目录还不存在：{}（请先启动一次 Blender）", user_dir.display()));
    }
    open_folder(&user_dir)?;
    Ok(user_dir.display().to_string())
}

/// 打开一个目录（与 `open_in_explorer` 同语义，供本模块内部复用）。
fn open_folder(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    result.map(|_| ()).map_err(|err| err.to_string())
}

/// 注册 `.blend` 文件关联（Windows，HKCU 下，无需管理员）。
#[tauri::command]
pub async fn register_file_association(state: State<'_, AppState>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let exe = exe.display().to_string();

    #[cfg(target_os = "windows")]
    {
        const PROGID: &str = "blenderlauncherv2.blend";
        let command_key = format!(r"HKCU\Software\Classes\{PROGID}\shell\open\command");
        let open_with = String::from(r"HKCU\Software\Classes\.blend\OpenWithProgids");
        let command = format!("\"{exe}\" \"%1\"");
        let commands: [Vec<&str>; 2] = [
            vec!["add", command_key.as_str(), "/ve", "/d", command.as_str(), "/f"],
            vec!["add", open_with.as_str(), "/v", PROGID, "/t", "REG_SZ", "/d", "", "/f"],
        ];
        for args in commands {
            let output = std::process::Command::new("reg")
                .args(&args)
                .output()
                .map_err(|err| err.to_string())?;
            if !output.status.success() {
                return Err(String::from_utf8_lossy(&output.stderr).into_owned());
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = exe;
        return Err("File association is only implemented for Windows".into());
    }

    let mut settings = state.settings.lock().unwrap();
    settings.register_blend_files = true;
    let snapshot = settings.clone();
    drop(settings);
    crate::service::settings::save_settings(&snapshot).map_err(|err| err.to_string())?;
    Ok(())
}

/// 读构建目录 `.blinfo` 里的真实 build hash（原版 Copy Build Hash）。
/// 未安装或无 `.blinfo` 时返回 None，由前端回退到版本号后缀。
#[tauri::command]
pub async fn get_build_hash(
    build_id: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let (install_path, _) = installed_path(&state, &build_id)?;
    Ok(crate::service::library::read_blinfo(&install_path).and_then(|entry| entry.build_hash))
}

/// 删除构建目录内的模板副本（无模板源，只清目录）。
#[tauri::command]
pub async fn install_template(build_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let (install_path, version) = installed_path(&state, &build_id)?;
    let major_minor = version.split('.').take(2).collect::<Vec<_>>().join(".");
    let target = install_path.join(&major_minor).join("datafiles").join("templates");
    std::fs::create_dir_all(&target).map_err(|err| err.to_string())?;
    Ok(target.display().to_string())
}

// ---------------------------------------------------------------- helpers

/// 把 BuildBranch 转成 `.blinfo` 里存的分支名（与 serde 的 lowercase 一致）。
fn branch_name(branch: crate::models::BuildBranch) -> &'static str {
    match branch {
        crate::models::BuildBranch::Stable => "stable",
        crate::models::BuildBranch::Daily => "daily",
        crate::models::BuildBranch::Experimental => "experimental",
        crate::models::BuildBranch::Patch => "patch",
    }
}

/// 按 Build 当前字段合成一条 `.blinfo` 条目。
///
/// 下载安装的构建**没有** `.blinfo`——下载器只解压归档，不写元数据文件。
/// 冻结更新和编辑对话框都必须能作用在这些构建上，所以缺失时不能报错，
/// 而是拿已经在 state 里的字段合成一条：版本号取 `Build.version`，
/// 提交时间取 `release_date`，显示名取 `label`（自定义名）。
///
/// `is_favorite` 写 false 是安全的：收藏存在 SQLite，扫描只把 `.blinfo` 里
/// 标记为收藏的并进去，不会把SQLite 里已有的收藏删掉。
fn synthesized_blinfo(build: &Build) -> BlinfoEntry {
    BlinfoEntry {
        branch: branch_name(build.branch).to_string(),
        subversion: build.version.clone(),
        build_hash: None,
        commit_time: build.release_date.to_rfc3339(),
        custom_name: build.label.clone().unwrap_or_default(),
        is_favorite: false,
        custom_executable: None,
        is_frozen: false,
    }
}

/// 取 Build 本体和安装路径（需要 Build 额外字段的命令用）。
fn installed_build(state: &AppState, build_id: &str) -> Result<(Build, PathBuf), String> {
    let build = {
        let builds = state.builds.lock().unwrap();
        builds
            .iter()
            .find(|build| build.id == build_id)
            .cloned()
            .ok_or_else(|| format!("Build not found: {build_id}"))?
    };
    let path = build
        .install_path
        .clone()
        .ok_or_else(|| "Build is not installed".to_string())?;
    Ok((build, path))
}

fn installed_path(state: &AppState, build_id: &str) -> Result<(PathBuf, String), String> {
    let builds = state.builds.lock().unwrap();
    let build = builds
        .iter()
        .find(|build| build.id == build_id)
        .ok_or_else(|| format!("Build not found: {build_id}"))?;
    let path = build
        .install_path
        .clone()
        .ok_or_else(|| "Build is not installed".to_string())?;
    Ok((path, build.version.clone()))
}

fn find_executable(dir: &Path) -> Option<PathBuf> {
    EXE_CANDIDATES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists())
}

/// 原版 `<dir>/portable`（Blender ≥4.2），否则 `<dir>/<major.minor>/config`。
fn portable_path(install_path: &Path, version: &str) -> PathBuf {
    let major_minor = version.split('.').take(2).collect::<Vec<_>>().join(".");
    match major_minor.as_str() {
        "4.2" | "4.3" | "4.4" | "4.5" | "4.6" | "4.7" | "4.8" | "4.9" | "5.0" | "5.1" | "5.2"
        | "5.3" | "5.4" | "5.5" | "5.6" | "5.7" | "5.8" | "5.9" => install_path.join("portable"),
        _ => install_path.join(major_minor).join("config"),
    }
}

/// 抽取 `prefix` 后到行尾的字段（原版正则的等价实现）。
fn field(text: &str, prefix: &str, suffix: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix(prefix) {
            let value = rest.trim_end_matches(suffix).trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 拼 `build commit date` + `build commit time`（原版 fill_blender_info）。
fn commit_time(text: &str) -> Option<String> {
    let date = field(text, "build commit date: ", "")?;
    let time = field(text, "build commit time: ", "")?;
    let naive = chrono::NaiveDateTime::parse_from_str(
        &format!("{date} {time}"),
        "%Y-%m-%d %H:%M",
    )
    .or_else(|_| chrono::NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M:%S"))
    .ok()?;
    Some(naive.and_utc().to_rfc3339())
}

// ---------------------------------------------------------------- .blinfo

#[derive(Serialize, Deserialize, Clone)]
struct BlinfoEntry {
    #[serde(default)]
    branch: String,
    #[serde(default)]
    subversion: String,
    #[serde(default)]
    build_hash: Option<String>,
    #[serde(default)]
    commit_time: String,
    #[serde(default)]
    custom_name: String,
    #[serde(default)]
    is_favorite: bool,
    #[serde(default)]
    custom_executable: Option<String>,
    #[serde(default)]
    is_frozen: bool,
}

#[derive(Serialize, Deserialize)]
struct BlinfoFile {
    file_version: String,
    blinfo: Vec<BlinfoEntry>,
}

fn write_blinfo(dir: &Path, input: &BuildInfoInput, executable: &str) -> std::io::Result<()> {
    // 编辑已有构建时保留 .blinfo 里不该被这个对话框改动的字段
    // （收藏、冻结）。原版 `build_info_edited` 是整份 BuildInfo 回写，
    // 我们只拿到对话框的字段，必须显式合并，否则每保存一次就丢一次收藏。
    let previous = read_blinfo_json(dir);
    let entry = BlinfoEntry {
        branch: input.branch.clone().unwrap_or_else(|| "custom".into()),
        subversion: input.subversion.clone(),
        build_hash: input.build_hash.clone().filter(|hash| !hash.is_empty()),
        commit_time: input
            .commit_time
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        custom_name: input.custom_name.trim().to_string(),
        is_favorite: previous.as_ref().is_some_and(|old| old.is_favorite),
        custom_executable: Some(executable.to_string()),
        is_frozen: previous.as_ref().is_some_and(|old| old.is_frozen),
    };
    write_blinfo_raw(dir, &entry)
}

fn write_blinfo_raw(dir: &Path, entry: &BlinfoEntry) -> std::io::Result<()> {
    let file = BlinfoFile {
        file_version: BLINFO_FILE_VERSION.to_string(),
        blinfo: vec![entry.clone()],
    };
    let text = serde_json::to_string_pretty(&file)
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    // 原版用非原子写；这里先写临时文件再改名，避免半截 JSON 让条目录丢失。
    let target = dir.join(".blinfo");
    let tmp = dir.join(".blinfo.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &target)
}

fn read_blinfo_json(dir: &Path) -> Option<BlinfoEntry> {
    let text = std::fs::read_to_string(dir.join(".blinfo")).ok()?;
    let parsed: BlinfoFile = serde_json::from_str(&text).ok()?;
    parsed.blinfo.into_iter().next()
}

/// 给扫描出的本地条目补上合成前缀判断用的工具（供未来 renamer 使用）。
#[allow(dead_code)]
fn is_local_url(url: &str) -> bool {
    url.starts_with(LOCAL_URL_PREFIX) || url.starts_with(LIBRARY_URL_PREFIX)
}

/// 保留给「更新检测」的版本比较：语义化优先，相同则比提交时间。
pub(crate) fn is_newer(candidate: &Build, installed: &Build) -> bool {
    let parse = |version: &str| {
        let cleaned: String = version
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+'))
            .collect();
        semver::Version::parse(&cleaned).ok()
    };
    match (parse(&candidate.version), parse(&installed.version)) {
        (Some(new), Some(old)) => {
            if new > old {
                true
            } else if new == old {
                candidate.release_date > installed.release_date
            } else {
                false
            }
        }
        _ => candidate.release_date > installed.release_date,
    }
}

/// 更新检测需要的判断条件：同 fork/branch/平台/架构，且是远程条目。
pub(crate) fn comparable(candidate: &Build, installed: &Build) -> bool {
    candidate.fork == installed.fork
        && candidate.branch == installed.branch
        && candidate.platform == installed.platform
        && candidate.architecture == installed.architecture
        && !candidate.is_installed
        && !is_local_url(&candidate.url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Architecture, BuildBranch, BuildFork, Platform};

    #[test]
    fn test_field_parsing() {
        let text = "Blender 4.5.3\nbuild commit date: 2025-02-11\nbuild commit time: 10:33\nbuild hash: abc1234\n";
        assert_eq!(field(text, "build hash: ", "").as_deref(), Some("abc1234"));
        assert_eq!(
            commit_time(text).as_deref(),
            Some("2025-02-11T10:33:00+00:00")
        );
    }

    #[test]
    fn test_portable_path_threshold() {
        assert_eq!(
            portable_path(Path::new("C:/lib/stable/b"), "4.5.0"),
            PathBuf::from("C:/lib/stable/b/portable")
        );
        assert_eq!(
            portable_path(Path::new("C:/lib/stable/b"), "3.6.9"),
            PathBuf::from("C:/lib/stable/b/3.6/config")
        );
    }

    /// 下载安装的构建没有 .blinfo，synthesized_blinfo 必须给出非空
    /// subversion / 有效 commit_time，否则编辑对话框 Save 被禁用、
    /// 冻结更新直接报错（审查发现的两个回归点）。
    #[test]
    fn test_synthesized_blinfo_for_downloaded_build() {
        let build = Build {
            id: "x".into(),
            version: "5.2.5".into(),
            branch: BuildBranch::Stable,
            fork: BuildFork::Official,
            platform: Platform::Windows,
            architecture: Architecture::X64,
            url: "https://example.com/a.zip".into(),
            size_bytes: 1,
            release_date: chrono::DateTime::parse_from_rfc3339("2026-11-25T04:02:14Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            release_notes_url: None,
            breaking_changes: None,
            is_installed: true,
            install_path: None,
            label: None,
            is_frozen: false,
            update_build_id: None,
        };
        let entry = synthesized_blinfo(&build);
        assert_eq!(entry.subversion, "5.2.5");
        assert_eq!(entry.branch, "stable");
        assert_eq!(
            entry.commit_time,
            "2026-11-25T04:02:14+00:00",
            "commit_time 必须可解析"
        );
        assert!(entry.custom_name.is_empty());
        assert!(!entry.is_frozen);
        assert!(!entry.is_favorite, "收藏存在 SQLite，不能被合成条目覆盖");
    }

    #[test]
    fn test_is_newer() {
        let make = |version: &str, date: &str| Build {
            id: version.into(),
            version: version.into(),
            branch: BuildBranch::Stable,
            fork: BuildFork::Official,
            platform: Platform::Windows,
            architecture: Architecture::X64,
            url: "https://example.com/a.zip".into(),
            size_bytes: 1,
            release_date: chrono::DateTime::parse_from_rfc3339(date)
                .unwrap()
                .with_timezone(&chrono::Utc),
            release_notes_url: None,
            breaking_changes: None,
            is_installed: false,
            install_path: None,
            label: None,
            is_frozen: false,
            update_build_id: None,
        };
        assert!(is_newer(&make("4.5.2", "2025-01-01T00:00:00Z"), &make("4.5.1", "2025-01-01T00:00:00Z")));
        assert!(!is_newer(&make("4.5.1", "2025-01-01T00:00:00Z"), &make("4.5.2", "2025-01-01T00:00:00Z")));
        assert!(is_newer(&make("4.5.1", "2025-02-01T00:00:00Z"), &make("4.5.1", "2025-01-01T00:00:00Z")));
    }
}
