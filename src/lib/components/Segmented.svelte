<script lang="ts">
	/**
	 * 分段控件。用于主题、代理类型这类互斥的少量选项——
	 * 比下拉框更直观（选项全可见），比一排独立按钮更像一个整体。
	 */
	let {
		items = [],
		value = '',
		onSelect = () => {}
	}: {
		items?: Array<{ id: string; label: string }>;
		value?: string;
		onSelect?: (id: string) => void;
	} = $props();
</script>

<div class="flex overflow-hidden rounded-sm2 border border-surface-4" role="group">
	{#each items as item (item.id)}
		<button
			class="h-8 px-3 text-[13px] transition-colors
				{value === item.id
				? 'bg-accent/15 font-medium text-ink-1'
				: 'text-ink-2 hover:bg-surface-3 hover:text-ink-1'}"
			aria-pressed={value === item.id}
			onclick={() => onSelect(item.id)}
		>
			{item.label}
		</button>
	{/each}
</div>
