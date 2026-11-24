use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 扩展类型。对应 `blender_manifest.toml` 的 `type` 字段。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionKind {
    AddOn,
    Theme,
    AssetLibrary,
}

impl ExtensionKind {
    /// 未知类型返回 `None`：仓库可以包含未来才定义的类型，
    /// 直接报错会让整个目录在这一天失效。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "add-on" => Some(Self::AddOn),
            "theme" => Some(Self::Theme),
            "asset-library" => Some(Self::AssetLibrary),
            _ => None,
        }
    }
}

/// `blender_manifest.toml` 里我们关心的字段。
///
/// 全部用 `Option`：manifest 由第三方编写，缺字段是常态而非异常。
/// 缺 `id`/`version` 时调用方回落到目录名，不丢弃整个包。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    /// `#[serde(default)]` 不可省：manifest 由第三方编写，`id`/`name`
    /// 缺失时整个文件会被判为解析失败而丢弃，而调用方本可以用目录名兜底。
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub tagline: Option<String>,
    #[serde(default)]
    pub maintainer: Option<String>,
    /// 原始字符串（`add-on` / `theme` / `asset-library`）。
    /// 保留原文以便把未知类型原样显示给用户。
    ///
    /// TOML 里字段名是 `type`，在 Rust 中是关键字，故重命名。
    #[serde(rename = "type", default)]
    pub kind_raw: String,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub license: Vec<String>,
    #[serde(default)]
    pub copyright: Vec<String>,
    #[serde(default)]
    pub blender_version_min: Option<String>,
    /// 语义是「最高**不**支持的版本」，比较时用 `<` 而非 `<=`。
    #[serde(default)]
    pub blender_version_max: Option<String>,
    /// `None` 表示全平台；`Some([])` 也按全平台处理。
    #[serde(default, deserialize_with = "lenient_opt")]
    pub platforms: Option<Vec<String>>,
    /// 权限名 → 自然语言说明。`network` / `scripts` 需要在 UI 上高亮。
    ///
    /// 规范要求写成表（`[permissions]`），但实测很多第三方包写成数组
    /// （`permissions = ["files", "network"]`，如 Light Wrangler、Geo-Scatter、
    /// Photographer）。用严格类型会让这些包整个被判为「损坏」——
    /// 一个展示用字段不该有这种杀伤力，所以类型不符时降级为 `None`。
    #[serde(default, deserialize_with = "lenient_opt")]
    pub permissions: Option<BTreeMap<String, String>>,
}

/// 反序列化失败时返回 `None` 而不是让整个结构体失败。
///
/// 只用于展示性、非关键字段。`id`/`version` 这类决定条目身份的不采用，
/// 因为「读出来了但这个字段是错的」比「整条读不出」更难排查。
fn lenient_opt<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value = Option::<toml::Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(raw) => Ok(raw.try_into().ok()),
    }
}

fn default_schema_version() -> String {
    "1.0.0".to_string()
}

impl ExtensionManifest {
    pub fn kind(&self) -> Option<ExtensionKind> {
        ExtensionKind::parse(&self.kind_raw)
    }
}

/// 磁盘上实际存在的扩展。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledExtension {
    /// 包目录名。通常等于 `manifest.id`，但不保证——用户可能手改目录名。
    pub pkg_id: String,
    /// 所属仓库目录名，如 `blender_org`、`user_default`、`system`。
    pub repo: String,
    pub path: PathBuf,
    pub manifest: ExtensionManifest,
    /// `None` = 尚未探测（Build 未装、进程超时、探测失败）。
    /// UI 必须把 `None` 显示为未知，不能当作已禁用。
    #[serde(default)]
    pub enabled: Option<bool>,
    /// 本程序接管该目录的标记。`None` 表示不是本程序装的，只读。
    #[serde(default)]
    pub managed: Option<ManagedMetadata>,
    /// 目录字节数。扫描时按需填充，避免每次列目录都全盘统计。
    #[serde(default)]
    pub size_bytes: u64,
}

/// 旧式 addon（`scripts/addons/*.py` 或 `*/__init__.py`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyAddon {
    pub module: String,
    pub path: PathBuf,
    pub name: String,
    pub version: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// 方案 B 的所有权标记，写在 `<pkg_dir>/.blmgr.json`。
///
/// 只有这个文件存在时本程序才会对该目录执行切换版本、移动、删除。
/// 理由见 docs/06 §7.3：Blender 卸载会递归删目录，如果目录是 junction
/// 指向中央库，删除会穿透链接把源内容一起删掉。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedMetadata {
    pub schema_version: u32,
    pub central_path: PathBuf,
    pub pkg_id: String,
    pub managed_since: chrono::DateTime<chrono::Utc>,
}

/// 当前程序对某个标记文件理解的版本。不匹配即视为「不是本程序接管」。
pub const MANAGED_SCHEMA_VERSION: u32 = 1;

/// 标记文件名。
pub const MANAGED_MARKER: &str = ".blmgr.json";

/// 一次扫描的完整结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionScan {
    pub extensions: Vec<InstalledExtension>,
    pub legacy: Vec<LegacyAddon>,
    /// 有目录但 manifest 读不出来。与 library 的 `corrupt_dirs` 同策略：
    /// 报给用户但不阻断其余条目的展示。
    pub corrupt: Vec<String>,
    /// 解析出的扩展根目录。UI 显示在标题里，便于用户确认看的是哪个 Build。
    pub ext_root: Option<PathBuf>,
    /// 启用状态的探测时间。`None` 表示从未探测过，此时 `enabled`
    /// 全为 `None`，UI 应显示「未知」并提供一个探测按钮。
    ///
    /// 故意不设 TTL：探测要起一个 5–20 秒的子进程，每次进页面重探
    /// 是不可接受的。让用户自己决定何时刷新，并把时间显示出来。
    pub enabled_checked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// 目录条目（来自本地 `index.json` 或线上 API）。未安装也有。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tagline: Option<String>,
    #[serde(default)]
    pub version: String,
    /// 上游字段名是 `type`。
    #[serde(rename = "type", default)]
    pub kind_raw: String,
    #[serde(default)]
    pub archive_url: String,
    #[serde(default)]
    pub archive_hash: Option<String>,
    #[serde(default)]
    pub archive_size: u64,
    #[serde(default)]
    pub blender_version_min: Option<String>,
    #[serde(default)]
    pub blender_version_max: Option<String>,
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub maintainer: Option<String>,
    #[serde(default)]
    pub license: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub permissions: Option<BTreeMap<String, String>>,
}

impl CatalogItem {
    pub fn kind(&self) -> Option<ExtensionKind> {
        ExtensionKind::parse(&self.kind_raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extension_kind_parsing() {
        assert_eq!(ExtensionKind::parse("add-on"), Some(ExtensionKind::AddOn));
        assert_eq!(ExtensionKind::parse("theme"), Some(ExtensionKind::Theme));
        assert_eq!(
            ExtensionKind::parse("asset-library"),
            Some(ExtensionKind::AssetLibrary)
        );
        // 未来类型不能让整个目录失效。
        assert_eq!(ExtensionKind::parse("keymap-set"), None);
        assert_eq!(
            ExtensionKind::parse("  add-on  "),
            Some(ExtensionKind::AddOn)
        );
    }

    #[test]
    fn test_permissions_as_array_does_not_reject_entry() {
        // 实测的真实写法：Light Wrangler / Geo-Scatter / Photographer 都把
        // permissions 写成数组。规范是表，但这些包在 Blender 里能正常加载，
        // 所以本程序必须能读，否则会把它们误报成损坏目录。
        let toml = r#"
id = "light_wrangler"
version = "4.2.34"
name = "Light Wrangler"
type = "add-on"
permissions = ["files", "network"]
license = ["SPDX:GPL-3.0-or-later"]
"#;
        let manifest: ExtensionManifest = toml::from_str(toml).unwrap();
        assert_eq!(manifest.id, "light_wrangler");
        assert!(manifest.permissions.is_none(), "类型不符应降级为 None");

        // 表形式仍要正常解析。
        let toml = r#"
id = "dlss_render"
version = "1.0.2"
name = "DLSS Render"
type = "add-on"
[permissions]
files = "Save rendered frames to the output folder"
"#;
        let manifest: ExtensionManifest = toml::from_str(toml).unwrap();
        let permissions = manifest.permissions.unwrap();
        assert_eq!(
            permissions.get("files").map(String::as_str),
            Some("Save rendered frames to the output folder")
        );
    }

    #[test]
    fn test_manifest_allows_missing_optional_fields() {
        // 真实 manifest 只写必填项（取自 CityGenerator）。
        let raw = r#"
schema_version = "1.0.0"
id = "CityGenerator"
version = "2.2.0"
name = "The City Generator2.0"
tagline = "An add-on to generate cityscapes using geometry nodes"
maintainer = "Andreas durr <cinematiccookie@gmail.com>"
type = "add-on"
blender_version_min = "4.2.0"
license = ["Royalty Free"]
"#;
        let manifest: ExtensionManifest = toml::from_str(raw).unwrap();
        assert_eq!(manifest.id, "CityGenerator");
        assert_eq!(manifest.version, "2.2.0");
        assert_eq!(manifest.kind(), Some(ExtensionKind::AddOn));
        assert!(manifest.platforms.is_none());
        assert!(manifest.permissions.is_none());
        assert!(manifest.website.is_none());
    }

    #[test]
    fn test_manifest_parses_permissions_and_platforms() {
        let raw = r#"
schema_version = "1.0.0"
id = "dlss_render"
version = "1.0.2"
name = "DLSS Render"
tagline = "Denoise with DLSS"
maintainer = "Armin5D"
type = "add-on"
blender_version_min = "5.3.0"
license = ["SPDX:GPL-3.0-or-later"]
platforms = ["windows-x64"]
[permissions]
files = "Save rendered frames to the output folder"
"#;
        let manifest: ExtensionManifest = toml::from_str(raw).unwrap();
        let platforms = manifest.platforms.expect("platforms 应被解析");
        assert_eq!(platforms, vec!["windows-x64"]);
        let permissions = manifest.permissions.expect("permissions 应被解析");
        assert_eq!(
            permissions.get("files").map(String::as_str),
            Some("Save rendered frames to the output folder")
        );
    }

    #[test]
    fn test_unknown_type_is_preserved_not_dropped() {
        let raw = r#"
id = "future-thing"
name = "Future"
version = "1.0.0"
type = "keymap-set"
"#;
        let manifest: ExtensionManifest = toml::from_str(raw).unwrap();
        assert_eq!(manifest.kind(), None);
        // 原文保留，UI 可以原样显示。
        assert_eq!(manifest.kind_raw, "keymap-set");
        // 缺失的 schema_version 回落到默认值。
        assert_eq!(manifest.schema_version, "1.0.0");
    }
}
