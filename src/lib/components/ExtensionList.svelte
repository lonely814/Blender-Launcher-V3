<script lang="ts">
	import { onMount } from 'svelte';
	import { windowRange } from '$lib/virtual';
	import { locale, makeTranslator } from '$lib/i18n';
	import type { ExtensionScan, InstalledExtension, LegacyAddon } from '$lib/types';

	/**
	 * 插件列表。与 BuildList 同构：行高固定 + 上下占位块，
	 * 只为可见窗口挂 DOM。
	 *
	 * 不复用 BuildList：它的每一列都绑死 `Build` 字段（version/branch/
	 * release_date/update_build_id），改成泛型要把 15 个 props 全部参数化，
	 * 换来的复用只有窗口计算那 4 行——那部分本来就在 $lib/virtual.ts 里。
	 *
	 * 行高 44 而不是 BuildList 的 40：插件行有两行文本（名称 + id·仓库），
	 * 单行放不下，压缩到 40 会截断。
	 */
	const ROW_HEIGHT = 44;

	let {
		scan = null as ExtensionScan | null,
		filter = '',
		onToggle = (_: InstalledExtension | LegacyAddon, __: boolean) => {},
		onReveal = (_: string) => {},
		onRemove = (_: InstalledExtension) => {},
		onManage = (_: InstalledExtension) => {},
		onDetect = () => {}
	}: {
		scan?: ExtensionScan | null;
		filter?: string;
		onToggle?: (item: InstalledExtension | LegacyAddon, enabled: boolean) => void;
		onReveal?: (path: string) => void;
		onRemove?: (item: InstalledExtension) => void;
		/** 打开版本管理（阶段 B）。只有扩展有版本，旧式插件不显示入口。 */
		onManage?: (item: InstalledExtension) => void;
		onDetect?: () => void;
	} = $props();

	const tr = $derived(makeTranslator($locale));

	/** 扩展与旧式插件统一成一个可显示的行，模板不必写两套。 */
	type Row =
		| { kind: 'ext'; item: InstalledExtension }
		| { kind: 'legacy'; item: LegacyAddon };

	const rows = $derived.by<Row[]>(() => {
		if (!scan) return [];
		const needle = filter.trim().toLowerCase();
		const match = (name: string, id: string) =>
			!needle || name.toLowerCase().includes(needle) || id.toLowerCase().includes(needle);

		const list: Row[] = [];
		for (const item of scan.extensions) {
			if (match(item.name, item.pkg_id)) list.push({ kind: 'ext', item });
		}
		for (const item of scan.legacy) {
			if (match(item.name, item.module)) list.push({ kind: 'legacy', item });
		}
		return list;
	});

	let scrollTop = $state(0);
	let viewportHeight = $state(0);
	const virtual = $derived(windowRange(rows.length, ROW_HEIGHT, scrollTop, viewportHeight));
	const slice = $derived(rows.slice(virtual.start, virtual.end));

	function handleScroll(event: Event): void {
		scrollTop = (event.currentTarget as HTMLDivElement).scrollTop;
	}

	/**
	 * `$state` 而不是普通变量：`$effect` 里要读它，普通变量在 runes 模式下
	 * 被读时不会建立依赖（Svelte 会报 non_reactive_update），
	 * 过滤把行数改小后那句钳制 scrollTop 的 effect 就不会重跑。
	 */
	let gridEl = $state<HTMLDivElement | undefined>();

	// 与 BuildList 同理：首帧测量让出一帧，
	// 否则 hydration 前的 clientHeight 是 0，只渲染 overscan 行。
	onMount(() => {
		const node = gridEl;
		if (!node) return;
		const sync = () => {
			const height = node.clientHeight;
			if (height !== viewportHeight) viewportHeight = height;
		};
		const frame = requestAnimationFrame(sync);
		const observer = new ResizeObserver(sync);
		observer.observe(node);
		return () => {
			cancelAnimationFrame(frame);
			observer.disconnect();
		};
	});

	// 过滤后行数变少，scrollTop 可能超出新的最大滚动距离。
	// 只写 DOM 不回写 $state，避免自触发成循环。
	$effect(() => {
		const total = rows.length * ROW_HEIGHT;
		const node = gridEl;
		if (!node) return;
		const max = Math.max(0, total - node.clientHeight);
		if (node.scrollTop > max) node.scrollTop = max;
	});

	function idOf(row: Row): string {
		return row.kind === 'ext' ? `${row.item.repo}.${row.item.pkg_id}` : row.item.module;
	}

	function nameOf(row: Row): string {
		return row.item.name || idOf(row);
	}

	function versionOf(row: Row): string {
		return row.item.version ?? '';
	}

	function pathOf(row: Row): string {
		return row.item.path;
	}

	/** 未探测过时是 `null`，必须显示「未知」而不是「已禁用」。 */
	function enabledOf(row: Row): boolean | null {
		return row.item.enabled;
	}

	function toggle(row: Row): void {
		const current = enabledOf(row);
		// 未知状态下点开关，语义是「设为启用」。
		onToggle(row.item, current !== true);
	}
</script>

{#if !scan}
	<div class="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
		<p class="text-[13px] text-ink-3">{tr('ext.noInstalled')}</p>
	</div>
{:else}
	<div class="flex h-full flex-col">
		<!-- 表头：列宽与行内一致，改动时两处一起改 -->
		<div
			class="flex shrink-0 items-center border-b border-surface-4 bg-surface-1 px-3 py-1.5 text-[12px] font-medium text-ink-3"
		>
			<div class="w-[52px] shrink-0"></div>
			<div class="min-w-0 flex-1">{tr('ext.title')}</div>
			<div class="w-[120px] shrink-0">{tr('col.version')}</div>
			<div class="w-[190px] shrink-0"></div>
		</div>

		<div
			class="min-h-0 flex-1 overflow-y-auto"
			role="grid"
			tabindex="-1"
			bind:this={gridEl}
			onscroll={handleScroll}
		>
			{#if rows.length === 0}
				<div class="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
					<p class="max-w-[320px] text-[13px] leading-relaxed text-ink-3">{tr('ext.empty')}</p>
					<p class="max-w-[320px] text-[12px] leading-relaxed text-ink-3/70">
						{tr('ext.emptyHint')}
					</p>
					{#if onDetect}
						<button
							class="h-7 rounded-sm2 border border-surface-4 px-3 text-[12px] text-ink-2 transition-colors hover:border-surface-5 hover:text-ink-1"
							onclick={onDetect}
						>
							{tr('ext.detect')}
						</button>
					{/if}
				</div>
			{:else}
				<div style="height: {virtual.padTop}px" aria-hidden="true"></div>
				{#each slice as row, index (idOf(row))}
					{@const absolute = virtual.start + index}
					<div
						class="flex items-center text-[13px]
							{absolute % 2 === 0 ? 'bg-surface-2' : 'bg-surface-alt'} hover:bg-surface-4/60"
						style="height: {ROW_HEIGHT}px"
						role="row"
					>
						<div class="flex w-[52px] shrink-0 items-center justify-center">
							<!-- 开关：三态。未知用空心圈 + 问号，不给「关」的视觉暗示 -->
							<button
								class="flex h-6 w-6 items-center justify-center rounded-sm2 transition
									{enabledOf(row) === true
									? 'text-success hover:bg-surface-3'
									: enabledOf(row) === false
										? 'text-ink-3 hover:bg-surface-3'
										: 'text-ink-3/60 hover:bg-surface-3'}"
								title="{enabledOf(row) === true
									? tr('ext.enabledOn')
									: enabledOf(row) === false
										? tr('ext.disabled')
										: tr('ext.unknown')}"
								onclick={() => toggle(row)}
							>
								{#if enabledOf(row) === true}
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6">
										<path d="M4 12l5 5L20 6" />
									</svg>
								{:else if enabledOf(row) === false}
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
										<circle cx="12" cy="12" r="8" />
									</svg>
								{:else}
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
										<circle cx="12" cy="12" r="8" stroke-dasharray="3 3" />
									</svg>
								{/if}
							</button>
						</div>

						<button
							class="min-w-0 flex-1 px-2 text-left"
							onclick={() => onReveal(pathOf(row))}
							title={pathOf(row)}
						>
							<div class="flex items-center gap-2">
								<span class="truncate text-ink-1">{nameOf(row)}</span>
								{#if row.kind === 'legacy'}
									<span class="shrink-0 rounded-sm border border-surface-4 px-1 text-[10px] text-ink-3">
										{tr('ext.legacy')}
									</span>
								{/if}
								{#if row.kind === 'ext' && row.item.managed}
									<span class="shrink-0 rounded-sm border border-accent/40 px-1 text-[10px] text-accent">
										managed
									</span>
								{/if}
							</div>
							<div class="truncate text-[11px] text-ink-3">{idOf(row)}</div>
						</button>

						<div class="w-[120px] shrink-0 truncate px-2 text-[12px] tabular-nums text-ink-2">
							{versionOf(row)}
						</div>

						<div class="flex w-[190px] shrink-0 items-center justify-end gap-1 px-2">
							<button
								class="h-6 rounded-sm2 px-2 text-[11px] text-ink-3 transition hover:bg-surface-3 hover:text-ink-1"
								onclick={() => onReveal(pathOf(row))}
							>
								{tr('ext.reveal')}
							</button>
							{#if row.kind === 'ext'}
								<button
									class="h-6 rounded-sm2 px-2 text-[11px] text-ink-3 transition hover:bg-surface-3 hover:text-ink-1"
									title={tr('ext.manageTitle')}
									onclick={() => onManage(row.item)}
								>
									{tr('ext.manage')}
								</button>
								<button
									class="h-6 rounded-sm2 px-2 text-[11px] text-danger/80 transition hover:bg-danger/10 hover:text-danger"
									onclick={() => onRemove(row.item)}
								>
									{tr('ext.remove')}
								</button>
							{/if}
						</div>
					</div>
				{/each}
				<div style="height: {virtual.padBottom}px" aria-hidden="true"></div>
			{/if}
		</div>
	</div>
{/if}
