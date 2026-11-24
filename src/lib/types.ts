export type BuildBranch = 'stable' | 'daily' | 'experimental' | 'patch';
export type BuildFork = 'official' | 'bforartists' | 'upbge';
export type Platform = 'windows' | 'macos' | 'linux';
export type Architecture = 'x64' | 'arm64';

export interface Build {
	id: string;
	version: string;
	branch: BuildBranch;
	fork: BuildFork;
	platform: Platform;
	architecture: Architecture;
	url: string;
	size_bytes: number;
	release_date: string;
	release_notes_url?: string;
	breaking_changes?: string[];
	is_installed: boolean;
	install_path?: string;
	/** 有新版本时指向那个版本的 id；Launch 按钮变成 Update。 */
	update_build_id?: string | null;
}

export interface BuildFilter {
	branch?: BuildBranch[];
	fork?: BuildFork[];
	installed_only?: boolean;
}

export interface DownloadTask {
	id: string;
	build_id: string;
	state: DownloadState;
	progress_bytes: number;
	total_bytes: number;
	speed_bps: number;
	eta_seconds: number;
	started_at: string;
	finished_at?: string;
	error?: string;
}

export type DownloadState =
	| 'queued'
	| 'downloading'
	| 'extracting'
	| 'verifying'
	| 'completed'
	| 'failed'
	| 'cancelled'
	| 'paused';

export interface LaunchedInstance {
	id: string;
	build_id: string;
	blend_file?: string;
	started_at: string;
	memory_mb: number;
	cpu_percent: number;
}

export interface LibraryStats {
	total_builds: number;
	total_size_bytes: number;
	by_branch: Partial<Record<BuildBranch, number>>;
	by_fork: Partial<Record<BuildFork, number>>;
	orphaned_builds: string[];
	corrupt_dirs: string[];
}

export interface CleanupResult {
	deleted_files: number;
	freed_bytes: number;
}

export interface Settings {
	install_dir: string;
	cache_dir: string;
	max_concurrent_downloads: number;
	auto_check_updates: boolean;
	update_check_interval_hours: number;
	register_blend_files: boolean;
	show_tray_icon: boolean;
	launch_on_startup: boolean;
	enable_high_dpi: boolean;
	launch_args: string[];
	scrape_bforartists: boolean;
	scrape_upbge: boolean;
	theme: 'dark' | 'light' | 'system';
	quick_launch_paths: string[];
	first_run_done: boolean;
	quick_launch_key_seq: string;
	show_update_button: boolean;
	launch_minimized_to_tray: boolean;
	enable_new_builds_notifications: boolean;
	/** 'classic' = 左侧竖排页签（原版）, 'wide' = 顶部页签 + 紧凑行, 'detail' = wide + 详情面板 */
	layout: 'classic' | 'wide' | 'detail';
	/** 强调色板；只换 accent，不影响 success/danger/info。 */
	accent: 'orange' | 'blue' | 'green' | 'purple' | 'rose';
	proxy: ProxySettings;
	/** 'auto' 或具体语言码（en/zh/ja/fr/es）。 */
	language: string;
	/** 是否已登记到 Windows「应用和功能」。 */
	auto_register_winget: boolean;
	/** Windows 11 Mica 背景材质。 */
	enable_mica: boolean;
	/** 插件中央库（阶段 B）。默认关闭：接管会改写扩展目录结构。 */
	enable_extension_central: boolean;
	/** 中央库根目录。`null` = `<数据目录>/extensions`。 */
	extension_central_dir: string | null;
	/** 下载加速镜像。`null` = 直连官方。 */
	download_mirror: DownloadMirror | null;
}

/** 支持的下载镜像站。 */
export type DownloadMirror = 'tuna';

/** 只支持 HTTP/HTTPS：SOCKS 需要 reqwest 的 socks feature，本实现不引入。 */
export interface ProxySettings {
	enabled: boolean;
	kind: 'http' | 'https';
	host: string;
	port: string;
	username: string;
	/** 明文存 settings.json。 */
	password: string;
}

/** 自定义构建对话框载荷（对应原版 BuildInfo）。 */
export interface BuildInfoInput {
	path: string;
	executable: string;
	subversion: string;
	build_hash?: string | null;
	commit_time?: string | null;
	branch?: string | null;
	custom_name: string;
}

export interface BuildDetails extends Build {
	release_notes_html?: string;
}

export interface RecentProject {
	blend_file: string;
	build_id: string;
	last_opened: number;
}

/** 主窗口页签（对应原版 Library / Downloads / Favorites）。 */
export type Tab = 'library' | 'downloads' | 'favorites' | 'extensions';

/** 表格视图模式，决定日期列标题。 */
export type TableMode = Tab;

/** 行操作按钮的语义。 */
export type RowAction = 'launch' | 'download' | 'running' | 'update';

export interface BuildRow {
	build: Build;
	action: RowAction;
	/** 运行中的实例数（仅 launch/running 行可能 > 0） */
	instances: number;
}

export interface MenuItem {
	label?: string;
	action?: () => void;
	separator?: boolean;
	disabled?: boolean;
	/** 子菜单（原版 Debug 子菜单） */
	children?: MenuItem[];
}

// ------------------------------------------------------------ 插件与扩展（docs/06）

/** 插件类型。`kind` 是上游 `blender_manifest.toml` 的 `type` 字段原值。 */
export type ExtensionKind = 'add-on' | 'theme' | 'asset-library';

/** 已安装的扩展（读 `blender_manifest.toml`）。 */
export interface InstalledExtension {
	pkg_id: string;
	repo: string;
	path: string;
	name: string;
	version: string;
	kind: string;
	/** `null` = 还没探测过，界面必须显示「未知」而不是「已禁用」。 */
	enabled: boolean | null;
	/** 目录带 `.blmgr.json`，归本程序管理（方案 B）。 */
	managed: boolean;
}

/** 旧式插件（`scripts/addons/*.py`，读 `bl_info`）。 */
export interface LegacyAddon {
	module: string;
	path: string;
	name: string;
	version: string | null;
	category: string | null;
	description: string | null;
	enabled: boolean | null;
}

/** 一次扫描的完整结果。 */
export interface ExtensionScan {
	extensions: InstalledExtension[];
	legacy: LegacyAddon[];
	/** 有目录但 manifest 读不出来。 */
	corrupt: string[];
	ext_root: string | null;
	/** 上次探测启用状态的时间（ISO 串）。`null` 表示从未探测过。 */
	enabled_checked_at: string | null;
}

/** 阶段 B 的管理结果。 */
export interface ExtensionManage {
	pkg_id: string;
	repo: string;
	managed: boolean;
	/** 中央库中当前版本的实际路径。 */
	central_path: string | null;
	/** 中央库里该扩展的全部版本，降序。 */
	versions: string[];
}

/** 官方站版本历史页里的一个历史版本（阶段 B2）。 */
export interface ExtensionVersion {
	version: string;
	archive_url: string;
	/** `sha256:<hex>`，下载后据此校验。 */
	archive_hash: string | null;
}

/** 官方目录条目（L2 本地 index.json / L3 联网）。 */
export interface CatalogItem {
	id: string;
	name: string;
	tagline: string | null;
	version: string;
	kind_raw: string;
	archive_url: string;
	archive_hash: string | null;
	archive_size: number;
	blender_version_min: string | null;
	blender_version_max: string | null;
	platforms: string[] | null;
	website: string | null;
	maintainer: string | null;
	license: string[];
	tags: string[];
}

/** 本程序自身的更新检查结果（`check_app_update`）。 */
export interface AppUpdate {
	current: string;
	latest: string;
	update_available: boolean;
	/** 发布页地址。 */
	html_url: string;
	/** 发布说明（Markdown 原文）。 */
	notes: string;
}
