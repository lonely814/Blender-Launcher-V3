use crate::error::{AppError, Result};
use crate::models::extension::{
    CatalogItem, ExtensionManifest, ExtensionScan, InstalledExtension, LegacyAddon,
    ManagedMetadata, MANAGED_MARKER, MANAGED_SCHEMA_VERSION,
};
use crate::models::Build;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 仓库自身元数据目录，不是扩展。
const REPO_META_DIR: &str = ".blender_ext";

/// 扩展自己的用户数据目录前缀，不是安装位置。
/// `bl_extension_ops.py::repo_user_directory` 会在 repo 下建 `.user/<module>`，
/// 里面是扩展运行时写的数据，扫描时必须排除。
const USER_DATA_DIR: &str = ".user";

/// Blender 用户资源目录（非便携构建）。
fn user_resource_root() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = dirs::data_dir()?.join("Blender Foundation").join("Blender");
    #[cfg(target_os = "macos")]
    let base = dirs::data_dir()?.join("Blender Foundation").join("Blender");
    #[cfg(target_os = "linux")]
    let base = dirs::data_dir()?.join("blender");

    Some(base)
}

/// 从版本串取 `major.minor`。`5.3.0` → `5.3`，`4.5` → `4.5`。
/// 取不到返回 `None`：自定义构建的版本串可能是任意文本。
pub fn version_dir_name(version: &str) -> Option<String> {
    let mut parts = version.split(['.', '-', '+']);
    let major = parts.next()?.trim();
    let minor = parts.next()?.trim();
    if major.is_empty() || minor.is_empty() {
        return None;
    }
    if !major.chars().all(|ch| ch.is_ascii_digit()) || !minor.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{major}.{minor}"))
}

/// 解析某个 Build 的扩展根目录。
///
/// 便携构建的判据是**构建目录内存在 `portable/` 目录**，而非
/// `<ver>/config`——本机实测 `blender-5.3.0-windows-x64/portable` 是一个
/// 指向另一版本便携目录的符号链接，Blender 的 `resource_path('USER')`
/// 直接返回它。用 `<ver>/config` 判断会漏掉这种共享便携配置的情况。
///
/// 环境变量 `BLENDER_USER_EXTENSIONS` 优先，与 Blender 自身一致。
pub fn resolve_ext_root(build: &Build) -> Result<PathBuf> {
    let install_path = build
        .install_path
        .as_ref()
        .ok_or_else(|| AppError::InvalidSettings(format!("构建 {} 没有安装路径", build.id)))?;
    resolve_ext_root_for(install_path, &build.version)
}

/// 同 [`resolve_ext_root`]，但只需要安装路径与版本号。
///
/// 安装/卸载路径上没有 `Build` 可传（CLI 只认目录），所以逻辑放在这里，
/// 避免为了拿一个路径去伪造一个 `Build`。
pub fn resolve_ext_root_for(install_path: &Path, version: &str) -> Result<PathBuf> {
    if let Ok(raw) = std::env::var("BLENDER_USER_EXTENSIONS") {
        let path = PathBuf::from(raw.trim());
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }

    let portable = install_path.join("portable");
    if portable.is_dir() {
        return Ok(portable.join("extensions"));
    }

    let version_dir = version_dir_name(version)
        .ok_or_else(|| AppError::InvalidSettings(format!("无法从版本号 {version:?} 解析目录名")))?;
    let root = user_resource_root()
        .ok_or_else(|| AppError::InvalidSettings("无法定位用户资源目录".to_string()))?;
    Ok(root.join(version_dir).join("extensions"))
}

/// 扫描一个扩展根目录下的全部已装扩展与旧式 addon。
///
/// 大量小文件读取，调用方应放到 `spawn_blocking`。
pub fn scan(root: &Path) -> ExtensionScan {
    let mut scan = ExtensionScan {
        ext_root: root.is_dir().then(|| root.to_path_buf()),
        ..ExtensionScan::default()
    };

    if !root.is_dir() {
        return scan;
    }

    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(path = %root.display(), error = %err, "扩展根目录不可读");
            return scan;
        }
    };

    for entry in entries.flatten() {
        let repo_path = entry.path();
        if !repo_path.is_dir() {
            continue;
        }
        let repo = entry.file_name().to_string_lossy().to_string();
        // 以 `.` 开头的是元数据（.blender_ext / .user / .cache），不是仓库。
        if repo.starts_with('.') {
            continue;
        }
        scan_repo(&repo_path, &repo, &mut scan);
    }

    // 旧式 addon 在 `<resources>/scripts/addons/`，即扩展根的上一级。
    if let Some(resources) = root.parent() {
        scan_legacy(resources, &mut scan);
    }

    scan.extensions
        .sort_by_key(|item| item.pkg_id.to_lowercase());
    scan.legacy.sort_by_key(|item| item.module.to_lowercase());
    scan
}

/// 扫描单个仓库目录：`<root>/<repo>/<pkg_id>/blender_manifest.toml`。
fn scan_repo(repo_path: &Path, repo: &str, scan: &mut ExtensionScan) {
    let Ok(entries) = std::fs::read_dir(repo_path) else {
        return;
    };

    for entry in entries.flatten() {
        let pkg_path = entry.path();
        if !pkg_path.is_dir() {
            continue;
        }
        let pkg_id = entry.file_name().to_string_lossy().to_string();
        if pkg_id.starts_with('.') || pkg_id == USER_DATA_DIR || pkg_id == REPO_META_DIR {
            continue;
        }

        match read_manifest(&pkg_path) {
            Ok(manifest) => {
                let managed = read_managed_marker(&pkg_path);
                scan.extensions.push(InstalledExtension {
                    pkg_id,
                    repo: repo.to_string(),
                    path: pkg_path,
                    manifest,
                    enabled: None,
                    managed,
                    size_bytes: 0,
                });
            }
            Err(err) => {
                // 有目录但读不出 manifest：记下来但不阻断其余条目。
                tracing::debug!(
                    path = %pkg_path.display(),
                    error = %err,
                    "扩展 manifest 不可读"
                );
                scan.corrupt.push(format!("{repo}/{pkg_id}"));
            }
        }
    }
}

/// 读 `blender_manifest.toml`。
pub fn read_manifest(pkg_path: &Path) -> Result<ExtensionManifest> {
    let manifest_path = pkg_path.join("blender_manifest.toml");
    let raw = std::fs::read_to_string(&manifest_path).map_err(AppError::from)?;
    let mut manifest: ExtensionManifest = toml::from_str(&raw)
        .map_err(|err| AppError::InvalidSettings(format!("manifest 解析失败: {err}")))?;

    // manifest 缺 id 时用目录名兜底，而不是丢弃这个包——
    // 用户已经装在盘上了，看不见比名字不准更糟。
    if manifest.id.trim().is_empty() {
        manifest.id = pkg_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
    }
    if manifest.name.trim().is_empty() {
        manifest.name = manifest.id.clone();
    }
    Ok(manifest)
}

/// 读所有权标记。不存在、损坏、版本不匹配都返回 `None`——
/// 判不准就当作「不是本程序接管」，宁可只读也不误删用户内容。
pub fn read_managed_marker(pkg_path: &Path) -> Option<ManagedMetadata> {
    let marker_path = pkg_path.join(MANAGED_MARKER);
    let raw = std::fs::read_to_string(marker_path).ok()?;
    let marker: ManagedMetadata = serde_json::from_str(&raw).ok()?;
    (marker.schema_version == MANAGED_SCHEMA_VERSION).then_some(marker)
}

/// 写所有权标记（原子写）。
pub fn write_managed_marker(pkg_path: &Path, marker: &ManagedMetadata) -> Result<()> {
    let json = serde_json::to_string_pretty(marker)
        .map_err(|err| AppError::InvalidSettings(err.to_string()))?;
    let target = pkg_path.join(MANAGED_MARKER);
    let temp = pkg_path.join(format!("{MANAGED_MARKER}.tmp"));
    std::fs::write(&temp, json).map_err(AppError::from)?;
    std::fs::rename(&temp, &target).map_err(AppError::from)?;
    Ok(())
}

/// 扫描旧式 addon：`<resources>/scripts/addons/{name}.py` 或 `{name}/__init__.py`。
fn scan_legacy(resources: &Path, scan: &mut ExtensionScan) {
    let addons_dir = resources.join("scripts").join("addons");
    let Ok(entries) = std::fs::read_dir(&addons_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let module = entry.file_name().to_string_lossy().to_string();
        if module.starts_with('.') || module == "__pycache__" {
            continue;
        }

        let (source, module_name) = if path.is_dir() {
            let init = path.join("__init__.py");
            if !init.is_file() {
                continue;
            }
            (init, module)
        } else if path.extension().is_some_and(|ext| ext == "py") {
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_default();
            (path, stem)
        } else {
            continue;
        };

        let Ok(text) = std::fs::read_to_string(&source) else {
            continue;
        };
        let Some(info) = parse_bl_info(&text) else {
            continue;
        };
        let name = info
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or(&module_name)
            .to_string();

        scan.legacy.push(LegacyAddon {
            module: module_name,
            path: source,
            name,
            version: info.get("version").and_then(format_version_tuple),
            category: info
                .get("category")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            description: info
                .get("description")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            enabled: None,
        });
    }
}

/// `(1, 3, 1)` → `"1.3.1"`。
fn format_version_tuple(value: &toml::Value) -> Option<String> {
    let array = value.as_array()?;
    let parts: Vec<String> = array
        .iter()
        .map(|item| item.as_integer().map(|num| num.to_string()))
        .collect::<Option<_>>()?;
    (!parts.is_empty()).then(|| parts.join("."))
}

/// 从 Python 源码里抽取 `bl_info = {...}` 的字面量。
///
/// 不执行 Python。做法是把 `{` 到匹配的 `}` 之间按行收集，再逐行用
/// `key = value` 的正则拆开；值只接受字符串、数字、元组/列表、布尔。
/// 任何一行解析失败就跳过那一行——`bl_info` 里常有注释、条件表达式、
/// 甚至 `__package__` 之类的变量，整块失败会让正常插件也看不见。
fn parse_bl_info(source: &str) -> Option<BTreeMap<String, toml::Value>> {
    let start = source.find("bl_info")?;
    let rest = &source[start..];
    let open = rest.find('{')?;
    let body_start = start + open + 1;

    // 括号配平找出收尾的 `}`。跳过字符串里的括号，避免
    // `"description": "a } b"` 这种内容提前截断。
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut in_string: Option<u8> = None;
    let mut index = body_start;
    while index < bytes.len() {
        let byte = bytes[index];
        match in_string {
            Some(quote) => {
                if byte == b'\\' {
                    index += 2;
                    continue;
                }
                if byte == quote {
                    in_string = None;
                }
            }
            None => match byte {
                b'"' | b'\'' => in_string = Some(byte),
                b'{' | b'[' | b'(' => depth += 1,
                b'}' | b']' | b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            },
        }
        index += 1;
    }
    let body = &source[body_start..index.min(source.len())];

    let mut map = BTreeMap::new();
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((key, value)) = split_bl_info_line(line) else {
            continue;
        };
        if let Some(parsed) = parse_python_literal(value) {
            map.insert(key, parsed);
        }
    }
    (!map.is_empty()).then_some(map)
}

/// 把 `"name": "Playblast",` 拆成 (`name`, `"Playblast"`)。
fn split_bl_info_line(line: &str) -> Option<(String, &str)> {
    let colon = line.find(':')?;
    let key = line[..colon].trim().trim_matches(['"', '\'']);
    let value = line[colon + 1..].trim().trim_end_matches(',');
    (!key.is_empty()
        && !value.is_empty()
        && key.chars().all(|ch| ch.is_alphanumeric() || ch == '_'))
    .then(|| (key.to_string(), value))
}

/// 解析 Python 字面量的子集：字符串、整数、布尔、元组/列表。
/// 其余形态（变量引用、函数调用、f-string）返回 `None`。
fn parse_python_literal(raw: &str) -> Option<toml::Value> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }

    if (text.starts_with('"') && text.ends_with('"') && text.len() >= 2)
        || (text.starts_with('\'') && text.ends_with('\'') && text.len() >= 2)
    {
        return Some(toml::Value::String(
            text[1..text.len() - 1]
                .replace("\\\"", "\"")
                .replace("\\'", "'"),
        ));
    }

    if text == "True" {
        return Some(toml::Value::Boolean(true));
    }
    if text == "False" {
        return Some(toml::Value::Boolean(false));
    }

    if let Ok(number) = text.parse::<i64>() {
        return Some(toml::Value::Integer(number));
    }

    // 元组或列表：`(1, 3, 1)` / `[1, 3, 1]`
    let inner = text
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .or_else(|| {
            text.strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
        })?;
    let mut items = Vec::new();
    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        items.push(toml::Value::Integer(part.parse::<i64>().ok()?));
    }
    Some(toml::Value::Array(items))
}

/// 解析本地仓库索引 `<root>/<repo>/.blender_ext/index.json`。
///
/// 结构是 `{ "blocklist": [], "data": [...] }`。字段名与线上 API 一致，
/// 所以同一个反序列化目标两处都能用。解析失败返回 `None`，
/// 调用方回落到联网刷新。
pub fn read_repo_index(repo_path: &Path) -> Option<Vec<CatalogItem>> {
    let path = repo_path.join(REPO_META_DIR).join("index.json");
    let raw = std::fs::read_to_string(path).ok()?;

    #[derive(serde::Deserialize)]
    struct Envelope {
        #[serde(default)]
        data: Vec<CatalogItem>,
    }

    let envelope: Envelope = serde_json::from_str(&raw).ok()?;
    (!envelope.data.is_empty()).then_some(envelope.data)
}

/// 聚合所有仓库的本地索引，按 id 去重（先出现的赢）。
pub fn read_all_repo_indexes(root: &Path) -> Vec<CatalogItem> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut seen = std::collections::BTreeSet::new();
    let mut items = Vec::new();
    for entry in entries.flatten() {
        let repo_path = entry.path();
        if !repo_path.is_dir() {
            continue;
        }
        let repo = entry.file_name().to_string_lossy().to_string();
        if repo.starts_with('.') {
            continue;
        }
        let Some(catalog) = read_repo_index(&repo_path) else {
            continue;
        };
        for item in catalog {
            if item.id.is_empty() || !seen.insert(item.id.clone()) {
                continue;
            }
            items.push(item);
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_dir_name() {
        assert_eq!(version_dir_name("5.3.0").as_deref(), Some("5.3"));
        assert_eq!(version_dir_name("4.5").as_deref(), Some("4.5"));
        assert_eq!(
            version_dir_name("5.4.0-alpha+main.abc").as_deref(),
            Some("5.4")
        );
        // 自定义构建的版本串不可解析时返回 None，不猜。
        assert_eq!(version_dir_name("custom"), None);
        assert_eq!(version_dir_name(""), None);
        assert_eq!(version_dir_name("x.y"), None);
    }

    #[test]
    fn test_parse_bl_info_real_addon() {
        // 取自本机 4.4 的 Playblast.py 头部。
        let source = r#"import bpy

bl_info = {
    "name": "Playblast",
    "author": "carlosmu <carlos.damian.munoz@gmail.com>",
    "blender": (3, 6, 0),
    "version": (1, 3, 1),
    "category": "Animation",
    "location": "3D View Main Menu",
    "description": "This addon allows you to keep renders options",
}

class PB_Prefs(bpy.types.AddonPreferences):
    pass
"#;
        let info = parse_bl_info(source).expect("bl_info 应被解析");
        assert_eq!(
            info.get("name").and_then(|value| value.as_str()),
            Some("Playblast")
        );
        assert_eq!(
            info.get("category").and_then(|value| value.as_str()),
            Some("Animation")
        );
        assert_eq!(
            info.get("version")
                .and_then(format_version_tuple)
                .as_deref(),
            Some("1.3.1")
        );
        // 未出现在 bl_info 里的键不应凭空存在。
        assert!(!info.contains_key("tracker_url"));
    }

    #[test]
    fn test_parse_bl_info_tolerates_unparseable_lines() {
        // 真实插件里 bl_info 可能引用变量、写注释、留空行。
        let source = r#"
bl_info = {
    # 这是注释
    "name": "Weird Addon",
    "version": (2, 0, 1),
    "blender": BLENDER_VERSION,
    "description": f"{__package__} 描述",
    "category": 'Object',
}
"#;
        let info = parse_bl_info(source).expect("至少能解析出可解析的字段");
        assert_eq!(
            info.get("name").and_then(|value| value.as_str()),
            Some("Weird Addon")
        );
        // 变量引用解析不了，跳过但保留其余字段。
        assert!(!info.contains_key("blender"));
        assert!(!info.contains_key("description"));
        // 单引号字符串也能解析。
        assert_eq!(
            info.get("category").and_then(|value| value.as_str()),
            Some("Object")
        );
    }

    #[test]
    fn test_parse_bl_info_no_block() {
        assert!(parse_bl_info("import bpy\nprint(1)\n").is_none());
    }

    #[test]
    fn test_bl_info_braces_inside_string_do_not_truncate() {
        let source = r#"
bl_info = {
    "name": "Brace } Test",
    "category": "{Object}",
}
"#;
        let info = parse_bl_info(source).expect("字符串里的括号不应截断");
        assert_eq!(
            info.get("name").and_then(|value| value.as_str()),
            Some("Brace } Test")
        );
        assert_eq!(
            info.get("category").and_then(|value| value.as_str()),
            Some("{Object}")
        );
    }

    #[test]
    fn test_managed_marker_roundtrip() {
        let dir = std::env::temp_dir().join(format!("blmgr-marker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let marker = ManagedMetadata {
            schema_version: MANAGED_SCHEMA_VERSION,
            central_path: PathBuf::from("D:/central/facetor/1.0.1"),
            pkg_id: "facetor".to_string(),
            managed_since: chrono::Utc::now(),
        };
        write_managed_marker(&dir, &marker).unwrap();

        let read = read_managed_marker(&dir).expect("应能读回");
        assert_eq!(read.pkg_id, "facetor");
        assert_eq!(read.schema_version, MANAGED_SCHEMA_VERSION);

        // schema_version 不匹配时必须当作「不是本程序接管」。
        let mut future = marker.clone();
        future.schema_version = MANAGED_SCHEMA_VERSION + 1;
        write_managed_marker(&dir, &future).unwrap();
        assert!(read_managed_marker(&dir).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_scan_skips_meta_and_user_data_dirs() {
        let root = std::env::temp_dir().join(format!("blmgr-scan-{}", std::process::id()));
        let repo = root.join("blender_org");
        let pkg = repo.join("facetor");
        let meta = repo.join(".blender_ext");
        let user = repo.join(".user");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::create_dir_all(&meta).unwrap();
        std::fs::create_dir_all(&user).unwrap();

        std::fs::write(
            pkg.join("blender_manifest.toml"),
            "id = \"facetor\"\nname = \"Facetor\"\nversion = \"1.0.1\"\ntype = \"add-on\"\n",
        )
        .unwrap();
        // 元数据目录里放一个同样合法的 manifest，它不应被当成扩展。
        std::fs::write(
            meta.join("blender_manifest.toml"),
            "id = \"meta\"\nname = \"Meta\"\nversion = \"9.9.9\"\n",
        )
        .unwrap();

        let scan = scan(&root);
        assert_eq!(scan.extensions.len(), 1, "只应看到真正的包");
        assert_eq!(scan.extensions[0].pkg_id, "facetor");
        assert_eq!(scan.extensions[0].repo, "blender_org");
        assert!(scan.corrupt.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_scan_records_corrupt_dirs_without_aborting() {
        let root = std::env::temp_dir().join(format!("blmgr-corrupt-{}", std::process::id()));
        let repo = root.join("blender_org");
        std::fs::create_dir_all(repo.join("good")).unwrap();
        std::fs::create_dir_all(repo.join("broken")).unwrap();

        std::fs::write(
            repo.join("good").join("blender_manifest.toml"),
            "id = \"good\"\nname = \"Good\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        // 坏 manifest：TOML 语法错误。
        std::fs::write(
            repo.join("broken").join("blender_manifest.toml"),
            "id = \"broken\nname = =",
        )
        .unwrap();

        let scan = scan(&root);
        assert_eq!(scan.extensions.len(), 1);
        assert_eq!(scan.corrupt.len(), 1);
        assert!(scan.corrupt[0].contains("broken"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_manifest_missing_id_falls_back_to_dir_name() {
        let root = std::env::temp_dir().join(format!("blmgr-noid-{}", std::process::id()));
        let pkg = root.join("blender_org").join("unnamed_pkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("blender_manifest.toml"),
            "name = \"Unnamed\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let manifest = read_manifest(&pkg).unwrap();
        assert_eq!(manifest.id, "unnamed_pkg", "缺 id 时用目录名兜底");

        std::fs::remove_dir_all(&root).ok();
    }
}

/// 真机验证：扫描本机实际的便携扩展目录。
///
/// 手动运行：`cargo test --lib live_scan -- --ignored --nocapture`
///
/// 单测里的合成目录证明不了「解析规则对真实 manifest 有效」——
/// 真实 manifest 的字段组合、缺省项、大小写都来自上游，只能拿实例验。
#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    #[ignore = "需要本机装有 Blender 与其扩展目录"]
    fn live_scan_real_extensions() {
        let root = Path::new("D:/Blender/stable/blender-5.3.0-windows-x64/portable/extensions");
        if !root.is_dir() {
            return;
        }
        let scan = scan(root);
        println!("extensions = {}", scan.extensions.len());
        println!("legacy = {}", scan.legacy.len());
        println!("corrupt = {:?}", scan.corrupt);

        assert!(!scan.extensions.is_empty(), "本机应已装扩展");
        assert!(
            scan.extensions.iter().all(|item| !item.pkg_id.is_empty()),
            "每个条目都应有 pkg_id"
        );
        assert!(
            scan.extensions
                .iter()
                .all(|item| item.repo != ".blender_ext"),
            "不能把仓库元数据目录当成扩展"
        );
        // 实测已知存在，用来确认 repo 解析没串位。
        assert!(
            scan.extensions
                .iter()
                .any(|item| item.repo == "blender_org" && item.pkg_id == "CityGenerator"),
            "应扫到 blender_org/CityGenerator"
        );
    }
}

/// 真机验证：读本机仓库索引，并与盘上已装版本对照。
///
/// 手动运行：`cargo test --lib live_index -- --ignored --nocapture`
#[cfg(test)]
mod live_index_tests {
    use super::*;

    #[test]
    #[ignore = "需要本机装有 Blender 与其扩展目录"]
    fn live_read_repo_index_and_compare() {
        let root = Path::new("D:/Blender/stable/blender-5.3.0-windows-x64/portable/extensions");
        if !root.is_dir() {
            return;
        }
        let catalog = read_all_repo_indexes(root);
        println!("catalog entries = {}", catalog.len());
        assert!(catalog.len() > 100, "本机索引应有上百条");

        // 每条目录项都应能定位下载地址，否则「安装」按钮是死的。
        let with_url = catalog
            .iter()
            .filter(|item| !item.archive_url.is_empty())
            .count();
        println!("with archive_url = {with_url}");
        assert_eq!(with_url, catalog.len(), "每条目录项都应有下载地址");

        // 与盘上已装版本对照，验证「可更新」判断不会把同版本误报为更新。
        let scan = scan(root);
        let mut compared = 0;
        let mut updatable = 0;
        for installed in &scan.extensions {
            let Some(entry) = catalog
                .iter()
                .find(|item| item.id.eq_ignore_ascii_case(&installed.pkg_id))
            else {
                continue;
            };
            compared += 1;
            if let (Ok(latest), Ok(current)) = (
                semver::Version::parse(entry.version.trim()),
                semver::Version::parse(installed.manifest.version.trim()),
            ) {
                if latest > current {
                    updatable += 1;
                    println!("update: {} {} -> {}", installed.pkg_id, current, latest);
                }
            }
        }
        println!("compared = {compared}, updatable = {updatable}");
        assert!(compared > 0, "至少应有一个已装扩展能在目录里找到");
    }
}
