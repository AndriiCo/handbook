<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { STRINGS, resolveLocale, type Locale } from '../../lib/i18n';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	let locale = $state<Locale>('en');
	let status = $state<'checking' | 'in' | 'out'>('checking');
	let customerRef = $state<string | null>(null);

	const api = (path: string) => `${resolve('/').replace(/\/$/, '')}/api/v1${path}`;

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

<nav class="customer-nav" aria-label="Customer">
	<a href={resolve('/(customer)/order')}>{t.orderLabel}</a>
	<a href={resolve('/(customer)/billing')}>{t.billingLabel}</a>
	<a href={resolve('/')} title="Logout is not wired yet — returns home">{t.logoutLabel}</a>
	<span class="session" aria-live="polite">
		{#if status === 'checking'}
			…
		{:else if status === 'in'}
			<code>{customerRef}</code>
		{:else}
			not logged in · <a href={resolve('/(customer)/login')}>{t.loginLabel}</a>
		{/if}
	</span>
</nav>

{@render children()}

<style>
	.customer-nav {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.25rem 1rem;
		padding: 0.5rem 1rem;
		border-bottom: 1px solid var(--border);
		background: var(--surface);
	}
	.customer-nav a {
		display: inline-flex;
		align-items: center;
		min-height: 44px;
		font-weight: 700;
	}
	.session {
		margin-left: auto;
		color: var(--text-muted);
		font-size: 0.875rem;
	}
	.session code {
		font-family: var(--font-mono);
		color: var(--text);
	}
</style>
