import type { Platform } from './types';

/** 从 webview UA 猜测本机平台（下载按钮的启用判断用）。 */
export function localPlatform(): Platform | null {
	const ua = typeof navigator !== 'undefined' ? navigator.userAgent : '';
	if (/Windows/i.test(ua)) return 'windows';
	if (/Macintosh|Mac OS X/i.test(ua)) return 'macos';
	if (/Linux/i.test(ua)) return 'linux';
	return null;
}
