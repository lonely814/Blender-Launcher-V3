use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub install_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub max_concurrent_downloads: u8,
    pub auto_check_updates: bool,
    pub update_check_interval_hours: u32,
    pub register_blend_files: bool,
    pub show_tray_icon: bool,
    pub launch_on_startup: bool,
    pub enable_high_dpi: bool,
    /// Default CLI args appended to every blender launch (docs/01 §2.4).
    #[serde(default)]
    pub launch_args: Vec<String>,
    /// 爬取非官方 fork（docs/01 §2.4 scrape_bforartists / scrape_upbge）。
    #[serde(default)]
    pub scrape_bforartists: bool,
    #[serde(default)]
    pub scrape_upbge: bool,
    pub theme: Theme,
    /// 快捷启动列表（构建的 install_path 字符串，原版 quick_launch_paths）。
    #[serde(default)]
    pub quick_launch_paths: Vec<String>,
    /// 首启动向导是否已完成。
    #[serde(default)]
    pub first_run_done: bool,
    /// 全局快捷启动热键（原版 quick_launch_key_seq，默认 Alt+F11）。
    #[serde(default = "default_hotkey")]
    pub quick_launch_key_seq: String,
    /// 显示 Update 按钮（原版 show_update_button）。
    #[serde(default = "default_true")]
    pub show_update_button: bool,
    /// 启动后最小化到托盘（原版 launch_minimized_to_tray）。
    #[serde(default)]
    pub launch_minimized_to_tray: bool,
    /// 新版本通知（原版 enable_new_builds_notifications）。
    #[serde(default = "default_true")]
    pub enable_new_builds_notifications: bool,
    /// 界面布局：classic = 左侧竖排页签（原版），wide = 顶部页签 + 紧凑行，
    /// detail = wide + 右侧详情面板。
    #[serde(default)]
    pub layout: Layout,
    /// 强调色板。只换 --accent，不动 success/danger/info 这些语义色。
    #[serde(default)]
    pub accent: Accent,
    /// 网络代理（爬取与下载共用同一次构造）。
    #[serde(default)]
    pub proxy: ProxySettings,
    /// 界面语言；"auto" 跟随系统区域。
    #[serde(default = "default_language")]
    pub language: String,
    /// 是否已登记到 Windows「应用和功能」（不代表 winget 可升级）。
    #[serde(default)]
    pub auto_register_winget: bool,
    /// Windows 11 云母（Mica）背景材质。不支持的平台自动忽略。
    #[serde(default = "default_true")]
    pub enable_mica: bool,
    /// 插件中央库（docs/06 §7.2）。默认关闭：接管会改写用户的扩展目录，
    /// 必须由用户显式开启一次，不能因为升级就悄悄打开。
    #[serde(default)]
    pub enable_extension_central: bool,
    /// 中央库根目录。`None` = `<data_root>/extensions`，随便携模式走。
    #[serde(default)]
    pub extension_central_dir: Option<PathBuf>,
    /// 下载加速镜像。`None` = 直连官方。
    ///
    /// 只影响下载地址，不影响爬取：列表仍从官方站读，否则镜像的目录结构
    /// 一变就识别不出有哪些构建可下。
    #[serde(default)]
    pub download_mirror: Option<DownloadMirror>,
}

/// 支持替换下载源的镜像站。
///
/// 当前只有清华 TUNA，且它只镜像 `release/`（stable 正式版）。
/// daily / experimental / patch 在 builder CDN 上，镜像站没有，
/// 选中镜像时这类构建自动回退官方地址——否则用户会得到 404。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DownloadMirror {
    Tuna,
}

impl DownloadMirror {
    /// 把官方下载地址改写成镜像地址；不适用时返回 `None`。
    ///
    /// `https://download.blender.org/release/Blender4.5/blender-4.5.0-windows-x64.zip`
    /// → `https://mirrors.tuna.tsinghua.edu.cn/blender/release/Blender4.5/blender-4.5.0-windows-x64.zip`
    pub fn rewrite(self, url: &str) -> Option<String> {
        const OFFICIAL: &str = "https://download.blender.org/release/";
        match self {
            Self::Tuna => {
                let rest = url.strip_prefix(OFFICIAL)?;
                Some(format!(
                    "https://mirrors.tuna.tsinghua.edu.cn/blender/release/{rest}"
                ))
            }
        }
    }
}

fn default_language() -> String {
    "auto".to_string()
}

/// 代理配置。只支持 HTTP/HTTPS：SOCKS 需要 reqwest 的 `socks` feature，
/// 本实现按「不引入新依赖」的约束不做。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub kind: ProxyKind,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub username: String,
    /// 明文存 settings.json —— 见 README 的安全说明。
    #[serde(default)]
    pub password: String,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: ProxyKind::Http,
            host: String::new(),
            port: String::new(),
            username: String::new(),
            password: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    #[default]
    Http,
    Https,
}

/// 主界面布局。只影响外壳与行密度，数据与交互完全共用。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Classic,
    Wide,
    /// Wide + 右侧 280px 详情面板（三段式）。
    Detail,
}

/// 强调色板。浅色主题下的取值一律压暗一档保证对比度。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Orange,
    Blue,
    Green,
    Purple,
    Rose,
}

fn default_true() -> bool {
    true
}

fn default_hotkey() -> String {
    "Alt+F11".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
    System,
}

impl Default for Settings {
    fn default() -> Self {
        // 路径统一走 service::settings 的解析函数：便携模式判定
        // （exe 旁有 portable 标记）只在那里实现一次，避免这里的默认值
        // 与 settings_file_path 走不同分支、把数据劈成两半。
        Self {
            install_dir: crate::service::settings::default_library_root(),
            cache_dir: crate::service::settings::cache_root(),
            max_concurrent_downloads: 2,
            auto_check_updates: true,
            update_check_interval_hours: 24,
            register_blend_files: false,
            show_tray_icon: true,
            launch_on_startup: false,
            enable_high_dpi: true,
            launch_args: Vec::new(),
            scrape_bforartists: false,
            scrape_upbge: false,
            theme: Theme::Dark,
            quick_launch_paths: Vec::new(),
            first_run_done: false,
            quick_launch_key_seq: default_hotkey(),
            show_update_button: true,
            launch_minimized_to_tray: false,
            enable_new_builds_notifications: true,
            layout: Layout::Classic,
            accent: Accent::Orange,
            proxy: ProxySettings::default(),
            language: default_language(),
            auto_register_winget: false,
            enable_mica: true,
            enable_extension_central: false,
            extension_central_dir: None,
            download_mirror: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mirror_rewrites_only_release_urls() {
        use DownloadMirror::Tuna;
        assert_eq!(
            Tuna.rewrite("https://download.blender.org/release/Blender4.5/blender-4.5.0-windows-x64.zip")
                .as_deref(),
            Some(
                "https://mirrors.tuna.tsinghua.edu.cn/blender/release/Blender4.5/blender-4.5.0-windows-x64.zip"
            )
        );
        // builder CDN 上的构建（daily/experimental/patch）镜像站没有。
        assert_eq!(
            Tuna.rewrite("https://cdn.builder.blender.org/download/daily/blender-5.4.0-windows.amd64-release.zip"),
            None
        );
        // 非 Blender 官方域名一律不改写。
        assert_eq!(
            Tuna.rewrite("https://github.com/polygoniq/extensions/raw/master/a.zip"),
            None
        );
    }

    #[test]
    fn test_mirror_setting_defaults_to_none_and_survives_missing_field() {
        // 老版本 settings.json 没有这个字段，必须能读成 None 而不是报错。
        let parsed: DownloadMirror = serde_json::from_str("\"tuna\"").unwrap();
        assert_eq!(parsed, DownloadMirror::Tuna);
        let settings: Settings = serde_json::from_str(
            r#"{"install_dir":"C:/Blender","cache_dir":"C:/cache","max_concurrent_downloads":2,
                "auto_check_updates":true,"update_check_interval_hours":24,
                "register_blend_files":false,"show_tray_icon":true,"launch_on_startup":false,
                "enable_high_dpi":true,"theme":"dark"}"#,
        )
        .unwrap();
        assert_eq!(settings.download_mirror, None);
    }
}
