use crate::error::{AppError, Result};
use crate::models::extension::{ExtensionScan, MANAGED_MARKER};
use crate::models::{Build, CatalogItem};
use crate::service::extension_probe;
use crate::service::extensions;
use crate::service::http::build_client;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{Emitter, State};

/// 官方目录端点。参数只有 `format` / `blender_version` / `platform`，
/// 分页与搜索服务端都不支持（见 docs/06 §2 F4）。
const CATALOG_URL: &str = "https://extensions.blender.org/api/v1/extensions/";

/// 联网刷新超时。目录约 2 MB，慢网络下 30 秒仍不够就放弃。
const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);

/// 从 `state.builds` 里取出指定 build 并确认已安装。
fn installed_build(state: &AppState, build_id: &str) -> Result<Build> {
    let builds = state.builds.lock().unwrap();
    let build = builds
        .iter()
        .find(|build| build.id == build_id)
        .ok_or_else(|| AppError::BuildNotFound(build_id.to_string()))?;
    if build.install_path.is_none() {
        return Err(AppError::InvalidSettings(format!(
            "构建 {} 未安装，无法管理插件",
            build.version
        )));
    }
    Ok(build.clone())
}

/// 扫描某个 Build 的插件目录（L1），并尽力套上缓存的启用状态。
#[tauri::command]
pub async fn scan_extensions(
    state: State<'_, AppState>,
    build_id: String,
) -> std::result::Result<ExtensionScanView, String> {
    do_scan(&state, &build_id)
        .await
        .map(ExtensionScanView::from)
        .map_err(String::from)
}

async fn do_scan(state: &AppState, build_id: &str) -> Result<ExtensionScan> {
    let build = installed_build(state, build_id)?;
    let root = extensions::resolve_ext_root(&build)?;

    // 目录遍历是阻塞 IO，且插件多时可能几千次 stat。
    let root_clone = root.clone();
    let mut scan = tokio::task::spawn_blocking(move || extensions::scan(&root_clone))
        .await
        .map_err(|err| AppError::InternalError(format!("扫描任务失败: {err}")))?;

    apply_cached_enabled(state, build_id, &mut scan);
    Ok(scan)
}

/// 把库里的启用状态套到扫描结果上。
///
/// 缓存里存的是「上次探测时每个已装条目的状态」，所以未命中的条目
/// 应当保持 `None`（那次探测还没装它），而不是被写成 `false`。
fn apply_cached_enabled(state: &AppState, build_id: &str, scan: &mut ExtensionScan) {
    let Some(db) = state.db.as_ref() else {
        return;
    };
    let Ok(rows) = db.extension_enabled(build_id) else {
        return;
    };
    if rows.is_empty() {
        return;
    }

    for extension in &mut scan.extensions {
        let target = format!("bl_ext.{}.{}", extension.repo, extension.pkg_id);
        if let Some((_, enabled, checked_at)) = rows.iter().find(|(module, _, _)| module == &target)
        {
            extension.enabled = Some(*enabled);
            scan.enabled_checked_at = Some(timestamp_to_datetime(*checked_at));
        }
    }
    for addon in &mut scan.legacy {
        if let Some((_, enabled, checked_at)) =
            rows.iter().find(|(module, _, _)| module == &addon.module)
        {
            addon.enabled = Some(*enabled);
            scan.enabled_checked_at = Some(timestamp_to_datetime(*checked_at));
        }
    }
}

fn timestamp_to_datetime(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(seconds, 0).unwrap_or_else(chrono::Utc::now)
}

/// 探测启用状态（A2）。会起一个 Blender 子进程，约 5 秒，最长 20 秒。
///
/// 探测失败时返回的 `enabled` 全为 `None`，前端必须显示为「未知」
/// 而不是「已禁用」。
#[tauri::command]
pub async fn probe_extensions(
    state: State<'_, AppState>,
    build_id: String,
) -> std::result::Result<ExtensionScanView, String> {
    probe_inner(&state, &build_id)
        .await
        .map(ExtensionScanView::from)
        .map_err(String::from)
}

async fn probe_inner(state: &AppState, build_id: &str) -> Result<ExtensionScan> {
    let build = installed_build(state, build_id)?;
    let install_path = build
        .install_path
        .clone()
        .ok_or_else(|| AppError::InternalError("Build is not installed".to_string()))?;

    let exe = extension_probe::blender_executable(&install_path).ok_or_else(|| {
        AppError::InternalError(format!(
            "{} 下找不到 Blender 可执行文件",
            install_path.display()
        ))
    })?;
    extension_probe::validate_exe(&exe)?;

    let exe_for_probe = exe.clone();
    let result =
        tokio::task::spawn_blocking(move || extension_probe::probe_enabled(&exe_for_probe))
            .await
            .map_err(|err| AppError::InternalError(format!("探测任务失败: {err}")))?;

    // 先扫描一遍拿到「盘上有哪些条目」，再据此把探测结果展开成
    // 每个条目一条的布尔快照。只存已启用的模块会导致未启用的条目
    // 读回时仍是未知，界面上永远显示问号。
    let mut scan = do_scan(state, build_id).await?;
    extension_probe::attach_enabled(&mut scan, &result)?;

    if let Some(error) = result.error.as_deref() {
        tracing::warn!(build = %build_id, error, "extension probe failed");
        return Ok(scan);
    }

    if let Some(db) = state.db.as_ref() {
        let mut rows: Vec<(String, bool)> = Vec::new();
        for extension in &scan.extensions {
            rows.push((
                format!("bl_ext.{}.{}", extension.repo, extension.pkg_id),
                extension.enabled.unwrap_or(false),
            ));
        }
        for addon in &scan.legacy {
            rows.push((addon.module.clone(), addon.enabled.unwrap_or(false)));
        }

        let db = std::sync::Arc::clone(db);
        let build_id_owned = build_id.to_string();
        let _ = tokio::task::spawn_blocking(move || {
            db.replace_extension_enabled(&build_id_owned, &rows)
        })
        .await;
        scan.enabled_checked_at = Some(chrono::Utc::now());
    }

    Ok(scan)
}

/// 读目录（L2 本地索引优先，缺失时回落到 L3 联网）。
#[tauri::command]
pub async fn get_extension_catalog(
    state: State<'_, AppState>,
    build_id: String,
    refresh: bool,
) -> std::result::Result<Vec<CatalogItem>, String> {
    let build = installed_build(&state, &build_id).map_err(String::from)?;
    let root = extensions::resolve_ext_root(&build).map_err(String::from)?;

    if !refresh {
        let root_clone = root.clone();
        let local =
            tokio::task::spawn_blocking(move || extensions::read_all_repo_indexes(&root_clone))
                .await
                .map_err(|err| format!("读取本地索引失败: {err}"))?;
        if !local.is_empty() {
            tracing::info!(count = local.len(), "catalog from local repo index");
            return Ok(local);
        }
    }

    // 本地没有索引：联网拉一次（用户从没在 Blender 里开过扩展页时会发生）。
    let proxy = state.settings.lock().unwrap().proxy.clone();
    let platform = platform_param(&build);
    let url = match platform {
        Some(platform) => format!("{CATALOG_URL}?format=json&platform={platform}"),
        None => format!("{CATALOG_URL}?format=json"),
    };

    let client =
        build_client(&proxy, "blender-launcher-v3", CATALOG_TIMEOUT).map_err(String::from)?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|err| format!("获取扩展目录失败: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("获取扩展目录失败: HTTP {}", response.status()));
    }
    let body = response
        .text()
        .await
        .map_err(|err| format!("读取扩展目录响应失败: {err}"))?;

    let mut items = parse_catalog(&body)?;
    items.sort_by_key(|item| item.name.to_lowercase());
    tracing::info!(count = items.len(), "catalog from remote API");
    Ok(items)
}

/// Blender 的 platform 参数格式：`windows-x64` / `macos-arm64` / `linux-x64`。
fn platform_param(build: &Build) -> Option<String> {
    let platform = match build.platform {
        crate::models::Platform::Windows => "windows",
        crate::models::Platform::Macos => "macos",
        crate::models::Platform::Linux => "linux",
    };
    let arch = match build.architecture {
        crate::models::Architecture::X64 => "x64",
        crate::models::Architecture::Arm64 => "arm64",
    };
    Some(format!("{platform}-{arch}"))
}

/// 解析目录响应。顶层是 `{blocklist: [], data: [...]}`。
///
/// 线上实测没有文档里说的 `version` 字段，所以只依赖 `data`。
fn parse_catalog(body: &str) -> Result<Vec<CatalogItem>> {
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(default)]
        data: Vec<CatalogItem>,
    }
    let envelope: Envelope = serde_json::from_str(body)
        .map_err(|err| AppError::Network(format!("扩展目录 JSON 解析失败: {err}")))?;
    Ok(envelope.data)
}

/// 安装扩展：下载 zip → 校验哈希 → 委派 Blender CLI。
///
/// 绝不自己往 `extensions/` 写文件（docs/06 §2 F5）。
#[tauri::command]
pub async fn install_extension(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    build_id: String,
    pkg_id: String,
    enable: bool,
) -> std::result::Result<InstalledExtensionView, String> {
    let build = installed_build(&state, &build_id).map_err(String::from)?;
    let install_path = build
        .install_path
        .clone()
        .ok_or_else(|| "Build is not installed".to_string())?;

    // 从目录里找到这个包，拿它的下载地址与哈希。
    let catalog = get_extension_catalog_inner(&state, &build).await?;
    let item = catalog
        .into_iter()
        .find(|item| item.id == pkg_id)
        .ok_or_else(|| format!("目录里找不到扩展 {pkg_id}"))?;

    if item.archive_url.is_empty() {
        return Err(format!("扩展 {pkg_id} 没有下载地址"));
    }

    // 下载到缓存目录，不落到 extensions/ 里。
    let cache = state.settings.lock().unwrap().cache_dir.clone();
    let download_dir = cache.join("extensions");
    std::fs::create_dir_all(&download_dir).map_err(|err| err.to_string())?;
    let filename = crate::service::downloader::filename_from_url(&item.archive_url);
    if filename.is_empty() || filename.contains("..") {
        return Err(format!("扩展 {pkg_id} 的下载文件名非法: {filename:?}"));
    }
    let zip_path = download_dir.join(&filename);

    if !zip_path.is_file() {
        download_to(&state, &item.archive_url, &zip_path).await?;
    }

    // 校验 sha256。目录给了哈希就必须对上，否则拒绝执行——
    // 装一个哈希不符的包等于把供应链风险直接放进 Blender。
    if let Some(expected) = item.archive_hash.as_deref() {
        verify_hash(&zip_path, expected)?;
    } else {
        tracing::warn!(pkg = %pkg_id, "archive_hash 缺失，跳过校验");
    }

    let ext_root = extensions::resolve_ext_root_for(&install_path, &build.version)?;
    let pkg_id = install_via_cli(&install_path, &ext_root, &zip_path, enable)?;

    let _ = app_handle.emit("extension_changed", ());
    scan_one(&install_path, &build.version, "user_default", &pkg_id).map_err(String::from)
}

/// 从本地文件安装（离线包导入 / 用户手动下载的 zip）。
#[tauri::command]
pub async fn install_extension_file(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    build_id: String,
    path: String,
    enable: bool,
) -> std::result::Result<InstalledExtensionView, String> {
    let build = installed_build(&state, &build_id).map_err(String::from)?;
    let install_path = build
        .install_path
        .clone()
        .ok_or_else(|| "Build is not installed".to_string())?;

    let zip_path = PathBuf::from(&path);
    if !zip_path.is_file() {
        return Err(format!("文件不存在: {path}"));
    }
    let extension = zip_path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if extension != "zip" {
        return Err(format!("只支持 .zip 扩展包，收到 .{extension}"));
    }

    // 同 `install_extension`：退出码 0 不代表装上，必须查磁盘。
    // 这里能确定 pkg_id（zip 里的 manifest 就是权威来源），
    // 所以不必退回到「最近修改的目录」这种猜测——猜错会把别的扩展报给用户。
    let ext_root = extensions::resolve_ext_root_for(&install_path, &build.version)?;
    let pkg_id = install_via_cli(&install_path, &ext_root, &zip_path, enable)?;

    let _ = app_handle.emit("extension_changed", ());
    scan_one(&install_path, &build.version, "user_default", &pkg_id).map_err(String::from)
}

/// 启用 / 禁用扩展。CLI 没有对应子命令，走 Python 表达式。
#[tauri::command]
pub async fn set_extension_enabled(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    build_id: String,
    repo: String,
    pkg_id: String,
    enabled: bool,
) -> std::result::Result<(), String> {
    let build = installed_build(&state, &build_id).map_err(String::from)?;
    let install_path = build
        .install_path
        .clone()
        .ok_or_else(|| "Build is not installed".to_string())?;

    // 两个值都来自前端，会被拼进 Python 源码，必须校验。
    //
    // 扩展：module 一定是 `bl_ext.<repo>.<pkg_id>`，两段都应是标识符。
    // 旧式插件（repo 为空）：module 是裸模块名，实测存在
    // `Empty & Collection Switcher` 这种带空格和 `&` 的名字，
    // 用标识符规则会误杀合法插件——改为整体 JSON 序列化，
    // 由 serde_json 生成合法的 Python 字符串字面量（含引号转义）。
    let module = if repo.is_empty() {
        if pkg_id.is_empty() {
            return Err("模块名为空".to_string());
        }
        pkg_id.clone()
    } else {
        if !is_safe_module_part(&repo) || !is_safe_module_part(&pkg_id) {
            return Err(format!("非法的扩展标识: {repo}.{pkg_id}"));
        }
        format!("bl_ext.{repo}.{pkg_id}")
    };
    let module_literal =
        serde_json::to_string(&module).map_err(|err| format!("模块名序列化失败: {err}"))?;
    let op = if enabled {
        "addon_enable"
    } else {
        "addon_disable"
    };
    // 每次改完都存盘，否则 Blender 退出时改动只留在内存里。
    let expr = format!(
        "import bpy;bpy.ops.preferences.{op}(module={module_literal});bpy.ops.wm.save_userpref();print('@@BLMGR_OK@@')"
    );

    let exe = extension_probe::blender_executable(&install_path)
        .ok_or_else(|| format!("{} 下找不到 Blender 可执行文件", install_path.display()))?;
    let output = std::process::Command::new(&exe)
        .args(["-b", "--python-expr", &expr])
        .output()
        .map_err(|err| format!("启动 Blender 失败: {err}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains("@@BLMGR_OK@@") {
        let stderr: String = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(500)
            .collect();
        return Err(format!("切换启用状态失败。stderr: {stderr}"));
    }

    let _ = app_handle.emit("extension_changed", ());
    Ok(())
}

/// 卸载扩展。委派 `extension remove`。
#[tauri::command]
pub async fn remove_extension(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    build_id: String,
    repo: String,
    pkg_id: String,
) -> std::result::Result<(), String> {
    let build = installed_build(&state, &build_id).map_err(String::from)?;
    let install_path = build
        .install_path
        .clone()
        .ok_or_else(|| "Build is not installed".to_string())?;

    if !is_safe_module_part(&repo) || !is_safe_module_part(&pkg_id) {
        return Err(format!("非法的扩展标识: {repo}.{pkg_id}"));
    }

    let pkg_path = extensions::resolve_ext_root(&build)
        .map_err(String::from)?
        .join(&repo)
        .join(&pkg_id);

    // 方案 B 的安全前提（docs/06 §7.3）：目录是 junction 时，
    // 让 Blender 递归删除会顺着链接把中央库的源内容一起删掉。
    // 先自己 unlink，再交给 CLI 清仓库索引。
    if pkg_path.join(MANAGED_MARKER).is_file() {
        remove_link(&pkg_path)?;
    }

    run_extension_cli(
        &install_path,
        &["remove".to_string(), format!("{repo}.{pkg_id}")],
    )
    .map_err(|err| format!("Blender 卸载扩展失败: {err}"))?;

    let _ = app_handle.emit("extension_changed", ());
    Ok(())
}

/// 接管一个已装扩展到中央库（阶段 B1）。
///
/// 只有用户明确要求时才执行：接管会改写扩展目录结构，
/// 把实现放在独立命令里，让「只是浏览插件」这条路径永远不写盘。
#[tauri::command]
pub async fn adopt_extension(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    build_id: String,
    repo: String,
    pkg_id: String,
) -> std::result::Result<ExtensionManageView, String> {
    let (ext_root, central_root) = manage_context(&state, &build_id).map_err(String::from)?;
    if !settings_enabled(&state) {
        return Err("中央库未启用，请先在设置里开启".to_string());
    }
    if !is_safe_module_part(&repo) || !is_safe_module_part(&pkg_id) {
        return Err(format!("非法的扩展标识: {repo}.{pkg_id}"));
    }

    let pkg_path = ext_root.join(&repo).join(&pkg_id);
    let manifest = extensions::read_manifest(&pkg_path).map_err(String::from)?;
    let version = manifest.version.clone();

    let pkg_path_for_task = pkg_path.clone();
    let pkg_id_for_task = pkg_id.clone();
    let marker = tokio::task::spawn_blocking(move || {
        crate::service::extension_central::adopt(
            &pkg_path_for_task,
            &central_root,
            &pkg_id_for_task,
            &version,
        )
    })
    .await
    .map_err(|err| format!("接管任务失败: {err}"))?
    .map_err(String::from)?;

    let _ = app_handle.emit("extension_changed", ());
    Ok(ExtensionManageView {
        pkg_id: pkg_id.clone(),
        repo,
        managed: true,
        central_path: Some(marker.central_path.display().to_string()),
        versions: central_versions(&state, &pkg_id),
    })
}

/// 切换到中央库里的另一个版本。
#[tauri::command]
pub async fn switch_extension_version(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    build_id: String,
    repo: String,
    pkg_id: String,
    version: String,
) -> std::result::Result<ExtensionManageView, String> {
    let (ext_root, central_root) = manage_context(&state, &build_id).map_err(String::from)?;
    if !is_safe_module_part(&repo) || !is_safe_module_part(&pkg_id) {
        return Err(format!("非法的扩展标识: {repo}.{pkg_id}"));
    }

    let pkg_path = ext_root.join(&repo).join(&pkg_id);
    let pkg_id_for_task = pkg_id.clone();
    let marker = tokio::task::spawn_blocking(move || {
        crate::service::extension_central::switch_version(
            &pkg_path,
            &central_root,
            &pkg_id_for_task,
            &version,
        )
    })
    .await
    .map_err(|err| format!("切换任务失败: {err}"))?
    .map_err(String::from)?;

    let _ = app_handle.emit("extension_changed", ());
    Ok(ExtensionManageView {
        pkg_id: pkg_id.clone(),
        repo,
        managed: true,
        central_path: Some(marker.central_path.display().to_string()),
        versions: central_versions(&state, &pkg_id),
    })
}

/// 解除接管：内容复制回原位置，目录恢复成普通文件夹。
#[tauri::command]
pub async fn release_extension(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    build_id: String,
    repo: String,
    pkg_id: String,
) -> std::result::Result<(), String> {
    let (ext_root, _) = manage_context(&state, &build_id).map_err(String::from)?;
    if !is_safe_module_part(&repo) || !is_safe_module_part(&pkg_id) {
        return Err(format!("非法的扩展标识: {repo}.{pkg_id}"));
    }

    let pkg_path = ext_root.join(&repo).join(&pkg_id);
    tokio::task::spawn_blocking(move || crate::service::extension_central::release(&pkg_path))
        .await
        .map_err(|err| format!("解除接管任务失败: {err}"))?
        .map_err(String::from)?;

    let _ = app_handle.emit("extension_changed", ());
    Ok(())
}

/// 某扩展在中央库里的可用版本。
#[tauri::command]
pub async fn list_extension_versions(
    state: State<'_, AppState>,
    build_id: String,
    pkg_id: String,
) -> std::result::Result<Vec<String>, String> {
    let (_, central_root) = manage_context(&state, &build_id).map_err(String::from)?;
    if !is_safe_module_part(&pkg_id) {
        return Err(format!("非法的扩展标识: {pkg_id}"));
    }
    Ok(crate::service::extension_central::versions(
        &central_root,
        &pkg_id,
    ))
}

/// 阶段 B 的管理结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManageView {
    pub pkg_id: String,
    pub repo: String,
    pub managed: bool,
    pub central_path: Option<String>,
    /// 中央库里该扩展的全部版本（降序），供切换下拉。
    pub versions: Vec<String>,
}

/// 阶段 B 需要的两个路径：扩展根与中央库根。
///
/// 不返回 `install_path`：接管/切换/解除都只动扩展根，
/// 多返回一个没人用的路径，就是多一处将来被误用的机会。
fn manage_context(state: &AppState, build_id: &str) -> Result<(PathBuf, PathBuf)> {
    let build = installed_build(state, build_id)?;
    let install_path = build
        .install_path
        .clone()
        .ok_or_else(|| AppError::InvalidSettings("构建未安装".to_string()))?;
    let ext_root = extensions::resolve_ext_root_for(&install_path, &build.version)?;
    Ok((ext_root, central_root(state)))
}

/// 中央库根目录：设置里的自定义路径，否则数据目录下的 `extensions/`。
fn central_root(state: &AppState) -> PathBuf {
    state
        .settings
        .lock()
        .unwrap()
        .extension_central_dir
        .clone()
        .unwrap_or_else(crate::service::settings::default_extension_central_root)
}

fn settings_enabled(state: &AppState) -> bool {
    state.settings.lock().unwrap().enable_extension_central
}

fn central_versions(state: &AppState, pkg_id: &str) -> Vec<String> {
    crate::service::extension_central::versions(&central_root(state), pkg_id)
}

/// 由调用方提供 URL 打开系统浏览器。
#[tauri::command]
pub async fn open_extension_page(url: String) -> std::result::Result<(), String> {
    // 只放行 http(s)：这个命令把 URL 交给 shell，放开 file:// 或
    // 自定义协议等于把 shell 交给远端内容。
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("只支持 http(s) 链接: {url}"));
    }
    if url.contains(['\n', '\r', '"', '\'', '`']) {
        return Err("链接含非法字符".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        // 用 rundll32 而不是 cmd start：cmd 会对 & 做额外解析。
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn()
            .map_err(|err| format!("打开链接失败: {err}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|err| format!("打开链接失败: {err}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|err| format!("打开链接失败: {err}"))?;
    }
    Ok(())
}

// ---------- 内部辅助 ----------

/// 前端展示用的精简结构。`InstalledExtension` 里的 `PathBuf` 在
/// Windows 上序列化成带反斜杠的字符串，前端拼路径容易出错，
/// 所以统一转成字符串。
///
/// 同时把 `manifest` 摊平：前端只读 `name`/`version`/`kind` 三个值，
/// 让 JS 侧按 `item.manifest.name` 逐层取字段，等于把 Rust 的类型结构
/// 泄漏到模板里，字段一改就要同步改前端。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledExtensionView {
    pub pkg_id: String,
    pub repo: String,
    pub path: String,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub enabled: Option<bool>,
    pub managed: bool,
}

/// 旧式插件的展示结构，理由同上。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyAddonView {
    pub module: String,
    pub path: String,
    pub name: String,
    pub version: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
}

/// 一次扫描的展示结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionScanView {
    pub extensions: Vec<InstalledExtensionView>,
    pub legacy: Vec<LegacyAddonView>,
    pub corrupt: Vec<String>,
    pub ext_root: Option<String>,
    pub enabled_checked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<crate::models::ExtensionScan> for ExtensionScanView {
    fn from(scan: crate::models::ExtensionScan) -> Self {
        Self {
            extensions: scan.extensions.into_iter().map(view).collect(),
            legacy: scan
                .legacy
                .into_iter()
                .map(|addon| LegacyAddonView {
                    module: addon.module,
                    path: addon.path.display().to_string(),
                    name: addon.name,
                    version: addon.version,
                    category: addon.category,
                    description: addon.description,
                    enabled: addon.enabled,
                })
                .collect(),
            corrupt: scan.corrupt,
            ext_root: scan.ext_root.map(|path| path.display().to_string()),
            enabled_checked_at: scan.enabled_checked_at,
        }
    }
}

fn view(extension: crate::models::InstalledExtension) -> InstalledExtensionView {
    InstalledExtensionView {
        pkg_id: extension.pkg_id.clone(),
        repo: extension.repo.clone(),
        path: extension.path.display().to_string(),
        name: extension.manifest.name.clone(),
        version: extension.manifest.version.clone(),
        kind: if extension.manifest.kind_raw.is_empty() {
            "add-on".to_string()
        } else {
            extension.manifest.kind_raw.clone()
        },
        enabled: extension.enabled,
        managed: extension.managed.is_some(),
    }
}

fn scan_all(install_path: &Path, version: &str) -> Result<ExtensionScan> {
    let root = extensions::resolve_ext_root_for(install_path, version)?;
    Ok(extensions::scan(&root))
}

fn scan_one(
    install_path: &Path,
    version: &str,
    repo: &str,
    pkg_id: &str,
) -> Result<InstalledExtensionView> {
    let scan = scan_all(install_path, version)?;
    scan.extensions
        .into_iter()
        .find(|item| item.repo == repo && item.pkg_id == pkg_id)
        .map(view)
        .ok_or_else(|| AppError::InternalError(format!("安装后未找到 {repo}/{pkg_id}")))
}

/// 模块名片段只允许字母、数字、下划线、连字符、点。
/// 这些值会被拼进 Python 源码字符串，必须挡住引号与分隔符。
fn is_safe_module_part(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
}

/// 删除 junction 或符号链接本身，不触碰目标内容。
///
/// 具体实现放在 `service::extension_central`，那里会先判类型再删：
/// 「先试 remove_dir、失败再试 remove_file」在一个普通目录上会删掉
/// 空目录本身——调用方本意是删链接，却把用户目录删了。
fn remove_link(path: &Path) -> Result<()> {
    crate::service::extension_central::remove_link(path)
}

async fn get_extension_catalog_inner(state: &AppState, build: &Build) -> Result<Vec<CatalogItem>> {
    let root = extensions::resolve_ext_root(build)?;
    let root_clone = root.clone();
    let local = tokio::task::spawn_blocking(move || extensions::read_all_repo_indexes(&root_clone))
        .await
        .map_err(|err| AppError::InternalError(format!("读取本地索引失败: {err}")))?;
    if !local.is_empty() {
        return Ok(local);
    }

    let proxy = state.settings.lock().unwrap().proxy.clone();
    let url = match platform_param(build) {
        Some(platform) => format!("{CATALOG_URL}?format=json&platform={platform}"),
        None => format!("{CATALOG_URL}?format=json"),
    };
    let client = build_client(&proxy, "blender-launcher-v3", CATALOG_TIMEOUT)?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|err| AppError::Network(format!("获取扩展目录失败: {err}")))?;
    let body = response
        .text()
        .await
        .map_err(|err| AppError::Network(format!("读取响应失败: {err}")))?;
    parse_catalog(&body)
}

async fn download_to(state: &AppState, url: &str, dest: &Path) -> Result<()> {
    let proxy = state.settings.lock().unwrap().proxy.clone();
    let client = build_client(&proxy, "blender-launcher-v3", Duration::from_secs(300))?;
    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(|err| AppError::Network(format!("下载失败: {err}")))?
        .bytes()
        .await
        .map_err(|err| AppError::Network(format!("读取下载内容失败: {err}")))?;
    std::fs::write(dest, &bytes).map_err(AppError::from)?;
    Ok(())
}

/// 校验 `sha256:<hex>` 形式的哈希。
fn verify_hash(path: &Path, expected: &str) -> Result<()> {
    use sha2::{Digest, Sha256};

    let expected = expected
        .strip_prefix("sha256:")
        .unwrap_or(expected)
        .to_lowercase();
    if expected.len() != 64 || !expected.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(AppError::InvalidSettings(format!(
            "哈希格式非法: {expected}"
        )));
    }

    let bytes = std::fs::read(path).map_err(AppError::from)?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        return Err(AppError::Network(format!(
            "扩展包哈希不符（期望 {expected}，实际 {actual}）"
        )));
    }
    Ok(())
}

/// 执行一次 `blender --command extension <args>`。
///
/// 不解析 stdout：`--command` 仍会加载全部已装插件，输出被插件日志
/// 彻底淹没（上百行 `Registered ...`），任何基于输出的判断都不可靠。
/// 只认退出码，成功与否由调用方重新扫描目录确认。
///
/// `install-file` 的启用开关是 `-e` / `--enable`（`bl_extension_cli.py`
/// 的 `install-file` 子命令），不存在 `--enable-on-install`。
fn run_extension_cli(install_path: &Path, args: &[String]) -> std::result::Result<(), String> {
    let exe = extension_probe::blender_executable(install_path)
        .unwrap_or_else(|| install_path.join("blender.exe"));

    let output = std::process::Command::new(&exe)
        .args(["--command", "extension"])
        .args(args)
        .output()
        .map_err(|err| format!("启动 Blender 失败: {err}"))?;

    if !output.status.success() {
        // stderr 里前 800 字符通常就包含真正的错误行，后面的都是插件日志。
        let stderr: String = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(800)
            .collect();
        return Err(format!(
            "退出码 {:?}。stderr: {stderr}",
            output.status.code()
        ));
    }
    Ok(())
}

/// 从扩展包里读 `id`，用来确定装好后应出现的目录名。
///
/// 不能靠 `filename_from_url` 推断：目录里的 zip 名是
/// `add-on-<id>-v<ver>.zip`，而用户自备的包文件名随意。
/// 也容忍恰好一层包裹目录（GitHub「Download ZIP」风格），
/// 与 Blender 自己的 `pkg_zipfile_detect_subdir_or_none` 一致。
fn read_package_id_from_zip(zip_path: &Path) -> std::result::Result<String, String> {
    use std::io::Read;

    let file = std::fs::File::open(zip_path).map_err(|err| format!("打开扩展包失败: {err}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|err| format!("扩展包不是合法 zip: {err}"))?;

    let mut manifest = String::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("读取扩展包条目失败: {err}"))?;
        let name = entry.name().to_string();
        // 只接受根目录或一层子目录下的 blender_manifest.toml。
        let relative = name.trim_start_matches("./");
        let parts: Vec<&str> = relative.split('/').collect();
        let is_manifest = matches!(
            parts.as_slice(),
            ["blender_manifest.toml"] | [_, "blender_manifest.toml"]
        );
        if !is_manifest {
            continue;
        }
        entry
            .read_to_string(&mut manifest)
            .map_err(|err| format!("读取 blender_manifest.toml 失败: {err}"))?;
        break;
    }

    if manifest.is_empty() {
        return Err("扩展包里没有 blender_manifest.toml".to_string());
    }

    let parsed: toml::Value = toml::from_str(&manifest)
        .map_err(|err| format!("blender_manifest.toml 解析失败: {err}"))?;
    parsed
        .get("id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "blender_manifest.toml 里缺少 id".to_string())
}

/// 装一个包，并确认它真的落到了盘上。
///
/// 为什么不能只信退出码：`user_default` 仓库在 Blender 里默认是**关闭**的。
/// 仓库关闭时 `install-file` 会静默地什么都不做却返回 0——
/// 2026-11-24 在 5.3.0 上实测复现：删掉 `user_default` 目录后安装，
/// 退出码 0、磁盘无任何变化。用户会以为装好了，去 Blender 里却找不到。
///
/// 因此这里做三件事：
/// 1. 从 zip 的 manifest 读权威 `id`，确定装好后应有的目录名；
/// 2. 目录不存在时启用仓库并重试一次（仓库只在偏好设置里开，无法从 CLI 传参）；
/// 3. 重试后仍不存在就报错，并把用户可执行的修复动作写清楚。
///
/// 返回实际装上的 `pkg_id`。
fn install_via_cli(
    install_path: &Path,
    ext_root: &Path,
    zip_path: &Path,
    enable: bool,
) -> std::result::Result<String, String> {
    let pkg_id = read_package_id_from_zip(zip_path)?;
    let target = ext_root.join("user_default").join(&pkg_id);

    run_extension_cli(install_path, &install_file_args(zip_path, enable))
        .map_err(|err| format!("Blender 安装扩展失败: {err}"))?;
    if target.is_dir() {
        return Ok(pkg_id);
    }

    // 头一次没装上：多半是仓库关闭。开启后重试。
    enable_local_repo(install_path)?;
    run_extension_cli(install_path, &install_file_args(zip_path, enable))
        .map_err(|err| format!("Blender 安装扩展失败: {err}"))?;
    if target.is_dir() {
        return Ok(pkg_id);
    }

    Err(format!(
        "安装未生效：{} 不存在。\
         已在 Blender 里启用 user_default 仓库并重试，仍未成功。\
         请在 Blender 的「编辑 → 偏好设置 → 扩展」里手动确认该仓库已勾选，\
         并检查磁盘权限。",
        target.display()
    ))
}

/// 确保 `user_default` 仓库处于启用状态。
///
/// 该仓库默认关闭，且 `install-file -r user_default` 在关闭时静默失败，
/// 所以在委派安装前后都要能打开它。仓库信息存在 `userpref.blend`，
/// 没有命令行开关，只能用 Python 表达式改并保存。
fn enable_local_repo(install_path: &Path) -> std::result::Result<(), String> {
    let exe = extension_probe::blender_executable(install_path)
        .unwrap_or_else(|| install_path.join("blender.exe"));

    // 单行表达式：`--python-expr` 传多行时要自己管缩进，一旦不对就是
    // IndentationError，而错误只会出现在被插件日志淹没的 stderr 里。
    let expr = "import bpy;\
                [setattr(repo, 'enabled', True) for repo in bpy.context.preferences.extensions.repos \
                if repo.module == 'user_default'];\
                bpy.ops.wm.save_userpref();\
                print('@@BLMGR_REPO_ENABLED@@')";

    let output = std::process::Command::new(&exe)
        .args(["-b", "--python-expr", expr])
        .output()
        .map_err(|err| format!("启用 user_default 仓库失败（无法启动 Blender）: {err}"))?;

    if !output.status.success() {
        let stderr: String = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(800)
            .collect();
        return Err(format!("启用 user_default 仓库失败。stderr: {stderr}"));
    }
    Ok(())
}

/// 拼 `install-file` 的参数。`enable` 为真才加 `-e`，
/// 否则装上但不启用。
fn install_file_args(zip: &Path, enable: bool) -> Vec<String> {
    let mut args = vec![
        "install-file".to_string(),
        "-r".to_string(),
        "user_default".to_string(),
    ];
    if enable {
        args.push("-e".to_string());
    }
    args.push(zip.display().to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 建一个本次进程专用的临时目录，调用方负责删除。
    fn unique_temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let nth = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("blmgr-ext-test-{}-{nth}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 在临时目录里造一个最小扩展包，`prefix` 用来控制是否带一层包裹目录。
    ///
    /// 不用 `tempfile`：为三个测试加一个 dev-dependency 不划算，
    /// `std::env::temp_dir()` 配进程号与计数器已经够隔离。
    fn make_zip(prefix: Option<&str>) -> (PathBuf, PathBuf) {
        let dir = unique_temp_dir();
        let path = dir.join("pkg.zip");
        let manifest = "schema_version = \"1.0.0\"
id = \"probe_pkg\"
version = \"0.1.0\"
";
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let name = match prefix {
            Some(prefix) => format!("{prefix}/blender_manifest.toml"),
            None => "blender_manifest.toml".to_string(),
        };
        writer.start_file(name, options).unwrap();
        writer.write_all(manifest.as_bytes()).unwrap();
        writer.finish().unwrap();
        (dir, path)
    }

    #[test]
    fn test_read_package_id_root_layout() {
        let (dir, path) = make_zip(None);
        assert_eq!(read_package_id_from_zip(&path).unwrap(), "probe_pkg");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_read_package_id_github_zip_layout() {
        // GitHub「Download ZIP」会在根下多包一层目录，Blender 也容忍恰好一层。
        let (dir, path) = make_zip(Some("repo-main"));
        assert_eq!(read_package_id_from_zip(&path).unwrap(), "probe_pkg");
        std::fs::remove_dir_all(dir).ok();
    }

    /// 真机验证：完整走一遍「仓库关闭 → 安装 → 自动启用并重试 → 卸载」。
    ///
    /// 一次性覆盖 install_via_cli 的两条分支，这是首次安装扩展时的真实路径。
    /// 前置条件：`user_default` 仓库处于关闭状态、其目录不存在
    /// （可在 Blender 里取消勾选，或删掉 `portable/extensions/user_default`）。
    ///
    /// 手动运行：`cargo test --lib live_install_retry -- --ignored --nocapture`
    #[test]
    #[ignore = "会真的往本机 Blender 扩展目录装一个临时插件并卸载"]
    fn live_install_retry_enables_repo_when_disabled() {
        let install_path = Path::new("D:/Blender/stable/blender-5.3.0-windows-x64");
        if !install_path.is_dir() {
            println!("跳过：本机没有该构建");
            return;
        }
        let zip_path = Path::new("C:/Temp/exttest/blmgr_probe.zip");
        if !zip_path.is_file() {
            println!("跳过：测试包不存在");
            return;
        }
        let ext_root = extensions::resolve_ext_root_for(install_path, "5.3.0").unwrap();
        let target = ext_root.join("user_default").join("blmgr_probe");
        let _ = std::fs::remove_dir_all(&target);

        let pkg_id = install_via_cli(install_path, &ext_root, zip_path, false)
            .expect("仓库关闭时应自动启用并装成功");
        assert_eq!(pkg_id, "blmgr_probe");
        assert!(target.is_dir(), "安装后目录应存在: {}", target.display());

        // 收尾：卸载，别给用户留下测试插件。
        run_extension_cli(
            install_path,
            &["remove".to_string(), "user_default.blmgr_probe".to_string()],
        )
        .expect("卸载应成功");
        assert!(!target.exists(), "卸载后目录应消失");
        println!("install+retry+remove ok");
    }

    /// 真机验证：`enable_local_repo` 的表达式能被 Blender 正确执行。
    ///
    /// 这条路径是「用户首次安装扩展」的必经之路——`user_default`
    /// 默认关闭，`install-file` 会静默失败。表达式写错的话只会在
    /// 用户机器上表现为「安装没反应」。
    ///
    /// 手动运行：`cargo test --lib live_enable_local_repo -- --ignored --nocapture`
    #[test]
    #[ignore = "会修改本机 Blender 的偏好设置"]
    fn live_enable_local_repo_expression_is_valid() {
        let install_path = Path::new("D:/Blender/stable/blender-5.3.0-windows-x64");
        if !install_path.is_dir() {
            return;
        }
        enable_local_repo(install_path).expect("启用 user_default 仓库应成功");
        println!("user_default repo enabled");
    }

    #[test]
    fn test_read_package_id_rejects_missing_manifest() {
        let dir = unique_temp_dir();
        let path = dir.join("empty.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("__init__.py", options).unwrap();
        writer.write_all(b"pass").unwrap();
        writer.finish().unwrap();
        let err = read_package_id_from_zip(&path).unwrap_err();
        assert!(err.contains("blender_manifest.toml"), "实际错误: {err}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_platform_param() {
        let mut build = Build {
            id: "x".into(),
            version: "5.3.0".into(),
            branch: crate::models::BuildBranch::Stable,
            fork: crate::models::BuildFork::Official,
            platform: crate::models::Platform::Windows,
            architecture: crate::models::Architecture::X64,
            url: String::new(),
            size_bytes: 0,
            release_date: chrono::Utc::now(),
            release_notes_url: None,
            breaking_changes: None,
            is_installed: true,
            install_path: None,
            update_build_id: None,
        };
        assert_eq!(platform_param(&build).as_deref(), Some("windows-x64"));

        build.platform = crate::models::Platform::Macos;
        build.architecture = crate::models::Architecture::Arm64;
        assert_eq!(platform_param(&build).as_deref(), Some("macos-arm64"));
    }

    #[test]
    fn test_parse_catalog_real_response_shape() {
        // 线上实测顶层只有 blocklist 与 data，没有 version。
        let body = r#"{
            "blocklist": [],
            "data": [
                {
                    "id": "facetor",
                    "schema_version": "1.0.0",
                    "name": "Facetor",
                    "version": "1.0.1",
                    "tagline": "Boolean operations",
                    "archive_hash": "sha256:fdc66849",
                    "archive_size": 128924,
                    "archive_url": "https://extensions.blender.org/download/sha256:fdc/add-on-facetor-v1.0.1.zip",
                    "type": "add-on",
                    "blender_version_min": "4.2.0",
                    "website": "https://example.com",
                    "maintainer": "someone",
                    "license": ["SPDX:GPL-3.0-or-later"],
                    "tags": ["Mesh"]
                }
            ]
        }"#;
        let items = parse_catalog(body).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "facetor");
        assert_eq!(items[0].kind(), Some(crate::models::ExtensionKind::AddOn));
        assert_eq!(items[0].archive_size, 128924);
    }

    #[test]
    fn test_parse_catalog_tolerates_unknown_fields_and_missing_optional() {
        let body = r#"{"blocklist":[],"data":[
            {"id":"minimal","name":"Minimal","version":"2.0.0","type":"theme",
             "something_new":"ignored"}
        ]}"#;
        let items = parse_catalog(body).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind(), Some(crate::models::ExtensionKind::Theme));
        assert!(items[0].archive_url.is_empty());
        assert!(items[0].platforms.is_none());
    }

    #[test]
    fn test_parse_catalog_empty_is_ok() {
        let items = parse_catalog(r#"{"blocklist":[],"data":[]}"#).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_parse_catalog_malformed_is_error() {
        assert!(parse_catalog("not json").is_err());
        // data 不是数组也要报错，不能静默当成空目录。
        assert!(parse_catalog(r#"{"blocklist":[],"data":"nope"}"#).is_err());
    }

    #[test]
    fn test_is_safe_module_part() {
        assert!(is_safe_module_part("blender_org"));
        assert!(is_safe_module_part("CAD_Sketcher"));
        assert!(is_safe_module_part("b4d_tools"));
        // 会破坏 Python 字符串或路径的必须拒。
        assert!(!is_safe_module_part("a'b"));
        assert!(!is_safe_module_part("a\"b"));
        assert!(!is_safe_module_part("a/b"));
        assert!(!is_safe_module_part("a\\b"));
        assert!(!is_safe_module_part("../etc"));
        assert!(!is_safe_module_part("a;b"));
        assert!(!is_safe_module_part(""));
    }

    #[test]
    fn test_verify_hash_rejects_bad_format() {
        let path = std::env::temp_dir().join("blmgr-hash-test.bin");
        std::fs::write(&path, b"hello").unwrap();

        // 长度不对。
        assert!(verify_hash(&path, "sha256:abc").is_err());
        // 非十六进制。
        assert!(verify_hash(&path, &format!("sha256:{}", "z".repeat(64))).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_verify_hash_accepts_correct_digest() {
        let path = std::env::temp_dir().join("blmgr-hash-ok.bin");
        std::fs::write(&path, b"hello").unwrap();

        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(b"hello"));
        assert!(verify_hash(&path, &format!("sha256:{digest}")).is_ok());
        // 大写的期望值也应接受。
        assert!(verify_hash(&path, &format!("sha256:{}", digest.to_uppercase())).is_ok());
        // 内容不符必须拒。
        assert!(verify_hash(&path, &format!("sha256:{}", "0".repeat(64))).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_open_extension_page_rejects_non_http() {
        // 这个命令把 URL 交给 shell，必须挡住本地文件与自定义协议。
        let runtime = tokio::runtime::Runtime::new().unwrap();
        assert!(runtime
            .block_on(open_extension_page(
                "file:///C:/Windows/System32/calc.exe".into()
            ))
            .is_err());
        assert!(runtime
            .block_on(open_extension_page("javascript:alert(1)".into()))
            .is_err());
        assert!(runtime.block_on(open_extension_page("".into())).is_err());
    }
}
