<script lang="ts">
	import { untrack } from 'svelte';
	import { fade, fly } from 'svelte/transition';
	import { prefersReducedMotion } from 'svelte/motion';
	import * as api from '$lib/api';
	import type { BuildInfoInput } from '$lib/types';
	import { locale, makeTranslator } from '$lib/i18n';

	let {
		path = '',
		/** 编辑已有构建时传入，字段预填（原版 CustomBuildDialogWindow(path, old_build_info)）。 */
		initial = null,
		onClose = () => {},
		onSaved = () => {}
	}: {
		path?: string;
		initial?: Partial<BuildInfoInput> | null;
		onClose?: () => void;
		onSaved?: () => void;
	} = $props();

	// 这些字段是「初始值即终值」：对话框由父组件按 path 挂载/卸载，
	// 打开期间 props 不会变，之后也不该被外部变化覆盖用户输入。
	// 用 untrack 明确表达这个意图，而不是让编译器猜。
	let executable = $state(untrack(() => initial?.executable ?? ''));
	let subversion = $state(untrack(() => initial?.subversion ?? ''));
	let buildHash = $state(untrack(() => initial?.build_hash ?? ''));
	let commitTime = $state(untrack(() => initial?.commit_time ?? ''));
	let branch = $state(untrack(() => initial?.branch ?? ''));
	let customName = $state(untrack(() => initial?.custom_name ?? ''));

	let executables = $state<string[]>([]);
	let busy = $state(false);
	let error = $state('');
	const tr = $derived(makeTranslator($locale));

	const versionValid = $derived(/^\d+\.\d+/.test(subversion.trim()));
	const exeValid = $derived(executable.trim().length > 0);
	const canAccept = $derived(versionValid && exeValid && !busy);

	async function loadExecutables(): Promise<void> {
		try {
			executables = await api.listExecutables(path);
			if (!executable && executables.length) executable = executables[0];
		} catch (err) {
			error = String(err);
		}
	}
	void loadExecutables();

	/** 跑 `blender -v` 填字段（原版 Auto-detect information）。 */
	async function autoDetect(): Promise<void> {
		busy = true;
		error = '';
		try {
			const info = await api.detectBuildInfo(path, executable);
			if (!subversion) subversion = info.subversion ?? '';
			if (!buildHash) buildHash = info.build_hash ?? '';
			if (!branch) branch = info.branch ?? '';
			if (!customName) customName = info.custom_name ?? '';
			commitTime = info.commit_time ?? commitTime;
		} catch (err) {
			error = String(err);
		} finally {
			busy = false;
		}
	}

	async function accept(): Promise<void> {
		busy = true;
		error = '';
		try {
			await api.saveCustomBuild({
				path,
				executable,
				subversion: subversion.trim(),
				build_hash: buildHash || null,
				commit_time: commitTime || null,
				branch: branch || null,
				custom_name: customName
			});
			onSaved();
			onClose();
		} catch (err) {
			error = String(err);
		} finally {
			busy = false;
		}
	}

	const folderLabel = $derived(path.split(/[\\/]/).slice(-2).join('/'));
</script>

<!-- 遮罩淡入、面板轻微上浮：150ms 足够表达「这是新出现的一层」，
     再长就开始挡操作。prefersReducedMotion 时不位移，只保留不透明度变化。 -->
<div
	class="fixed inset-0 z-50 flex items-center justify-center bg-scrim/50"
	role="dialog"
	aria-modal="true"
	transition:fade={{ duration: 120 }}
>
	<div
		class="w-[520px] rounded-sm2 border border-surface-4 bg-surface-2 p-4 shadow-2xl"
		transition:fly={{ y: prefersReducedMotion.current ? 0 : 8, duration: 150 }}
	>
		<h2 class="mb-1 text-[15px] font-semibold text-ink-1">{tr('dialog.title')}</h2>
		<p class="mb-3 text-xs text-ink-3">{tr('dialog.subtitle')}: <span class="font-mono">{folderLabel}</span></p>

		<div class="mb-2 flex items-center gap-2 text-xs text-ink-2">
			<span class="text-danger">*</span>
			<span class="w-28 shrink-0">{tr('dialog.executable')}:</span>
			<select
				class="min-w-0 flex-1 rounded-sm2 border border-surface-4 bg-surface-3 px-2 py-1 text-sm text-ink-1 focus:border-accent focus:outline-none"
				bind:value={executable}
			>
				{#if !executables.includes(executable) && executable}
					<option value={executable}>{executable}</option>
				{/if}
				{#each executables as name (name)}
					<option value={name}>{name}</option>
				{/each}
			</select>
		</div>

		<button
			class="mb-3 rounded-sm2 bg-surface-4 px-3 py-1 text-xs text-ink-1 hover:text-white disabled:opacity-50"
			disabled={!exeValid || busy}
			onclick={autoDetect}
		>
			{tr('dialog.autodetect')}
		</button>

		<div class="space-y-2 text-xs text-ink-2">
			<label class="flex items-center gap-2">
				<span class="w-28 shrink-0">{tr('dialog.subversion')}:</span>
				<input
					class="min-w-0 flex-1 rounded-sm2 border bg-surface-3 px-2 py-1 text-sm text-ink-1 focus:outline-none
						{versionValid || !subversion ? 'border-surface-4 focus:border-accent' : 'border-danger'}"
					bind:value={subversion}
					placeholder="4.5.0"
				/>
			</label>
			<label class="flex items-center gap-2">
				<span class="w-28 shrink-0">{tr('dialog.hash')}:</span>
				<input
					class="min-w-0 flex-1 rounded-sm2 border border-surface-4 bg-surface-3 px-2 py-1 font-mono text-sm text-ink-1 focus:border-accent focus:outline-none"
					bind:value={buildHash}
				/>
			</label>
			<label class="flex items-center gap-2">
				<span class="w-28 shrink-0">{tr('dialog.commitTime')}:</span>
				<input
					class="min-w-0 flex-1 rounded-sm2 border border-surface-4 bg-surface-3 px-2 py-1 text-sm text-ink-1 focus:border-accent focus:outline-none"
					bind:value={commitTime}
					placeholder="2025-02-11T10:33:00Z"
				/>
			</label>
			<label class="flex items-center gap-2">
				<span class="w-28 shrink-0">{tr('dialog.branch')}:</span>
				<input
					class="min-w-0 flex-1 rounded-sm2 border border-surface-4 bg-surface-3 px-2 py-1 text-sm text-ink-1 focus:border-accent focus:outline-none"
					bind:value={branch}
					placeholder="custom"
				/>
			</label>
			<label class="flex items-center gap-2">
				<span class="w-28 shrink-0">{tr('dialog.customName')}:</span>
				<input
					class="min-w-0 flex-1 rounded-sm2 border border-surface-4 bg-surface-3 px-2 py-1 text-sm text-ink-1 focus:border-accent focus:outline-none"
					bind:value={customName}
				/>
			</label>
		</div>

		{#if error}
			<p class="mt-3 break-words text-xs text-danger">{error}</p>
		{/if}

		<div class="mt-4 flex justify-end gap-2">
			<button
				class="rounded-sm2 bg-accent px-4 py-1.5 text-sm font-semibold text-accent-ink hover:bg-accent/85 disabled:opacity-50"
				disabled={!canAccept}
				onclick={accept}
			>
				{busy ? tr('dialog.saving') : tr('dialog.accept')}
			</button>
			<button
				class="rounded-sm2 bg-surface-4 px-4 py-1.5 text-sm text-ink-1 hover:text-white"
				onclick={onClose}
			>
				{tr('dialog.cancel')}
			</button>
		</div>
	</div>
</div>
