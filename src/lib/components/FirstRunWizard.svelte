<script lang="ts">
	import { fade, fly } from 'svelte/transition';
	import { prefersReducedMotion } from 'svelte/motion';
	import * as api from '$lib/api';
	import { settings, reloadSettings } from '$lib/stores';
	import { locale, makeTranslator } from '$lib/i18n';

	let { onDone = () => {} }: { onDone?: () => void } = $props();

	let step = $state(0);
	// 初始值只在首次渲染读一次 settings；向导本身就负责让用户改这个值，
	// 后续 settings 变化不应覆盖用户已输入的路径。
	let libraryDir = $state($settings?.install_dir ?? '');
	/** 仓库可见性（原版 show_* 开关；本实现用 scrape_* 落地）。 */
	let visible = $state<Record<string, boolean>>({
		Stable: true,
		Daily: true,
		'Experimental and Patch': true,
		Bforartists: true,
		UPBGE: false,
		'UPBGE Weekly': false
	});
	let runInBackground = $state(true);
	let busy = $state(false);
	let error = $state('');
	const tr = $derived(makeTranslator($locale));

	const steps = ['Welcome', 'Library', 'Repositories', 'Background'];

	async function browse(): Promise<void> {
		try {
			const picked = await api.pickDirectory();
			if (picked) libraryDir = picked;
		} catch (err) {
			error = String(err);
		}
	}

	async function finish(): Promise<void> {
		busy = true;
		error = '';
		try {
			if ($settings) {
				await api.updateSettings({
					...$settings,
					install_dir: libraryDir || $settings.install_dir,
					scrape_bforartists: visible['Bforartists'],
					scrape_upbge: visible['UPBGE'] || visible['UPBGE Weekly'],
					show_tray_icon: runInBackground
				});
				await reloadSettings();
				await api.scanLibrary();
			}
			await api.setFirstRunDone();
			await reloadSettings();
			onDone();
		} catch (err) {
			error = String(err);
		} finally {
			busy = false;
		}
	}
</script>

<div
	class="fixed inset-0 z-50 flex items-center justify-center bg-scrim/60"
	role="dialog"
	aria-modal="true"
	transition:fade={{ duration: 120 }}
>
	<div
		class="flex w-[560px] flex-col rounded-sm2 border border-surface-4 bg-surface-2 shadow-2xl"
		transition:fly={{ y: prefersReducedMotion.current ? 0 : 8, duration: 150 }}
	>
		<div class="border-b border-surface-4 px-5 py-3">
			<h2 class="text-[15px] font-semibold text-ink-1">
				{#if step === 0}
					{tr('wizard.welcome.title')}
				{:else if step === 1}
					{tr('wizard.library.title')}
				{:else if step === 2}
					{tr('wizard.repos.title')}
				{:else}
					{tr('wizard.background.title')}
				{/if}
			</h2>
			<p class="mt-1 text-xs text-ink-3">{tr('wizard.step', { current: step + 1, total: steps.length })}</p>
		</div>

		<div class="min-h-[220px] px-5 py-4 text-sm text-ink-1">
			{#if step === 0}
				<p class="leading-relaxed text-ink-2">
					{tr('wizard.welcome.body')}
				</p>
			{:else if step === 1}
				<p class="mb-3 text-xs text-ink-3">
					{tr('wizard.library.body')}
				</p>
				<div class="flex gap-2">
					<input
						class="min-w-0 flex-1 rounded-sm2 border border-surface-4 bg-surface-3 px-3 py-2 text-sm text-ink-1 focus:border-accent focus:outline-none"
						bind:value={libraryDir}
					/>
					<button
						class="shrink-0 rounded-sm2 bg-surface-4 px-3 py-2 text-sm text-ink-1 hover:text-white"
						onclick={browse}
					>
						Browse…
					</button>
				</div>
			{:else if step === 2}
				<p class="mb-3 text-xs text-ink-3">
					{tr('wizard.repos.body')}
				</p>
				<div class="space-y-1.5">
					{#each Object.keys(visible) as label (label)}
						<label class="flex items-center gap-2 text-sm">
							<input type="checkbox" class="accent-accent" bind:checked={visible[label]} />
							{label}
						</label>
					{/each}
				</div>
			{:else}
				<p class="mb-3 text-xs text-ink-3">
					{tr('wizard.background.body')}
				</p>
				<label class="flex items-center gap-2 text-sm">
					<input type="checkbox" class="accent-accent" bind:checked={runInBackground} />
					{tr('wizard.background.toggle')}
				</label>
			{/if}

			{#if error}
				<p class="mt-3 break-words text-xs text-danger">{error}</p>
			{/if}
		</div>

		<div class="flex items-center justify-between border-t border-surface-4 px-5 py-3">
			<button
				class="rounded-sm2 px-3 py-1.5 text-sm text-ink-3 hover:text-white"
				onclick={() => void finish()}
			>
				{tr('wizard.skip')}
			</button>
			<div class="flex gap-2">
				{#if step > 0}
					<button
						class="rounded-sm2 bg-surface-4 px-4 py-1.5 text-sm text-ink-1 hover:text-white"
						onclick={() => (step -= 1)}
					>
						{tr('wizard.back')}
					</button>
				{/if}
				{#if step < steps.length - 1}
					<button
						class="rounded-sm2 bg-accent px-4 py-1.5 text-sm font-semibold text-accent-ink hover:bg-accent/85"
						disabled={step === 1 && !libraryDir}
						onclick={() => (step += 1)}
					>
						{tr('wizard.next')}
					</button>
				{:else}
					<button
						class="rounded-sm2 bg-accent px-4 py-1.5 text-sm font-semibold text-accent-ink hover:bg-accent/85 disabled:opacity-50"
						disabled={busy}
						onclick={() => void finish()}
					>
						{busy ? tr('dialog.saving') : tr('wizard.finish')}
					</button>
				{/if}
			</div>
		</div>
	</div>
</div>
