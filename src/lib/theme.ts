import { writable, get } from 'svelte/store';
import { getCurrentWindow } from '@tauri-apps/api/window';

export type ThemeMode = 'dark' | 'light' | 'system';
export type Accent = 'orange' | 'blue' | 'green' | 'purple' | 'rose';
export type ResolvedTheme = 'dark' | 'light';

/**
 * 主题解析与落地。DOM 是唯一真相：切换只写 documentElement 的
 * data-theme，CSS 变量随之换整套配色，组件不需要知道主题存在。
 *
 * mode='system' 时跟随系统，并订阅系统变化（Windows 切深色模式实时生效）。
 */

export const themeMode = writable<ThemeMode>('dark');
export const resolvedTheme = writable<ResolvedTheme>('dark');
export const accentName = writable<Accent>('orange');
/** Mica 是否实际生效（Win11 + 用户开启）。false 时外壳用不透明底色。 */
export const micaActive = writable(false);

/**
 * Mica 只在 Windows 11 22H2+ 由 DWM 提供。这里做能力探测：
 * UA 里的 Windows 版本不可靠（WebView2 会冻结 UA），改用
 * CSS.supports + 平台判断的组合，失败就当作不支持（回退纯色）。
 */
export function detectMicaSupport(): boolean {
	if (typeof navigator === 'undefined') return false;
	const platform = (navigator as Navigator & { userAgentData?: { platform?: string } })
		.userAgentData?.platform ?? navigator.platform ?? '';
	if (!/win/i.test(platform)) return false;
	// Win11 的 UA 仍显示 "Windows NT 10.0"，靠 UA-CH 的 platformVersion 更准。
	const version = (navigator as Navigator & { userAgentData?: { platformVersion?: string } })
		.userAgentData?.platformVersion;
	if (version) return Number.parseInt(version, 10) >= 13;
	// 拿不到 platformVersion 时保守认为可用（DWM 不支持会静默忽略，只是没有材质）。
	return true;
}

/** 开关 Mica：写 data-mica 并同步 store。 */
export function setMica(enabled: boolean): void {
	const active = enabled && detectMicaSupport();
	micaActive.set(active);
	if (typeof document !== 'undefined') {
		document.documentElement.dataset.mica = active ? 'on' : 'off';
	}
}

const media =
	typeof window !== 'undefined' ? window.matchMedia('(prefers-color-scheme: dark)') : null;

function systemTheme(): ResolvedTheme {
	return media?.matches ? 'dark' : 'light';
}

function apply(mode: ThemeMode, accent?: Accent): void {
	if (typeof document === 'undefined') return;
	const resolved = mode === 'system' ? systemTheme() : mode;
	document.documentElement.dataset.theme = resolved;
	resolvedTheme.set(resolved);
	if (accent) {
		document.documentElement.dataset.accent = accent;
		accentName.set(accent);
	}

	// 原生边框/阴影跟随（无边框窗口下影响系统级投影与 snap 预览）。
	// 权限缺失或 web 环境时静默失败，不影响 CSS 已生效的主题。
	void getCurrentWindow()
		.setTheme(resolved)
		.catch(() => undefined);
}

/** 设置主题模式：更新 store、写 DOM、同步原生窗口。 */
export function setThemeMode(mode: ThemeMode, accent?: Accent): void {
	themeMode.set(mode);
	apply(mode, accent);
}

/** 只换强调色板（不重算明暗）。 */
export function setAccent(accent: Accent): void {
	accentName.set(accent);
	if (typeof document !== 'undefined') document.documentElement.dataset.accent = accent;
}

/** 应用启动时调用一次：初始化当前值并挂上系统变化监听。 */
export function initTheme(mode: ThemeMode, accent: Accent = 'orange'): void {
	apply(mode, accent);
	themeMode.set(mode);
	media?.addEventListener('change', () => {
		if (get(themeMode) === 'system') apply('system');
	});
}

/** 当前模式（供非响应式调用点读取）。 */
export function currentMode(): ThemeMode {
	return get(themeMode);
}
