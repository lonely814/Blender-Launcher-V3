<script lang="ts">
	import { buildsLoading, buildsRefreshedAt, startRefresh, checkLibrary } from '$lib/stores';
	import { formatClock } from '$lib/format';
	import { locale, makeTranslator } from '$lib/i18n';

	let { version = '', status = 'Ready' }: { version?: string; status?: string } = $props();

	// $derived 依赖 $locale，语言切换时 tr 换新（与 $: 语义相同，但只在该值时重算）。
	const tr = $derived(makeTranslator($locale));
</script>

<footer
	class="mica-surface flex shrink-0 items-center justify-between border-t border-surface-4 bg-surface-2 px-3 py-1.5 text-sm text-ink-2"
>
	<div class="flex items-center gap-2">
		{#if $buildsLoading}
			<span class="text-accent">{tr('status.checking')}</span>
		{:else}
			<button class="font-medium text-ink-1 hover:text-white" onclick={startRefresh}>
				{status === 'Ready' ? tr('action.check') : status}
			</button>
			<span class="text-ink-3">|</span>
			<span>
				{tr('status.lastCheck')}
				{#if $buildsRefreshedAt}
					{formatClock($buildsRefreshedAt)}
				{:else}
					{tr('status.never')}
				{/if}
			</span>
			<button class="ml-2 text-ink-3 hover:text-white" onclick={checkLibrary}>
				{tr('status.scanLibrary')}
			</button>
		{/if}
	</div>
	<span class="text-ink-3">{version}</span>
</footer>
