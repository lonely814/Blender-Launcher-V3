/**
 * windowRange 的边界断言。直接 `node tests/virtual.test.mjs` 运行。
 *
 * Node 24 内置 TypeScript 类型擦除，可以直接 import .ts，
 * 不需要为这一个函数装 vitest。
 */
import assert from 'node:assert/strict';
import { windowRange } from '../src/lib/virtual.ts';

let passed = 0;
function check(name, fn) {
	fn();
	passed += 1;
	console.log(`ok ${passed} - ${name}`);
}

const H = 40;

check('空列表不产生任何行', () => {
	const r = windowRange(0, H, 0, 800);
	assert.deepEqual(r, { start: 0, end: 0, padTop: 0, padBottom: 0 });
});

check('行高为 0 时退化为空（防止除零）', () => {
	assert.deepEqual(windowRange(10, 0, 0, 800), { start: 0, end: 0, padTop: 0, padBottom: 0 });
});

check('首帧视口高度 0 时仍渲染 overscan 行，不出现空白', () => {
	const r = windowRange(701, H, 0, 0);
	assert.equal(r.start, 0);
	assert.ok(r.end > 0, 'end 必须 > 0，否则首帧渲染空列表');
	assert.equal(r.padBottom, (701 - r.end) * H);
});

check('顶部：start 被夹到 0，不出现负偏移', () => {
	const r = windowRange(701, H, 0, 800);
	assert.equal(r.start, 0);
	assert.equal(r.padTop, 0);
	// 800 / 40 = 20 行可见 + 6 行 overscan
	assert.equal(r.end, 26);
});

check('中间：上下占位之和 + 渲染行数 = 总行数', () => {
	const r = windowRange(701, H, 4000, 800);
	assert.equal(r.start, 100 - 6);
	assert.equal(r.end, Math.ceil((4000 + 800) / H) + 6);
	const rendered = r.end - r.start;
	assert.equal(r.padTop + rendered * H + r.padBottom, 701 * H);
});

check('到底：end 被夹到 count，padBottom 为 0', () => {
	// 701 行 * 40 = 28040 总高，视口 800 → 最大 scrollTop = 27240
	const r = windowRange(701, H, 27240, 800);
	assert.equal(r.end, 701);
	assert.equal(r.padBottom, 0);
	assert.ok(r.start < 701);
});

check('滚过头（列表缩短后的残留 scrollTop）：不会让 start > count', () => {
	const r = windowRange(20, H, 999999, 800);
	assert.ok(r.start <= 19, `start=${r.start} 越界`);
	assert.ok(r.end <= 20);
	assert.equal(r.padBottom, 0);
	assert.equal(r.padTop + (r.end - r.start) * H + r.padBottom, 20 * H);
});

check('负 scrollTop（某些 IME / 缩放场景）按 0 处理', () => {
	assert.deepEqual(windowRange(100, H, -200, 800), windowRange(100, H, 0, 800));
});

check('行数少于视口：全部渲染、无占位', () => {
	const r = windowRange(5, H, 0, 800);
	assert.deepEqual(r, { start: 0, end: 5, padTop: 0, padBottom: 0 });
});

check('单行列表', () => {
	const r = windowRange(1, H, 0, 800);
	assert.deepEqual(r, { start: 0, end: 1, padTop: 0, padBottom: 0 });
});

check('紧凑行高 32px 同样成立（算法与行高无关）', () => {
	const r = windowRange(701, 32, 3200, 560);
	assert.equal(r.start, 100 - 6);
	assert.equal(r.padTop + (r.end - r.start) * 32 + r.padBottom, 701 * 32);
});

check('遍历全部滚动位置：不变量恒成立', () => {
	const count = 333;
	for (let top = -100; top <= count * H + 100; top += 37) {
		const r = windowRange(count, H, top, 640);
		assert.ok(r.start >= 0 && r.start < count, `start 越界 top=${top} start=${r.start}`);
		assert.ok(r.end >= r.start && r.end <= count, `end 越界 top=${top} end=${r.end}`);
		assert.equal(r.padTop, r.start * H, `padTop 不一致 top=${top}`);
		assert.equal(r.padBottom, (count - r.end) * H, `padBottom 不一致 top=${top}`);
		assert.ok(
			r.padTop + (r.end - r.start) * H + r.padBottom === count * H,
			`总高不守恒 top=${top}`
		);
	}
});

console.log(`\n${passed}/${passed} passed`);
