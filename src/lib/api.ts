import { invoke } from '@tauri-apps/api/core';
import type {
	Build,
	BuildFilter,
	BuildInfoInput,
	CatalogItem,
	CleanupResult,
	DownloadTask,
	BuildDetails,
	ExtensionScan,
	InstalledExtension,
	LaunchedInstance,
	LibraryStats,
	RecentProject,
	Settings
} from './types';

/**
 * Tauri IPC 封装层（docs/01 §3）。
 * Tauri 2 的 invoke 来自 @tauri-apps/api/core（v1 的 .../tauri 已移除）。
 * Rust 侧 snake_case 参数名自动映射为 camelCase。
 */

export async function getBuilds(filter: BuildFilter | null = null): Promise<Build[]> {
	return invoke<Build[]>('get_builds', { filter });
}

export async function refreshBuilds(): Promise<void> {
	return invoke('refresh_builds');
}

/** 上次检查时间（RFC3339），从未检查过为 null。 */
export async function getLastCheck(): Promise<string | null> {
	return invoke<string | null>('get_last_check');
}

export async function getSettings(): Promise<Settings> {
	return invoke<Settings>('get_settings');
}

export async function updateSettings(settings: Settings): Promise<void> {
	return invoke('update_settings', { newSettings: settings });
}

export async function startDownload(buildId: string): Promise<string> {
	return invoke<string>('start_download', { buildId });
}

export async function pauseDownload(taskId: string): Promise<void> {
	return invoke('pause_download', { taskId });
}

export async function resumeDownload(taskId: string): Promise<void> {
	return invoke('resume_download', { taskId });
}

export async function cancelDownload(taskId: string): Promise<void> {
	return invoke('cancel_download', { taskId });
}

export async function getActiveDownloads(): Promise<DownloadTask[]> {
	return invoke<DownloadTask[]>('get_active_downloads');
}

export async function launchBlender(
	buildId: string,
	blendFile?: string,
	args?: string[],
	env?: Record<string, string>,
	openLast = false
): Promise<string> {
	return invoke<string>('launch_blender', { buildId, blendFile, args, env, openLast });
}

export async function getRunningInstances(): Promise<LaunchedInstance[]> {
	return invoke<LaunchedInstance[]>('get_running_instances');
}

export async function killInstance(instanceId: string): Promise<void> {
	return invoke('kill_instance', { instanceId });
}

export async function scanLibrary(): Promise<void> {
	return invoke('scan_library');
}

export async function removeBuild(
	buildId: string,
	deleteUserData = false,
	toTrash = true
): Promise<void> {
	return invoke('remove_build', { buildId, deleteUserData, toTrash });
}

export async function getLibraryStats(): Promise<LibraryStats> {
	return invoke<LibraryStats>('get_library_stats');
}

export async function cleanupLibrary(): Promise<CleanupResult> {
	return invoke<CleanupResult>('cleanup_library');
}

export async function getBuildDetails(buildId: string): Promise<BuildDetails> {
	return invoke<BuildDetails>('get_build_details', { buildId });
}

export async function searchBuilds(query: string, limit = 20): Promise<Build[]> {
	return invoke<Build[]>('search_builds', { query, limit });
}

export async function getRecentProjects(limit = 20): Promise<RecentProject[]> {
	return invoke<RecentProject[]>('get_recent_projects', { limit });
}

export async function forgetRecentProject(blendFile: string): Promise<void> {
	return invoke('forget_recent_project', { blendFile });
}

export async function getFavorites(): Promise<string[]> {
	return invoke<string[]>('get_favorites');
}

export async function setFavorite(buildId: string, favorite: boolean): Promise<void> {
	return invoke('set_favorite', { buildId, favorite });
}

export async function openInExplorer(path: string): Promise<void> {
	return invoke('open_in_explorer', { path });
}

// ---------------------------------------------------------------- 构建条目

export async function listExecutables(path: string): Promise<string[]> {
	return invoke<string[]>('list_executables', { path });
}

export async function detectBuildInfo(
	path: string,
	executable?: string
): Promise<BuildInfoInput> {
	return invoke<BuildInfoInput>('detect_build_info', { path, executable });
}

export async function saveCustomBuild(input: BuildInfoInput): Promise<Build> {
	return invoke<Build>('save_custom_build', { input });
}

export async function moveBuildToCategory(buildId: string, branch: string): Promise<void> {
	return invoke('move_build_to_category', { buildId, branch });
}

export async function createShortcut(buildId: string, destination: string): Promise<void> {
	return invoke('create_shortcut', { buildId, destination });
}

export async function createSymlink(buildId: string): Promise<string> {
	return invoke<string>('create_symlink', { buildId });
}

export async function togglePortable(buildId: string): Promise<boolean> {
	return invoke<boolean>('toggle_portable', { buildId });
}

export async function isPortable(buildId: string): Promise<boolean> {
	return invoke<boolean>('is_portable', { buildId });
}

export async function registerFileAssociation(): Promise<void> {
	return invoke('register_file_association');
}

export async function installTemplate(buildId: string): Promise<string> {
	return invoke<string>('install_template', { buildId });
}

/** `.blinfo` 里的真实 build hash；无则 null。 */
export async function getBuildHash(buildId: string): Promise<string | null> {
	return invoke<string | null>('get_build_hash', { buildId });
}

// ---------------------------------------------------------------- 托盘 / 快捷启动

export async function getQuickLaunch(): Promise<Build[]> {
	return invoke<Build[]>('get_quick_launch');
}

export async function toggleQuickLaunch(buildId: string): Promise<boolean> {
	return invoke<boolean>('toggle_quick_launch', { buildId });
}

export async function quickLaunch(): Promise<void> {
	return invoke('quick_launch');
}

export async function setFirstRunDone(): Promise<void> {
	return invoke('set_first_run_done');
}

export async function pickDirectory(): Promise<string | null> {
	return invoke<string | null>('pick_directory');
}

// ---------------------------------------------------------------- Windows 登记

/** 登记到「应用和功能」。不写 WinGetPackageIdentifier，winget upgrade 不接管。 */
export async function registerWithWinget(): Promise<void> {
	return invoke('register_with_winget');
}

export async function isWingetRegistered(): Promise<boolean> {
	return invoke<boolean>('is_winget_registered');
}

// ---------------------------------------------------------- 插件与扩展（docs/06）

export async function scanExtensions(buildId: string): Promise<ExtensionScan> {
	return invoke<ExtensionScan>('scan_extensions', { buildId });
}

/**
 * 探测启用状态。会起一个 Blender 子进程（约 5 秒，最长 20 秒），
 * 所以只在用户明确点「检测」时调用，不在进页面时自动触发。
 */
export async function probeExtensions(buildId: string): Promise<ExtensionScan> {
	return invoke<ExtensionScan>('probe_extensions', { buildId });
}

/**
 * 读扩展目录。本地 `index.json` 命中时不联网；`refresh` 为真则跳过本地直接拉线上。
 */
export async function getExtensionCatalog(
	buildId: string,
	refresh = false
): Promise<CatalogItem[]> {
	return invoke<CatalogItem[]>('get_extension_catalog', { buildId, refresh });
}

export async function installExtension(
	buildId: string,
	pkgId: string,
	enable: boolean
): Promise<InstalledExtension> {
	return invoke<InstalledExtension>('install_extension', { buildId, pkgId, enable });
}

export async function installExtensionFile(
	buildId: string,
	path: string,
	enable: boolean
): Promise<InstalledExtension> {
	return invoke<InstalledExtension>('install_extension_file', { buildId, path, enable });
}

export async function setExtensionEnabled(
	buildId: string,
	repo: string,
	pkgId: string,
	enabled: boolean
): Promise<void> {
	return invoke('set_extension_enabled', { buildId, repo, pkgId, enabled });
}

export async function removeExtension(
	buildId: string,
	repo: string,
	pkgId: string
): Promise<void> {
	return invoke('remove_extension', { buildId, repo, pkgId });
}

export async function openExtensionPage(url: string): Promise<void> {
	return invoke('open_extension_page', { url });
}
