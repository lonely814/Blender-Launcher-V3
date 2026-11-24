/**
 * 定高列表的窗口计算。
 *
 * 为什么手写而不是 @tanstack/svelte-virtual：这张表的行高是常量（由主按钮
 * 的 h-7/h-6 加内边距唯一确定，40px / 32px），TanStack 的主要价值——动态测量、
 * ResizeObserver 逐行矫正——在这里为 0，却要多带 @tanstack/virtual-core 与
 * 一层 Svelte 适配。计算部分抽成纯函数后可以用 Node 直接跑断言，没有额外收益
 * 可换。
 *
 * 唯一的硬约束：行高必须真的是常量。BuildList 因此给每行写死了 height，
 * 不依赖内容撑开——否则一行的摘要变两行，下面的偏移就全错。
 */

export interface WindowRange {
	/** 首个渲染行的索引（含 overscan）。 */
	start: number;
	/** 末个渲染行的索引，不含。 */
	end: number;
	/** 顶部占位高度 = start * rowHeight。 */
	padTop: number;
	/** 底部占位高度 = (count - end) * rowHeight。 */
	padBottom: number;
}

/**
 * 视口 → 需要真实挂载的行区间。
 *
 * overscan 是视口外多渲染的行数：滚动时浏览器先触发 scroll 再绘制，
 * 没有 overscan 会在快速滚动时闪出空白。6 行 ≈ 200px，够一次滚轮的量。
 *
 * 首帧视口高度为 0（ResizeObserver 还没回调）时仍返回 overscan 行，
 * 而不是空列表——否则会出现「列表已加载但一片空白」的一帧。
 */
export function windowRange(
	count: number,
	rowHeight: number,
	scrollTop: number,
	viewportHeight: number,
	overscan = 6
): WindowRange {
	if (count <= 0 || rowHeight <= 0) {
		return { start: 0, end: 0, padTop: 0, padBottom: 0 };
	}

	const top = Math.max(0, scrollTop);
	const visible = Math.max(0, viewportHeight);

	// 先按像素算，再夹到 [0, count]；start 单独再夹一次，
	// 避免「滚过头 + 列表突然变短」时 start > count 导致 end < start。
	let start = Math.floor(top / rowHeight) - overscan;
	let end = Math.ceil((top + visible) / rowHeight) + overscan;

	start = clamp(start, 0, count - 1);
	end = clamp(end, start, count);

	return {
		start,
		end,
		padTop: start * rowHeight,
		padBottom: (count - end) * rowHeight
	};
}

function clamp(value: number, min: number, max: number): number {
	return Math.min(Math.max(value, min), max);
}
