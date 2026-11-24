<script lang="ts">
	import '../app.css';
	import { onMount } from 'svelte';
	import { initTheme, setMica, setThemeMode, type Accent, type ThemeMode } from '$lib/theme';
	import { settings } from '$lib/stores';
	import { resolveLanguage, setLocale } from '$lib/i18n';

	// Svelte 5：布局的默认 slot 变成 children snippet。
	let { children } = $props();

	// 尽早落地主题减少首帧闪白；settings 到位后以持久化值为准。
	onMount(() => {
		initTheme('dark');
		// 主题与语言都依赖 settings，但 settings 要等 IPC 回来。
		// 在此之前先用系统语言，避免英文界面闪一下。
		setLocale(resolveLanguage(undefined));
	});

	// settings 加载完成 / 设置页保存，都会把主题、语言、材质同步到 DOM。
	$effect(() => {
		setLocale(resolveLanguage($settings?.language));
	});

	// Mica 在不支持的系统上会自动降级为不透明底。
	$effect(() => {
		setMica($settings?.enable_mica ?? true);
	});

	$effect(() => {
		if ($settings?.theme) {
			setThemeMode($settings.theme as ThemeMode, ($settings.accent ?? 'orange') as Accent);
		}
	});
</script>

{@render children?.()}
