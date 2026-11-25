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

<!--
  原版是 QTabBar(RoundedWest)：窄条 44px + 竖排文字，选中项底色更亮。

  宽度从 68px 收到 44px，与 Qt 的 RoundedWest 竖排页签内边距一致
  （tab 文字左右各 6px + 字号 13px）。收窄后单字宽度刚好够，
  选中态靠底色区分，不靠右侧留白。

  溢出：容器给 min-h-0 + overflow-hidden，每个 tab 用 flex-1 均分高度，
  标签在自身框内 overflow-hidden + text-ellipsis。窗口高度很小时按钮
  不会有最小高度撑着往下溢，而是被压扁——标签跟着被裁成省略号，
  不会顶出容器外。
-->
<nav
	class="mica-surface flex w-[44px] shrink-0 flex-col overflow-hidden border-r border-surface-1 bg-surface-2"
>
	{#each items as item (item.id)}
		<button
			class="relative flex min-h-[36px] flex-1 items-center justify-center overflow-hidden border-b border-surface-1 px-1 transition
				{active === item.id
					? 'bg-surface-4 text-ink-1'
					: 'text-ink-2 hover:bg-surface-3 hover:text-ink-1'}"
			onclick={() => onSelect(item.id)}
			title={item.label}
		>
			<span
				class="max-h-full overflow-hidden text-ellipsis whitespace-nowrap text-[12px] leading-none [writing-mode:vertical-rl] rotate-180"
			>
				{item.label}
			</span>
			{#if item.count}
				<span
					class="absolute bottom-0.5 right-0.5 rounded-full bg-surface-5 px-1 text-[9px] font-semibold leading-tight text-ink-1"
				>
					{item.count}
				</span>
			{/if}
		</button>
	{/each}
</nav>
