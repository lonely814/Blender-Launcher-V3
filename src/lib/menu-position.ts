/**
 * 右键菜单的视口钳制。
 *
 * 从 ContextMenu.svelte 抽出来，是因为这段逻辑有两个真实的坑：
 * 1. 不能假设菜单宽度固定——中文和德语文案长度差很多，`innerWidth - 260` 必然算错。
 * 2. 翻转后仍可能越界（比如菜单比视口还高），必须再夹一次。
 *
 * 纯函数便于单测，也让组件只负责渲染。
 */

export interface MenuRect {
	width: number;
	height: number;
}

export interface Viewport {
	width: number;
	height: number;
}

export interface Placement {
	left: number;
	top: number;
	/** 菜单可用高度上限；超出时内部滚动，而不是被裁掉。 */
	maxHeight: number;
	/** 是否翻转到了鼠标左侧（调试用） */
	flippedX: boolean;
	flippedY: boolean;
}

const EDGE = 8;
const MIN_HEIGHT = 160;

/** 游标点 → 菜单摆放位置。 */
export function placeMenu(
	cursor: { x: number; y: number },
	menu: MenuRect,
	viewport: Viewport
): Placement {
	let left = cursor.x;
	let top = cursor.y;
	let flippedX = false;
	let flippedY = false;

	// 右侧放不下：翻到游标左边；仍越界则贴左边缘。
	if (left + menu.width + EDGE > viewport.width) {
		left = cursor.x - menu.width;
		flippedX = true;
	}
	left = clamp(left, EDGE, Math.max(EDGE, viewport.width - menu.width - EDGE));

	// 下方放不下：翻到游标上方；仍越界则贴上边缘。
	if (top + menu.height + EDGE > viewport.height) {
		top = cursor.y - menu.height;
		flippedY = true;
	}
	top = clamp(top, EDGE, Math.max(EDGE, viewport.height - Math.min(menu.height, viewport.height) - EDGE));

	return {
		left,
		top,
		maxHeight: Math.max(MIN_HEIGHT, viewport.height - top - EDGE),
		flippedX,
		flippedY
	};
}

/** 子菜单相对父级条目摆放：右侧优先，放不下翻左侧。 */
export function placeSubmenu(
	anchor: { left: number; top: number; width: number; height: number },
	panel: MenuRect,
	viewport: Viewport
): { left: number; top: number; flipped: boolean } {
	const fitsRight = anchor.left + anchor.width + panel.width + EDGE <= viewport.width;
	const left = fitsRight
		? anchor.left + anchor.width - 2
		: clamp(anchor.left - panel.width + 2, EDGE, Math.max(EDGE, viewport.width - panel.width - EDGE));
	const top = clamp(anchor.top - 4, EDGE, Math.max(EDGE, viewport.height - panel.height - EDGE));
	return { left, top, flipped: !fitsRight };
}

function clamp(value: number, min: number, max: number): number {
	return Math.min(Math.max(value, min), max);
}
