<script lang="ts">
	import { onMount } from 'svelte';
	import { fade, fly } from 'svelte/transition';
	import { prefersReducedMotion } from 'svelte/motion';
	import { locale, makeTranslator } from '$lib/i18n';
	import { isNewerVersion } from '$lib/version';
	import type { CatalogItem, ExtensionScan, InstalledExtension, LegacyAddon } from '$lib/types';

	/**
	 * 扩展目录（blender_org 等仓库的 index.json）。
	 *
	 * 只负责「看 + 触发安装」。真正的下载与 CLI 委派在后端做，
	 * 这里不做任何文件操作，装完由父组件重扫。
	 *
	 * 目录条目最多 1480 条，但对话框是临时打开的短列表，
	 * 不做虚拟滚动：加一个搜索框把候选压到几十条更实用。
	 */
	let {
		items = [] as CatalogItem[],
		scan = null as ExtensionScan | null,
		loading = false,
		error = '',
		onClose = () => {},
		onInstall = (_: CatalogItem) => {},
		onOpenPage = (_: string) => {},
		installing = null as string | null
	}: {
		items?: CatalogItem[];
		scan?: ExtensionScan | null;
		loading?: boolean;
		error?: string;
		onClose?: () => void;
		onInstall?: (item: CatalogItem) => void;
		onOpenPage?: (url: string) => void;
		installing?: string | null;
	} = $props();

	const tr = $derived(makeTranslator($locale));
	let query = $state('');
	let onlyUpdatable = $state(false);

	/** 已装扩展按 id 建索引，用来标「已装」和「有更新」。 */
	const installedById = $derived.by(() => {
		const map = new Map<string, InstalledExtension>();
		for (const item of scan?.extensions ?? []) map.set(item.pkg_id.toLowerCase(), item);
		return map;
	});

	const installedLegacy = $derived.by(() => {
		const map = new Map<string, LegacyAddon>();
		for (const item of scan?.legacy ?? []) map.set(item.module.toLowerCase(), item);
		return map;
	});

	const rows = $derived.by(() => {
		const needle = query.trim().toLowerCase();
		return items
			.filter((item) => {
				if (needle) {
					const hit =
						item.id.toLowerCase().includes(needle) ||
						item.name.toLowerCase().includes(needle) ||
						(item.tagline ?? '').toLowerCase().includes(needle);
					if (!hit) return false;
				}
				if (onlyUpdatable && !hasUpdate(item)) return false;
				return true;
			})
			.slice()
			.sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
	});

	function installedOf(item: CatalogItem): InstalledExtension | undefined {
		return installedById.get(item.id.toLowerCase());
	}

	/**
	 * 有没有新版本。只用目录版本与已装 manifest 版本比：
	 * 目录里的版本是该扩展的最新发布版，manifest 是盘上实际那份。
	 */
	function hasUpdate(item: CatalogItem): boolean {
		const installed = installedOf(item);
		if (!installed) return false;
		return isNewerVersion(item.version, installed.version);
	}

	function isInstalled(item: CatalogItem): boolean {
		return installedOf(item) !== undefined;
	}

	/** 旧式 addon 同名时提示「已作为旧式插件存在」，避免用户重复安装。 */
	function legacyClash(item: CatalogItem): boolean {
		return installedLegacy.has(item.id.toLowerCase());
	}

	onMount(() => {
		const onKey = (event: KeyboardEvent) => {
			if (event.key === 'Escape') onClose();
		};
		window.addEventListener('keydown', onKey);
		return () => window.removeEventListener('keydown', onKey);
	});
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center bg-scrim/50" transition:fade={{ duration: 120 }}>
	<!--
		遮罩层不挂 role="dialog"：它只是背景，真正的对话框是下面这个面板。
		把 role 挂在这里会让读屏把整层当作焦点目标，而它既不可聚焦也无内容。
		点背景关闭用 onclick，键盘关闭走 onMount 里注册的 Escape。
	-->
	<button
		type="button"
		class="absolute inset-0 cursor-default"
		aria-label={tr('ext.close')}
		tabindex="-1"
		onclick={onClose}
	></button>
	<div
		class="relative flex h-[560px] w-[760px] flex-col rounded-sm2 border border-surface-4 bg-surface-2 shadow-2xl"
		role="dialog"
		aria-modal="true"
		aria-label={tr('ext.catalogTitle')}
		transition:fly={{ y: prefersReducedMotion.current ? 0 : 8, duration: 150 }}
	>
		<div class="flex shrink-0 items-center gap-3 border-b border-surface-4 px-4 py-3">
			<h2 class="text-[15px] font-semibold text-ink-1">{tr('ext.catalogTitle')}</h2>
			<span class="text-[12px] text-ink-3">{tr('ext.catalogCount').replace('{count}', String(items.length))}</span>
			<input
				type="search"
				placeholder={tr('ext.catalogSearch')}
				class="ml-auto h-7 w-52 rounded-sm2 border border-surface-4 bg-surface-1 px-2 text-[12px] text-ink-1 placeholder:text-ink-3 focus:border-accent focus:outline-none"
				bind:value={query}
			/>
			<label class="flex shrink-0 items-center gap-1.5 text-[12px] text-ink-3">
				<input type="checkbox" class="accent-accent" bind:checked={onlyUpdatable} />
				{tr('ext.onlyUpdatable')}
			</label>
			<button
				class="h-7 w-7 shrink-0 rounded-sm2 text-ink-3 transition hover:bg-surface-3 hover:text-ink-1"
				title={tr('ext.close')}
				onclick={onClose}
			>
				✕
			</button>
		</div>

		{#if error}
			<div class="shrink-0 border-b border-danger/30 bg-danger/10 px-4 py-2 text-[12px] text-danger">
				{error}
			</div>
		{/if}

		<div class="min-h-0 flex-1 overflow-y-auto">
			{#if loading}
				<div class="flex h-full items-center justify-center text-[13px] text-ink-3">
					{tr('ext.catalogLoading')}
				</div>
			{:else if rows.length === 0}
				<div class="flex h-full items-center justify-center px-6 text-center text-[13px] text-ink-3">
					{items.length === 0 ? tr('ext.catalogEmpty') : tr('ext.catalogNoMatch')}
				</div>
			{:else}
				{#each rows as item (item.id)}
					<div class="flex items-center gap-3 border-b border-surface-4/50 px-4 py-2.5 hover:bg-surface-3/60">
						<div class="min-w-0 flex-1">
							<div class="flex items-center gap-2">
								<span class="truncate text-[13px] text-ink-1">{item.name || item.id}</span>
								{#if isInstalled(item)}
									<span class="shrink-0 rounded-sm border border-surface-4 px-1 text-[10px] text-ink-3">
										{tr('ext.badgeInstalled')}
									</span>
								{/if}
								{#if hasUpdate(item)}
									<span class="shrink-0 rounded-sm border border-accent/40 px-1 text-[10px] text-accent">
										{tr('ext.badgeUpdate')}
									</span>
								{:else if legacyClash(item)}
									<span class="shrink-0 rounded-sm border border-surface-4 px-1 text-[10px] text-ink-3">
										{tr('ext.badgeLegacy')}
									</span>
								{/if}
							</div>
							<div class="truncate text-[11px] text-ink-3">{item.tagline ?? item.id}</div>
						</div>

						<span class="w-[76px] shrink-0 truncate text-right text-[12px] tabular-nums text-ink-2">
							{item.version}
						</span>

						<div class="flex w-[150px] shrink-0 items-center justify-end gap-1">
							{#if item.website}
								<button
									class="h-6 rounded-sm2 px-2 text-[11px] text-ink-3 transition hover:bg-surface-3 hover:text-ink-1"
									onclick={() => onOpenPage(item.website ?? '')}
								>
									{tr('ext.web')}
								</button>
							{/if}
							<button
								class="h-6 rounded-sm2 border border-surface-4 px-2 text-[11px] text-ink-2 transition hover:border-surface-5 hover:text-ink-1 disabled:opacity-50"
								disabled={installing !== null}
								onclick={() => onInstall(item)}
							>
								{installing === item.id
									? tr('ext.installing')
									: hasUpdate(item)
										? tr('ext.update')
										: isInstalled(item)
											? tr('ext.reinstall')
											: tr('ext.install')}
							</button>
						</div>
					</div>
				{/each}
			{/if}
		</div>

		<div class="shrink-0 border-t border-surface-4 px-4 py-2 text-[11px] leading-relaxed text-ink-3">
			{tr('ext.catalogFootNote')}
		</div>
	</div>
</div>
