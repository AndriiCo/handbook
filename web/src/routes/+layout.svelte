<script lang="ts">
	import '@fontsource-variable/public-sans';
	import '@fontsource-variable/jetbrains-mono';
	import '@fontsource-variable/source-serif-4';
	import '../app.css';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { STRINGS, resolveLocale, persistLocale, LOCALES, type Locale } from '#lib/i18n';
	import { listAuthorities } from '#lib/public-ui/handbook.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as Avatar from '#lib/components/ui/avatar/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import { Toaster } from '#lib/components/ui/sonner/index.js';
	import SearchIcon from '@lucide/svelte/icons/search';
	import SunIcon from '@lucide/svelte/icons/sun';
	import MoonIcon from '@lucide/svelte/icons/moon';
	import MonitorIcon from '@lucide/svelte/icons/monitor';
	import LanguagesIcon from '@lucide/svelte/icons/languages';
	import LandmarkIcon from '@lucide/svelte/icons/landmark';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import LogInIcon from '@lucide/svelte/icons/log-in';
	import CheckIcon from '@lucide/svelte/icons/check';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	type Theme = 'light' | 'dark' | 'auto';
	const THEME_KEY = 'theme';
	const THEME_ORDER: Theme[] = ['auto', 'light', 'dark'];

	let locale = $state<Locale>('en');
	let theme = $state<Theme>('auto');
	let cmdOpen = $state(false);
	let authorities = $state<
		{ slug: string; name: string; country: string; fact_count: number }[] | null
	>(null);
	let status = $state<'checking' | 'in' | 'out'>('checking');
	let customerRef = $state<string | null>(null);

	const home = resolve('/(public)');
	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const apiRoot = home.replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;
	const asset = (path: string) => `${apiRoot}/${path}`;
	const authorityHref = (slug: string) =>
		resolve('/(public)/authority/[slug]', { slug });

	function effective(pref: Theme): 'light' | 'dark' {
		if (pref !== 'auto') return pref;
		return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
	}

	function applyTheme(value: Theme) {
		theme = value;
		try {
			localStorage.setItem(THEME_KEY, value);
		} catch {
			// ignore
		}
		document.documentElement.setAttribute('data-theme', effective(value));
	}

	function cycleTheme() {
		applyTheme(THEME_ORDER[(THEME_ORDER.indexOf(theme) + 1) % THEME_ORDER.length]);
	}

	function applyLocale(value: Locale) {
		locale = value;
		persistLocale(value);
		document.documentElement.lang = value;
		const url = new URL(location.href);
		url.searchParams.set('l', value);
		history.replaceState(null, '', url);
	}

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	async function loadAuthorities() {
		if (authorities !== null) return;
		try {
			const res = await get('/authorities');
			const rows = res.body?.authorities;
			authorities = Array.isArray(rows) ? rows : [];
		} catch {
			authorities = [];
		}
	}

	onMount(() => {
		locale = resolveLocale();
		document.documentElement.lang = locale;
		try {
			const stored = localStorage.getItem(THEME_KEY);
			applyTheme(stored === 'light' || stored === 'dark' || stored === 'auto' ? stored : 'auto');
		} catch {
			applyTheme('auto');
		}
		const mq = window.matchMedia('(prefers-color-scheme: dark)');
		const onChange = () => {
			if (theme === 'auto') document.documentElement.setAttribute('data-theme', effective('auto'));
		};
		mq.addEventListener('change', onChange);

		const onKey = (e: KeyboardEvent) => {
			if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
				e.preventDefault();
				cmdOpen = !cmdOpen;
			}
		};
		window.addEventListener('keydown', onKey);

		get('/me')
			.then((res) => {
				const ref =
					res.ok && typeof res.body?.customer_ref === 'string' ? res.body.customer_ref : null;
				customerRef = ref;
				status = ref ? 'in' : 'out';
			})
			.catch(() => {
				status = 'out';
			});

		return () => {
			mq.removeEventListener('change', onChange);
			window.removeEventListener('keydown', onKey);
		};
	});

	$effect(() => {
		if (cmdOpen) void loadAuthorities();
	});

	const t = $derived(STRINGS[locale]);
	const localeName = $derived({ en: 'English', es: 'Español', fr: 'Français' } as const);
</script>

<!-- Theme-init snippet lives in src/app.html: the SPA fallback shell is not
     prerendered, so layout <svelte:head> content would never reach it. -->
<!-- "masthead"/"menus" tokens stay: app.css @media print hides ".masthead .menus". -->
<a
	class="absolute -left-[9999px] focus-visible:top-2 focus-visible:left-2 focus-visible:z-50 focus-visible:rounded-sm focus-visible:bg-bg focus-visible:px-4 focus-visible:py-2"
	href="#content"
>
	Skip to content
</a>

<Tooltip.Provider>
	<header
		class="masthead sticky top-0 z-40 border-b border-line bg-bg/85 backdrop-blur supports-[backdrop-filter]:bg-bg/70"
	>
		<div class="mx-auto flex h-14 max-w-[1080px] items-center gap-2 px-4">
			<a href={home} class="flex min-w-0 items-center gap-2 no-underline">
				<img src={asset('favicon.svg')} alt="" width="28" height="28" class="size-7 flex-none" />
				<span class="hidden truncate font-semibold tracking-tight text-ink min-[400px]:inline">
					Apostille Handbook
				</span>
			</a>
			<button
				type="button"
				onclick={() => (cmdOpen = true)}
				class="ml-1 flex h-9 min-w-0 flex-1 cursor-pointer items-center gap-2 rounded-md border border-line bg-surface px-3 text-sm text-ink-muted transition-colors hover:border-focus hover:text-ink sm:max-w-64"
				aria-keyshortcuts="meta+k control+k"
			>
				<SearchIcon class="size-4 flex-none" aria-hidden="true" />
				<span class="hidden truncate sm:inline">{t.searchPlaceholder}</span>
				<span class="truncate sm:hidden">Search…</span>
				<kbd
					class="ml-auto hidden flex-none items-center gap-0.5 rounded-xs border border-line bg-bg px-1 font-mono text-xs sm:inline-flex"
					aria-hidden="true"
				>
					<span>⌘</span><span>K</span>
				</kbd>
			</button>
			<div class="menus ml-auto flex flex-none items-center gap-1">
				<Tooltip.Root>
					<Tooltip.Trigger
						onclick={cycleTheme}
						class="inline-flex size-9 cursor-pointer items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-surface hover:text-ink"
						aria-label={`${t.themeLabel}: ${theme}`}
					>
						{#if theme === 'light'}
							<SunIcon class="size-[18px]" aria-hidden="true" />
						{:else if theme === 'dark'}
							<MoonIcon class="size-[18px]" aria-hidden="true" />
						{:else}
							<MonitorIcon class="size-[18px]" aria-hidden="true" />
						{/if}
					</Tooltip.Trigger>
					<Tooltip.Content side="bottom">{t.themeLabel}: {theme}</Tooltip.Content>
				</Tooltip.Root>
				<DropdownMenu.Root>
					<DropdownMenu.Trigger
						class="inline-flex size-9 cursor-pointer items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-surface hover:text-ink"
						aria-label={`${t.langLabel}: ${localeName[locale]}`}
					>
						<LanguagesIcon class="size-[18px]" aria-hidden="true" />
					</DropdownMenu.Trigger>
					<DropdownMenu.Content align="end" class="min-w-36">
						<DropdownMenu.Label>{t.langLabel}</DropdownMenu.Label>
						<DropdownMenu.Separator />
						{#each LOCALES as code (code)}
							<DropdownMenu.Item
								onSelect={() => applyLocale(code)}
								class="flex items-center justify-between"
							>
								{localeName[code]}
								{#if code === locale}
									<CheckIcon class="size-4" aria-hidden="true" />
								{/if}
							</DropdownMenu.Item>
						{/each}
					</DropdownMenu.Content>
				</DropdownMenu.Root>
				{#if status === 'in' && customerRef}
					<DropdownMenu.Root>
						<DropdownMenu.Trigger
							class="ml-1 inline-flex cursor-pointer items-center rounded-full"
							aria-label={customerRef}
						>
							<Avatar.Root class="size-8">
								<Avatar.Fallback class="bg-accent-fill font-mono text-xs text-accent-ink">
									{customerRef.slice(0, 2).toUpperCase()}
								</Avatar.Fallback>
							</Avatar.Root>
						</DropdownMenu.Trigger>
						<DropdownMenu.Content align="end" class="min-w-48">
							<DropdownMenu.Label class="font-mono font-normal">{customerRef}</DropdownMenu.Label>
							<DropdownMenu.Separator />
							<DropdownMenu.Item onSelect={() => goto(resolve('/(customer)/orders'))}>
								{t.orderLabel}s
							</DropdownMenu.Item>
							<DropdownMenu.Item onSelect={() => goto(resolve('/(customer)/order'))}>
								New {t.orderLabel.toLowerCase()}
							</DropdownMenu.Item>
							<DropdownMenu.Item onSelect={() => goto(resolve('/(customer)/billing'))}>
								{t.billingLabel}
							</DropdownMenu.Item>
						</DropdownMenu.Content>
					</DropdownMenu.Root>
				{:else if status === 'out'}
					<a
						href={resolve('/(customer)/login')}
						class="ml-1 inline-flex h-9 items-center gap-1.5 rounded-md bg-accent-fill px-3 text-sm font-medium text-accent-ink no-underline transition-colors hover:bg-accent-hover"
					>
						<LogInIcon class="size-4" aria-hidden="true" />
						{t.loginLabel}
					</a>
				{:else}
					<span class="ml-1 inline-block size-8 animate-pulse rounded-full bg-surface-2" aria-hidden="true"></span>
				{/if}
			</div>
		</div>
	</header>

	<Command.Dialog
		bind:open={cmdOpen}
		title={t.searchLabel}
		description={t.searchPlaceholder}
	>
		<Command.Input placeholder={t.searchPlaceholder} />
		<Command.List>
			<Command.Empty>No results found.</Command.Empty>
			{#if authorities === null}
				<Command.Loading>Loading authorities…</Command.Loading>
			{:else}
				<Command.Group heading="Authorities">
					{#each authorities as a (a.slug)}
						<Command.LinkItem
							class="text-foreground"
							href={authorityHref(a.slug)}
							value={`${a.name} ${a.country} ${a.slug}`}
							onclick={() => (cmdOpen = false)}
						>
							<LandmarkIcon class="size-4" aria-hidden="true" />
							<span class="truncate">{a.name}</span>
							<span class="ml-auto flex-none font-mono text-xs text-ink-muted">
								{a.fact_count}
							</span>
						</Command.LinkItem>
					{/each}
				</Command.Group>
			{/if}
			<Command.Separator />
			<Command.Group heading="Pages">
				<Command.LinkItem class="text-foreground" href={home} value="home start" onclick={() => (cmdOpen = false)}>
					<FileTextIcon class="size-4" aria-hidden="true" />
					Home
				</Command.LinkItem>
				<Command.LinkItem
					class="text-foreground"
					href={resolve('/(customer)/order')}
					value="new order start application"
					onclick={() => (cmdOpen = false)}
				>
					<FileTextIcon class="size-4" aria-hidden="true" />
					New {t.orderLabel.toLowerCase()}
				</Command.LinkItem>
				<Command.LinkItem
					class="text-foreground"
					href={resolve('/(customer)/orders')}
					value="orders list status"
					onclick={() => (cmdOpen = false)}
				>
					<FileTextIcon class="size-4" aria-hidden="true" />
					{t.orderLabel}s
				</Command.LinkItem>
			</Command.Group>
		</Command.List>
	</Command.Dialog>
</Tooltip.Provider>

<main id="content" class="mx-auto max-w-[1080px] p-4 md:p-8">
	{@render children()}
</main>

<footer class="border-t border-line">
	<div
		class="mx-auto flex max-w-[1080px] flex-col gap-3 px-4 py-6 text-sm text-ink-muted md:flex-row md:items-center"
	>
		<p class="m-0">Apostille Handbook — general information, not legal advice.</p>
		<nav class="flex flex-wrap gap-x-4 gap-y-1" aria-label="Footer">
			<a class="text-ink-muted no-underline hover:text-ink hover:underline" href={home}>Home</a>
			<a
				class="text-ink-muted no-underline hover:text-ink hover:underline"
				href={resolve('/(customer)/order')}>New {t.orderLabel.toLowerCase()}</a
			>
			<a
				class="text-ink-muted no-underline hover:text-ink hover:underline"
				href={resolve('/(customer)/orders')}>{t.orderLabel}s</a
			>
		</nav>
		<a
			class="flex items-center gap-2 no-underline md:ml-auto"
			href="https://andrii.co"
			title="Andrii℠ — service mark of Andrii Co. USPTO Serial No. 50060860, Ukraine No. m202617219, Australia No. 2696315 (all pending)"
		>
			<span class="text-ink-muted">by</span>
			<img
				src={asset('service-mark.svg')}
				alt=""
				width="56"
				height="56"
				class="h-14 w-auto flex-none"
				aria-hidden="true"
			/>
			<span class="font-semibold text-ink">Andrii℠</span>
		</a>
	</div>
</footer>

<Toaster />
