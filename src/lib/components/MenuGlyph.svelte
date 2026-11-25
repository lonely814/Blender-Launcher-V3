<script lang="ts">
	import type { MenuIcon } from '$lib/types';

	/**
	 * 右键菜单图标。
	 *
	 * 原版给 QAction 挂了 16px 图标（folder/settings/delete/file/quick_launch/
	 * favorite/update）。这里用内联 SVG 而不是图标字体或图片：
	 * 颜色跟 `currentColor` 走，选中态白字时图标自动变白，不需要两套资源。
	 */
	let { name }: { name: MenuIcon } = $props();
</script>

<svg
	width="14"
	height="14"
	viewBox="0 0 24 24"
	fill="none"
	stroke="currentColor"
	stroke-width="1.8"
	stroke-linecap="round"
	stroke-linejoin="round"
	aria-hidden="true"
>
	{#if name === 'file'}
		<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
		<path d="M14 3v5h5" />
	{:else if name === 'quickLaunch'}
		<path d="M13 2 3 14h7l-1 8 10-12h-7z" />
	{:else if name === 'favorite'}
		<path d="m12 3 2.8 5.9 6.2.8-4.5 4.3 1.2 6.2-5.7-3.1-5.7 3.1 1.2-6.2L3 9.7l6.2-.8z" />
	{:else if name === 'update'}
		<path d="M12 19V5M5 12l7-7 7 7" />
	{:else if name === 'folder'}
		<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
	{:else if name === 'settings'}
		<circle cx="12" cy="12" r="3" />
		<path
			d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1A1.7 1.7 0 0 0 4.6 9a1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"
		/>
	{:else if name === 'delete'}
		<path d="M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2M6 6l1 14a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-14" />
	{:else if name === 'trash'}
		<path d="M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2" />
		<path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
		<path d="M10 11v6M14 11v6" />
	{/if}
</svg>
