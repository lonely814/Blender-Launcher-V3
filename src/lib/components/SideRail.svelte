<script lang="ts">
	let {
		items = [],
		active = '',
		onSelect = () => {}
	}: {
		items?: Array<{ id: string; label: string; count?: number }>;
		active?: string;
		onSelect?: (id: string) => void;
	} = $props();
</script>

<!-- 原版是 QTabBar(RoundedWest)：窄条 + 竖排文字，选中项底色更亮 -->
<nav class="mica-surface flex w-[68px] shrink-0 flex-col border-r border-surface-1 bg-surface-2">
	{#each items as item (item.id)}
		<button
			class="relative flex items-center justify-center border-b border-surface-1 py-3 transition
				{active === item.id
					? 'bg-surface-4 text-ink-1'
					: 'text-ink-2 hover:bg-surface-3 hover:text-ink-1'}"
			onclick={() => onSelect(item.id)}
			title={item.label}
		>
			<span class="rotate-180 whitespace-nowrap text-[13px] [writing-mode:vertical-rl]">
				{item.label}
			</span>
			{#if item.count}
				<span
					class="absolute bottom-1 right-1 rounded-full bg-surface-5 px-1.5 text-[10px] font-semibold text-ink-1"
				>
					{item.count}
				</span>
			{/if}
		</button>
	{/each}
</nav>
