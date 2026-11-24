/** @type {import('tailwindcss').Config} */
export default {
	content: ['./src/**/*.{html,js,svelte,ts}'],
	theme: {
		extend: {
			// 颜色全部走 CSS 变量（app.css 的 :root / [data-theme]）。
			// <alpha-value> 让 bg-surface-4/60、ring-accent/50 这类修饰符继续生效，
			// 主题切换时不需要改任何组件类名。
			colors: {
				surface: {
					0: 'rgb(var(--surface-0) / <alpha-value>)',
					1: 'rgb(var(--surface-1) / <alpha-value>)',
					2: 'rgb(var(--surface-2) / <alpha-value>)',
					3: 'rgb(var(--surface-3) / <alpha-value>)',
					4: 'rgb(var(--surface-4) / <alpha-value>)',
					5: 'rgb(var(--surface-5) / <alpha-value>)',
					// 表格交替行（原版 #2D2D2D，比 surface-2 稍亮）
					alt: 'rgb(var(--surface-alt) / <alpha-value>)'
				},
				accent: 'rgb(var(--accent) / <alpha-value>)',
				// accent 作底色时的前景色（随主题与色板反转）
				'accent-ink': 'rgb(var(--accent-ink) / <alpha-value>)',
				success: 'rgb(var(--success) / <alpha-value>)',
				danger: 'rgb(var(--danger) / <alpha-value>)',
				info: 'rgb(var(--info) / <alpha-value>)',
				ink: {
					1: 'rgb(var(--ink-1) / <alpha-value>)',
					2: 'rgb(var(--ink-2) / <alpha-value>)',
					3: 'rgb(var(--ink-3) / <alpha-value>)'
				},
				scrim: 'rgb(var(--scrim) / <alpha-value>)'
			},
			fontFamily: {
				sans: ['var(--font-sans)'],
				mono: ['var(--font-mono)']
			},
			borderRadius: {
				sm2: '2px'
			}
		}
	},
	plugins: []
};
