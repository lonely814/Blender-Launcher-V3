<script lang="ts">
	/**
	 * 设置项行：标签在左、控件在右，Windows 11 设置页的原生范式。
	 *
	 * 开关类行用 `toggle` + `checked` + `onclick` 直接声明，
	 * 由本组件把可见标签同时用作开关的无障碍名称——
	 * 手写 `<Switch>` 时很容易漏掉 aria-label，读屏会读成「开关」。
	 * 其他控件放在默认 slot 里。
	 */
	import type { Snippet } from 'svelte';
	import Switch from './Switch.svelte';

	let {
		label = '',
		hint = '',
		disabled = false,
		last = false,
		toggle = false,
		checked = false,
		onclick = () => {},
		children
	}: {
		label?: string;
		/** 次级说明。一行以内讲清后果，不要复述标签。 */
		hint?: string;
		disabled?: boolean;
		/** 分组内最后一行不画分隔线。 */
		last?: boolean;
		/** 渲染成开关行。 */
		toggle?: boolean;
		checked?: boolean;
		onclick?: (event: MouseEvent) => void;
		children?: Snippet;
	} = $props();
</script>

<div
	class="flex items-start gap-4 px-4 py-3 {disabled ? 'opacity-50' : ''}
		{last ? '' : 'border-b border-surface-4/60'}"
>
	<div class="min-w-0 flex-[0_1_46%] pt-[3px]">
		<div class="text-[13px] leading-5 text-ink-1">{label}</div>
		{#if hint}
			<p class="mt-0.5 text-[12px] leading-snug text-ink-3">{hint}</p>
		{/if}
	</div>
	<div class="flex min-w-0 flex-1 items-center justify-end gap-2">
		{#if toggle}
			<Switch {checked} {disabled} {label} {onclick} />
		{:else}
			{@render children?.()}
		{/if}
	</div>
</div>
