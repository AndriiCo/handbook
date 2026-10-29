<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { STRINGS, resolveLocale, type Locale } from '../../lib/i18n';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	let locale = $state<Locale>('en');
	let status = $state<'checking' | 'in' | 'out'>('checking');
	let customerRef = $state<string | null>(null);

	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	onMount(async () => {
		locale = resolveLocale();
		try {
			const res = await fetch(api('/me'), { credentials: 'same-origin' });
			if (!res.ok) throw new Error(String(res.status));
			const data = await res.json();
			customerRef = typeof data?.customer_ref === 'string' ? data.customer_ref : null;
			status = customerRef ? 'in' : 'out';
		} catch {
			status = 'out';
		}
	});

	const t = $derived(STRINGS[locale]);
</script>

<nav
	class="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-line bg-surface px-4 py-2"
	aria-label="Customer"
>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(customer)/orders')}
		>{t.orderLabel}s</a
	>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(customer)/order')}
		>New {t.orderLabel}</a
	>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(customer)/uploads')}>Uploads</a>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(customer)/precheck')}>Precheck</a>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(customer)/forms')}>Forms</a>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(customer)/shipping')}>Shipping</a>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(customer)/billing')}
		>{t.billingLabel}</a
	>
	<a
		class="inline-flex min-h-11 items-center font-bold"
		href={resolve('/(public)')}
		title="Logout is not wired yet — returns home">{t.logoutLabel}</a
	>
	<span class="ml-auto text-sm text-ink-muted" aria-live="polite">
		{#if status === 'checking'}
			…
		{:else if status === 'in'}
			<code class="font-mono text-ink">{customerRef}</code>
		{:else}
			not logged in ·
			<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(customer)/login')}
				>{t.loginLabel}</a
			>
		{/if}
	</span>
</nav>

{@render children()}

<footer class="mt-8 border-t border-line px-4 py-3 text-sm text-ink-muted">
	<a class="inline-flex min-h-11 items-center underline" href="/privacy">Privacy Policy</a>
</footer>
