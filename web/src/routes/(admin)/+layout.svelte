<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import type { LayoutProps } from './$types';

	// Staff shell. Ships ONLY in the web-admin build (admin.andrii.ai);
	// the public build's routes dir has no (admin) group at all.
	let { children }: LayoutProps = $props();

	// Client-side session gate (onMount only — never runs during
	// prerender, so the static build stays green). /admin/login
	// and /admin/enrol are exempt (redirect loop otherwise).
	onMount(async () => {
		const path = window.location.pathname;
		if (path === '/admin/login' || path === '/admin/enrol') return;
		try {
			const res = await fetch('/apostille/api/v1/admin/me', { credentials: 'same-origin' });
			if (res.status === 401 || res.status === 403) {
				window.location.href = resolve('/(admin)/admin/login');
			}
		} catch {
			// Offline or cert-less: pages surface their own errors;
			// never strand the user on a blank shell.
		}
	});
</script>

<div class="staff-banner" role="note">
	Staff only — served from admin.andrii.ai. Every action is audited.
</div>

<nav class="admin-nav" aria-label="Admin">
	<a href={resolve('/(admin)/admin')}>Dashboard</a>
	<a href={resolve('/(admin)/admin/triage')}>Triage</a>
	<a href={resolve('/(admin)/admin/console')}>Call console</a>
	<a href={resolve('/(admin)/calls')}>Voice calls</a>
	<a href={resolve('/(admin)/admin/config')}>Config</a>
	<a href={resolve('/(admin)/workers')}>Workers</a>
</nav>

{@render children()}

<style>
	.staff-banner {
		background: var(--warning-bg);
		color: var(--text);
		border-bottom: 1px solid var(--border);
		padding: 0.5rem 1rem;
		font-size: 0.875rem;
		font-weight: 700;
	}
	.admin-nav {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.25rem 1rem;
		padding: 0.5rem 1rem;
		border-bottom: 1px solid var(--border);
		background: var(--surface);
	}
	.admin-nav a {
		display: inline-flex;
		align-items: center;
		min-height: 44px;
		font-weight: 700;
	}
</style>
