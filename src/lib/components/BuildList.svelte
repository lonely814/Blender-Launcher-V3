<script lang="ts">
	import { onMount } from 'svelte';
	import type { Build, BuildRow, Tab } from '$lib/types';
	import { formatDateWithTime } from '$lib/format';
	import { settings } from '$lib/stores';
	import { locale, makeTranslator, intlLocale } from '$lib/i18n';
	import { windowRange } from '$lib/virtual';

	let {
		rows = [],
		mode = 'library',
		selected = new Set<string>(),
		/** 焦点行（详情面板用），与 selected 语义分离：多选时也只有一个焦点。 */
		onFocus = () => {},
		onAction = () => {},
		onUpdate = () => {},
		onContextMenu = () => {},
		onDefault = () => {},
		onSelect = () => {},
		onDeleteSelection = () => {},
		onSort = () => {},
		onReload = null,
		emptyText = 'Nothing to show yet',
		/** 行密度：default = 原版 40px 行高；compact = Wide 布局的 28px。 */
		density = 'default',
		sortKey = 'date',
		sortAsc = false
	}: {
		rows?: BuildRow[];
		mode?: Tab;
		selected?: Set<string>;
		onFocus?: (id: string) => void;
		onAction?: (build: Build) => void;
		onUpdate?: (build: Build) => void;
		onContextMenu?: (build: Build, event: MouseEvent) => void;
		onDefault?: (build: Build, openLast?: boolean) => void;
		onSelect?: (ids: Set<string>) => void;
		onDeleteSelection?: () => void;
		onSort?: (key: 'version' | 'branch' | 'date') => void;
		onReload?: (() => void) | null;
		emptyText?: string;
		density?: 'default' | 'compact';
		sortKey?: 'version' | 'branch' | 'date';
		sortAsc?: boolean;
	} = $props();

	/** 时间列可在「提交时间 / build hash」间切换（原版 DateTimeWidget 行为）。 */
	let showHash = $state(false);

	/** 连选锚点。不是 UI 状态，用普通变量，改动不需要触发重渲染。 */
	let anchor: string | null = null;

	/**
	 * 虚拟滚动状态。
	 *
	 * 行高必须精确等于实际渲染高度，否则偏移会随滚动累积成错位。
	 * 这里同时用常量算偏移、又给每行写死 style height——两层保险：
	 * 即使将来某列内边距改了，行高仍由这里强制，不会让 windowRange 算错。
	 * 常量的来源是行内最高的那个单元格，不是估计值：
	 * default = 按钮容器 h-7(28) + py-1.5(12) = 40；
	 * compact = 按钮容器 h-6(24) + py-0.5(4) = 28。
	 * 改动任何一个高度或内边距类名都必须同步这里，否则行会错位。
	 */
	const ROW_HEIGHT = { default: 40, compact: 28 } as const;

	let scrollTop = $state(0);
	let viewportHeight = $state(0);

	const rowHeight = $derived(ROW_HEIGHT[density]);
	const virtual = $derived(windowRange(rows.length, rowHeight, scrollTop, viewportHeight));
	const slice = $derived(rows.slice(virtual.start, virtual.end));

	function handleScroll(event: Event): void {
		scrollTop = (event.currentTarget as HTMLDivElement).scrollTop;
	}

	/**
	 * 视口高度只能靠测量：它由 flex 布局决定，没有任何常量可推。
	 *
	 * gridEl 用普通变量而不是 $state：它只在本组件内部被读，
	 * 让它参与响应式毫无意义，反而会多一轮无关的失效。
	 *
	 * 首次测量放在 requestAnimationFrame 而不是 onMount 同步段：
	 * SSR 产出的首帧 DOM 在 hydration 完成后才完成布局，
	 * 同步读 clientHeight 可能拿到 0 或 SSG 时的旧值。
	 */
	let gridEl: HTMLDivElement | undefined;

	onMount(() => {
		const node = gridEl;
		if (!node) return;
		const sync = () => {
			const height = node.clientHeight;
			if (height !== viewportHeight) viewportHeight = height;
		};
		// 先让出一帧，确保布局完成再读。
		const frame = requestAnimationFrame(sync);
		const observer = new ResizeObserver(sync);
		observer.observe(node);
		return () => {
			cancelAnimationFrame(frame);
			observer.disconnect();
		};
	});

	// 过滤/排序让行数变少后，scrollTop 可能超出新的最大滚动距离。
	// 只改 DOM 的 scrollTop，不回写 $state：前者会触发 scroll 事件，
	// handleScroll 再把真实值同步回来。若这里直接写 scrollTop 状态，
	// 本 effect 既读又写同一个值，会自触发成循环。
	$effect(() => {
		const total = rows.length * rowHeight;
		const node = gridEl;
		if (!node) return;
		const max = Math.max(0, total - node.clientHeight);
		if (node.scrollTop > max) node.scrollTop = max;
	});

	// $locale 出现在这里，语言切换时整个 tr 重新生成，模板里的 tr(...) 随之更新。
	const tr = $derived(makeTranslator($locale));
	const dateHeader = $derived(mode === 'downloads' ? tr('col.uploadTime') : tr('col.commitTime'));
	// 原版 Show Update Button 开关：关掉就不显示更新图标（但仍保留更新检测数据）。
	const showUpdate = $derived(($settings?.show_update_button ?? true) !== false);
	const compact = $derived(density === 'compact');
	const cellPad = $derived(compact ? 'px-3 py-1' : 'px-3 py-1.5');
	const btnSize = $derived(compact ? 'h-6 text-[12px]' : 'h-7 text-[13px]');

	// 纯函数：读 $state 时由调用点（模板）建立依赖，无需 $derived 包一层。
	function updateOf(b: Build): string | null | undefined {
		return showUpdate ? b.update_build_id : null;
	}
	function arrow(key: string): string {
		return sortKey === key ? (sortAsc ? '▲' : '▼') : '';
	}
	function actionLabel(action: string): string {
		return action === 'download'
			? tr('action.download')
			: action === 'running'
				? tr('action.running')
				: tr('action.launch');
	}
	function dateText(build: Build): string {
		return formatDateWithTime(build.release_date, intlLocale($locale));
	}

	function hashOf(build: Build): string {
		const plus = build.version.lastIndexOf('+');
		if (plus === -1) return build.version;
		const tail = build.version.slice(plus + 1);
		const dot = tail.lastIndexOf('.');
		return dot === -1 ? tail.slice(0, 12) : tail.slice(dot + 1, dot + 13);
	}

	/** 原版 QTableView 语义：单击选中，Ctrl 切换，Shift 连选（仅 Library 页）。 */
	function handleClick(build: Build, event: MouseEvent): void {
		onFocus(build.id);
		if (mode !== 'library') {
			onSelect(new Set([build.id]));
			anchor = build.id;
			return;
		}
		if (event.shiftKey && anchor) {
			const ids = rows.map((row) => row.build.id);
			const from = ids.indexOf(anchor);
			const to = ids.indexOf(build.id);
			if (from !== -1 && to !== -1) {
				const [lo, hi] = from < to ? [from, to] : [to, from];
				onSelect(new Set(ids.slice(lo, hi + 1)));
				return;
			}
		}
		if (event.ctrlKey || event.metaKey) {
			const next = new Set(selected);
			if (next.has(build.id)) next.delete(build.id);
			else next.add(build.id);
			onSelect(next);
			anchor = build.id;
			return;
		}
		onSelect(new Set([build.id]));
		anchor = build.id;
	}

	function handleContext(build: Build, event: MouseEvent): void {
		if (!selected.has(build.id)) onSelect(new Set([build.id]));
		onContextMenu(build, event);
	}

	function handleKey(event: KeyboardEvent): void {
		// 单行与多选都支持 Delete；具体删哪些由上层按 is_installed 过滤。
		if (event.key === 'Delete' && selected.size > 0) {
			event.preventDefault();
			onDeleteSelection();
		}
	}
</script>

<svelte:window onkeydown={handleKey} />

<div class="flex h-full flex-col overflow-hidden">
	<!-- 列头：原版三列可点击排序；Custom 页左侧是 Reload 按钮 -->
	<div class="flex shrink-0 items-center border-b border-surface-1 bg-surface-3 text-[13px] font-semibold text-ink-1">
		{#if onReload}
			<div class="w-[104px] shrink-0 px-1 {compact ? 'py-0.5' : 'py-1.5'}">
				<button
					class="h-6 w-[92px] rounded-sm2 bg-surface-4 text-[12px] text-ink-1 hover:text-white"
					title={tr('action.reload')}
					onclick={onReload}
				>
					{tr('action.reload')}
				</button>
			</div>
		{:else}
			<div class="{compact ? 'w-[98px]' : 'w-[104px]'} shrink-0"></div>
		{/if}
		<button class="min-w-[85px] flex-[0_0_140px] px-3 py-2 text-left hover:text-white" onclick={() => onSort('version')}>
			{tr('col.version')} <span class="text-[10px] text-accent">{arrow('version')}</span>
		</button>
		<button class="min-w-[80px] flex-1 px-3 py-2 text-left hover:text-white" onclick={() => onSort('branch')}>
			{tr('col.branch')} <span class="text-[10px] text-accent">{arrow('branch')}</span>
		</button>
		<button
			class="w-[152px] shrink-0 px-3 py-2 text-right hover:text-white"
			title={showHash ? 'Show commit time' : 'Show build hash'}
			onclick={() => (showHash = !showHash)}
		>
			{showHash ? tr('col.buildHash') : dateHeader} <span class="text-[10px] text-accent">{arrow('date')}</span>
		</button>
		<div class="w-[28px] shrink-0"></div>
	</div>

	<!-- 行高由 ROW_HEIGHT 决定，滚动与测量都挂在这个容器上：
	     只有它是 overflow-y-auto，windowRange 的 scrollTop/viewportHeight 才有意义。 -->
	<div
		class="min-h-0 flex-1 overflow-y-auto"
		role="grid"
		tabindex="-1"
		bind:this={gridEl}
		onscroll={handleScroll}
	>
		{#if rows.length === 0}
			<!-- 空态是行动指引，不是一句判决：给出下一步能做什么 -->
			<div class="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
				<svg
					width="28"
					height="28"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="1.4"
					class="text-ink-3/60"
					aria-hidden="true"
				>
					<rect x="3" y="4" width="18" height="16" rx="2" />
					<path d="M3 9h18M8 13h8M8 16h5" />
				</svg>
				<p class="max-w-[280px] text-[13px] leading-relaxed text-ink-3">{emptyText}</p>
				{#if onReload}
					<button
						class="h-7 rounded-sm2 border border-surface-4 px-3 text-[12px] text-ink-2 transition-colors hover:border-surface-5 hover:text-ink-1"
						onclick={onReload}
					>
						{tr('action.reload')}
					</button>
				{/if}
			</div>
		{:else}
			<!-- 顶部占位：只占高度不渲染，把可见行推到正确位置 -->
			<div style="height: {virtual.padTop}px" aria-hidden="true"></div>
			{#each slice as row, index (row.build.id)}
				{@const absolute = virtual.start + index}
				<!-- 原版行：交替底色 surface-2 / surface-alt，选中态更亮。
				     高度写死为 rowHeight——windowRange 的偏移基于常量，
				     让内容自由撑高会让下面的行整体错位。 -->
				<div
					class="flex cursor-default items-center {compact ? 'text-[12px]' : 'text-[13px]'}
						{selected.has(row.build.id)
						? 'bg-surface-5 ring-1 ring-inset ring-accent/50'
						: absolute % 2 === 0
							? 'bg-surface-2 hover:bg-surface-4/60'
							: 'bg-surface-alt hover:bg-surface-4/60'}
						{row.action === 'running' ? 'ring-1 ring-inset ring-accent/40' : ''}"
					style="height: {rowHeight}px"
					role="row"
					aria-selected={selected.has(row.build.id)}
					tabindex="0"
					onclick={(event) => handleClick(row.build, event)}
					oncontextmenu={(event) => {
						event.preventDefault();
						handleContext(row.build, event);
					}}
					ondblclick={(event) => onDefault(row.build, event.shiftKey)}
					onkeydown={(event) => event.key === 'Enter' && onDefault(row.build, event.shiftKey)}
				>
					<div class="flex shrink-0 items-center gap-1 px-1 {compact ? 'w-[98px] py-0.5' : 'w-[104px] py-1.5'}">
						<button
							class="{btnSize} rounded-sm2 px-2 font-medium text-white transition
								{updateOf(row.build) ? (compact ? 'w-[62px]' : 'w-[70px]') : compact ? 'w-[86px]' : 'w-[92px]'}
								{row.action === 'running'
								? 'bg-accent hover:bg-accent/85'
								: 'bg-success hover:bg-success/85'}"
							onclick={(event) => {
								event.stopPropagation();
								onAction(row.build);
							}}
						>
							{actionLabel(row.action)}
						</button>
					</div>
					<!-- 原版独立的 25px 更新图标按钮，出现在 Launch 与版本号之间 -->
					{#if updateOf(row.build)}
						<div class="w-[22px] shrink-0">
							<button
								class="flex h-6 w-6 items-center justify-center rounded-sm2 text-info transition hover:bg-surface-4 hover:brightness-125"
								title={tr('action.update')}
								onclick={(event) => {
									event.stopPropagation();
									onUpdate(row.build);
								}}
							>
								<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4">
									<path d="M12 19V5M5 12l7-7 7 7" />
								</svg>
							</button>
						</div>
					{/if}
					<div
						class="truncate {cellPad} text-ink-1
							{updateOf(row.build) ? 'min-w-[60px] flex-[0_0_115px]' : 'min-w-[85px] flex-[0_0_140px]'}"
						title={row.build.version}
					>
						{row.build.version}
					</div>
					<div class="min-w-[80px] flex-1 truncate {cellPad} capitalize text-ink-2">
						{row.build.branch}
					</div>
					<div class="w-[152px] shrink-0 truncate {cellPad} text-right tabular-nums text-ink-2">
						{showHash ? hashOf(row.build) : dateText(row.build)}
					</div>
					<div class="flex w-[28px] shrink-0 items-center justify-center">
						{#if row.instances > 0}
							<span
								class="flex h-[17px] min-w-[17px] items-center justify-center rounded-full bg-accent px-1 text-[10px] font-semibold text-accent-ink"
								title={tr('action.runningInstances')}
							>
								{row.instances}
							</span>
						{/if}
					</div>
				</div>
			{/each}
			<div style="height: {virtual.padBottom}px" aria-hidden="true"></div>
		{/if}
	</div>
</div>
