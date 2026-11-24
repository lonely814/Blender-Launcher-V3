<script lang="ts">
	import { settings, reloadSettings } from '$lib/stores';
	import * as api from '$lib/api';
	import { formatBytes } from '$lib/format';
	import { locale, makeTranslator, LANGUAGES, setLocale, type Lang } from '$lib/i18n';
	import { micaActive, setAccent, setMica, setThemeMode, type Accent, type ThemeMode } from '$lib/theme';
	import SettingRow from './SettingRow.svelte';
	import Segmented from './Segmented.svelte';
	import type { AppUpdate, CleanupResult, LibraryStats, Settings as SettingsType } from '$lib/types';

	let { onSaved = () => {} }: { onSaved?: () => void } = $props();

	type SectionKey = 'general' | 'appearance' | 'connection' | 'builds' | 'plugins' | 'library';
	type MsgKey = Parameters<ReturnType<typeof makeTranslator>>[0];

	let section = $state<SectionKey>('general');
	let editable = $state<SettingsType | null>(null);
	let launchArgsText = $state('');
	let saving = $state(false);
	let message = $state<string | null>(null);
	let error = $state<string | null>(null);

	let stats = $state<LibraryStats | null>(null);
	let cleanupResult = $state<CleanupResult | null>(null);
	let scanning = $state(false);
	let wingetBusy = $state(false);
	let wingetRegistered = $state(false);

	// 本程序自身的版本检查（不是 Blender 构建的更新检测）。
	let appVersion = $state('');
	let updateChecking = $state(false);
	let updateResult = $state<AppUpdate | null>(null);
	let updateError = $state('');

	$effect(() => {
		void import('@tauri-apps/api/app')
			.then(({ getVersion }) => getVersion())
			.then((version) => (appVersion = version))
			.catch(() => undefined);
	});

	async function checkUpdate(): Promise<void> {
		updateChecking = true;
		updateError = '';
		updateResult = null;
		try {
			updateResult = await api.checkAppUpdate();
		} catch (err) {
			updateError = String(err);
		} finally {
			updateChecking = false;
		}
	}

	const tr = $derived(makeTranslator($locale));

	// settings 到达后初始化一次编辑副本。用「尚未初始化」做守卫而非每次覆盖，
	// 否则后台 settings_changed 事件会冲掉用户正在编辑但未保存的值。
	$effect(() => {
		const current = $settings;
		if (current && !editable) {
			editable = structuredClone(current);
			launchArgsText = (editable.launch_args ?? []).join(' ');
		}
	});

	// 切到连接页时才去查 WinGet 注册状态，避免无谓的 IPC。
	$effect(() => {
		if (section !== 'connection') return;
		void api
			.isWingetRegistered()
			.then((ok) => (wingetRegistered = ok))
			.catch(() => undefined);
	});

	/** 统一控件外观：同高、无边框重叠、聚焦只亮一圈。 */
	const field =
		'h-8 rounded-sm2 border border-surface-4 bg-surface-3 px-2.5 text-[13px] text-ink-1 ' +
		'outline-none transition-colors focus:border-accent';

	type BoolKey =
		| 'auto_check_updates'
		| 'register_blend_files'
		| 'show_tray_icon'
		| 'launch_on_startup'
		| 'enable_high_dpi'
		| 'scrape_bforartists'
		| 'scrape_upbge'
		| 'show_update_button'
		| 'launch_minimized_to_tray'
		| 'enable_new_builds_notifications'
		| 'enable_mica'
		| 'enable_extension_central';

	function flag(key: BoolKey, value: boolean): void {
		if (editable) editable = { ...editable, [key]: value };
	}

	/** 开关：自绘，避免各平台原生 checkbox 形状不一。 */
	function toggle(key: BoolKey): void {
		flag(key, !editable?.[key]);
	}

	async function save(): Promise<void> {
		if (!editable) return;
		saving = true;
		message = null;
		error = null;
		try {
			const next: SettingsType = {
				...editable,
				launch_args: launchArgsText.split(/\s+/).filter(Boolean),
				max_concurrent_downloads: Math.max(1, Math.min(8, editable.max_concurrent_downloads || 1)),
				// 输入框里清空 = 用默认目录；留空串会让后端当成相对路径。
				extension_central_dir: editable.extension_central_dir?.trim() || null
			};
			await api.updateSettings(next);
			settings.set(next);
			message = tr('settings.saved');
			onSaved();
		} catch (err) {
			error = String(err);
		} finally {
			saving = false;
		}
	}

	async function browseInstallDir(): Promise<void> {
		const picked = await api.pickDirectory();
		if (picked && editable) editable = { ...editable, install_dir: picked };
	}

	/** 中央库目录留空 = 用数据目录，所以允许取消选择即清空。 */
	async function browseCentralDir(): Promise<void> {
		const picked = await api.pickDirectory();
		if (picked && editable) editable = { ...editable, extension_central_dir: picked };
	}

	async function scan(): Promise<void> {
		scanning = true;
		error = null;
		try {
			await api.scanLibrary();
			stats = await api.getLibraryStats();
		} catch (err) {
			error = String(err);
		} finally {
			scanning = false;
		}
	}

	async function cleanup(): Promise<void> {
		error = null;
		try {
			cleanupResult = await api.cleanupLibrary();
		} catch (err) {
			error = String(err);
		}
	}

	async function refreshStats(): Promise<void> {
		try {
			stats = await api.getLibraryStats();
		} catch (err) {
			error = String(err);
		}
	}

	async function registerAssociation(): Promise<void> {
		error = null;
		try {
			await api.registerFileAssociation();
			if ($settings) await api.updateSettings({ ...$settings, register_blend_files: true });
			await reloadSettings();
		} catch (err) {
			error = String(err);
		}
	}

	async function registerWinget(): Promise<void> {
		wingetBusy = true;
		error = null;
		try {
			await api.registerWithWinget();
			wingetRegistered = await api.isWingetRegistered();
		} catch (err) {
			error = String(err);
		} finally {
			wingetBusy = false;
		}
	}

	const sections: Array<{ id: SectionKey; key: MsgKey }> = [
		{ id: 'general', key: 'settings.nav.general' },
		{ id: 'appearance', key: 'settings.nav.appearance' },
		{ id: 'connection', key: 'settings.nav.connection' },
		{ id: 'builds', key: 'settings.nav.builds' },
		{ id: 'plugins', key: 'settings.nav.plugins' },
		{ id: 'library', key: 'settings.nav.library' }
	];

	const themes: Array<{ id: ThemeMode; key: MsgKey }> = [
		{ id: 'dark', key: 'settings.theme.dark' },
		{ id: 'light', key: 'settings.theme.light' },
		{ id: 'system', key: 'settings.theme.system' }
	];

	const layouts: Array<{ id: SettingsType['layout']; key: MsgKey; desc: MsgKey }> = [
		{ id: 'classic', key: 'settings.layout.classic', desc: 'settings.layout.classicDesc' },
		{ id: 'wide', key: 'settings.layout.wide', desc: 'settings.layout.wideDesc' },
		{ id: 'detail', key: 'settings.layout.detail', desc: 'settings.layout.detailDesc' }
	];

	/** 色板预览色块使用 dark 档取值，仅用于展示。 */
	const accents: Array<{ id: Accent; label: string; rgb: string }> = [
		{ id: 'orange', label: 'Orange', rgb: '232 163 61' },
		{ id: 'blue', label: 'Blue', rgb: '92 147 201' },
		{ id: 'green', label: 'Green', rgb: '107 168 92' },
		{ id: 'purple', label: 'Purple', rgb: '168 124 214' },
		{ id: 'rose', label: 'Rose', rgb: '214 110 124' }
	];
</script>

<div class="mx-auto flex h-full w-full max-w-[980px] gap-6 px-6 py-5">
	<!-- 左侧导航：与 Windows 11 设置一致。顶部分页在窄窗口下会折行，竖排不会。 -->
	<nav class="w-[168px] shrink-0">
		<h1 class="mb-3 flex h-[22px] items-center px-3 text-[15px] font-semibold text-ink-1">{tr('settings.title')}</h1>
		<ul class="space-y-0.5">
			{#each sections as item (item.id)}
				<li>
					<button
						class="relative w-full rounded-sm2 px-3 py-1.5 text-left text-[13px] transition-colors
							{section === item.id
							? 'bg-surface-4 font-medium text-ink-1'
							: 'text-ink-2 hover:bg-surface-3 hover:text-ink-1'}"
						onclick={() => (section = item.id)}
					>
						{#if section === item.id}
							<span
								class="absolute left-0 top-1/2 h-4 w-[2px] -translate-y-1/2 rounded-full bg-accent"
							></span>
						{/if}
						{tr(item.key)}
					</button>
				</li>
			{/each}
		</ul>
	</nav>

	<div class="flex min-w-0 flex-1 flex-col">
		<!-- 与左侧标题同高的占位，让右侧首组标题和「设置」基线下沉一致 -->
		<div class="mb-3 h-[22px] shrink-0"></div>
		<div class="min-h-0 flex-1 overflow-y-auto pb-4">
			{#if editable}
				{#if section === 'general'}
					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.general.paths')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.general.libraryFolder')} hint={tr('settings.general.libraryFolderHint')}>
								<input class="{field} min-w-0 flex-1 max-w-[320px]" title={editable.install_dir} bind:value={editable.install_dir} />
								<button class="h-8 rounded-sm2 bg-surface-4 px-3 text-[13px] text-ink-1 transition-colors hover:bg-surface-5" onclick={browseInstallDir}>
									{tr('settings.general.browse')}
								</button>
							</SettingRow>
							<SettingRow label={tr('settings.general.cacheFolder')} hint={tr('settings.general.cacheFolderHint')} last>
								<input class="{field} min-w-0 flex-1 max-w-[320px]" title={editable.cache_dir} bind:value={editable.cache_dir} />
							</SettingRow>
						</div>
					</section>

					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.general.association')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.general.register')} hint={tr('settings.general.registerHint')} last>
								{#if $settings?.register_blend_files}
									<span class="text-[12px] text-success">{tr('settings.general.registered')}</span>
								{:else}
									<button class="h-8 rounded-sm2 bg-surface-4 px-3 text-[13px] text-ink-1 transition-colors hover:bg-surface-5" onclick={registerAssociation}>
										{tr('settings.general.register')}
									</button>
								{/if}
							</SettingRow>
						</div>
					</section>

					<section>
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.general.about')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.general.appVersion')} last>
								<span class="text-[12px] tabular-nums text-ink-2">{appVersion}</span>
								<button
									class="ml-3 h-8 rounded-sm2 bg-surface-4 px-3 text-[13px] text-ink-1 transition-colors hover:bg-surface-5 disabled:opacity-50"
									disabled={updateChecking}
									onclick={checkUpdate}
								>
									{updateChecking ? tr('settings.general.checkingUpdate') : tr('settings.general.checkUpdate')}
								</button>
							</SettingRow>
						</div>
						{#if updateResult}
							<p class="mt-2 px-1 text-[11.5px] text-ink-3">
								{#if updateResult.update_available}
									{tr('settings.general.updateAvailable')
										.replace('{latest}', updateResult.latest)
										.replace('{current}', updateResult.current)}
									<button class="ml-1 text-accent hover:underline" onclick={() => void api.openExtensionPage(updateResult!.html_url)}>
										{tr('settings.general.openRelease')}
									</button>
								{:else}
									{tr('settings.general.upToDate').replace('{current}', updateResult.current)}
								{/if}
							</p>
						{/if}
						{#if updateError}
							<p class="mt-2 px-1 text-[11.5px] text-danger">{updateError}</p>
						{/if}
					</section>

					<section>
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.general.behavior')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.autoCheck')} toggle checked={editable.auto_check_updates} onclick={() => toggle('auto_check_updates')} />
							<SettingRow label={tr('settings.showUpdate')} toggle checked={editable.show_update_button} onclick={() => toggle('show_update_button')} />
							<SettingRow label={tr('settings.notify')} toggle checked={editable.enable_new_builds_notifications} onclick={() => toggle('enable_new_builds_notifications')} />
							<SettingRow label={tr('settings.highDpi')} toggle checked={editable.enable_high_dpi} onclick={() => toggle('enable_high_dpi')} />
							<SettingRow label={tr('settings.tray')} hint={tr('settings.trayHint')} toggle checked={editable.show_tray_icon} onclick={() => toggle('show_tray_icon')} />
							<SettingRow label={tr('settings.startMinimized')} hint={tr('settings.startMinimizedHint')} disabled={!editable.show_tray_icon} last toggle checked={editable.launch_minimized_to_tray} onclick={() => toggle('launch_minimized_to_tray')} />
						</div>
					</section>
				{:else if section === 'appearance'}
					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.appearance.window')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow
								label={tr('settings.appearance.mica')}
								hint={$micaActive ? tr('settings.appearance.micaActive') : tr('settings.appearance.micaUnavailable')}
								toggle
								checked={editable.enable_mica}
								onclick={() => {
									const next = !editable!.enable_mica;
									editable = { ...editable!, enable_mica: next };
									setMica(next);
								}}
							/>
							<SettingRow label={tr('settings.appearance.theme')} hint={tr('settings.appearance.themeHint')} last>
								<Segmented
									items={themes.map((item) => ({ id: item.id, label: tr(item.key) }))}
									value={editable.theme}
									onSelect={(id) => {
										editable = { ...editable!, theme: id as ThemeMode };
										setThemeMode(id as ThemeMode, editable!.accent as Accent);
									}}
								/>
							</SettingRow>
						</div>
					</section>

					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.appearance.accentGroup')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.appearance.accent')} hint={tr('settings.appearance.accentHint')} last>
								<div class="flex gap-1.5">
									{#each accents as item (item.id)}
										<button
											class="h-7 w-7 rounded-sm2 border-2 transition-transform hover:scale-110
												{editable.accent === item.id ? 'border-ink-1' : 'border-transparent'}"
											style="background: rgb({item.rgb})"
											title={item.label}
											aria-label={item.label}
											onclick={() => {
												editable = { ...editable!, accent: item.id };
												setAccent(item.id);
											}}
										></button>
									{/each}
								</div>
							</SettingRow>
						</div>
					</section>

					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.appearance.languageGroup')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.appearance.language')} hint={tr('settings.appearance.languageHint')} last>
								<select
									class="{field} w-[180px]"
									value={editable.language}
									onchange={(event) => {
										const value = event.currentTarget.value;
										editable = { ...editable!, language: value };
										if (value !== 'auto') setLocale(value as Lang);
									}}
								>
									<option value="auto">{tr('settings.appearance.languageAuto')}</option>
									{#each LANGUAGES as lang (lang.id)}
										<option value={lang.id}>{lang.label}</option>
									{/each}
								</select>
							</SettingRow>
						</div>
					</section>

					<section>
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.appearance.layoutGroup')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<!-- 布局是三选一的视觉选择，需要整行宽度来并排展示缩略说明，
							     不适合走 SettingRow 的「标签左、控件右」结构 -->
							<div class="px-4 py-3">
								<div class="text-[13px] leading-5 text-ink-1">{tr('settings.appearance.layout')}</div>
								<p class="mt-0.5 text-[12px] leading-snug text-ink-3">{tr('settings.appearance.layoutHint')}</p>
								<div class="mt-2.5 grid grid-cols-3 gap-2">
									{#each layouts as item (item.id)}
										<button
											class="rounded-sm2 border px-3 py-2 text-left transition-colors
												{editable.layout === item.id
												? 'border-accent bg-accent/10'
												: 'border-surface-4 hover:border-surface-5'}"
											onclick={() => (editable = { ...editable!, layout: item.id })}
										>
											<span class="block text-[12.5px] font-medium {editable.layout === item.id ? 'text-ink-1' : 'text-ink-2'}">
												{tr(item.key)}
											</span>
											<span class="mt-1 block text-[11.5px] leading-snug text-ink-3">{tr(item.desc)}</span>
										</button>
									{/each}
								</div>
							</div>
						</div>
					</section>
				{:else if section === 'connection'}
					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.connection.windows')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.connection.registerApp')} hint={tr('settings.connection.registerHint')} last>
								{#if wingetRegistered}
									<span class="text-[12px] text-success">{tr('settings.connection.registered')}</span>
								{:else}
									<button
										class="h-8 rounded-sm2 bg-surface-4 px-3 text-[13px] text-ink-1 transition-colors hover:bg-surface-5 disabled:opacity-50"
										disabled={wingetBusy}
										onclick={registerWinget}
									>
										{wingetBusy ? tr('settings.connection.registering') : tr('settings.connection.registerApp')}
									</button>
								{/if}
							</SettingRow>
						</div>
					</section>

					<section>
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.connection.proxy')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow
								label={tr('settings.connection.proxyEnable')}
								toggle
								checked={editable.proxy.enabled}
								onclick={() =>
									(editable = {
										...editable!,
										proxy: { ...editable!.proxy, enabled: !editable!.proxy.enabled }
									})}
							/>
							<SettingRow label={tr('settings.connection.type')} disabled={!editable.proxy.enabled}>
								<Segmented
									items={[
										{ id: 'http', label: 'HTTP' },
										{ id: 'https', label: 'HTTPS' }
									]}
									value={editable.proxy.kind}
									onSelect={(id) =>
										(editable = {
											...editable!,
											proxy: { ...editable!.proxy, kind: id as 'http' | 'https' }
										})}
								/>
							</SettingRow>
							<SettingRow label={tr('settings.connection.host')} disabled={!editable.proxy.enabled}>
								<input class="{field} w-[180px]" placeholder="127.0.0.1" bind:value={editable.proxy.host} disabled={!editable.proxy.enabled} />
							</SettingRow>
							<SettingRow label={tr('settings.connection.port')} disabled={!editable.proxy.enabled}>
								<input type="text" class="{field} w-[96px] text-right tabular-nums" placeholder="8080" bind:value={editable.proxy.port} disabled={!editable.proxy.enabled} />
							</SettingRow>
							<SettingRow label={tr('settings.connection.username')} disabled={!editable.proxy.enabled}>
								<input class="{field} w-[180px]" bind:value={editable.proxy.username} disabled={!editable.proxy.enabled} />
							</SettingRow>
							<SettingRow label={tr('settings.connection.password')} hint={tr('settings.connection.proxyHint')} disabled={!editable.proxy.enabled} last>
								<input type="password" class="{field} w-[180px]" bind:value={editable.proxy.password} disabled={!editable.proxy.enabled} />
							</SettingRow>
						</div>
						<p class="mt-2 px-1 text-[11.5px] text-ink-3">{tr('settings.connection.socksHint')}</p>
					</section>

					<section>
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.connection.mirror')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.connection.mirror')} hint={tr('settings.connection.mirrorHint')} last>
								<select
									class="{field} w-[180px]"
									value={editable.download_mirror ?? 'official'}
									onchange={(event) => {
										const value = event.currentTarget.value;
										editable = {
											...editable!,
											download_mirror: value === 'official' ? null : (value as 'tuna')
										};
									}}
								>
									<option value="official">{tr('settings.connection.mirrorOfficial')}</option>
									<option value="tuna">{tr('settings.connection.mirrorTuna')}</option>
								</select>
							</SettingRow>
						</div>
						<p class="mt-2 px-1 text-[11.5px] text-ink-3">{tr('settings.connection.mirrorNote')}</p>
					</section>
				{:else if section === 'builds'}
					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.builds.sources')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.builds.bforartists')} toggle checked={editable.scrape_bforartists} onclick={() => toggle('scrape_bforartists')} />
							<SettingRow label={tr('settings.builds.upbge')} hint={tr('settings.builds.sourcesHint')} last toggle checked={editable.scrape_upbge} onclick={() => toggle('scrape_upbge')} />
						</div>
					</section>

					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.builds.checking')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.builds.interval')} last>
								<input type="number" min="1" max="168" class="{field} w-[80px] text-right tabular-nums" bind:value={editable.update_check_interval_hours} />
								<span class="text-[12px] text-ink-3">{tr('settings.builds.intervalUnit')}</span>
							</SettingRow>
						</div>
					</section>

					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.builds.downloads')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.builds.maxConcurrent')} hint={tr('settings.builds.maxConcurrentHint')} last>
								<input type="number" min="1" max="8" class="{field} w-[80px] text-right tabular-nums" bind:value={editable.max_concurrent_downloads} />
							</SettingRow>
						</div>
					</section>

					<section>
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.builds.launching')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.builds.hotkey')} hint={tr('settings.builds.hotkeyHint')}>
								<input class="{field} w-[140px] font-mono text-[12px]" bind:value={editable.quick_launch_key_seq} />
							</SettingRow>
							<SettingRow label={tr('settings.builds.args')} hint={tr('settings.builds.argsHint')} last>
								<input class="{field} w-[240px] font-mono text-[12px]" bind:value={launchArgsText} placeholder="--factory-startup" />
							</SettingRow>
						</div>
					</section>
				{:else if section === 'plugins'}
					<section>
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.ext.title')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow
								label={tr('settings.ext.central')}
								hint={tr('settings.ext.centralHint')}
								last={!editable.enable_extension_central}
								toggle
								checked={editable.enable_extension_central}
								onclick={() => toggle('enable_extension_central')}
							/>
							{#if editable.enable_extension_central}
								<SettingRow label={tr('settings.ext.centralDir')} hint={tr('settings.ext.centralDirHint')} last>
									<input class="{field} min-w-0 flex-1 max-w-[320px]" bind:value={editable.extension_central_dir} />
									<button
										class="h-8 rounded-sm2 bg-surface-4 px-3 text-[13px] text-ink-1 transition-colors hover:bg-surface-5"
										onclick={browseCentralDir}
									>
										{tr('settings.general.browse')}
									</button>
								</SettingRow>
							{/if}
						</div>
					</section>
				{:else}
					<section class="mb-5">
						<h2 class="mb-2 px-1 text-[13px] font-semibold text-ink-2">{tr('settings.library.title')}</h2>
						<div class="overflow-hidden rounded-sm2 border border-surface-4 bg-surface-2">
							<SettingRow label={tr('settings.library.scan')} last>
								<button class="h-8 rounded-sm2 bg-surface-4 px-3 text-[13px] text-ink-1 transition-colors hover:bg-surface-5 disabled:opacity-50" disabled={scanning} onclick={scan}>
									{scanning ? tr('settings.library.scanning') : tr('settings.library.scan')}
								</button>
								<button class="h-8 rounded-sm2 bg-surface-4 px-3 text-[13px] text-ink-1 transition-colors hover:bg-surface-5" onclick={refreshStats}>
									{tr('settings.library.reloadStats')}
								</button>
								<button class="h-8 rounded-sm2 bg-surface-4 px-3 text-[13px] text-ink-1 transition-colors hover:bg-surface-5" onclick={cleanup}>
									{tr('settings.library.cleanup')}
								</button>
							</SettingRow>
						</div>
					</section>

					{#if stats}
						<section class="mb-5">
							<div class="grid grid-cols-4 gap-px overflow-hidden rounded-sm2 border border-surface-4 bg-surface-4">
								{#each [
									{ label: 'settings.library.installed' as const, value: String(stats.total_builds) },
									{ label: 'settings.library.totalSize' as const, value: formatBytes(stats.total_size_bytes) },
									{ label: 'settings.library.orphaned' as const, value: String(stats.orphaned_builds.length) },
									{ label: 'settings.library.corrupt' as const, value: String(stats.corrupt_dirs?.length ?? 0) }
								] as item (item.label)}
									<div class="bg-surface-2 px-3 py-2.5">
										<div class="text-[11.5px] text-ink-3">{tr(item.label)}</div>
										<div class="mt-0.5 text-[16px] font-semibold tabular-nums text-ink-1">{item.value}</div>
									</div>
								{/each}
							</div>
						</section>
					{/if}

					{#if stats && (stats.corrupt_dirs?.length ?? 0) > 0}
						<section class="mb-5">
							<h2 class="mb-2 px-1 text-[13px] font-semibold text-danger">{tr('settings.library.corruptTitle')}</h2>
							<div class="overflow-hidden rounded-sm2 border border-danger/40 bg-danger/5 p-2">
								{#each stats.corrupt_dirs as dir (dir)}
									<p class="break-all px-1 py-0.5 font-mono text-[11.5px] text-ink-2">{dir}</p>
								{/each}
							</div>
						</section>
					{/if}

					{#if cleanupResult}
						<p class="px-1 text-[12px] text-success">
							{tr('settings.library.cleaned', { files: cleanupResult.deleted_files, size: formatBytes(cleanupResult.freed_bytes) })}
						</p>
					{/if}
				{/if}
			{:else}
				<p class="px-1 text-[13px] text-ink-3">{tr('settings.loading')}</p>
			{/if}
		</div>

		<!-- 底部保存栏：固定在内容区底部，长页面也随手可保存 -->
		{#if editable}
			<div class="flex shrink-0 items-center gap-3 border-t border-surface-4 pt-3">
				<button
					class="h-8 rounded-sm2 bg-accent px-4 text-[13px] font-medium text-accent-ink transition-opacity hover:opacity-90 disabled:opacity-50"
					disabled={saving}
					onclick={save}
				>
					{saving ? tr('settings.saving') : tr('settings.save')}
				</button>
				{#if message}
					<span class="text-[12px] text-success">{message}</span>
				{/if}
				{#if error}
					<span class="min-w-0 truncate text-[12px] text-danger" title={error}>{error}</span>
				{/if}
			</div>
		{/if}
	</div>
</div>
