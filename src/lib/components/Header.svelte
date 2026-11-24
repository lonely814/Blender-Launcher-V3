<script lang="ts">
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import type { Tab } from '$lib/types';
	import { buildsLoading, downloads } from '$lib/stores';
	import { formatSpeed, progressPercent } from '$lib/format';
	import { locale, makeTranslator } from '$lib/i18n';
	import * as api from '$lib/api';

	let {
		tab = 'library',
		onTab = () => {},
		onSettings = () => {}
	}: {
		tab?: Tab;
		onTab?: (tab: Tab) => void;
		onSettings?: () => void;
	} = $props();

	const appWindow = getCurrentWindow();

	// $derived 而不是 $:：这些值不产生副作用，纯派生用 $derived 语义更准，
	// 也避免 $: 语句在 runes 模式下被当作 effect 重复执行。
	const tr = $derived(makeTranslator($locale));
	const tabs = $derived<Array<{ id: Tab; label: string }>>([
		{ id: 'library', label: tr('tab.library') },
		{ id: 'downloads', label: tr('tab.downloads') },
		{ id: 'favorites', label: tr('tab.favorites') },
		{ id: 'extensions', label: tr('tab.extensions') }
	]);

	const activeTasks = $derived(
		$downloads.filter((task) => !['completed', 'failed', 'cancelled'].includes(task.state))
	);
	const activeDownloads = $derived(activeTasks.length);

	/**
	 * 聚合进度。按字节加权，而不是把各任务百分比求平均——
	 * 后者会让 5 MB 的小包和 400 MB 的大包各占一半权重，
	 * 进度条与实际等待时间无关。
	 */
	const transfer = $derived.by(() => {
		let done = 0;
		let total = 0;
		let speed = 0;
		for (const task of activeTasks) {
			speed += task.speed_bps;
			if (task.total_bytes > 0) {
				done += task.progress_bytes;
				total += task.total_bytes;
			}
		}
		return { done, total, speed, percent: progressPercent(done, total) };
	});

	function openRepo(): void {
		void api
			.openExtensionPage('https://github.com/lonely814/Blender-Launcher-V3')
			.catch(() => undefined);
	}
</script>

<!-- 原版无系统标题栏：自绘 32px 标题栏（拖拽区 + 窗口按钮） -->
<header class="shrink-0 select-none bg-surface-1">
	<div class="flex h-8 items-center" data-tauri-drag-region role="toolbar" tabindex="-1">
		<button
			class="ml-1 flex h-6 w-6 items-center justify-center rounded-sm2 text-ink-2 transition hover:bg-surface-4 hover:text-ink-1"
			title={tr('action.settings')}
			onclick={onSettings}
		>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
				<circle cx="12" cy="12" r="3" />
				<path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9v.09a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
			</svg>
		</button>

		<span class="flex-1 truncate px-2 text-center text-[13px] font-medium text-ink-2" data-tauri-drag-region>
			Blender Launcher
			{#if $buildsLoading}
				<span class="ml-2 text-accent">checking…</span>
			{/if}
		</span>

		<!--
			传输指示器：放在标题栏而不是某个 Tab 的侧栏里。
			下载会在用户离开「下载」页后继续跑，进度只挂在那个页面上
			等于让人不知道自己在等什么。
		-->
		{#if activeTasks.length > 0}
			<button
				class="mr-1 flex h-6 shrink-0 items-center gap-1.5 rounded-sm2 px-2 text-[11px] text-ink-2 transition hover:bg-surface-3 hover:text-ink-1"
				title={tr('transfer.title')}
				onclick={() => onTab('downloads')}
			>
				<span class="relative flex h-3 w-3 items-center justify-center">
					<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
						<path d="M12 3v12m0 0l-4-4m4 4l4-4M4 19h16" />
					</svg>
				</span>
				<span class="tabular-nums">
					{#if transfer.percent !== null}
						{transfer.percent.toFixed(0)}%
					{:else}
						{activeTasks.length}
					{/if}
				</span>
				{#if transfer.speed > 0}
					<span class="tabular-nums text-ink-3">{formatSpeed(transfer.speed)}</span>
				{/if}
			</button>
		{/if}

		<button
			class="mr-1 flex h-6 w-6 shrink-0 items-center justify-center rounded-sm2 text-ink-2 transition hover:bg-surface-4 hover:text-ink-1"
			title={tr('action.repository')}
			onclick={openRepo}
		>
			<!-- GitHub 标记：官方 octicon mark-github 的路径 -->
			<svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
				<path
					d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"
				/>
			</svg>
		</button>

		<div class="flex h-full">
			<button
				class="flex h-8 w-11 items-center justify-center text-ink-2 transition hover:bg-surface-3 hover:text-ink-1"
				title={tr('window.minimize')}
				onclick={() => void appWindow.minimize()}
			>
				<svg width="10" height="10" viewBox="0 0 10 10"><path d="M2 5h6" stroke="currentColor" stroke-width="1.2" /></svg>
			</button>
			<button
				class="flex h-8 w-11 items-center justify-center text-ink-2 transition hover:bg-surface-3 hover:text-ink-1"
				title={tr('window.maximize')}
				onclick={() => void appWindow.toggleMaximize()}
			>
				<svg width="10" height="10" viewBox="0 0 10 10"><rect x="2" y="2" width="6" height="6" fill="none" stroke="currentColor" stroke-width="1.2" /></svg>
			</button>
			<button
				class="flex h-8 w-11 items-center justify-center text-ink-2 transition hover:bg-danger hover:text-white"
				title={tr('window.close')}
				onclick={() => void appWindow.close()}
			>
				<svg width="10" height="10" viewBox="0 0 10 10"><path d="M2 2l6 6M8 2l-6 6" stroke="currentColor" stroke-width="1.2" /></svg>
			</button>
		</div>
	</div>

	<!-- 原版主 Tab：选中项底部橙色下划线 -->
	<nav class="flex gap-px px-2">
		{#each tabs as item (item.id)}
			<button
				class="relative px-3.5 py-1.5 text-[13px] font-medium transition
					{tab === item.id ? 'bg-surface-3 text-ink-1' : 'text-ink-2 hover:text-ink-1'}"
				onclick={() => onTab(item.id)}
			>
				{item.label}
				{#if item.id === 'downloads' && activeDownloads > 0}
					<span class="ml-1.5 rounded-full bg-accent px-1.5 text-[10px] font-semibold text-accent-ink">
						{activeDownloads}
					</span>
				{/if}
				{#if tab === item.id}
					<span class="absolute inset-x-0 -bottom-px h-[2px] bg-accent"></span>
				{/if}
			</button>
		{/each}
	</nav>
</header>
