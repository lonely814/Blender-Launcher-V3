<script lang="ts">
	import { onMount } from 'svelte';
	import { locale, makeTranslator } from '$lib/i18n';
	import type { ExtensionManage, ExtensionVersion, InstalledExtension } from '$lib/types';

	/**
	 * 单个扩展的版本管理对话框（阶段 B）。
	 *
	 * 做四件事：接管、切换版本、从官方站添加某个历史版本、解除接管。
	 * 所有文件操作都在 Rust 侧，这里只负责把「用户点了什么」翻译成一次
	 * 命令调用，并把失败原文显示出来——接管会移动用户的插件目录，
	 * 任何失败都必须让用户看见原因，不能吞掉。
	 */
	let {
		item,
		managed = null as ExtensionManage | null,
		busy = false,
		error = '',
		history = [] as ExtensionVersion[],
		historyLoading = false,
		historyError = '',
		adding = null as string | null,
		onAdopt = () => {},
		onSwitch = (_: string) => {},
		onAddVersion = (_: string) => {},
		onRelease = () => {},
		onClose = () => {}
	}: {
		item: InstalledExtension;
		managed?: ExtensionManage | null;
		busy?: boolean;
		error?: string;
		/** 官方站的历史版本；为空表示拿不到或还没加载。 */
		history?: ExtensionVersion[];
		historyLoading?: boolean;
		historyError?: string;
		/** 正在下载的版本号，用来只禁用那一行。 */
		adding?: string | null;
		onAdopt?: () => void;
		onSwitch?: (version: string) => void;
		onAddVersion?: (version: string) => void;
		onRelease?: () => void;
		onClose?: () => void;
	} = $props();

	const tr = $derived(makeTranslator($locale));

	/**
	 * 是否已接管。注意 `managed` 这个 prop 是整个 ExtensionManage 对象，
	 * 判断时必须看它的 `managed` 布尔字段——直接写 `{#if managed}` 只判断对象
	 * 是否存在，会让没接管过的包也显示「解除接管」并且永远不显示「接管」按钮。
	 */
	const adopted = $derived(managed?.managed === true);

	/** 当前生效的版本 = 中央库路径的末段；未接管时用 manifest 里的版本。 */
	const currentVersion = $derived.by(() => {
		if (managed?.central_path) {
			const parts = managed.central_path.split(/[\\/]/);
			return parts[parts.length - 1] ?? item.version;
		}
		return item.version;
	});

	/** 中央库里还没有的官方版本——已经有的不必再下载（点一下就能切）。 */
	const downloadable = $derived.by(() => {
		const local = new Set(managed?.versions ?? []);
		return history.filter((entry) => !local.has(entry.version));
	});

	onMount(() => {
		const onKey = (event: KeyboardEvent) => {
			if (event.key === 'Escape') onClose();
		};
		window.addEventListener('keydown', onKey);
		return () => window.removeEventListener('keydown', onKey);
	});
</script>
<div class="fixed inset-0 z-50 flex items-center justify-center bg-scrim/50">
	<button
		type="button"
		class="absolute inset-0 cursor-default"
		tabindex="-1"
		aria-label={tr('ext.close')}
		onclick={onClose}
	></button>

	<div
		class="relative flex w-[520px] flex-col rounded-sm2 border border-surface-4 bg-surface-2 shadow-2xl"
		role="dialog"
		aria-modal="true"
		aria-label={tr('ext.manageTitle')}
	>
		<div class="border-b border-surface-4 px-5 py-3">
			<h2 class="text-[15px] font-semibold text-ink-1">{item.name || item.pkg_id}</h2>
			<p class="mt-0.5 truncate text-[11.5px] text-ink-3">{item.repo}.{item.pkg_id}</p>
		</div>

		<div class="flex max-h-[420px] flex-col gap-3 overflow-y-auto px-5 py-4">
			<div class="flex items-baseline justify-between text-[13px]">
				<span class="text-ink-3">{tr('ext.currentVersion')}</span>
				<span class="tabular-nums text-ink-1">{currentVersion}</span>
			</div>

			{#if managed && adopted && managed.central_path}
				<div class="flex items-baseline justify-between gap-4 text-[13px]">
					<span class="shrink-0 text-ink-3">{tr('ext.centralPath')}</span>
					<span class="truncate text-right text-[11.5px] text-ink-2" title={managed.central_path}>
						{managed.central_path}
					</span>
				</div>
			{/if}

			{#if managed && adopted && managed.versions.length > 0}
				<div class="mt-1">
					<div class="mb-1.5 text-[12px] text-ink-3">{tr('ext.availableVersions')}</div>
					<div class="overflow-hidden rounded-sm2 border border-surface-4">
						{#each managed.versions as version (version)}
							<button
								class="flex w-full items-center justify-between px-3 py-2 text-left text-[13px] transition-colors
									{version === currentVersion
									? 'bg-accent/15 text-ink-1'
									: 'text-ink-2 hover:bg-surface-3 hover:text-ink-1'}"
								disabled={busy}
								onclick={() => onSwitch(version)}
							>
								<span class="tabular-nums">{version}</span>
								{#if version === currentVersion}
									<span class="text-[11px] text-accent">{tr('ext.versionInUse')}</span>
								{/if}
							</button>
						{/each}
					</div>
				</div>
			{:else}
				<p class="rounded-sm2 border border-surface-4 bg-surface-1 px-3 py-2 text-[12px] leading-relaxed text-ink-3">
					{tr('ext.adoptHint')}
				</p>
			{/if}

			<!-- 官方站的历史版本：只列出中央库里还没有的那些 -->
			<div class="mt-1">
				<div class="mb-1.5 flex items-baseline gap-2">
					<span class="text-[12px] text-ink-3">{tr('ext.historyTitle')}</span>
					{#if historyLoading}
						<span class="text-[11px] text-ink-3/70">{tr('ext.historyLoading')}</span>
					{/if}
				</div>
				{#if historyError}
					<p class="text-[11.5px] leading-relaxed text-warning">{historyError}</p>
				{:else if !historyLoading && history.length === 0}
					<p class="text-[11.5px] leading-relaxed text-ink-3/80">{tr('ext.historyEmpty')}</p>
				{:else if downloadable.length === 0}
					<p class="text-[11.5px] leading-relaxed text-ink-3/80">{tr('ext.historyAllLocal')}</p>
				{:else}
					<div class="overflow-hidden rounded-sm2 border border-surface-4">
						{#each downloadable as entry (entry.version)}
							<div class="flex items-center justify-between px-3 py-1.5 text-[13px]">
								<span class="tabular-nums text-ink-2">{entry.version}</span>
								<button
									class="h-6 rounded-sm2 border border-surface-4 px-2 text-[11px] text-ink-2 transition-colors hover:border-surface-5 hover:text-ink-1 disabled:opacity-50"
									disabled={busy || adding !== null}
									onclick={() => onAddVersion(entry.version)}
								>
									{adding === entry.version ? tr('ext.historyAdding') : tr('ext.historyAdd')}
								</button>
							</div>
						{/each}
					</div>
					<p class="mt-1.5 text-[11px] leading-relaxed text-ink-3/70">
						{tr('ext.historyHint')}
					</p>
				{/if}
			</div>

			{#if error}
				<p class="rounded-sm2 border border-danger/40 bg-danger/5 px-3 py-2 text-[12px] leading-relaxed text-danger">
					{error}
				</p>
			{/if}
		</div>

		<div class="flex items-center justify-between gap-2 border-t border-surface-4 px-5 py-3">
			{#if adopted}
				<button
					class="h-8 rounded-sm2 px-3 text-[13px] text-danger/80 transition-colors hover:bg-danger/10 hover:text-danger disabled:opacity-50"
					disabled={busy}
					onclick={onRelease}
				>
					{tr('ext.release')}
				</button>
			{:else}
				<span></span>
			{/if}
			<div class="flex items-center gap-2">
				<button
					class="h-8 rounded-sm2 border border-surface-4 px-3 text-[13px] text-ink-2 transition-colors hover:text-ink-1"
					onclick={onClose}
				>
					{tr('ext.close')}
				</button>
				{#if !adopted}
					<button
						class="h-8 rounded-sm2 bg-accent px-3 text-[13px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
						disabled={busy}
						onclick={onAdopt}
					>
						{busy ? tr('ext.adopting') : tr('ext.adopt')}
					</button>
				{/if}
			</div>
		</div>
	</div>
</div>
