<script lang="ts">
	/**
	 * 自绘开关。各平台原生 checkbox 的尺寸和圆角都不一样，
	 * 在密集的设置行里会显得参差，所以统一自绘。
	 *
	 * 点击走回调 prop，不再用组件事件：runes 模式下父组件写 `on:click`
	 * 收不到子组件派发的事件（只在 legacy 模式带 createEventDispatcher 才行），
	 * 症状是开关视觉正常、外部状态永远不更新。
	 */
	let {
		checked = false,
		disabled = false,
		label = '',
		onclick = () => {}
	}: {
		checked?: boolean;
		disabled?: boolean;
		/** 无障碍标签：开关本身没有可见文字，必须由调用方给出。 */
		label?: string;
		onclick?: (event: MouseEvent) => void;
	} = $props();
</script>

<button
	role="switch"
	aria-checked={checked}
	aria-label={label}
	{disabled}
	class="relative h-[20px] w-[36px] shrink-0 rounded-full transition-colors duration-150
		{checked ? 'bg-accent' : 'bg-surface-5'}
		{disabled ? 'cursor-not-allowed opacity-60' : 'hover:brightness-110'}"
	{onclick}
>
	<span
		class="absolute top-[3px] h-[14px] w-[14px] rounded-full bg-white shadow-sm transition-all duration-150
			{checked ? 'left-[19px]' : 'left-[3px]'}"
	></span>
</button>
