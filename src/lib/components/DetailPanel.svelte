<script lang="ts">
	import type { Build } from '$lib/types';
	import { formatBytes, formatDateWithTime } from '$lib/format';
	import * as api from '$lib/api';
	import { appError } from '$lib/stores';
	import { locale, makeTranslator, intlLocale } from '$lib/i18n';

	let {
		build = null,
		/** 列表里选中了多少项（多选时提示「正在查看其中一行」）。 */
		selectedCount = 0,
		onLaunch = () => {},
		onClose = () => {}
	}: {
		/** 当前焦点构建；null 时显示空态。 */
		build?: Build | null;
		selectedCount?: number;
		onLaunch?: (build: Build) => void;
		onClose?: () => void;
	} = $props();

	let hash = $state<string | null>(null);
	let notes = $state<string | null>(null);
	let notesLoading = $state(false);
	let busy = $state(false);

	// 焦点变化时重置详情缓存，避免显示上一个构建的 hash。
	// hashFor 故意用普通 let：它不是 UI 状态，若声明成 $state 会被本 effect
	// 自身写入而反复触发，变成死循环。
	let hashFor: string | null = null;
	$effect(() => {
		const id = build?.id ?? null;
		if (id === hashFor) return;
		hashFor = id;
		hash = null;
		notes = null;
	});

	const tr = $derived(makeTranslator($locale));

	async function loadHash(): Promise<void> {
		if (!build) return;
		try {
			hash = await api.getBuildHash(build.id);
		} catch (err) {
			appError.set(String(err));
		}
	}

	async function loadNotes(): Promise<void> {
		if (!build) return;
		notesLoading = true;
		try {
			const details = await api.getBuildDetails(build.id);
			notes = details.release_notes_html ?? tr('detail.noNotes');
		} catch (err) {
			notes = String(err);
		} finally {
			notesLoading = false;
		}
	}

	async function copyHash(): Promise<void> {
		if (!build) return;
		const value = hash ?? build.version;
		await navigator.clipboard.writeText(value);
	}

	async function reveal(): Promise<void> {
		if (!build?.install_path) return;
		try {
			await api.openInExplorer(build.install_path);
		} catch (err) {
			appError.set(String(err));
		}
	}

	const label = 'text-[11.5px] text-ink-3';
	const value = 'mt-0.5 break-all font-mono text-[12px] text-ink-1';
</script>

<aside class="mica-surface flex h-full w-[280px] shrink-0 flex-col border-l border-surface-4 bg-surface-2">
	<div class="flex shrink-0 items-center justify-between border-b border-surface-4 px-3 py-2">
		<h2 class="text-[12px] font-semibold text-ink-2">{tr('detail.title')}</h2>
		<button class="text-ink-2 hover:text-ink-1" title={tr('detail.hide')} onclick={onClose}>✕</button>
	</div>

	{#if !build}
		<div class="flex flex-1 items-center justify-center px-4 text-center text-[13px] text-ink-3">
			{tr('detail.empty')}
		</div>
	{:else}
		<div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">
			{#if selectedCount > 1}
				<p class="rounded-sm2 bg-accent/10 px-2 py-1 text-[11px] text-ink-2">
					{tr('detail.multiSelected', { count: selectedCount })}
				</p>
			{/if}

			<div>
				<div class="truncate text-[15px] font-semibold text-ink-1" title={build.version}>
					{build.version}
				</div>
				<div class="mt-0.5 text-xs capitalize text-ink-2">
					{build.fork} · {build.branch} · {build.platform}/{build.architecture}
				</div>
			</div>

			{#if build.update_build_id}
				<div class="rounded-sm2 border border-accent/40 bg-accent/10 px-2 py-1.5 text-[11px] text-ink-2">
					{tr('detail.updateAvailable')}
				</div>
			{/if}

			<div>
				<div class={label}>{tr('detail.commitTime')}</div>
				<div class={value}>{formatDateWithTime(build.release_date, intlLocale($locale))}</div>
			</div>

			{#if !build.url.startsWith('local:') && !build.url.startsWith('library:')}
				<div>
					<div class={label}>{tr('detail.size')}</div>
					<div class={value}>{build.size_bytes ? formatBytes(build.size_bytes) : tr('detail.unknown')}</div>
				</div>
			{/if}

			{#if build.install_path}
				<div>
					<div class="flex items-center justify-between">
						<span class={label}>{tr('detail.hash')}</span>
						{#if hash === null}
							<button class="text-[11px] text-accent hover:underline" onclick={loadHash}>{tr('detail.load')}</button>
						{:else}
							<button class="text-[11px] text-accent hover:underline" onclick={copyHash}>{tr('detail.copy')}</button>
						{/if}
					</div>
					<div class={value}>{hash ?? '—'}</div>
				</div>

				<div>
					<div class="flex items-center justify-between">
						<span class={label}>{tr('detail.path')}</span>
						<button class="text-[11px] text-accent hover:underline" onclick={reveal}>{tr('detail.open')}</button>
					</div>
					<div class={value}>{build.install_path}</div>
				</div>
			{/if}

			{#if build.release_notes_url}
				<div>
					<div class="flex items-center justify-between">
						<span class={label}>{tr('detail.notes')}</span>
						<button
							class="text-[11px] text-accent hover:underline disabled:opacity-50"
							disabled={notesLoading}
							onclick={loadNotes}
						>
							{notesLoading ? tr('detail.reloading') : tr('detail.fetch')}
						</button>
					</div>
					{#if notes}
						<div class="mt-1 max-h-64 overflow-y-auto rounded-sm2 border border-surface-4 bg-surface-1 p-2 text-[11px] leading-relaxed text-ink-2">
							<!-- release notes 是外部 HTML，按纯文本展示避免注入。 -->
							{notes.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ').trim()}
						</div>
					{/if}
				</div>
			{/if}
		</div>

		{#if build.is_installed}
			<div class="shrink-0 border-t border-surface-4 p-3">
				<button
					class="w-full rounded-sm2 bg-success px-3 py-2 text-sm font-medium text-white transition hover:bg-success/85 disabled:opacity-50"
					disabled={busy}
					onclick={() => {
						busy = true;
						onLaunch(build);
						setTimeout(() => (busy = false), 800);
					}}
				>
					{tr('action.launch')}
				</button>
			</div>
		{/if}
	{/if}
</aside>
