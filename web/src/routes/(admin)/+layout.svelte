<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import type { LayoutProps } from './$types';

	// Staff shell. Ships ONLY in the web-admin build (admin.andrii.ai);
	// the public build's routes dir has no (admin) group at all.
	let { children }: LayoutProps = $props();

	// Client-side session gate (onMount only — never runs during
	// prerender, so the static build stays green). /staff-login
	// and /enrol are exempt (redirect loop otherwise), and
	// /config is exempt while the setup window is open (no
	// session can exist yet — the page runs the wizard instead).
	onMount(async () => {
		const path = window.location.pathname;
		if (path === '/staff-login' || path === '/enrol') return;
		if (path === '/config') {
			try {
				const setup = await fetch('/apostille/api/v1/setup/status', {
					credentials: 'same-origin'
				});
				if (setup.ok && (await setup.json().catch(() => ({})))?.setup_required) return;
			} catch {
				// Fall through to the session check below.
			}
		}
		try {
			const res = await fetch('/apostille/api/v1/me', { credentials: 'same-origin' });
			if (res.status === 401 || res.status === 403) {
				window.location.href = resolve('/(admin)/staff-login');
			}
		} catch {
			// Offline or cert-less: pages surface their own errors;
			// never strand the user on a blank shell.
		}
	});
</script>

<div class="border-b border-line bg-warning-bg px-4 py-2 text-sm font-bold" role="note">
	Staff only — served from admin.andrii.ai. Every action is audited.
</div>

<nav
	class="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-line bg-surface px-4 py-2"
	aria-label="Admin"
>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(admin)/triage')}>Triage</a>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(admin)/trace')}>Trace</a>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(admin)/review')}>Review</a>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(admin)/calls')}>Voice calls</a>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(admin)/config')}>Config</a>
	<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(admin)/workers')}>Workers</a>
</nav>

{@render children()}
