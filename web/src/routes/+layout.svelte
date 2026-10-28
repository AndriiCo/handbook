<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import '../app.css';
	import { STRINGS, resolveLocale, persistLocale, type Locale } from '../lib/i18n';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	type Theme = 'light' | 'dark' | 'auto';
	const THEME_KEY = 'theme';

	let locale = $state<Locale>('en');
	let theme = $state<Theme>('auto');

	function applyTheme(value: Theme) {
		theme = value;
		try {
			localStorage.setItem(THEME_KEY, value);
		} catch {
			// ignore
		}
		if (value === 'auto') {
			document.documentElement.removeAttribute('data-theme');
		} else {
			document.documentElement.setAttribute('data-theme', value);
		}
	}

	function applyLocale(value: Locale) {
		locale = value;
		persistLocale(value);
		document.documentElement.lang = value;
		const url = new URL(location.href);
		url.searchParams.set('l', value);
		history.replaceState(null, '', url);
	}

	onMount(() => {
		locale = resolveLocale();
		document.documentElement.lang = locale;
		try {
			const stored = localStorage.getItem(THEME_KEY);
			if (stored === 'light' || stored === 'dark' || stored === 'auto') theme = stored;
		} catch {
			// ignore
		}
	});

	const t = $derived(STRINGS[locale]);
</script>

<!-- Theme-init snippet lives in src/app.html: the SPA fallback shell is not
     prerendered, so layout <svelte:head> content would never reach it. -->
<!-- "masthead"/"menus" tokens stay: app.css @media print hides ".masthead .menus". -->
<a
	class="absolute -left-[9999px] focus-visible:top-2 focus-visible:left-2 focus-visible:z-10 focus-visible:bg-bg focus-visible:px-4 focus-visible:py-2"
	href="#content"
>
	Skip to content
</a>

<header class="masthead flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b border-line px-4 py-3">
	<a class="font-display text-[1.1rem] font-bold text-ink no-underline" href={resolve('/(public)')}>
		Apostille Handbook
	</a>
	<div class="menus flex gap-4">
		<label class="flex items-center gap-[0.4rem] text-sm text-ink-muted">
			<span>{t.themeLabel}</span>
			<select
				class="min-h-11 rounded-xs border border-line bg-surface text-ink [font:inherit]"
				value={theme}
				onchange={(e) => applyTheme(e.currentTarget.value as Theme)}
			>
				<option value="auto">System</option>
				<option value="light">Light</option>
				<option value="dark">Dark</option>
			</select>
		</label>
		<label class="flex items-center gap-[0.4rem] text-sm text-ink-muted">
			<span>{t.langLabel}</span>
			<select
				class="min-h-11 rounded-xs border border-line bg-surface text-ink [font:inherit]"
				value={locale}
				onchange={(e) => applyLocale(e.currentTarget.value as Locale)}
			>
				<option value="en">English</option>
				<option value="es">Español</option>
				<option value="fr">Français</option>
			</select>
		</label>
	</div>
</header>

<main id="content" class="mx-auto max-w-[1080px] p-4 md:p-8">
	{@render children()}
</main>

<footer class="border-t border-line p-4 text-center text-sm text-ink-muted">
	<p>Apostille Handbook — public shell skeleton. General information, not legal advice.</p>
</footer>
