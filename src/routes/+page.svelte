<script lang="ts">
	import { onMount } from 'svelte';
	import { fade } from 'svelte/transition';
	import { getCurrentWebview } from '@tauri-apps/api/webview';
	import { getVersion } from '@tauri-apps/api/app';
	import Header from '$lib/components/Header.svelte';
	import SideRail from '$lib/components/SideRail.svelte';
	import TopTabs from '$lib/components/TopTabs.svelte';
	import DetailPanel from '$lib/components/DetailPanel.svelte';
	import ResizeHandles from '$lib/components/ResizeHandles.svelte';
	import BuildList from '$lib/components/BuildList.svelte';
	import StatusBar from '$lib/components/StatusBar.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import SettingsForm from '$lib/components/SettingsForm.svelte';
	import DownloadItem from '$lib/components/DownloadItem.svelte';
	import BuildEntryDialog from '$lib/components/BuildEntryDialog.svelte';
	import CatalogDialog from '$lib/components/CatalogDialog.svelte';
	import FirstRunWizard from '$lib/components/FirstRunWizard.svelte';
	import ExtensionList from '$lib/components/ExtensionList.svelte';
	import ExtensionManageDialog from '$lib/components/ExtensionManageDialog.svelte';
	import {
		appError,
		builds,
		checkLibrary,
		downloads,
		favoriteIds,
		initStores,
		instances,
		quickLaunchIds,
		reloadBuilds,
		reloadDownloads,
		reloadInstances,
		reloadQuickLaunch,
		settings,
		toggleFavorite
	} from '$lib/stores';
	import * as api from '$lib/api';
	import { locale, makeTranslator } from '$lib/i18n';
	import type {
		Build,
		BuildRow,
		CatalogItem,
		DownloadTask,
		ExtensionScan,
		ExtensionManage,
		ExtensionVersion,
		InstalledExtension,
		LegacyAddon,
		MenuItem,
		Tab
	} from '$lib/types';

	type SubTab = {
		id: string;
		label: string;
		match: (build: Build) => boolean;
	};

	let tab = $state<Tab>('library');
	let subTabId = $state('stable');
	let settingsOpen = $state(false);
	let search = $state('');
	let menu = $state<{ x: number; y: number; build: Build | null } | null>(null);
	let dropped = $state(false);
	let selected = $state<Set<string>>(new Set());

	let sortKey = $state<'version' | 'branch' | 'date'>('date');
	let sortAsc = $state(false);

	/** 未识别的 custom 子目录 → 弹 Build Entry Creator。 */
	let dialogPath = $state<string | null>(null);
	let dialogInitial = $state<null | Record<string, unknown>>(null);
	let wizardOpen = $state(false);

	/** 应用版本来自 Tauri（tauri.conf.json），不要在界面里硬编码。 */
	let appVersion = $state('');
	onMount(() => {
		void getVersion().then((v) => (appVersion = v));
		void initStores();

		// 原生拖放：Hover 显示提示层，Drop 打开文件。
		let unlisten: (() => void) | undefined;
		void getCurrentWebview()
			.onDragDropEvent((event) => {
				if (event.payload.type === 'over') {
					dropped = true;
				} else if (event.payload.type === 'drop') {
					dropped = false;
					const path = event.payload.paths?.[0];
					if (path) void openDroppedFile(path);
				} else {
					dropped = false;
				}
			})
			.then((fn) => (unlisten = fn));

		return () => unlisten?.();
	});

	$effect(() => {
		if ($settings && !$settings.first_run_done) wizardOpen = true;
	});

	/** wide / detail 布局：子页签移到顶部，行密度收一档。经典布局保持原版结构。 */
	const tr = $derived(makeTranslator($locale));
	// 注意：wide 只含 'wide'。detail 是独立的更宽分支，
	// 若把 detail 也并进 wide，模板里靠前的 wide 分支会把它截走。
	const wide = $derived($settings?.layout === 'wide');
	const detailMode = $derived($settings?.layout === 'detail');

	/** 详情面板的焦点行，与 selected（操作选中）语义分离。 */
	let focusId = $state<string | null>(null);
	let detailOpen = $state(true);
	const focusBuild = $derived($builds.find((b) => b.id === focusId) ?? null);

	// ---------- 插件页（docs/06） ----------

	/**
	 * 插件读取的是「某个已装版本」的扩展目录，所以必须选一个 Build。
	 * 默认取最近安装的一个，用户可以在下拉里改。
	 */
	let extBuildId = $state<string | null>(null);
	let extScan = $state<ExtensionScan | null>(null);
	let extLoading = $state(false);
	let extProbing = $state(false);
	let extFilter = $state('');

	// 扩展目录对话框。
	let extCatalogOpen = $state(false);
	let extCatalog = $state<CatalogItem[]>([]);
	let extCatalogLoading = $state(false);
	let extCatalogError = $state('');
	let extInstalling = $state<string | null>(null);

	const installedBuilds = $derived($builds.filter((build) => build.is_installed));

	// 已装列表变化时（装完新版本、删掉当前选的），把选择收敛到仍存在的一项。
	// 只写 extBuildId，不再读它以外的状态，所以不会自触发。
	$effect(() => {
		const list = installedBuilds;
		if (list.length === 0) {
			if (extBuildId !== null) {
				extBuildId = null;
				extScan = null;
			}
			return;
		}
		if (extBuildId && list.some((build) => build.id === extBuildId)) return;
		// 取最新版本，而不是列表第一项：列表默认按时间倒序，
		// 但用户可能改过排序。
		const newest = [...list].sort((a, b) => b.release_date.localeCompare(a.release_date))[0];
		extBuildId = newest.id;
	});

	// 切换版本后自动扫描一次。扫描是纯读文件，几毫秒，可以自动做；
	// 启用状态探测要起 Blender 进程，必须用户点按钮。
	$effect(() => {
		const id = extBuildId;
		if (!id) return;
		void loadExtensions(id);
	});

	async function loadExtensions(buildId: string): Promise<void> {
		extLoading = true;
		try {
			extScan = await api.scanExtensions(buildId);
		} catch (err) {
			appError.set(String(err));
			extScan = null;
		} finally {
			extLoading = false;
		}
	}

	async function detectExtensions(): Promise<void> {
		const id = extBuildId;
		if (!id || extProbing) return;
		extProbing = true;
		try {
			extScan = await api.probeExtensions(id);
		} catch (err) {
			appError.set(String(err));
		} finally {
			extProbing = false;
		}
	}

	async function toggleExtension(
		item: InstalledExtension | LegacyAddon,
		enabled: boolean
	): Promise<void> {
		const id = extBuildId;
		if (!id) return;
		try {
			// 旧式插件没有 repo，传空串让后端走裸模块名分支。
			const repo = 'repo' in item ? item.repo : '';
			const pkgId = 'pkg_id' in item ? item.pkg_id : item.module;
			await api.setExtensionEnabled(id, repo, pkgId, enabled);

			// 就地更新，不重扫：scan 本身不读启用状态，重扫会把刚写的值抹回 null。
			// 换一个新对象引用，让 ExtensionList 的 $derived 重新求值。
			item.enabled = enabled;
			if (extScan) extScan = { ...extScan };
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function removeExtension(item: InstalledExtension): Promise<void> {
		const id = extBuildId;
		if (!id) return;
		const message = tr('ext.confirmRemove').replace('{name}', item.name || item.pkg_id);
		if (!window.confirm(message)) return;
		try {
			await api.removeExtension(id, item.repo, item.pkg_id);
			await loadExtensions(id);
		} catch (err) {
			appError.set(String(err));
		}
	}

	// 阶段 B：单个扩展的版本管理对话框。
	let manageItem = $state<InstalledExtension | null>(null);
	let manageState = $state<ExtensionManage | null>(null);
	let manageBusy = $state(false);
	let manageError = $state('');
	// 官方站历史版本（B2）。拿不到就留空，对话框里只显示已入库的版本。
	let manageHistory = $state<ExtensionVersion[]>([]);
	let manageHistoryLoading = $state(false);
	let manageHistoryError = $state('');
	let manageAdding = $state<string | null>(null);

	async function openManage(item: InstalledExtension): Promise<void> {
		const id = extBuildId;
		if (!id) return;
		manageItem = item;
		manageError = '';
		manageState = null;
		manageHistory = [];
		manageHistoryError = '';
		try {
			const versions = await api.listExtensionVersions(id, item.pkg_id);
			manageState = {
				pkg_id: item.pkg_id,
				repo: item.repo,
				managed: item.managed,
				central_path: null,
				versions
			};
		} catch (err) {
			manageError = String(err);
		}
		// 历史版本要联网，单独等：失败也不影响接管/切换这些本地操作。
		manageHistoryLoading = true;
		try {
			manageHistory = await api.listExtensionHistory(id, item.repo, item.pkg_id);
		} catch (err) {
			manageHistoryError = String(err);
		} finally {
			manageHistoryLoading = false;
		}
	}

	async function adoptExtension(): Promise<void> {
		const id = extBuildId;
		const item = manageItem;
		if (!id || !item) return;
		manageBusy = true;
		manageError = '';
		try {
			manageState = await api.adoptExtension(id, item.repo, item.pkg_id);
			await loadExtensions(id);
		} catch (err) {
			manageError = String(err);
		} finally {
			manageBusy = false;
		}
	}

	async function switchVersion(version: string): Promise<void> {
		const id = extBuildId;
		const item = manageItem;
		if (!id || !item) return;
		manageBusy = true;
		manageError = '';
		try {
			manageState = await api.switchExtensionVersion(id, item.repo, item.pkg_id, version);
			await loadExtensions(id);
		} catch (err) {
			manageError = String(err);
		} finally {
			manageBusy = false;
		}
	}

	/** 下载官方站的一个历史版本进中央库；已接管时后端会顺带切过去。 */
	async function addVersion(version: string): Promise<void> {
		const id = extBuildId;
		const item = manageItem;
		if (!id || !item || manageAdding) return;
		manageAdding = version;
		manageError = '';
		try {
			// 命令返回的 `versions` 已经包含新下的这一版，不必再查一次。
			manageState = await api.addExtensionVersion(id, item.repo, item.pkg_id, version);
			await loadExtensions(id);
		} catch (err) {
			manageError = String(err);
		} finally {
			manageAdding = null;
		}
	}

	async function releaseExtension(): Promise<void> {
		const id = extBuildId;
		const item = manageItem;
		if (!id || !item) return;
		const message = tr('ext.releaseHint');
		if (!window.confirm(message)) return;
		manageBusy = true;
		manageError = '';
		try {
			await api.releaseExtension(id, item.repo, item.pkg_id);
			manageState = null;
			manageItem = null;
			await loadExtensions(id);
		} catch (err) {
			manageError = String(err);
		} finally {
			manageBusy = false;
		}
	}

	function revealPath(path: string): void {
		void api.openInExplorer(path).catch((err) => appError.set(String(err)));
	}

	/** 打开目录对话框：先显示已有结果，再在后台刷新（首次要联网，可能慢）。 */
	async function openCatalog(): Promise<void> {
		const id = extBuildId;
		if (!id) return;
		extCatalogOpen = true;
		extCatalogLoading = true;
		extCatalogError = '';
		try {
			extCatalog = await api.getExtensionCatalog(id);
		} catch (err) {
			extCatalogError = String(err);
		} finally {
			extCatalogLoading = false;
		}
	}

	async function installFromCatalog(item: CatalogItem): Promise<void> {
		const id = extBuildId;
		if (!id || extInstalling) return;
		extInstalling = item.id;
		try {
			// enable=true：装完就启用。用户点「安装」的意图通常就是马上能用，
			// 之后可以在列表里单独关掉。
			await api.installExtension(id, item.id, true);
			await loadExtensions(id);
		} catch (err) {
			appError.set(String(err));
		} finally {
			extInstalling = null;
		}
	}

	/** 从本地 zip 安装（离线包、作者直接给的构建）。 */
	async function importExtensionFile(): Promise<void> {
		const id = extBuildId;
		if (!id) return;
		const { open } = await import('@tauri-apps/plugin-dialog');
		const picked = await open({
			title: tr('ext.importFile'),
			multiple: false,
			filters: [{ name: 'Extension package', extensions: ['zip'] }]
		});
		if (typeof picked !== 'string') return;
		try {
			await api.installExtensionFile(id, picked, true);
			await loadExtensions(id);
		} catch (err) {
			appError.set(String(err));
		}
	}

	// 焦点行消失（删除/切页/重扫）时回落到当前第一行，避免面板显示已不存在的构建。
	// 条件收敛：写完后 focusId 必然存在于 rows 中，effect 再跑一次就跳过。
	$effect(() => {
		if (!detailMode) return;
		const list = rows;
		if (list.length === 0) return;
		if (list.some((row) => row.build.id === focusId)) return;
		focusId = list[0].build.id;
	});

	/** 原版 Custom 页 = 库 custom/ 下的条目；用 url 前缀判断（id 是哈希）。 */
	const isCustom = (b: Build) => b.url.startsWith('local:');

	const REPO_TABS: SubTab[] = [
		{
			id: 'stable',
			label: 'Stable',
			match: (b) => !isCustom(b) && b.fork === 'official' && b.branch === 'stable'
		},
		{
			id: 'daily',
			label: 'Daily',
			match: (b) => !isCustom(b) && b.fork === 'official' && b.branch === 'daily'
		},
		{
			id: 'experimental',
			label: 'Experimental',
			match: (b) => !isCustom(b) && b.fork === 'official' && (b.branch === 'experimental' || b.branch === 'patch')
		},
		{ id: 'bforartists', label: 'Bforartists', match: (b) => !isCustom(b) && b.fork === 'bforartists' },
		{
			id: 'upbge',
			label: 'UPBGE',
			match: (b) => !isCustom(b) && b.fork === 'upbge' && b.branch === 'stable'
		},
		{
			id: 'upbge-weekly',
			label: 'UPBGE Weekly',
			match: (b) => !isCustom(b) && b.fork === 'upbge' && b.branch !== 'stable'
		}
	];
	const CUSTOM_TAB: SubTab = { id: 'custom', label: 'Custom', match: isCustom };

	const subTabs = $derived<SubTab[]>(
		tab === 'library'
			? [...REPO_TABS, CUSTOM_TAB]
			: tab === 'downloads'
				? REPO_TABS
				: [{ id: 'favorites', label: 'Favorites', match: () => true }]
	);

	$effect(() => {
		if (!subTabs.some((item) => item.id === subTabId)) subTabId = subTabs[0].id;
	});

	const instancesByBuild = $derived(
		(() => {
			const map = new Map<string, number>();
			for (const instance of $instances) {
				map.set(instance.build_id, (map.get(instance.build_id) ?? 0) + 1);
			}
			return map;
		})()
	);

	/** 数据集：库=已安装，下载=未安装的远程，收藏=收藏项。 */
	const pageBuilds = $derived(
		$builds.filter((build) => {
			if (tab === 'library') return build.is_installed;
			if (tab === 'downloads') return !build.is_installed;
			return $favoriteIds.has(build.id);
		})
	);

	const filtered = $derived(
		pageBuilds.filter((build) => {
			const sub = subTabs.find((item) => item.id === subTabId);
			if (sub && !sub.match(build)) return false;
			if (!search) return true;
			const needle = search.toLowerCase();
			return (
				build.version.toLowerCase().includes(needle) ||
				build.branch.toLowerCase().includes(needle) ||
				build.fork.toLowerCase().includes(needle)
			);
		})
	);

	function compare(a: Build, b: Build): number {
		if (sortKey === 'version') {
			// 版本号按数字段比较，"4.10" 才能排在 "4.9" 后面。
			const left = a.version.split(/[.+-]/).map((part) => Number.parseInt(part, 10) || 0);
			const right = b.version.split(/[.+-]/).map((part) => Number.parseInt(part, 10) || 0);
			for (let i = 0; i < Math.max(left.length, right.length); i += 1) {
				const diff = (left[i] ?? 0) - (right[i] ?? 0);
				if (diff !== 0) return diff;
			}
			return Date.parse(a.release_date) - Date.parse(b.release_date);
		}
		if (sortKey === 'branch') return a.branch.localeCompare(b.branch);
		return Date.parse(a.release_date) - Date.parse(b.release_date);
	}

	const rows = $derived(
		filtered
			.map<BuildRow>((build) => ({
				build,
				action: build.is_installed
					? (instancesByBuild.get(build.id) ?? 0) > 0
						? ('running' as const)
						: ('launch' as const)
					: ('download' as const),
				instances: instancesByBuild.get(build.id) ?? 0
			}))
			// 原版默认按时间倒序（最新在前）。
			.sort((a, b) => (sortAsc ? compare(a.build, b.build) : compare(b.build, a.build)))
	);

	const railItems = $derived(
		subTabs.map((item) => ({
			id: item.id,
			label: item.label,
			count: pageBuilds.filter((build) => item.match(build)).length
		}))
	);

	const activeTasks = $derived(
		[...$downloads].sort((a, b) => {
			const terminal = ['completed', 'failed', 'cancelled'];
			return Number(terminal.includes(a.state)) - Number(terminal.includes(b.state));
		})
	);

	const emptyText = $derived(
		tab === 'downloads'
			? filtered.length === 0 && pageBuilds.length > 0
				? tr('empty.downloads')
				: tr('empty.library')
			: tab === 'favorites'
				? tr('empty.favorites')
				: tr('empty.library')
	);

	function handleSort(key: 'version' | 'branch' | 'date'): void {
		if (key === sortKey) sortAsc = !sortAsc;
		else {
			sortKey = key;
			sortAsc = key === 'branch';
		}
	}

	async function handleRowAction(build: Build): Promise<void> {
		try {
			if (build.is_installed) {
				await api.launchBlender(build.id);
				await reloadInstances();
			} else {
				await api.startDownload(build.id);
				await reloadDownloads();
			}
		} catch (err) {
			appError.set(String(err));
		}
	}

	/** 点更新图标：下载新版（原版先检查是否已在下，再替换安装）。 */
	async function handleUpdate(build: Build): Promise<void> {
		if (!build.update_build_id) return;
		try {
			await api.startDownload(build.update_build_id);
			await reloadDownloads();
			tab = 'downloads';
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function handleRowDefault(build: Build, openLast = false): Promise<void> {
		if (!build.is_installed) {
			// Custom 页未识别的条目：双击初始化（原版 UnrecoBuildWidget）。
			if (build.url.startsWith('remote-unreco:')) initCustom(build);
			return;
		}
		try {
			await api.launchBlender(build.id, undefined, undefined, undefined, openLast);
			await reloadInstances();
		} catch (err) {
			appError.set(String(err));
		}
	}

	function initCustom(build: Build): void {
		dialogPath = build.install_path ?? null;
		dialogInitial = null;
	}

	function openMenu(build: Build | null, event: MouseEvent): void {
		// 只要鼠标坐标：溢出翻转、贴边平移、超高滚动都由 ContextMenu 的
		// floating-ui 中间件处理，这里不再手算窗口尺寸。
		menu = { x: event.clientX || 0, y: event.clientY || 0, build };
	}

	async function copyHash(build: Build): Promise<void> {
		// 优先用 .blinfo 里的真实 hash；没有才回退到版本号里的 commit 段。
		let hash: string | null = null;
		try {
			hash = await api.getBuildHash(build.id);
		} catch {
			hash = null;
		}
		if (!hash) {
			const plus = build.version.lastIndexOf('+');
			hash = plus === -1 ? build.version : (build.version.slice(plus + 1).split('.').pop() ?? build.version);
		}
		await navigator.clipboard.writeText(hash);
	}

	async function debugLaunch(build: Build, extra: string[]): Promise<void> {
		try {
			await api.launchBlender(build.id, undefined, extra);
			await reloadInstances();
		} catch (err) {
			appError.set(String(err));
		}
	}

	/**
	 * 右键菜单条目，顺序与图标对齐原版 `library_widget.py:276-318`：
	 *
	 *   打开上一个文件 / 快速启动 / 收藏 / 更新
	 *   ──────
	 *   调试 ▸ / 注册扩展 / 快捷方式 / 符号链接 / 模板 / 便携 / 复制哈希 / 冻结更新
	 *   ──────
	 *   发布说明
	 *   构建文件夹 / 配置文件夹 / 编辑构建… / 删除
	 *
	 * 之前缺「显示配置文件夹」和「冻结更新」，且删除项没有回收站/永久删除
	 * 的 Shift 切换，与截图里的原版菜单对不上。
	 */
	function menuItems(build: Build | null): MenuItem[] {
		if (!build) return [];
		// 原版：多选时右键只给 Delete（library_widget.py:347-352）。
		if (selected.size > 1) {
			return [
				{
					label: tr('menu.deleteMany', { count: selected.size }),
					icon: 'delete',
					action: () => void confirmRemoveSelection()
				}
			];
		}
		const installed = build.is_installed;
		const isFavorite = $favoriteIds.has(build.id);
		const inQuickLaunch = $quickLaunchIds.has(build.id);
		const items: MenuItem[] = [];

		if (installed) {
			items.push({
				label: tr('menu.openPrevious'),
				icon: 'file',
				action: () => void handleRowDefault(build, true)
			});
			items.push({
				label: inQuickLaunch ? tr('menu.quickLaunchRemove') : tr('menu.quickLaunchAdd'),
				icon: 'quickLaunch',
				action: () => void toggleQuick(build.id)
			});
		}
		items.push({
			label: isFavorite ? tr('menu.favRemove') : tr('menu.favAdd'),
			icon: 'favorite',
			action: () => void toggleFavorite(build.id)
		});

		if (build.update_build_id && ($settings?.show_update_button ?? true)) {
			items.push({ label: tr('menu.update'), icon: 'update', action: () => void handleUpdate(build) });
		}

		if (installed) {
			items.push({ separator: true });
			items.push({
				label: tr('menu.debug'),
				children: [
					{ label: tr('menu.debugLog'), action: () => void debugLaunch(build, ['--debug']) },
					{ label: tr('menu.factoryStartup'), action: () => void debugLaunch(build, ['--factory-startup']) },
					{ label: tr('menu.debugGpu'), action: () => void debugLaunch(build, ['--debug-gpu']) },
					{
						label: tr('menu.debugGpuWorkaround'),
						action: () => void debugLaunch(build, ['--debug-gpu-force-workarounds'])
					}
				]
			});
			items.push({
				label: tr('menu.register'),
				action: () => void registerAssociation()
			});
			items.push({
				label: tr('menu.shortcut'),
				action: () => void makeShortcut(build)
			});
			items.push({
				label: tr('menu.symlink'),
				action: () => void makeSymlink(build)
			});
			items.push({
				label: tr('menu.template'),
				action: () => void installTemplate(build)
			});
			items.push({
				label: tr('menu.portable'),
				action: () => void togglePortable(build)
			});
			items.push({ label: tr('menu.copyHash'), action: () => void copyHash(build) });
			// 原版 freezeUpdate 文案随状态切换（冻结更新 / 解除冻结更新）。
			items.push({
				label: build.is_frozen ? tr('menu.unfreezeUpdate') : tr('menu.freezeUpdate'),
				action: () => void toggleFrozen(build)
			});
			items.push({
				label: tr('menu.renameBranch'),
				children: ['stable', 'daily', 'experimental', 'patch'].map((branchName) => ({
					label: branchName,
					action: () => void moveBranch(build, branchName)
				}))
			});
		}

		if (build.release_notes_url) {
			items.push({ separator: true });
			items.push({
				label: tr('menu.notes'),
				action: () => window.open(build.release_notes_url!, '_blank')
			});
		}

		if (build.install_path) {
			items.push({
				label: tr('menu.folder'),
				icon: 'folder',
				action: () => void api.openInExplorer(build.install_path!)
			});
			// 原版 showConfigFolderAction：便携模式显示「显示便携配置文件夹」。
			items.push({
				label: tr('menu.configFolder'),
				icon: 'folder',
				action: () => void openConfigFolder(build)
			});
		}

		if (installed) {
			items.push({
				label: tr('menu.edit'),
				icon: 'settings',
				action: () => void openEditDialog(build)
			});
			// 原版删除项：按住 Shift 在「移至回收站 / 从磁盘删除」之间切换。
			// withShift 由 ContextMenu 在点击时传入，决定 toTrash 的真假。
			items.push({
				label: tr('menu.trash'),
				altLabel: tr('menu.deleteOne'),
				icon: 'trash',
				action: (withShift) => void confirmRemove(build, !withShift)
			});
		}
		return items;
	}

	async function toggleQuick(buildId: string): Promise<void> {
		try {
			await api.toggleQuickLaunch(buildId);
			await reloadQuickLaunch();
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function registerAssociation(): Promise<void> {
		try {
			await api.registerFileAssociation();
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function makeShortcut(build: Build): Promise<void> {
		const { save } = await import('@tauri-apps/plugin-dialog');
		const destination = await save({
			title: 'Choose shortcut destination',
			defaultPath: `Blender ${build.version}.lnk`
		});
		if (!destination) return;
		try {
			await api.createShortcut(build.id, destination);
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function makeSymlink(build: Build): Promise<void> {
		try {
			await api.createSymlink(build.id);
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function installTemplate(build: Build): Promise<void> {
		try {
			await api.installTemplate(build.id);
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function togglePortable(build: Build): Promise<void> {
		try {
			await api.togglePortable(build.id);
		} catch (err) {
			appError.set(String(err));
		}
	}

	/** 冻结/解冻更新（原版 freeze_update）：冻结后不再提示该构建有新版。 */
	async function toggleFrozen(build: Build): Promise<void> {
		try {
			await api.setBuildFrozen(build.id, !build.is_frozen);
			await reloadBuilds();
		} catch (err) {
			appError.set(String(err));
		}
	}

	/** 显示配置文件夹：便携优先，否则系统用户配置目录。 */
	async function openConfigFolder(build: Build): Promise<void> {
		try {
			await api.openConfigFolder(build.id);
		} catch (err) {
			appError.set(String(err));
		}
	}

	/**
	 * 打开「编辑构建」对话框。
	 *
	 * 字段必须从 `.blinfo` 原样读回来，不能拿 `Build` 拼：`build_hash` 与
	 * `commit_time` 不在 `Build` 上，拼出来的对象会让 write_blinfo 把这两个
	 * 字段重置掉——用户改了自定义名，构建哈希却没了，看上去就是「保存无效」。
	 */
	async function openEditDialog(build: Build): Promise<void> {
		try {
			const info = await api.getBuildInfo(build.id);
			dialogInitial = { ...info };
			dialogPath = info.path || build.install_path || null;
		} catch (err) {
			// .blinfo 缺失时仍允许打开，只是字段为空，由用户手填。
			dialogInitial = {
				executable: '',
				subversion: build.version,
				branch: build.branch,
				custom_name: build.label ?? ''
			};
			dialogPath = build.install_path ?? null;
			appError.set(String(err));
		}
	}

	async function moveBranch(build: Build, branch: string): Promise<void> {
		try {
			await api.moveBuildToCategory(build.id, branch);
			await checkLibrary();
		} catch (err) {
			appError.set(String(err));
		}
	}

	/**
	 * 删除一个构建。`toTrash` 决定回收站还是永久删除：
	 * 原版默认回收站，按住 Shift 反转为永久删除（library_widget.py:364-371）。
	 */
	async function confirmRemove(build: Build, toTrash = true): Promise<void> {
		const message = toTrash
			? tr('confirm.trashOne', { version: build.version })
			: tr('confirm.deleteOne', { version: build.version });
		if (!window.confirm(message)) return;
		try {
			await api.removeBuild(build.id, false, toTrash);
			await reloadBuilds();
		} catch (err) {
			appError.set(String(err));
		}
	}

	/** 多选批量删除（原版 remove_from_drive_extended）。 */
	async function confirmRemoveSelection(): Promise<void> {
		const ids = [...selected].filter((id) => $builds.find((b) => b.id === id)?.is_installed);
		if (ids.length === 0) return;
		if (!window.confirm(tr('confirm.deleteMany', { count: ids.length }))) return;
		try {
			for (const id of ids) await api.removeBuild(id, false);
			selected = new Set();
			await reloadBuilds();
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function handleTask(task: DownloadTask, kind: 'pause' | 'resume' | 'cancel'): Promise<void> {
		try {
			if (kind === 'pause') await api.pauseDownload(task.id);
			else if (kind === 'resume') await api.resumeDownload(task.id);
			else await api.cancelDownload(task.id);
		} catch (err) {
			appError.set(String(err));
		}
	}

	/**
	 * Tauri 2 窗口默认 dragDropEnabled=true，在原生层接管拖放，
	 * HTML5 的 drop 事件收不到；必须用 onDragDropEvent，它给的是真实路径
	 * （HTML5 的 File.name 只有文件名，没法传给 Blender）。
	 */
	async function openDroppedFile(path: string): Promise<void> {
		if (!path.toLowerCase().endsWith('.blend')) {
			appError.set(tr('error.dropBlendOnly'));
			return;
		}
		const target = $builds.find((build) => build.is_installed);
		if (!target) {
			appError.set(tr('error.noInstalled'));
			return;
		}
		try {
			await api.launchBlender(target.id, path);
			await reloadInstances();
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function reloadCustom(): Promise<void> {
		await checkLibrary();
	}

	async function afterDialogSave(): Promise<void> {
		await checkLibrary();
	}
</script>

<div
	class="relative flex h-screen flex-col overflow-hidden app-shell"
	role="application"
>
	<Header
		tab={tab}
		onTab={(next) => {
			tab = next;
			settingsOpen = false;
			selected = new Set();
		}}
		onSettings={() => (settingsOpen = !settingsOpen)}
	/>

	{#if dropped}
		<!-- 拖放反馈：只做淡入。位移在这里会让人误以为有可拖拽的卡片。 -->
		<div
			class="pointer-events-none absolute inset-0 z-40 flex items-center justify-center bg-accent/10"
			transition:fade={{ duration: 100 }}
		>
			<div class="rounded-sm2 border-2 border-dashed border-accent px-8 py-6 text-accent">
				Drop a .blend file to open it
			</div>
		</div>
	{/if}

	<div class="flex min-h-0 flex-1">
		{#if settingsOpen}
			<!-- 滚动交给 SettingsForm 内部：双层 overflow 会让内层的 h-full
			     拿不到确定高度，页面底部的布局卡片被裁掉 -->
			<div class="min-h-0 flex-1 overflow-hidden">
				<SettingsForm />
			</div>
		{:else if tab === 'extensions'}
			<!-- 插件页与库布局无关：扩展目录属于某个具体 Build，
			     与排序/子页签/多选都不共享状态，所以放在布局分支之前。 -->
			<div class="flex min-w-0 flex-1 flex-col">
				<div class="flex shrink-0 items-center gap-3 border-b border-surface-4 bg-surface-1 px-3 py-2">
					{#if installedBuilds.length > 0}
						<label class="flex items-center gap-2 text-[12px] text-ink-3">
							{tr('ext.pickBuild')}
							<select
								class="h-7 rounded-sm2 border border-surface-4 bg-surface-2 px-2 text-[12px] text-ink-1"
								value={extBuildId ?? ''}
								onchange={(event) => (extBuildId = event.currentTarget.value)}
							>
								{#each installedBuilds as build (build.id)}
									<option value={build.id}>{build.version}</option>
								{/each}
							</select>
						</label>
					{/if}

					{#if extScan}
						<span class="min-w-0 truncate text-[12px] text-ink-3" title={extScan.ext_root ?? ''}>
							{tr('ext.count').replace('{count}', String(extScan.extensions.length + extScan.legacy.length))}
						</span>
						{#if extScan.corrupt.length > 0}
							<span class="shrink-0 text-[12px] text-warning" title={extScan.corrupt.join('\n')}>
								{tr('ext.corrupt').replace('{count}', String(extScan.corrupt.length))}
							</span>
						{/if}
					{/if}

				<input
					type="search"
					placeholder={tr('search.placeholder')}
					class="ml-auto h-7 w-48 rounded-sm2 border border-surface-4 bg-surface-2 px-2 text-[12px] text-ink-1 placeholder:text-ink-3 focus:border-accent focus:outline-none"
					bind:value={extFilter}
				/>
				<button
					class="h-7 shrink-0 rounded-sm2 border border-surface-4 px-3 text-[12px] text-ink-2 transition-colors hover:border-surface-5 hover:text-ink-1 disabled:opacity-50"
					disabled={!extBuildId}
					title={tr('ext.catalogFootNote')}
					onclick={() => void openCatalog()}
				>
					{tr('ext.openCatalog')}
				</button>
				<button
					class="h-7 shrink-0 rounded-sm2 border border-surface-4 px-3 text-[12px] text-ink-2 transition-colors hover:border-surface-5 hover:text-ink-1 disabled:opacity-50"
					disabled={!extBuildId}
					title={tr('ext.importHint')}
					onclick={() => void importExtensionFile()}
				>
					{tr('ext.importFile')}
				</button>
				<button
					class="h-7 shrink-0 rounded-sm2 border border-surface-4 px-3 text-[12px] text-ink-2 transition-colors hover:border-surface-5 hover:text-ink-1 disabled:opacity-50"
					disabled={!extBuildId || extProbing}
					title={tr('ext.detectedHint')}
					onclick={detectExtensions}
				>
					{extProbing ? tr('ext.detecting') : tr('ext.detect')}
				</button>
			</div>

			{#if extScan?.enabled_checked_at}
				<div class="shrink-0 border-b border-surface-4/60 bg-surface-2 px-3 py-1 text-[11px] text-ink-3">
					{tr('ext.detectedAt').replace(
						'{time}',
						new Date(extScan.enabled_checked_at).toLocaleString()
					)}
				</div>
			{:else}
				<div class="shrink-0 border-b border-surface-4/60 bg-surface-2 px-3 py-1 text-[11px] text-ink-3">
					{tr('ext.neverDetected')}
				</div>
			{/if}

				<div class="min-h-0 flex-1">
					<ExtensionList
						scan={extScan}
						filter={extFilter}
						onToggle={(item, enabled) => void toggleExtension(item, enabled)}
						onReveal={revealPath}
						onRemove={(item) => void removeExtension(item)}
						onManage={(item) => void openManage(item)}
						onDetect={detectExtensions}
					/>
				</div>
			</div>
		{:else if wide}
			<!-- Wide：顶部横向子页签 + 紧凑行，表格吃满宽度 -->
			<div class="flex min-w-0 flex-1 flex-col">
				<TopTabs items={railItems} active={subTabId} onSelect={(id) => (subTabId = id)} />
				<div class="min-h-0 flex-1">
					<BuildList
						{rows}
						mode={tab}
						{selected}
						{sortKey}
						{sortAsc}
						{emptyText}
						density="compact"
						onSelect={(ids) => (selected = ids)}
						onAction={handleRowAction}
						onUpdate={handleUpdate}
						onContextMenu={(build, event) => openMenu(build, event)}
						onDefault={handleRowDefault}
						onSort={handleSort}
						onDeleteSelection={() => void confirmRemoveSelection()}
						onReload={tab === 'library' && subTabId === 'custom' ? reloadCustom : null}
					/>
				</div>
			</div>
		{:else if detailMode}
			<!-- Detail：顶部页签 + 列表 + 右侧详情面板（三段式） -->
			<div class="flex min-w-0 flex-1 flex-col">
				<div class="flex items-center border-b border-surface-4 bg-surface-2">
					<div class="min-w-0 flex-1">
						<TopTabs items={railItems} active={subTabId} onSelect={(id) => (subTabId = id)} />
					</div>
					{#if !detailOpen}
						<button
							class="mr-2 shrink-0 rounded-sm2 bg-surface-4 px-2 py-1 text-[11px] text-ink-2 hover:text-ink-1"
							title={tr('detail.show')}
							onclick={() => (detailOpen = true)}
						>
							Details
						</button>
					{/if}
				</div>
				<div class="flex min-h-0 flex-1">
					<div class="min-w-0 flex-1">
						<BuildList
							{rows}
							mode={tab}
							{selected}
							{sortKey}
							{sortAsc}
							{emptyText}
							density="compact"
							onSelect={(ids) => (selected = ids)}
							onFocus={(id) => (focusId = id)}
							onAction={handleRowAction}
							onUpdate={handleUpdate}
							onContextMenu={(build, event) => openMenu(build, event)}
							onDefault={handleRowDefault}
							onSort={handleSort}
							onDeleteSelection={() => void confirmRemoveSelection()}
							onReload={tab === 'library' && subTabId === 'custom' ? reloadCustom : null}
						/>
					</div>
					{#if detailOpen}
						<DetailPanel
							build={focusBuild}
							selectedCount={selected.size}
							onLaunch={handleRowAction}
							onClose={() => (detailOpen = false)}
						/>
					{/if}
				</div>
			</div>
		{:else}
			<SideRail items={railItems} active={subTabId} onSelect={(id) => (subTabId = id)} />

			<div class="min-w-0 flex-1">
				<BuildList
					{rows}
					mode={tab}
					{selected}
					{sortKey}
					{sortAsc}
					{emptyText}
					onSelect={(ids) => (selected = ids)}
					onAction={handleRowAction}
					onUpdate={handleUpdate}
					onContextMenu={(build, event) => openMenu(build, event)}
					onDefault={handleRowDefault}
					onSort={handleSort}
					onDeleteSelection={() => void confirmRemoveSelection()}
					onReload={tab === 'library' && subTabId === 'custom' ? reloadCustom : null}
				/>
			</div>

			{#if tab === 'downloads' && activeTasks.length > 0}
				<aside class="w-[300px] shrink-0 overflow-y-auto border-l border-surface-4 p-3">
					<h2 class="mb-2 text-[12px] font-semibold text-ink-2">
						Transfers
					</h2>
					<div class="space-y-2">
						{#each activeTasks as task (task.id)}
							<DownloadItem
								task={task}
								onPause={(t) => handleTask(t, 'pause')}
								onResume={(t) => handleTask(t, 'resume')}
								onCancel={(t) => handleTask(t, 'cancel')}
								onRetry={(t) => void api.startDownload(t.build_id)}
							/>
						{/each}
					</div>
				</aside>
			{/if}
		{/if}
	</div>

	<div class="flex shrink-0 items-center gap-2 border-t border-surface-4 bg-surface-2 px-3 py-1">
		<input
			type="search"
			placeholder={tr('search.placeholder')}
			class="w-56 rounded-sm border border-surface-4 bg-surface-1 px-2 py-1 text-sm text-ink-1 placeholder:text-ink-3 focus:border-accent focus:outline-none"
			bind:value={search}
		/>
		{#if $appError}
			<span class="min-w-0 truncate text-xs text-danger" title={$appError}>{$appError}</span>
		{/if}
		<button class="ml-auto shrink-0 text-xs text-ink-3 hover:text-white" onclick={checkLibrary}>
			{tr('status.rescanLibrary')}
		</button>
	</div>

	<StatusBar version={appVersion} />

	{#if menu}
		<ContextMenu
			x={menu.x}
			y={menu.y}
			items={menuItems(menu.build)}
			onClose={() => (menu = null)}
		/>
	{/if}

	{#if dialogPath}
		<BuildEntryDialog
			path={dialogPath}
			initial={dialogInitial as never}
			onClose={() => (dialogPath = null)}
			onSaved={afterDialogSave}
		/>
	{/if}

	{#if wizardOpen}
		<FirstRunWizard onDone={() => (wizardOpen = false)} />
	{/if}

	{#if extCatalogOpen}
		<CatalogDialog
			items={extCatalog}
			scan={extScan}
			loading={extCatalogLoading}
			error={extCatalogError}
			installing={extInstalling}
			onClose={() => (extCatalogOpen = false)}
			onInstall={(item) => void installFromCatalog(item)}
			onOpenPage={(url) => void api.openExtensionPage(url).catch((err) => appError.set(String(err)))}
		/>
	{/if}

	{#if manageItem}
		<ExtensionManageDialog
			item={manageItem}
			managed={manageState}
			busy={manageBusy}
			error={manageError}
			history={manageHistory}
			historyLoading={manageHistoryLoading}
			historyError={manageHistoryError}
			adding={manageAdding}
			onAdopt={() => void adoptExtension()}
			onSwitch={(version) => void switchVersion(version)}
			onAddVersion={(version) => void addVersion(version)}
			onRelease={() => void releaseExtension()}
			onClose={() => {
				manageItem = null;
				manageState = null;
				manageError = '';
				manageHistory = [];
				manageHistoryError = '';
			}}
		/>
	{/if}

	<!-- 无边框窗口的四边四角缩放手柄（shadow:true 时 tao 只补上边缘） -->
	<ResizeHandles />
</div>
