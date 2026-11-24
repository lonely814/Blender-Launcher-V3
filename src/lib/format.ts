/** 字节数 → "406.5 MB"。 */
export function formatBytes(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes <= 0) return '—';
	const units = ['B', 'KB', 'MB', 'GB', 'TB'];
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit += 1;
	}
	const digits = value >= 100 || unit === 0 ? 0 : 1;
	return `${value.toFixed(digits)} ${units[unit]}`;
}

/** bytes/s → "12.3 MB/s"。 */
export function formatSpeed(bps: number): string {
	if (!Number.isFinite(bps) || bps <= 0) return '—';
	return `${formatBytes(bps)}/s`;
}

/** 秒 → "1h 23m" / "5m 12s" / "42s"。 */
export function formatEta(seconds: number): string {
	if (!Number.isFinite(seconds) || seconds <= 0) return '—';
	const total = Math.round(seconds);
	if (total >= 3600) return `${Math.floor(total / 3600)}h ${Math.floor((total % 3600) / 60)}m`;
	if (total >= 60) return `${Math.floor(total / 60)}m ${total % 60}s`;
	return `${total}s`;
}

/** ISO 8601 → 本地日期 "2026-11-22"。 */
export function formatDate(iso: string): string {
	if (!iso) return '—';
	const date = new Date(iso);
	if (Number.isNaN(date.getTime())) return iso.slice(0, 10);
	return date.toISOString().slice(0, 10);
}

/** 进度百分比 0-100，未知总量返回 null。 */
export function progressPercent(progressBytes: number, totalBytes: number): number | null {
	if (totalBytes <= 0) return null;
	return Math.min(100, Math.max(0, (progressBytes / totalBytes) * 100));
}

/**
 * ISO 8601 → 本地化日期时间。
 * 英文保持原版风格 "13 Oct 26, 18:58"（月名缩写 + 两位年），
 * 其它语言交给 Intl.DateTimeFormat，避免手拼出"2026年10月13日"这种硬编码。
 */
export function formatDateWithTime(iso: string, locale = 'en-US'): string {
	if (!iso) return '—';
	const date = new Date(iso);
	if (Number.isNaN(date.getTime())) return iso;

	if (locale.startsWith('en')) {
		const day = String(date.getDate()).padStart(2, '0');
		const month = date.toLocaleString('en-US', { month: 'short' });
		const year = String(date.getFullYear()).slice(-2);
		const time = `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
		return `${day} ${month} ${year}, ${time}`;
	}

	// 四位年：两位年在中文里会变成 "26年" 这种歧义写法。
	return new Intl.DateTimeFormat(locale, {
		year: 'numeric',
		month: '2-digit',
		day: '2-digit',
		hour: '2-digit',
		minute: '2-digit',
		hour12: false
	}).format(date);
}

/** Date → "22:51"。 */
export function formatClock(date: Date): string {
	return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}
