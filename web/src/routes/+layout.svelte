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
<a class="skip" href="#content">Skip to content</a>

<header class="masthead">
	<a class="wordmark" href={resolve('/(public)')}>Apostille Handbook</a>
	<div class="menus">
		<label>
			<span>{t.themeLabel}</span>
			<select value={theme} onchange={(e) => applyTheme(e.currentTarget.value as Theme)}>
				<option value="auto">System</option>
				<option value="light">Light</option>
				<option value="dark">Dark</option>
			</select>
		</label>
		<label>
			<span>{t.langLabel}</span>
			<select value={locale} onchange={(e) => applyLocale(e.currentTarget.value as Locale)}>
				<option value="en">English</option>
				<option value="es">Español</option>
				<option value="fr">Français</option>
			</select>
		</label>
	</div>
</header>

<main id="content">
	{@render children()}
</main>

<footer>
	<p>Apostille Handbook — public shell skeleton. General information, not legal advice.</p>
</footer>

<style>
	.skip {
		position: absolute;
		left: -9999px;
	}
	.skip:focus-visible {
		left: 0.5rem;
		top: 0.5rem;
		z-index: 10;
		background: var(--bg);
		padding: 0.5rem 1rem;
	}
	.masthead {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem 1rem;
		padding: 0.75rem 1rem;
		border-bottom: 1px solid var(--border);
	}
	.wordmark {
		font-family: var(--font-display);
		font-weight: 700;
		font-size: 1.1rem;
		color: var(--text);
		text-decoration: none;
	}
	.menus {
		display: flex;
		gap: 1rem;
	}
	.menus label {
		display: flex;
		align-items: center;
		gap: 0.4rem;
		font-size: 0.875rem;
		color: var(--text-muted);
	}
	select {
		min-height: 44px;
		background: var(--surface);
		color: var(--text);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
	}
	main {
		max-width: 1080px;
		margin: 0 auto;
		padding: 1rem;
	}
	footer {
		border-top: 1px solid var(--border);
		padding: 1rem;
		color: var(--text-muted);
		font-size: 0.875rem;
		text-align: center;
	}
	@media (min-width: 720px) {
		main {
			padding: 2rem;
		}
	}
</style>
