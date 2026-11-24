use crate::error::AppError;
use crate::models::{Architecture, Build, BuildBranch, BuildFork, Platform};
use crate::Result;
use chrono::{DateTime, NaiveDateTime, Utc};
use soup::{prelude::*, Soup};
use std::cmp::Reverse;
use std::collections::HashSet;
use std::time::Duration;

/// builder.blender.org listing pages (server-rendered `<li class="t-row build">` rows).
const BUILDER_BASE: &str = "https://builder.blender.org/download";
/// Official stable releases, one Apache directory listing per major version.
const RELEASE_BASE: &str = "https://download.blender.org/release";
/// 非官方 fork 的 GitHub Releases API（2026-11 实测可用，无需 token；
/// 无认证限额 60 次/小时，仅拉最近几个 release）。
const BFORARTISTS_RELEASES_URL: &str =
    "https://api.github.com/repos/Bforartists/Bforartists/releases";
const UPBGE_RELEASES_URL: &str = "https://api.github.com/repos/UPBGE/upbge/releases";
const USER_AGENT: &str = "BlenderLauncher/3.0";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How many most-recent stable major versions (e.g. `Blender5.3/`) to scrape.
const STABLE_VERSIONS_TO_SCRAPE: usize = 3;
/// 每个 fork 拉取的最近 release 数。
const FORK_RELEASES_TO_SCRAPE: usize = 3;
/// Installer/archive extensions accepted as downloadable builds.
/// `.sha256`, `.msix` and `.tar.bz2` are deliberately skipped.
const BUILD_EXTENSIONS: [&str; 5] = ["zip", "msi", "dmg", "tar.xz", "tar.gz"];

/// Scrapes Blender build listings from blender.org.
///
/// Verified data sources (2026-11):
/// - Stable:       `https://download.blender.org/release/BlenderX.Y/`
/// - Daily:        `https://builder.blender.org/download/daily/`
/// - Experimental: `https://builder.blender.org/download/experimental/`
/// - Patch:        `https://builder.blender.org/download/patch/`
///
/// Note: `https://builder.blender.org/download/branches/` 301-redirects to
/// `/daily/`, so experimental builds are scraped from `/experimental/`.
/// That page only lists the latest build per branch; each branch has an
/// `all` subpage for its full history (not scraped yet).
pub struct Scraper {
    client: reqwest::Client,
}

impl Scraper {
    /// 无代理（测试/离线场景用）。
    pub fn new() -> Self {
        Self::with_proxy(&crate::models::ProxySettings::default())
    }

    /// 带代理配置构造。客户端构造失败时回退到无代理，避免
    /// 一个填错的代理让整个爬取链路直接不可用。
    pub fn with_proxy(proxy: &crate::models::ProxySettings) -> Self {
        let client = crate::service::http::build_client(proxy, USER_AGENT, REQUEST_TIMEOUT)
            .unwrap_or_else(|err| {
                tracing::warn!(error = %err, "proxy client failed, falling back to direct connection");
                reqwest::Client::builder()
                    .user_agent(USER_AGENT)
                    .timeout(REQUEST_TIMEOUT)
                    .build()
                    .expect("failed to build fallback HTTP client")
            });
        Self { client }
    }

    /// Scrapes all official sources concurrently (docs/01 §6.3: one
    /// independent task per source). A failing source is logged and skipped
    /// instead of failing the whole refresh.
    pub async fn scrape_all(&self) -> Result<Vec<Build>> {
        let (stable, daily, experimental, patch) = tokio::join!(
            self.fetch_builds(BuildFork::Official, BuildBranch::Stable),
            self.fetch_builds(BuildFork::Official, BuildBranch::Daily),
            self.fetch_builds(BuildFork::Official, BuildBranch::Experimental),
            self.fetch_builds(BuildFork::Official, BuildBranch::Patch),
        );

        let mut builds = Vec::new();
        for source in [stable, daily, experimental, patch] {
            match source {
                Ok(list) => builds.extend(list),
                Err(err) => tracing::warn!(error = %err, "build source scrape failed"),
            }
        }

        Ok(dedup_by_id(builds))
    }

    /// Scrapes one (fork, branch) combination.
    pub async fn fetch_builds(&self, fork: BuildFork, branch: BuildBranch) -> Result<Vec<Build>> {
        match fork {
            BuildFork::Official => match branch {
                BuildBranch::Stable => self.fetch_stable().await,
                BuildBranch::Daily => self.fetch_builder_page("daily", BuildBranch::Daily).await,
                BuildBranch::Experimental => {
                    self.fetch_builder_page("experimental", BuildBranch::Experimental)
                        .await
                }
                BuildBranch::Patch => self.fetch_builder_page("patch", BuildBranch::Patch).await,
            },
            // fork 源只有稳定发布/每周构建，branch 由源决定，调用方的值被忽略。
            BuildFork::Bforartists => {
                self.fetch_fork_releases(
                    BFORARTISTS_RELEASES_URL,
                    BuildFork::Bforartists,
                    BuildBranch::Stable,
                )
                .await
            }
            BuildFork::Upbge => {
                self.fetch_fork_releases(UPBGE_RELEASES_URL, BuildFork::Upbge, BuildBranch::Daily)
                    .await
            }
        }
    }

    /// GitHub Releases API 拉取 fork 构建列表（Bforartists/Upbge）。
    /// 资产过滤到可解压格式（zip/tar.xz/tar.gz/dmg），安装器（exe/deb/
    /// AppImage）跳过。
    async fn fetch_fork_releases(
        &self,
        api_url: &str,
        fork: BuildFork,
        branch: BuildBranch,
    ) -> Result<Vec<Build>> {
        let url = format!("{api_url}?per_page={FORK_RELEASES_TO_SCRAPE}");
        let text = self.fetch_text(&url).await?;
        let releases: Vec<GitHubRelease> =
            serde_json::from_str(&text).map_err(AppError::SerializationError)?;

        let mut builds = Vec::new();
        for release in releases {
            for asset in &release.assets {
                let Some((version, platform, architecture)) = parse_fork_asset(&asset.name, fork)
                else {
                    tracing::debug!(asset = %asset.name, ?fork, "asset skipped");
                    continue;
                };
                builds.push(Build {
                    id: build_id(&asset.browser_download_url, platform),
                    version,
                    branch,
                    fork,
                    platform,
                    architecture,
                    url: asset.browser_download_url.clone(),
                    size_bytes: asset.size,
                    release_date: release
                        .published_at
                        .as_deref()
                        .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
                        .map(|time| time.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now),
                    release_notes_url: release.html_url.clone(),
                    breaking_changes: None,
                    is_installed: false,
                    install_path: None,
                            update_build_id: None,
                });
            }
        }
        tracing::info!(?fork, count = builds.len(), "fork releases scraped");
        Ok(builds)
    }

    /// 拉取 release notes（docs/01 §3.1 `get_build_details`）。
    ///
    /// 稳定版指向官方文档 `https://developer.blender.org/docs/release_notes/
    /// <major.minor>/`（2026-11 实测 200）。Daily/experimental/patch 没有
    /// 公开 release notes（builder 页无此链接，commit 页被 Cloudflare 403），
    /// 返回 None，由前端回退到 commit 链接。
    pub async fn fetch_release_notes(&self, build: &Build) -> Result<Option<String>> {
        let Some(url) = release_notes_url(build) else {
            return Ok(None);
        };
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .and_then(|response| response.error_for_status());
        match response {
            Ok(response) => Ok(Some(response.text().await.unwrap_or_default())),
            Err(err) => {
                tracing::warn!(url = %url, error = %err, "release notes fetch failed");
                Ok(None)
            }
        }
    }

    /// Stable builds live in per-major-version directories, e.g.
    /// `https://download.blender.org/release/Blender4.5/`. The index is read
    /// first, then the most recent directories (sequential: only ~3 requests).
    async fn fetch_stable(&self) -> Result<Vec<Build>> {
        let index = self.fetch_text(&format!("{RELEASE_BASE}/")).await?;
        let dirs = latest_release_dirs(&index, STABLE_VERSIONS_TO_SCRAPE);
        tracing::info!(?dirs, "scraping stable release directories");

        let mut builds = Vec::new();
        for dir in &dirs {
            let base_url = format!("{RELEASE_BASE}/{}", dir.trim_end_matches('/'));
            let html = self.fetch_text(&format!("{base_url}/")).await?;
            builds.extend(parse_release_dir(&html, &base_url));
        }
        Ok(builds)
    }

    async fn fetch_builder_page(&self, segment: &str, branch: BuildBranch) -> Result<Vec<Build>> {
        let url = format!("{BUILDER_BASE}/{segment}/");
        let html = self.fetch_text(&url).await?;
        let builds = parse_builder_page(&html, branch, BuildFork::Official);
        tracing::info!(url = %url, count = builds.len(), "builder page scraped");
        Ok(builds)
    }

    async fn fetch_text(&self, url: &str) -> Result<String> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|err| AppError::Network(err.to_string()))?;
        response
            .text()
            .await
            .map_err(|err| AppError::Network(err.to_string()))
    }

    /// Platform of the machine the launcher runs on (default download filter).
    pub fn detect_platform() -> Platform {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Macos
        } else {
            Platform::Linux
        }
    }

    /// CPU architecture of the machine the launcher runs on.
    pub fn detect_architecture() -> Architecture {
        if cfg!(target_arch = "x86_64") {
            Architecture::X64
        } else {
            Architecture::Arm64
        }
    }
}

impl Default for Scraper {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses a builder.blender.org listing page (`daily`, `experimental`, `patch`).
///
/// Each build is an `<li class="t-row build is-windows is-arch-amd64">` row
/// containing a `b-reference` commit link, a `b-date` div whose `title` is
/// ISO-8601, and a download anchor with a `Download (<size><unit>)` title.
/// Rows whose download URL is not an accepted installer (e.g. `.msix`,
/// `.sha256`) are skipped.
fn parse_builder_page(html: &str, branch: BuildBranch, fork: BuildFork) -> Vec<Build> {
    let soup = Soup::new(html);
    let mut builds = Vec::new();

    for row in soup.tag("li").find_all() {
        let Some(class) = row.get("class") else {
            continue;
        };
        if !(class.contains("t-row") && class.contains("build")) {
            continue;
        }
        let Some(platform) = platform_from_row_class(&class) else {
            tracing::warn!(class = %class, "unknown platform class, row skipped");
            continue;
        };

        // soup queries only run from a document root, so re-parse the row's
        // serialized HTML to inspect its anchors and cells.
        let row_soup = Soup::new(&row.display());
        let mut download_url = None;
        let mut size_bytes = 0;
        let mut release_notes_url = None;
        let mut release_date = None;

        for anchor in row_soup.tag("a").find_all() {
            let class = anchor.get("class").unwrap_or_default();
            let href = anchor.get("href").unwrap_or_default();
            let title = anchor.get("title").unwrap_or_default();

            if class.contains("b-reference") {
                // The commit page is the closest thing to release notes for
                // a daily/experimental/patch build.
                release_notes_url = Some(href.clone());
            }
            if title.starts_with("Download") {
                size_bytes = parse_size_title(&title);
                download_url = Some(href);
            }
        }

        // Fallback when the download anchor lost its title: any anchor that
        // is not the SHA link and points at an accepted installer.
        let url = match download_url {
            Some(url) if is_build_file(&url) => url,
            _ => row_soup
                .tag("a")
                .find_all()
                .find_map(|anchor| {
                    let class = anchor.get("class").unwrap_or_default();
                    let href = anchor.get("href").unwrap_or_default();
                    (!class.contains("b-sha") && is_build_file(&href)).then_some(href)
                })
                .unwrap_or_default(),
        };
        if url.is_empty() {
            continue;
        }

        release_date = release_date.or_else(|| {
            row_soup
                .tag("div")
                .find_all()
                .find(|cell| cell.get("class").unwrap_or_default().contains("b-date"))
                .and_then(|cell| cell.get("title"))
                .and_then(|title| DateTime::parse_from_rfc3339(&title).ok())
                .map(|date| date.with_timezone(&Utc))
        });

        let Some((version, architecture)) = parse_builder_filename(&url) else {
            tracing::warn!(url = %url, "unrecognized build filename, row skipped");
            continue;
        };
        let Some(release_date) = release_date else {
            tracing::warn!(url = %url, "row has no parsable date, skipped");
            continue;
        };

        builds.push(Build {
            id: build_id(&url, platform),
            version,
            branch,
            fork,
            platform,
            architecture,
            url,
            size_bytes,
            release_date,
            release_notes_url,
            breaking_changes: None,
            is_installed: false,
            install_path: None,
            update_build_id: None,
        });
    }

    builds
}

/// Parses one Apache directory listing page
/// (`download.blender.org/release/BlenderX.Y/`). Entries look like:
///
/// ```text
/// <a href="blender-4.5.0-linux-x64.tar.xz">blender-4.5.0-linux-x64.tar.xz</a>     15-Jul-2025 08:40     376170648
/// ```
fn parse_release_dir(html: &str, base_url: &str) -> Vec<Build> {
    let mut builds = Vec::new();

    for line in html.lines() {
        let Some(start) = line.find("<a href=\"blender-") else {
            continue;
        };
        let after_start = &line[start + "<a href=\"".len()..];
        let Some(end) = after_start.find('"') else {
            continue;
        };
        let filename = &after_start[..end];
        let Some((version, platform, architecture)) = parse_release_filename(filename) else {
            continue;
        };

        // Metadata columns follow the closing anchor: "<date> <time> <size>".
        let Some(link_end) = after_start[end..].find("</a>") else {
            continue;
        };
        let mut columns = after_start[end + link_end + "</a>".len()..].split_whitespace();
        let (Some(date), Some(time), Some(size)) = (columns.next(), columns.next(), columns.next())
        else {
            continue;
        };
        let Ok(release_date) =
            NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%d-%b-%Y %H:%M")
        else {
            continue;
        };
        let Ok(size_bytes) = size.parse::<u64>() else {
            continue;
        };

        let url = format!("{base_url}/{filename}");
        builds.push(Build {
            id: build_id(&url, platform),
            version,
            branch: BuildBranch::Stable,
            fork: BuildFork::Official,
            platform,
            architecture,
            url,
            size_bytes,
            release_date: release_date.and_utc(),
            release_notes_url: None,
            breaking_changes: None,
            is_installed: false,
            install_path: None,
            update_build_id: None,
        });
    }

    builds
}

/// Picks the `limit` most recent `Blender<MAJOR>.<MINOR>/` names from the
/// release index listing, newest first.
fn latest_release_dirs(html: &str, limit: usize) -> Vec<String> {
    const PREFIX: &str = "<a href=\"Blender";
    let mut dirs: Vec<(u32, u32, String)> = Vec::new();
    let mut rest = html;

    while let Some(pos) = rest.find(PREFIX) {
        rest = &rest[pos + "<a href=\"".len()..];
        let Some(end) = rest.find('"') else {
            break;
        };
        let name = &rest[..end];
        let Some(version) = name
            .strip_prefix("Blender")
            .and_then(|version| version.strip_suffix('/'))
        else {
            continue;
        };
        let Some((major, minor)) = version
            .split_once('.')
            .and_then(|(major, minor)| major.parse::<u32>().ok().zip(minor.parse::<u32>().ok()))
        else {
            continue;
        };
        dirs.push((major, minor, name.to_string()));
    }

    dirs.sort_by_key(|(major, minor, _)| (Reverse(*major), Reverse(*minor)));
    dirs.into_iter()
        .take(limit)
        .map(|(_, _, name)| name)
        .collect()
}

/// `blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release.zip`
/// → `("5.4.0-alpha+main.1205aee3644f", X64)`.
///
/// The version keeps its variant suffix (`-alpha+main.<hash>`,
/// `-stable+main-PR<id>.<hash>`), which makes it unique per commit.
fn parse_builder_filename(url: &str) -> Option<(String, Architecture)> {
    let filename = url.rsplit('/').next()?;
    if !is_build_file(filename) {
        return None;
    }
    let (version, _platform, architecture) = parse_builder_stem(filename)?;
    Some((version, architecture))
}

/// Builder-style filename stem without extension, e.g.
/// `blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release`
/// → `("5.4.0-alpha+main.1205aee3644f", Windows, X64)`. Used by the
/// library scanner to identify extracted build directories.
pub fn parse_builder_stem(stem: &str) -> Option<(String, Platform, Architecture)> {
    let rest = stem.strip_prefix("blender-")?;

    const MARKERS: [(&str, Platform); 3] = [
        ("-windows.", Platform::Windows),
        ("-linux.", Platform::Linux),
        ("-darwin.", Platform::Macos),
    ];
    for (marker, platform) in MARKERS {
        let Some(index) = rest.find(marker) else {
            continue;
        };
        let version = &rest[..index];
        if version.is_empty() {
            return None;
        }
        let tail = &rest[index + marker.len()..];
        let arch_token = tail.split(['-', '.']).next()?;
        return Some((
            version.to_string(),
            platform,
            architecture_from_token(arch_token)?,
        ));
    }
    None
}

/// `blender-4.5.0-windows-x64.msi` → `("4.5.0", Windows, X64)`.
fn parse_release_filename(filename: &str) -> Option<(String, Platform, Architecture)> {
    if !is_build_file(filename) {
        return None;
    }
    parse_release_stem(filename)
}

/// Release-style filename stem without extension, e.g.
/// `blender-4.5.0-windows-x64` → `("4.5.0", Windows, X64)`.
pub fn parse_release_stem(stem: &str) -> Option<(String, Platform, Architecture)> {
    let rest = stem.strip_prefix("blender-")?;

    const MARKERS: [(&str, Platform); 3] = [
        ("-windows-", Platform::Windows),
        ("-macos-", Platform::Macos),
        ("-linux-", Platform::Linux),
    ];
    for (marker, platform) in MARKERS {
        let Some(index) = rest.find(marker) else {
            continue;
        };
        let version = &rest[..index];
        if version.is_empty() {
            return None;
        }
        let tail = &rest[index + marker.len()..];
        let arch_token = tail.split('.').next()?;
        return Some((
            version.to_string(),
            platform,
            architecture_from_token(arch_token)?,
        ));
    }
    None
}

fn architecture_from_token(token: &str) -> Option<Architecture> {
    match token {
        "x64" | "amd64" | "x86_64" => Some(Architecture::X64),
        "arm64" => Some(Architecture::Arm64),
        _ => None,
    }
}

/// 版本号 → 官方 release notes 地址。
///
/// 仅稳定版有公开文档：`4.5.17` → `.../release_notes/4.5/`。
/// 带 `-alpha+main.<hash>` / `-stable+main-PR...` 变体后缀的构建
/// （daily/experimental/patch）返回 None——它们没有公开 release notes。
pub(crate) fn release_notes_url(build: &Build) -> Option<String> {
    if build.branch != BuildBranch::Stable || build.fork != BuildFork::Official {
        return None;
    }
    let mut parts = build.version.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    // 变体版本（含 `-`）不是正式发布，跳过。
    if minor.contains('-') {
        return None;
    }
    Some(format!(
        "https://developer.blender.org/docs/release_notes/{major}.{minor}/"
    ))
}

/// GitHub Releases API 响应子集（只声明用到的字段）。
#[derive(serde::Deserialize)]
struct GitHubRelease {
    html_url: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(serde::Deserialize)]
struct GitHubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// GitHub fork 资产名 → (version, platform, architecture)。
fn parse_fork_asset(name: &str, fork: BuildFork) -> Option<(String, Platform, Architecture)> {
    match fork {
        BuildFork::Bforartists => parse_bforartists_asset(name),
        BuildFork::Upbge => parse_upbge_asset(name),
        BuildFork::Official => None,
    }
}

/// `Bforartists-5.2.3-Windows.zip` → ("5.2.3", Windows, X64)；
/// `-Linux.tar.xz` → Linux X64；`-MacSilicon.zip` / `-Mac-Silicon.dmg` →
/// Macos Arm64。安装器（exe/deb/AppImage）与 releasenote.txt 不匹配 → None。
fn parse_bforartists_asset(name: &str) -> Option<(String, Platform, Architecture)> {
    let stem = archive_stem(name)?;
    let rest = stem.strip_prefix("Bforartists-")?;
    if let Some(version) = rest.strip_suffix("-MacSilicon") {
        return non_empty(version)
            .map(|version| (version.to_string(), Platform::Macos, Architecture::Arm64));
    }
    const MARKERS: [(&str, Platform, Architecture); 3] = [
        ("-Windows", Platform::Windows, Architecture::X64),
        ("-Linux", Platform::Linux, Architecture::X64),
        ("-Mac-Silicon", Platform::Macos, Architecture::Arm64),
    ];
    for (marker, platform, architecture) in MARKERS {
        if let Some(version) = rest.strip_suffix(marker) {
            return non_empty(version).map(|version| (version.to_string(), platform, architecture));
        }
    }
    None
}

/// `upbge-0.53-alpha-windows-x86_64-2026-11-15.zip` →
/// ("0.53-alpha-2026-11-15", Windows, X64)。日期并入版本串以区分每周构建。
fn parse_upbge_asset(name: &str) -> Option<(String, Platform, Architecture)> {
    let stem = archive_stem(name)?;
    let rest = stem.strip_prefix("upbge-")?;
    const MARKERS: [(&str, Platform); 2] = [
        ("-windows-", Platform::Windows),
        ("-linux-", Platform::Linux),
    ];
    for (marker, platform) in MARKERS {
        if let Some(index) = rest.find(marker) {
            let version_prefix = &rest[..index];
            let tail = &rest[index + marker.len()..];
            let (arch_token, date) = tail.split_once('-')?;
            let architecture = architecture_from_token(arch_token)?;
            let version = if date.is_empty() {
                version_prefix.to_string()
            } else {
                format!("{version_prefix}-{date}")
            };
            if version.is_empty() {
                return None;
            }
            return Some((version, platform, architecture));
        }
    }
    None
}

fn non_empty(value: &str) -> Option<&str> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn platform_from_row_class(class: &str) -> Option<Platform> {
    if class.contains("is-windows") {
        Some(Platform::Windows)
    } else if class.contains("is-darwin") {
        Some(Platform::Macos)
    } else if class.contains("is-linux") {
        Some(Platform::Linux)
    } else {
        None
    }
}

/// `Download (406.50MB)` → bytes; unparsable input → 0.
fn parse_size_title(title: &str) -> u64 {
    let Some(inner) = title.split('(').nth(1) else {
        return 0;
    };
    let inner = inner.trim_end_matches(')');
    let Some(split) = inner.find(|c: char| !c.is_ascii_digit() && c != '.') else {
        return 0;
    };
    let Ok(value) = inner[..split].parse::<f64>() else {
        return 0;
    };
    let multiplier: f64 = match &inner[split..] {
        "B" => 1.0,
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        "TB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return 0,
    };
    (value * multiplier) as u64
}

/// True when the URL points at an accepted installer/archive.
fn is_build_file(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    BUILD_EXTENSIONS.iter().any(|ext| path.ends_with(ext))
}

/// `blender-4.5.0-windows-x64.zip` → `blender-4.5.0-windows-x64`；非安装包 → None。
/// 提取后的顶层目录名等于该 stem，library 扫描用它匹配远程条目。
pub fn archive_stem(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let filename = path.rsplit('/').next()?;
    // 双段扩展名必须最先匹配，否则会被误当普通文件名漏掉。
    const EXTENSIONS: [&str; 5] = ["tar.xz", "tar.gz", "zip", "msi", "dmg"];
    for ext in EXTENSIONS {
        if let Some(base) = filename.strip_suffix(&format!(".{ext}")) {
            return Some(base.to_string());
        }
    }
    None
}

/// Stable build identity per docs/01 §2.1: hash of url + platform.
///
/// FNV-1a (public domain) is used instead of `DefaultHasher` because the
/// latter's algorithm is not guaranteed across Rust versions and ids are
/// persisted in SQLite. The URL is unique per artifact (version × variant ×
/// arch × installer), so the pair is collision-free for our purposes.
pub fn build_id(url: &str, platform: Platform) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in format!("{url}|{:?}", platform).bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn dedup_by_id(builds: Vec<Build>) -> Vec<Build> {
    let mut seen = HashSet::new();
    let mut unique = Vec::with_capacity(builds.len());
    for build in builds {
        if seen.insert(build.id.clone()) {
            unique.push(build);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors real rows captured from builder.blender.org/download/daily/.
    const BUILDER_PAGE_HTML: &str = r#"
    <ul>
    <li class="t-row build is-windows is-arch-amd64">
    <a href="https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release.zip" class="t-cell b-version">Blender 5.4.0</a>
    <a class="t-cell b-reference" href="https://projects.blender.org/blender/blender/commit/1205aee3644f" title="See commit">1205aee3644f</a>
    <div class="t-cell b-date" title="2026-11-22T05:57:09+00:00">22 Nov 05:57</div>
    <div class="t-cell b-arch build-architecture"><span class="build-platform">windows</span> x64</div>
    <div class="t-cell b-down"><a href="https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release.zip" title="Download (406.50MB)">Download</a></div>
    </li>
    <li class="t-row build is-linux is-arch-x86_64">
    <a href="https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-linux.x86_64-release.tar.xz" class="t-cell b-version">Blender 5.4.0</a>
    <div class="t-cell b-date" title="2026-11-22T01:41:51+00:00">22 Nov 01:41</div>
    <div class="t-cell b-down"><a href="https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-linux.x86_64-release.tar.xz" title="Download (380.10MB)">Download</a></div>
    </li>
    <li class="t-row build is-windows is-arch-amd64">
    <a href="https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release.msix.sha256" class="t-cell b-version">Blender 5.4.0</a>
    <div class="t-cell b-date" title="2026-11-22T05:57:09+00:00">22 Nov 05:57</div>
    <div class="t-cell b-down"><a href="https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release.msix.sha256" title="Download (64.00B)">Download</a></div>
    </li>
    </ul>
    "#;

    #[test]
    fn test_parse_builder_page() {
        let builds = parse_builder_page(BUILDER_PAGE_HTML, BuildBranch::Daily, BuildFork::Official);
        assert_eq!(builds.len(), 2, "msix.sha256 row must be skipped");

        let windows = &builds[0];
        assert_eq!(windows.version, "5.4.0-alpha+main.1205aee3644f");
        assert_eq!(windows.platform, Platform::Windows);
        assert_eq!(windows.architecture, Architecture::X64);
        assert_eq!(windows.branch, BuildBranch::Daily);
        assert_eq!(windows.fork, BuildFork::Official);
        assert_eq!(windows.size_bytes, 426_246_144); // 406.50 * 1024^2
        assert_eq!(windows.url, "https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release.zip");
        assert_eq!(
            windows.release_notes_url.as_deref(),
            Some("https://projects.blender.org/blender/blender/commit/1205aee3644f")
        );
        assert_eq!(
            windows.release_date.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "2026-11-22T05:57:09"
        );
        assert!(!windows.is_installed);
        assert!(windows.install_path.is_none());
        assert_eq!(windows.id.len(), 16);

        let linux = &builds[1];
        assert_eq!(linux.platform, Platform::Linux);
        assert_eq!(linux.architecture, Architecture::X64);
        assert!(linux.size_bytes > 380 * 1024 * 1024 && linux.size_bytes < 381 * 1024 * 1024);
    }

    #[test]
    fn test_parse_builder_page_fallback_without_download_title() {
        // Row whose download anchor has no title attribute: the b-version
        // anchor (same archive URL) is used as fallback.
        let html = r#"
        <li class="t-row build is-darwin is-arch-arm64">
        <a href="https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-darwin.arm64-release.dmg" class="t-cell b-version">Blender 5.4.0</a>
        <div class="t-cell b-date" title="2026-11-22T03:00:00+00:00">22 Nov 03:00</div>
        <div class="t-cell b-down"><a href="https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-darwin.arm64-release.dmg">Download</a></div>
        </li>
        "#;
        let builds = parse_builder_page(html, BuildBranch::Daily, BuildFork::Official);
        assert_eq!(builds.len(), 1);
        assert_eq!(builds[0].platform, Platform::Macos);
        assert_eq!(builds[0].architecture, Architecture::Arm64);
        assert!(builds[0].url.ends_with("-darwin.arm64-release.dmg"));
        assert_eq!(builds[0].size_bytes, 0);
    }

    #[test]
    fn test_parse_builder_filename() {
        let (version, arch) = parse_builder_filename(
            "https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release.zip",
        ).unwrap();
        assert_eq!(version, "5.4.0-alpha+main.1205aee3644f");
        assert_eq!(arch, Architecture::X64);

        // Patch builds embed the PR number; the dash inside must survive.
        let (version, arch) = parse_builder_filename(
            "https://cdn.builder.blender.org/download/patch/blender-4.5.12-stable+main-PR162828.197a85c74202-windows.amd64-release.zip",
        ).unwrap();
        assert_eq!(version, "4.5.12-stable+main-PR162828.197a85c74202");
        assert_eq!(arch, Architecture::X64);

        assert!(
            parse_builder_filename("https://example.com/blender-5.4.0-release.msix.sha256")
                .is_none()
        );
        assert!(parse_builder_filename(
            "https://cdn.builder.blender.org/download/daily/blender-5.4.0-alpha+main.1205aee3644f-windows.amd64-release.msix"
        )
        .is_none(), "msix must be rejected by extension");
        assert!(parse_builder_filename("https://example.com/notablender.zip").is_none());
    }

    /// Mirrors real rows from download.blender.org/release/Blender4.5/.
    const RELEASE_DIR_HTML: &str = "\
<a href=\"blender-4.5.0-windows-x64.msi\">blender-4.5.0-windows-x64.msi</a>          15-Jul-2025 08:40      355679488
<a href=\"blender-4.5.0-linux-x64.tar.xz\">blender-4.5.0-linux-x64.tar.xz</a>        15-Jul-2025 08:40      376170648
<a href=\"blender-4.5.0-windows-arm64.msix\">blender-4.5.0-windows-arm64.msix</a>    15-Jul-2025 08:40      360000000
<a href=\"blender-4.5.0.sha256\">blender-4.5.0.sha256</a>                            15-Jul-2025 08:40             321
<a href=\"blender-4.2.9-linux-x64.tar.bz2\">blender-4.2.9-linux-x64.tar.bz2</a>      12-Nov-2024 10:02      300000000
";

    #[test]
    fn test_parse_release_dir() {
        let builds = parse_release_dir(
            RELEASE_DIR_HTML,
            "https://download.blender.org/release/Blender4.5",
        );
        assert_eq!(builds.len(), 2, "msix, sha256 and tar.bz2 must be skipped");

        let msi = &builds[0];
        assert_eq!(msi.version, "4.5.0");
        assert_eq!(msi.branch, BuildBranch::Stable);
        assert_eq!(msi.platform, Platform::Windows);
        assert_eq!(msi.architecture, Architecture::X64);
        assert_eq!(msi.size_bytes, 355_679_488);
        assert_eq!(
            msi.release_date.format("%Y-%m-%dT%H:%M").to_string(),
            "2025-07-15T08:40"
        );
        assert_eq!(
            msi.url,
            "https://download.blender.org/release/Blender4.5/blender-4.5.0-windows-x64.msi"
        );

        let tar = &builds[1];
        assert_eq!(tar.platform, Platform::Linux);
        assert_eq!(tar.architecture, Architecture::X64);
        assert_eq!(tar.size_bytes, 376_170_648);
    }

    #[test]
    fn test_latest_release_dirs() {
        let index = "\
<a href=\"Blender2.93/\">Blender2.93/</a>
<a href=\"Blender4.2/\">Blender4.2/</a>
<a href=\"Blender4.5/\">Blender4.5/</a>
<a href=\"Blender5.3/\">Blender5.3/</a>
<a href=\"BlenderMAC/\">BlenderMAC/</a>
<a href=\"../\">../</a>
";
        assert_eq!(
            latest_release_dirs(index, 3),
            vec!["Blender5.3/", "Blender4.5/", "Blender4.2/"]
        );
        assert_eq!(
            latest_release_dirs(index, 10).len(),
            4,
            "non-version dirs must be ignored"
        );
    }

    #[test]
    fn test_parse_size_title() {
        assert_eq!(parse_size_title("Download (406.50MB)"), 426_246_144);
        assert_eq!(parse_size_title("Download (64.00B)"), 64);
        assert_eq!(parse_size_title("Download (1.50GB)"), 1_610_612_736);
        assert_eq!(parse_size_title("Download (2.00KB)"), 2_048);
        assert_eq!(parse_size_title("Download (n/a)"), 0);
        assert_eq!(parse_size_title("no size here"), 0);
    }

    #[test]
    fn test_build_id_stable_and_distinct() {
        let url = "https://cdn.builder.blender.org/download/daily/blender-5.4.0.zip";
        let a = build_id(url, Platform::Windows);
        let b = build_id(url, Platform::Windows);
        let c = build_id(url, Platform::Linux);
        let d = build_id(
            "https://cdn.builder.blender.org/download/daily/blender-5.4.1.zip",
            Platform::Windows,
        );
        assert_eq!(a, b, "same url + platform must map to the same id");
        assert_ne!(a, c, "platform must influence the id");
        assert_ne!(a, d, "url must influence the id");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// 2026-11 实测的 GitHub fork 资产名（docs/01 §2.4 fork 源）。
    #[test]
    fn test_parse_fork_assets() {
        // Bforartists（assets 见 api.github.com/repos/Bforartists/Bforartists/releases）
        let (version, platform, arch) =
            parse_fork_asset("Bforartists-5.2.3-Windows.zip", BuildFork::Bforartists).unwrap();
        assert_eq!(version, "5.2.3");
        assert_eq!(platform, Platform::Windows);
        assert_eq!(arch, Architecture::X64);

        let (version, platform, arch) =
            parse_fork_asset("Bforartists-5.2.3-Linux.tar.xz", BuildFork::Bforartists).unwrap();
        assert_eq!(version, "5.2.3");
        assert_eq!(platform, Platform::Linux);

        let (version, platform, arch) =
            parse_fork_asset("Bforartists-5.2.3-MacSilicon.zip", BuildFork::Bforartists).unwrap();
        assert_eq!(version, "5.2.3");
        assert_eq!(platform, Platform::Macos);
        assert_eq!(arch, Architecture::Arm64);

        let (version, platform, arch) =
            parse_fork_asset("Bforartists-5.2.3-Mac-Silicon.dmg", BuildFork::Bforartists).unwrap();
        assert_eq!(version, "5.2.3");
        assert_eq!(platform, Platform::Macos);

        // 安装器 / 文档不解析。
        assert!(parse_fork_asset("Install_Bforartists5_523.exe", BuildFork::Bforartists).is_none());
        assert!(parse_fork_asset("bforartists_5.2.3.deb", BuildFork::Bforartists).is_none());
        assert!(
            parse_fork_asset("Bforartists-5.2.3-x86_64.AppImage", BuildFork::Bforartists).is_none()
        );
        assert!(
            parse_fork_asset("releasenote_bforartists5_523.txt", BuildFork::Bforartists).is_none()
        );

        // Upbge（weekly builds，日期并入版本串）。
        let (version, platform, arch) = parse_fork_asset(
            "upbge-0.53-alpha-windows-x86_64-2026-11-15.zip",
            BuildFork::Upbge,
        )
        .unwrap();
        assert_eq!(version, "0.53-alpha-2026-11-15");
        assert_eq!(platform, Platform::Windows);
        assert_eq!(arch, Architecture::X64);

        let (version, platform, arch) = parse_fork_asset(
            "upbge-0.53-alpha-linux-x86_64-2026-11-15.tar.gz",
            BuildFork::Upbge,
        )
        .unwrap();
        assert_eq!(version, "0.53-alpha-2026-11-15");
        assert_eq!(platform, Platform::Linux);
        assert_eq!(arch, Architecture::X64);
    }

    #[test]
    fn test_is_build_file() {
        assert!(is_build_file(
            "https://cdn.example.com/blender-5.0.0-windows.amd64-release.zip"
        ));
        assert!(is_build_file(
            "https://cdn.example.com/blender-4.5.0-linux-x64.tar.xz"
        ));
        assert!(is_build_file(
            "https://cdn.example.com/blender-4.5.0-windows-x64.msi?query=1"
        ));
        assert!(!is_build_file(
            "https://cdn.example.com/blender-4.5.0.sha256"
        ));
        assert!(!is_build_file(
            "https://cdn.example.com/blender-4.5.0-windows-x64.msix"
        ));
        assert!(!is_build_file(
            "https://cdn.example.com/blender-4.2.9-linux-x64.tar.bz2"
        ));
    }

    /// Live smoke test against builder.blender.org; excluded from default
    /// `cargo test` so CI stays offline-deterministic.
    #[tokio::test]
    #[ignore = "hits the network; run explicitly with `cargo test -- --ignored`"]
    async fn scrape_daily_live() {
        let scraper = Scraper::new();
        let builds = scraper
            .fetch_builds(BuildFork::Official, BuildBranch::Daily)
            .await
            .unwrap();
        assert!(!builds.is_empty());
        assert!(builds
            .iter()
            .all(|build| !build.url.is_empty() && build.size_bytes > 0 && !build.id.is_empty()));
    }

    /// Live smoke test against the GitHub Releases fork source (docs/01 §2.4).
    #[tokio::test]
    #[ignore = "hits the network; run explicitly with `cargo test -- --ignored`"]
    async fn scrape_bforartists_live() {
        let scraper = Scraper::new();
        let builds = scraper
            .fetch_builds(BuildFork::Bforartists, BuildBranch::Stable)
            .await
            .unwrap();
        assert!(!builds.is_empty());
        assert!(builds
            .iter()
            .all(|build| build.fork == BuildFork::Bforartists && !build.url.is_empty()));
    }
}
