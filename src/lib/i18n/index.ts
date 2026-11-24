import { writable, get } from 'svelte/store';
import en from './locales/en';
import zh from './locales/zh';

/**
 * 轻量 i18n。
 *
 * 没有引入 i18next / svelte-i18n：文案量约 150 条，框架带来的收益不足以
 * 抵消它对包体和心智的占用。字典是普通对象，key 有类型约束（见 locales/en.ts），
 * 拼错 key 会在编译期报错而不是运行时显示原始 key。
 *
 * 响应式：`t()` 内部读 `get(locale)`，Svelte 4 只在模板里出现 store 引用时
 * 才追踪依赖。组件里统一用 `$: tr = makeTranslator($locale)` 建一个依赖
 * locale 的包装函数，语言切换时整体重算。
 */

export type Lang = 'en' | 'zh';
export type MessageKey = keyof typeof en;

export const LANGUAGES: Array<{ id: Lang; label: string }> = [
	{ id: 'en', label: 'English' },
	{ id: 'zh', label: '简体中文' }
];

export const locale = writable<Lang>('en');

const dicts: Record<Lang, Record<MessageKey, string>> = {
	en,
	zh
};

function format(template: string, vars?: Record<string, string | number>): string {
	if (!vars) return template;
	let text = template;
	for (const [name, value] of Object.entries(vars)) {
		text = text.replaceAll(`{${name}}`, String(value));
	}
	return text;
}

/** 非响应式翻译（事件处理、命令式代码用）。 */
export function t(key: MessageKey, vars?: Record<string, string | number>): string {
	const table = dicts[get(locale)] ?? en;
	return format(table[key] ?? en[key] ?? key, vars);
}

/**
 * 返回一个绑定到具体语言的翻译函数。
 * 在组件里写成 `$: tr = makeTranslator($locale)`，语言一变 tr 就换新，
 * 模板里的 `{tr('...')}` 随之重新求值。
 */
export function makeTranslator(lang: Lang): (key: MessageKey, vars?: Record<string, string | number>) => string {
	const table = dicts[lang] ?? en;
	return (key, vars) => format(table[key] ?? en[key] ?? key, vars);
}

export function setLocale(lang: Lang): void {
	locale.set(lang);
}

/** 当前语言的 BCP-47 标签，交给 Intl.DateTimeFormat 用。 */
export function intlLocale(lang: Lang): string {
	return { en: 'en-US', zh: 'zh-CN' }[lang] ?? 'en-US';
}

/** 从 settings 的 language 字段解析出实际语言；'auto' 时按系统区域猜。 */
export function resolveLanguage(value: string | undefined): Lang {
	if (value && value !== 'auto' && value in dicts) return value as Lang;
	const nav = typeof navigator !== 'undefined' ? navigator.language.toLowerCase() : 'en';
	return nav.startsWith('zh') ? 'zh' : 'en';
}
