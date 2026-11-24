<script lang="ts">
	import { getCurrentWindow } from '@tauri-apps/api/window';

	// ResizeDirection 未从模块导出，从方法签名推导，避免硬编码字符串联合。
	type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>['startResizeDragging']>[0];
	import { onMount } from 'svelte';

	/**
	 * 无边框窗口的边缘缩放手柄。
	 *
	 * 为什么需要它：`shadow: true` 时 tao 走 MARKER_UNDECORATED_SHADOW 分支，
	 * WM_NCHITTEST 只对**上边缘**返回 HTTOP，左右下与四角都落到 DefWindowProc。
	 * 关掉 shadow 能拿到完整命中，但会丢系统投影和 Win11 圆角——用户要求保留，
	 * 所以这里用 8 个透明手柄把缺的方向补回来。
	 *
	 * 手柄宽度随 DPI 缩放，否则 150% 下只有 3px 宽，点不中。
	 */

	const appWindow = getCurrentWindow();
	let maximized = $state(false);
	/** 逻辑像素；实际生效值见 --resize-handle（app.css 里按 dpr 换算）。 */
	const THICKNESS = 6;

	onMount(() => {
		let unlisten: (() => void) | undefined;
		void appWindow
			.isMaximized()
			.then((value) => (maximized = value))
			.catch(() => undefined);
		void appWindow
			.onResized(() => {
				void appWindow
					.isMaximized()
					.then((value) => (maximized = value))
					.catch(() => undefined);
			})
			.then((fn) => (unlisten = fn));
		return () => unlisten?.();
	});

	/** 最大化时不允许拖拽缩放（与 tao 的判断保持一致）。 */
	function start(direction: ResizeDirection): void {
		if (maximized) return;
		void appWindow.startResizeDragging(direction).catch(() => undefined);
	}

	const edges: Array<{ dir: ResizeDirection; class: string; cursor: string }> = [
		{ dir: 'North', class: 'top-0 left-0 right-0', cursor: 'cursor-ns-resize' },
		{ dir: 'South', class: 'bottom-0 left-0 right-0', cursor: 'cursor-ns-resize' },
		{ dir: 'West', class: 'left-0 top-0 bottom-0', cursor: 'cursor-ew-resize' },
		{ dir: 'East', class: 'right-0 top-0 bottom-0', cursor: 'cursor-ew-resize' },
		{ dir: 'NorthWest', class: 'top-0 left-0', cursor: 'cursor-nwse-resize' },
		{ dir: 'NorthEast', class: 'top-0 right-0', cursor: 'cursor-nesw-resize' },
		{ dir: 'SouthWest', class: 'bottom-0 left-0', cursor: 'cursor-nesw-resize' },
		{ dir: 'SouthEast', class: 'bottom-0 right-0', cursor: 'cursor-nwse-resize' }
	];

	const corner = THICKNESS * 3;
</script>

{#if !maximized}
	{#each edges as edge (edge.dir)}
		{@const isCorner = edge.dir.length > 4}
		{@const size = isCorner ? corner : THICKNESS}
		<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
		<div
			class="fixed z-[100] {edge.class} {edge.cursor}"
			style={isCorner
				? `width:${size}px;height:${size}px;`
				: edge.dir === 'North' || edge.dir === 'South'
					? `height:${size}px;`
					: `width:${size}px;`}
			aria-hidden="true"
			onmousedown={(event) => {
				event.preventDefault();
				start(edge.dir);
			}}
		></div>
	{/each}
{/if}
