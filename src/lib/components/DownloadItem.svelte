<script lang="ts">
	import { builds } from '$lib/stores';
	import { formatBytes, formatDate, formatEta, formatSpeed, progressPercent } from '$lib/format';
	import type { DownloadState, DownloadTask } from '$lib/types';

	let {
		task,
		onPause = () => {},
		onResume = () => {},
		onCancel = () => {},
		onRetry = () => {}
	}: {
		task: DownloadTask;
		onPause?: (task: DownloadTask) => void;
		onResume?: (task: DownloadTask) => void;
		onCancel?: (task: DownloadTask) => void;
		onRetry?: (task: DownloadTask) => void;
	} = $props();

	const version = $derived($builds.find((build) => build.id === task.build_id)?.version ?? task.build_id);
	const percent = $derived(progressPercent(task.progress_bytes, task.total_bytes));

	const stateColors: Record<DownloadState, string> = {
		queued: 'text-ink-2',
		downloading: 'text-accent',
		extracting: 'text-accent',
		verifying: 'text-accent',
		completed: 'text-success',
		failed: 'text-danger',
		cancelled: 'text-ink-3',
		paused: 'text-accent'
	};
	const stateColor = $derived(stateColors[task.state]);

	const terminalStates: DownloadState[] = ['completed', 'failed', 'cancelled'];
</script>

<div class="rounded-sm2 border border-surface-4 bg-surface-2 p-4">
	<div class="mb-2 flex items-center justify-between gap-3">
		<div class="min-w-0">
			<h3 class="truncate text-sm font-semibold text-white">Blender {version}</h3>
			<p class="text-xs text-ink-3">
				{formatBytes(task.progress_bytes)}
				{#if task.total_bytes > 0}
					/ {formatBytes(task.total_bytes)}
				{/if}
				· {formatSpeed(task.speed_bps)} · ETA {formatEta(task.eta_seconds)}
			</p>
		</div>
		<div class="flex items-center gap-2">
			<span class="text-xs font-medium capitalize {stateColor}">{task.state}</span>
			{#if task.state === 'downloading' || task.state === 'queued'}
				<button
					class="rounded-sm2 bg-surface-4 px-2 py-1 text-xs text-ink-1 hover:text-white"
					onclick={() => onPause(task)}
				>
					Pause
				</button>
			{:else if task.state === 'paused'}
				<button
					class="rounded-sm2 bg-accent px-2 py-1 text-xs font-semibold text-accent-ink"
					onclick={() => onResume(task)}
				>
					Resume
				</button>
			{/if}
			{#if task.state === 'failed'}
				<button
					class="rounded-sm2 bg-accent px-2 py-1 text-xs font-semibold text-accent-ink"
					onclick={() => onRetry(task)}
				>
					Retry
				</button>
			{/if}
			{#if !terminalStates.includes(task.state)}
				<button
					class="rounded-sm2 bg-surface-4 px-2 py-1 text-xs text-ink-1 hover:text-danger"
					onclick={() => onCancel(task)}
				>
					Cancel
				</button>
			{/if}
		</div>
	</div>

	{#if task.error}
		<p class="mb-2 truncate text-xs text-danger" title={task.error}>{task.error}</p>
	{/if}

	<div class="h-1.5 w-full overflow-hidden rounded-full bg-surface-4">
		<div
			class="h-full rounded-full transition-all
				{task.state === 'failed' ? 'bg-danger' : task.state === 'completed' ? 'bg-success' : 'bg-accent'}"
			style="width: {percent ?? (task.state === 'completed' ? 100 : 0)}%"
		></div>
	</div>

	<p class="mt-1 text-[11px] text-ink-3">started {formatDate(task.started_at)}</p>
</div>
