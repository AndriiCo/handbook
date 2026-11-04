<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import {
		CANCELLED_COPY,
		cookieValue,
		isTransportFailure,
		loginErrorCopy,
		loginPasskey
	} from '../../../lib/admin/staff-auth.js';
	import { fetchSetupStatus } from '../../../lib/admin/setup-client.js';
	import { safeNext } from '../../../lib/admin/http-errors.js';

	// Staff sign-in (spec customer-auth.md §5.3): user_id + UV passkey
	// ceremony. No client certificate: the admin host's IP allowlist
	// gates the branch. `?next=` (set when a session expired mid-use)
	// is honoured after sign-in, same-origin console paths only.
	const api = (path: string) => `/apostille/api/v1${path}`;

	let account = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);
	let me = $state<{ user_id: string; role: string } | null>(null);
	let next = $state(safeNext(null));
	// null = unknown (status unreachable): the setup link still shows.
	let setupRequired = $state<boolean | null>(null);
	const webauthnOk = typeof navigator !== 'undefined' && !!navigator.credentials?.get;

	onMount(async () => {
		next = safeNext(new URLSearchParams(window.location.search).get('next'));
		try {
			setupRequired = (await fetchSetupStatus('/apostille/api/v1')).setup_required;
		} catch {
			setupRequired = null;
		}
	});

	async function post(path: string, body: unknown) {
		const res = await fetch(api(path), {
			method: 'POST',
			headers: {
				'content-type': 'application/json',
				'x-csrf': cookieValue(document.cookie, 'hb_csrf')
			},
			credentials: 'same-origin',
			body: JSON.stringify(body)
		});
		const parsed = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body: parsed };
	}

	async function login() {
		error = null;
		busy = true;
		try {
			const res = await loginPasskey(account.trim(), {
				post,
				credentials: navigator.credentials
			});
			if (!res.ok) {
				error = loginErrorCopy(res.status, res.body?.error ?? '');
				return;
			}
			const check = await fetch(api('/me'), { credentials: 'same-origin' });
			if (!check.ok) {
				// Carry the denial text (it names the refused IP) into the copy.
				const detail = await check
					.clone()
					.json()
					.catch(() => null);
				error = loginErrorCopy(
					check.status,
					typeof detail?.error === 'string' ? detail.error : ''
				);
				return;
			}
			me = await check.json();
			window.location.replace(next);
		} catch (err) {
			if (err instanceof DOMException && err.name === 'NotAllowedError') {
				error = CANCELLED_COPY;
			} else if (isTransportFailure(err)) {
				error =
					'Could not reach the admin host — check the connection and reload. (Refused networks answer with a 403 page naming the address instead.)';
			} else {
				error = 'Sign-in failed — try again.';
			}
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>Staff sign-in — Apostille Handbook</title>
</svelte:head>

<h1 class="m-0 mb-4 text-h2 font-semibold">Staff sign-in</h1>

{#if !webauthnOk}
	<Notice variant="warning" heading="Passkeys unavailable.">
		<p>This browser cannot do passkeys. Use a current Safari, Chrome, Edge, or Firefox.</p>
	</Notice>
{:else if me}
	<Notice variant="success" heading="Signed in.">
		<p>Continuing to <code>{next}</code>…</p>
	</Notice>
{:else if setupRequired}
	<Notice variant="info" heading="This server has no staff passkey yet.">
		<p class="m-0">
			Start with first-time setup: paste the setup key the server printed at boot, then enrol
			the first passkey. Sign-in works from then on.
		</p>
	</Notice>
	<p class="mt-4">
		<Button href={resolve('/(admin)/config')} size="lg" class="min-h-11 w-full px-4 text-base sm:w-auto">
			First-time setup →
		</Button>
	</p>
{:else}
	<form
		class="grid gap-3"
		onsubmit={(e) => {
			e.preventDefault();
			login();
		}}
	>
		<label class="grid gap-1 font-bold" for="staff-account">Staff user ID</label>
		<Input
			id="staff-account"
			class="h-11 font-mono text-base"
			type="text"
			bind:value={account}
			autocomplete="username"
			spellcheck={false}
			placeholder="32-character ID from enrolment"
			required
		/>
		<Button
			type="submit"
			size="lg"
			class="min-h-11 w-full px-4 text-base"
			disabled={busy || account.trim() === ''}
		>
			{busy ? 'Waiting for your passkey…' : 'Sign in with passkey'}
		</Button>
	</form>

	<p class="mt-6 text-sm text-ink-muted">
		New staff join with an invite link from an admin.
		{#if setupRequired === null}
			Setting up a new server? <a class="font-semibold text-accent-strong underline" href={resolve('/(admin)/config')}
				>First-time setup →</a
			>
		{/if}
	</p>
{/if}

{#if error}
	<p class="mt-4 font-bold" role="alert">{error}</p>
{/if}
