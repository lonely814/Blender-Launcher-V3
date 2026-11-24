<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { placeMenu, placeSubmenu } from '$lib/menu-position';
	import type { MenuItem } from '$lib/types';

	let {
		x = 0,
		y = 0,
		build = null,
		items = [],
		onClose = () => {}
	}: {
		x?: number;
		y?: number;
		build?: { version: string } | null;
		items?: MenuItem[];
		onClose?: () => void;
	} = $props();

	/**
	 * 右键菜单。
	 *
	 * 定位不再手算 `innerWidth - 260`：菜单宽度随文案变化（中英文长度差很大），
	 * 固定值必然算错。这里在挂载后用实际尺寸钳制到视口内。
	 *
	 * 曾尝试 @floating-ui/dom，但它的 computePosition 是异步的，在 WebView2 里
	 * 与挂载时机叠加后会拿不到正确尺寸，症状是菜单停在 opacity:0。
	 * 改用同步的 getBoundingClientRect，行为可预测、易调试。
	 */

	let element = $state<HTMLDivElement>();
	let openSub = $state<string | null>(null);
	let activeIndex = $state(0);
	let placed = $state(false);
	let panelLeft = $state(0);
	let panelTop = $state(0);
	let panelMaxHeight = $state(0);

	/**
	 * 挂载后按实际尺寸钳制到视口内（算法见 menu-position.ts，有单测覆盖）。
	 *
	 * 测量前必须先撤掉 max-height：定位时会把这个值写进内联样式，
	 * 首次渲染时它是 0，元素被裁到只剩上下 padding（实测 10px）。
	 * 拿着 10px 去算翻转，菜单会以为「下方放得下」而落到视口外。
	 */
	async function place(): Promise<void> {
		await tick();
		if (!element) return;
		const previous = element.style.maxHeight;
		element.style.maxHeight = 'none';
		const rect = element.getBoundingClientRect();
		element.style.maxHeight = previous;
		const result = placeMenu(
			{ x, y },
			{ width: rect.width, height: rect.height },
			{ width: window.innerWidth, height: window.innerHeight }
		);
		panelLeft = result.left;
		panelTop = result.top;
		panelMaxHeight = result.maxHeight;
		placed = true;
	}

	// onMount 而非 $effect：effect 在 bind:this 之前就可能跑，拿不到 element，
	// 且 effect 里读 element 会让它自己成为依赖而产生重复执行。
	onMount(() => {
		void place();
	});

	let subElements = new Map<string, HTMLDivElement>();

	/** action：把子菜单节点登记到 map，卸载时移除。 */
	function registerPanel(node: HTMLDivElement, label: string) {
		if (label) subElements.set(label, node);
		return {
			destroy() {
				if (label) subElements.delete(label);
			}
		};
	}

	let subLeft = $state(0);
	let subTop = $state(0);

	/** 子菜单定位（算法见 menu-position.ts）。 */
	async function placeSub(label: string): Promise<void> {
		await tick();
		const anchor = element?.querySelector<HTMLElement>(`[data-sub="${label}"]`);
		const panel = subElements.get(label);
		if (!anchor || !panel) return;
		const a = anchor.getBoundingClientRect();
		const p = panel.getBoundingClientRect();
		const result = placeSubmenu(
			{ left: a.left, top: a.top, width: a.width, height: a.height },
			{ width: p.width, height: p.height },
			{ width: window.innerWidth, height: window.innerHeight }
		);
		subLeft = result.left;
		subTop = result.top;
	}

	$effect(() => {
		const label = openSub;
		if (label) void placeSub(label);
	});

	/**
	 * 关闭判定。
	 *
	 * 必须用 click 而不是 mousedown：右键时浏览器先发 mousedown、再发 contextmenu，
	 * 菜单在 contextmenu 里才创建，但那次 mousedown 已经在冒泡路上——
	 * 新挂载的 window 监听器会在同一个 tick 收到它并把菜单立刻关掉。
	 * 症状是「右键毫无反应」。click 只会在完整的按下+抬起之后触发，避开这个竞态。
	 *
	 * 注意 Svelte 5 的 svelte:window 事件监听在挂载时同步注册，
	 * 这个竞态窗口依然存在，所以 click 的选择保持不变。
	 */
	function handleWindowClick(event: MouseEvent): void {
		if (element && !element.contains(event.target as Node)) onClose();
	}

	const actionable = $derived(
		items
			.map((item, index) => ({ item, index }))
			.filter(({ item }) => !item.separator && !item.disabled)
	);

	function move(delta: number): void {
		if (actionable.length === 0) return;
		const position = actionable.findIndex(({ index }) => index === activeIndex);
		const next = (position + delta + actionable.length) % actionable.length;
		activeIndex = actionable[next].index;
	}

	function activate(index: number): void {
		const item = items[index];
		if (!item || item.disabled) return;
		if (item.children?.length) {
			openSub = item.label ?? null;
			return;
		}
		item.action?.();
		onClose();
	}

	function handleKey(event: KeyboardEvent): void {
		switch (event.key) {
			case 'Escape':
				event.preventDefault();
				if (openSub) openSub = null;
				else onClose();
				break;
			case 'ArrowDown':
				event.preventDefault();
				move(1);
				break;
			case 'ArrowUp':
				event.preventDefault();
				move(-1);
				break;
			case 'Enter':
			case ' ':
				event.preventDefault();
				activate(activeIndex);
				break;
			case 'ArrowRight':
				if (items[activeIndex]?.children?.length) {
					event.preventDefault();
					openSub = items[activeIndex].label ?? null;
				}
				break;
			case 'ArrowLeft':
				if (openSub) {
					event.preventDefault();
					openSub = null;
				}
				break;
		}
	}
</script>

<svelte:window onclick={handleWindowClick} />

<div
	bind:this={element}
	class="fixed z-50 min-w-[240px] overflow-y-auto rounded-sm2 border border-surface-4 bg-surface-3 py-1 text-sm text-ink-1 shadow-2xl
		{placed ? 'opacity-100' : 'opacity-0'}"
	style="left: {panelLeft}px; top: {panelTop}px; max-height: {panelMaxHeight}px;"
	role="menu"
	tabindex="-1"
	onkeydown={handleKey}
	onclick={(event) => event.stopPropagation()}
>
	{#if build}
		<div class="truncate px-3 py-1 text-xs text-ink-3">{build.version}</div>
	{/if}
	{#each items as item, index (index)}
		{#if item.separator}
			<div class="my-1 border-t border-surface-4"></div>
		{:else if item.children?.length}
			<div class="relative">
				<button
					data-sub={item.label}
					class="flex w-full items-center justify-between gap-2 px-3 py-1.5 text-left transition
						{item.disabled
						? 'cursor-not-allowed text-ink-3'
						: index === activeIndex
							? 'bg-surface-4 text-white'
							: 'hover:bg-surface-4 hover:text-white'}"
					disabled={item.disabled}
					role="menuitem"
					aria-haspopup="menu"
					aria-expanded={openSub === item.label}
					onmouseenter={() => {
						openSub = item.label ?? null;
						activeIndex = index;
					}}
					onclick={() => (openSub = openSub === item.label ? null : (item.label ?? null))}
				>
					<span>{item.label}</span>
					<span class="text-[10px] text-ink-3">▸</span>
				</button>
				{#if openSub === item.label}
					<div
						use:registerPanel={item.label ?? ''}
						class="fixed z-50 min-w-[200px] rounded-sm2 border border-surface-4 bg-surface-3 py-1 shadow-2xl"
						style="left: {subLeft}px; top: {subTop}px;"
						role="menu"
					>
						{#each item.children as child (child.label)}
							<button
								class="flex w-full items-center gap-2 px-3 py-1.5 text-left transition
									{child.disabled ? 'cursor-not-allowed text-ink-3' : 'hover:bg-surface-4 hover:text-white'}"
								disabled={child.disabled}
								role="menuitem"
								onclick={() => {
									child.action?.();
									onClose();
								}}
							>
								{child.label}
							</button>
						{/each}
					</div>
				{/if}
			</div>
		{:else}
			<button
				class="flex w-full items-center gap-2 px-3 py-1.5 text-left transition
					{item.disabled
					? 'cursor-not-allowed text-ink-3'
					: index === activeIndex
						? 'bg-surface-4 text-white'
						: 'hover:bg-surface-4 hover:text-white'}"
				disabled={item.disabled}
				role="menuitem"
				onmouseenter={() => (activeIndex = index)}
				onclick={() => {
					item.action?.();
					onClose();
				}}
			>
				{item.label ?? ''}
			</button>
		{/if}
	{/each}
</div>
