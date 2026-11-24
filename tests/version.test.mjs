/**
 * 版本比较的单测。直接跑：`node tests/version.test.mjs`
 *
 * 用 .mjs 而不是 vitest：本项目的 tests/ 目录下已有同风格的
 * 纯 node 断言脚本（virtual.test.mjs），不为此引入测试框架。
 */
import assert from 'node:assert/strict';
// Node 22.6+ 能直接 import .ts（原生类型擦除），不需要编译步骤，
// 所以测的就是真正会被打包进前端的那份代码，不是复制品。
import { compareVersions, isNewerVersion } from '../src/lib/version.ts';

let passed = 0;
function check(name, actual, expected) {
	assert.equal(actual, expected, `${name}: expected ${expected}, got ${actual}`);
	passed += 1;
}

// 数值比较，不是字符串比较。
check('4.10 > 4.9', compareVersions('4.10', '4.9') > 0, true);
check('4.9 < 4.10', compareVersions('4.9', '4.10') < 0, true);
check('5.3.0 > 5.2.9', compareVersions('5.3.0', '5.2.9') > 0, true);
check('equal', compareVersions('2.2.0', '2.2.0'), 0);

// 段数不同。
check('1.2 == 1.2.0', compareVersions('1.2', '1.2.0'), 0);
check('1.2.1 > 1.2', compareVersions('1.2.1', '1.2') > 0, true);

// 预发布后缀被丢弃。
check('2.2.0-beta == 2.2.0', compareVersions('2.2.0-beta', '2.2.0'), 0);
check('2.2.0+main == 2.2.0', compareVersions('2.2.0+main.abcdef', '2.2.0'), 0);

// 解析不出数字不抛异常。
check('garbage vs 1.0', compareVersions('abc', '1.0') < 0, true);

// isNewerVersion 的边界。
check('newer detected', isNewerVersion('2.3.0', '2.2.0'), true);
check('same is not newer', isNewerVersion('2.2.0', '2.2.0'), false);
check('older is not newer', isNewerVersion('2.1.0', '2.2.0'), false);
check('empty candidate', isNewerVersion('', '2.2.0'), false);
check('empty installed', isNewerVersion('2.2.0', ''), false);

console.log(`version.test.mjs: ${passed} assertions passed`);
