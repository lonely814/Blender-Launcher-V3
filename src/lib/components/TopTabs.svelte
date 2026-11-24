<script lang="ts">
	/**
	 * Wide 布局的横向子页签。
	 * 与 Classic 的 SideRail 同源数据（items/active/onSelect），
	 * 只是排布从竖排 68px 窄条换成横向一条，把省的宽度全给表格。
	 */
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

<div
	class="mica-surface flex shrink-0 items-center gap-0.5 overflow-x-auto border-b border-surface-4 bg-surface-2 px-2"
	role="tablist"
	tabindex="-1"
>
	{#each items as item (item.id)}
		<button
			class="relative flex shrink-0 items-center gap-1.5 whitespace-nowrap px-3 py-1.5 text-[13px] transition
				{active === item.id
				? 'text-ink-1'
				: 'text-ink-2 hover:text-ink-1'}"
			role="tab"
			aria-selected={active === item.id}
			title={item.label}
			onclick={() => onSelect(item.id)}
		>
			{item.label}
			{#if item.count}
				<span
					class="rounded-full px-1.5 text-[10px] font-semibold
						{active === item.id ? 'bg-accent text-accent-ink' : 'bg-surface-4 text-ink-2'}"
				>
					{item.count}
				</span>
			{/if}
			{#if active === item.id}
				<span class="absolute inset-x-1 bottom-0 h-[2px] bg-accent"></span>
			{/if}
		</button>
	{/each}
</div>
