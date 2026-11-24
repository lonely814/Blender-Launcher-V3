<script lang="ts">
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import type { Tab } from '$lib/types';
	import { buildsLoading, downloads } from '$lib/stores';
	import { locale, makeTranslator } from '$lib/i18n';

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
		{ id: 'favorites', label: tr('tab.favorites') }
	]);

	const activeDownloads = $derived(
		$downloads.filter((task) => !['completed', 'failed', 'cancelled'].includes(task.state)).length
	);
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
