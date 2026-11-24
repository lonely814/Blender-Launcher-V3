/**
 * 比较两个版本串，返回负数 / 0 / 正数。
 *
 * 只用于「有没有新版本」这一个判断，不追求 SemVer 完备性：
 * - 数字段按数值比较（`4.10` > `4.9`，字符串比较会得出相反结果）
 * - 段数不同时短的一方补 0（`1.2` == `1.2.0`）
 * - 预发布后缀（`-alpha`、`+main`）直接丢弃：本程序只拿它和目录里的
 *   正式版本号比，`2.2.0-beta` 与 `2.2.0` 视为同版本，宁可漏报一次
 *   更新，也不要在界面上反复提示一个用户装不上的版本
 * - 解析不出数字的段按 0 处理，绝不抛异常（上游 manifest 由第三方手写）
 */
export function compareVersions(a: string, b: string): number {
	const left = numericParts(a);
	const right = numericParts(b);
	const length = Math.max(left.length, right.length);
	for (let index = 0; index < length; index += 1) {
		const diff = (left[index] ?? 0) - (right[index] ?? 0);
		if (diff !== 0) return diff;
	}
	return 0;
}

function numericParts(version: string): number[] {
	const core = version.trim().split(/[-+]/)[0] ?? '';
	return core.split('.').map((part) => {
		const digits = part.match(/^\d+/)?.[0];
		return digits ? Number(digits) : 0;
	});
}

/** 目录版本比已装版本新时才返回 true。相等或更旧都返回 false。 */
export function isNewerVersion(candidate: string, installed: string): boolean {
	if (!candidate || !installed) return false;
	return compareVersions(candidate, installed) > 0;
}
