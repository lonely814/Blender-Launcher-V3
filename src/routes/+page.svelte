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
	import FirstRunWizard from '$lib/components/FirstRunWizard.svelte';
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
	import type { Build, BuildRow, DownloadTask, MenuItem, Tab } from '$lib/types';

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

	function menuItems(build: Build | null): MenuItem[] {
		if (!build) return [];
		// 原版：多选时右键只给 Delete（library_widget.py:347-352）。
		if (selected.size > 1) {
			return [
				{
					label: tr('menu.deleteMany', { count: selected.size }),
					action: () => void confirmRemoveSelection()
				}
			];
		}
		const installed = build.is_installed;
		const isFavorite = $favoriteIds.has(build.id);
		const inQuickLaunch = $quickLaunchIds.has(build.id);
		const items: MenuItem[] = [];

		if (installed) {
			items.push({ label: tr('menu.openPrevious'), action: () => void handleRowDefault(build, true) });
			items.push({
				label: inQuickLaunch ? tr('menu.quickLaunchRemove') : tr('menu.quickLaunchAdd'),
				action: () => void toggleQuick(build.id)
			});
		}
		items.push({
			label: isFavorite ? tr('menu.favRemove') : tr('menu.favAdd'),
			action: () => void toggleFavorite(build.id)
		});

		if (build.update_build_id && ($settings?.show_update_button ?? true)) {
			items.push({ label: tr('menu.update'), action: () => void handleUpdate(build) });
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
				action: () => void api.openInExplorer(build.install_path!)
			});
		}

		if (installed) {
			items.push({
				label: tr('menu.edit'),
				action: () => {
					dialogPath = build.install_path ?? null;
					dialogInitial = {
						executable: '',
						subversion: build.version,
						branch: build.branch,
						custom_name: ''
					};
				}
			});
			items.push({ separator: true });
			items.push({
				label: tr('menu.deleteOne'),
				action: () => void confirmRemove(build)
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

	async function moveBranch(build: Build, branch: string): Promise<void> {
		try {
			await api.moveBuildToCategory(build.id, branch);
			await checkLibrary();
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function confirmRemove(build: Build): Promise<void> {
		if (!window.confirm(tr('confirm.deleteOne', { version: build.version }))) return;
		try {
			await api.removeBuild(build.id, false);
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
			build={menu.build}
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

	<!-- 无边框窗口的四边四角缩放手柄（shadow:true 时 tao 只补上边缘） -->
	<ResizeHandles />
</div>
