import { writable, type Writable } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import * as api from './api';
import type { Build, DownloadTask, LaunchedInstance, RecentProject, Settings } from './types';

/**
 * 全局状态 + 后端事件接线（docs/01 §4 事件表）。
 * 用轻量 writable stores 而非 TanStack Query：缓存/TTL 已在后端实现，
 * 前端只做镜像 + 事件驱动更新。
 */

export const builds: Writable<Build[]> = writable([]);
export const buildsLoading: Writable<boolean> = writable(false);
export const downloads: Writable<DownloadTask[]> = writable([]);
export const instances: Writable<LaunchedInstance[]> = writable([]);
export const settings: Writable<Settings | null> = writable(null);
export const recentProjects: Writable<RecentProject[]> = writable([]);
export const favoriteIds: Writable<Set<string>> = writable(new Set());
export const quickLaunchIds: Writable<Set<string>> = writable(new Set());
export const appError: Writable<string | null> = writable(null);
export const buildsRefreshedAt: Writable<Date | null> = writable(null);
/** 托盘等外部来源请求切换主 Tab，页面消费后置空。 */
export const requestedTab: Writable<string | null> = writable(null);

let initialized = false;
const unlisteners: UnlistenFn[] = [];

/** 拉取 builds（可带过滤），TTL 缓存在后端。 */
export async function reloadBuilds(filter = null): Promise<void> {
	buildsLoading.set(true);
	try {
		builds.set(await api.getBuilds(filter));
		appError.set(null);
	} catch (err) {
		appError.set(String(err));
	} finally {
		buildsLoading.set(false);
	}
}

/** 强制重爬（顶栏/状态栏 "Check"）。 */
export async function startRefresh(): Promise<void> {
	buildsLoading.set(true);
	try {
		await api.refreshBuilds();
		await reloadBuilds();
		buildsRefreshedAt.set(new Date());
	} catch (err) {
		appError.set(String(err));
	} finally {
		buildsLoading.set(false);
	}
}

/** 重新扫描本地库目录，把已安装状态合并进来。 */
export async function checkLibrary(): Promise<void> {
	buildsLoading.set(true);
	try {
		await api.scanLibrary();
		await reloadBuilds();
	} catch (err) {
		appError.set(String(err));
	} finally {
		buildsLoading.set(false);
	}
}

export async function reloadDownloads(): Promise<void> {
	try {
		downloads.set(await api.getActiveDownloads());
	} catch (err) {
		appError.set(String(err));
	}
}

export async function reloadInstances(): Promise<void> {
	try {
		instances.set(await api.getRunningInstances());
	} catch (err) {
		appError.set(String(err));
	}
}

export async function reloadRecentProjects(): Promise<void> {
	try {
		recentProjects.set(await api.getRecentProjects());
	} catch (err) {
		appError.set(String(err));
	}
}

export async function reloadFavorites(): Promise<void> {
	try {
		favoriteIds.set(new Set(await api.getFavorites()));
	} catch (err) {
		appError.set(String(err));
	}
}

export async function toggleFavorite(buildId: string): Promise<void> {
	let next = false;
	favoriteIds.update((current) => {
		next = !current.has(buildId);
		return current;
	});
	await api.setFavorite(buildId, next);
	favoriteIds.update((current) => {
		const copy = new Set(current);
		if (next) copy.add(buildId);
		else copy.delete(buildId);
		return copy;
	});
}

export async function reloadSettings(): Promise<void> {
	try {
		settings.set(await api.getSettings());
	} catch (err) {
		appError.set(String(err));
	}
}

/** 快捷启动列表（托盘菜单同源），只取 id 供右键菜单判断。 */
export async function reloadQuickLaunch(): Promise<void> {
	try {
		const builds = await api.getQuickLaunch();
		quickLaunchIds.set(new Set(builds.map((build) => build.id)));
	} catch (err) {
		appError.set(String(err));
	}
}

/** 应用启动时调用一次：拉初始数据 + 注册后端事件监听。 */
export async function initStores(): Promise<void> {
	if (initialized) return;
	initialized = true;

	// 启动时先扫本地库（原版行为：启动即读取本地构建），再拉远程缓存。
	try {
		await api.scanLibrary();
	} catch (err) {
		appError.set(String(err));
	}

	await Promise.all([
		reloadBuilds(),
		reloadDownloads(),
		reloadInstances(),
		reloadRecentProjects(),
		reloadFavorites(),
		reloadQuickLaunch(),
		reloadSettings(),
		// 上次检查时间来自后端（落盘在 builds_last_refresh），重启不丢。
		api
			.getLastCheck()
			.then((at) => buildsRefreshedAt.set(at ? new Date(at) : null))
			.catch(() => undefined)
	]);

	const progressUnlisten = await listen<DownloadTask>('download_progress', (event) => {
		const task = event.payload;
		downloads.update((list) => {
			const index = list.findIndex((item) => item.id === task.id);
			if (index === -1) return [...list, task];
			const next = [...list];
			next[index] = task;
			return next;
		});
	});

	const buildsUpdatedUnlisten = await listen('builds_updated', () => {
		void reloadBuilds();
	});

	const libraryScannedUnlisten = await listen('library_scanned', () => {
		void reloadBuilds();
	});

	const instanceStartedUnlisten = await listen<LaunchedInstance>('instance_started', (event) => {
		instances.update((list) => [...list, event.payload]);
		void reloadRecentProjects();
	});

	const instanceExitedUnlisten = await listen<{ instanceId: string; exitCode: number | null }>(
		'instance_exited',
		(event) => {
			instances.update((list) => list.filter((item) => item.id !== event.payload.instanceId));
		}
	);

	// 下载完成 = 磁盘上多了个解压好的构建，必须重扫才能进库页。
	// 后端解压到 <library>/<类别>/ 后并不改 is_installed（那是扫描的职责）。
	const downloadCompletedUnlisten = await listen<{ buildId: string }>(
		'download_completed',
		() => {
			void checkLibrary();
		}
	);

	// 失败只刷新任务列表，别把一条失败消息变成整库重扫。
	const downloadFailedUnlisten = await listen<{ buildId: string; error: string }>(
		'download_failed',
		(event) => {
			appError.set(`Download failed: ${event.payload.error}`);
			void reloadDownloads();
		}
	);

	// settings_changed 由设置页/向导/托盘触发，集中在这里同步，
	// 避免每个调用点各自 reloadSettings。
	const settingsChangedUnlisten = await listen('settings_changed', () => {
		void reloadSettings();
	});

	// 托盘 "Favorites" 项：切到收藏页。
	const showFavoritesUnlisten = await listen('show_favorites', () => {
		requestedTab.set('favorites');
		void reloadFavorites();
	});

	unlisteners.push(
		progressUnlisten,
		buildsUpdatedUnlisten,
		libraryScannedUnlisten,
		instanceStartedUnlisten,
		instanceExitedUnlisten,
		downloadCompletedUnlisten,
		downloadFailedUnlisten,
		settingsChangedUnlisten,
		showFavoritesUnlisten
	);
}

/** 下载中（未到终态）的任务，按 build_id 查找。 */
export function activeTaskForBuild(list: DownloadTask[], buildId: string): DownloadTask | undefined {
	return list.find(
		(task) =>
			task.build_id === buildId &&
			!['completed', 'failed', 'cancelled'].includes(task.state)
	);
}
