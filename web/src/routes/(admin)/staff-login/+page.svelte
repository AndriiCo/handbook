<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import {
		CANCELLED_COPY,
		cookieValue,
		isTransportFailure,
		loginErrorCopy,
		loginPasskey
	} from '../../../lib/admin/staff-auth.js';

	// Staff login (spec customer-auth.md §5.3): user_id + UV
	// passkey ceremony, on a browser holding the staff client
	// cert. The /me check doubles as the cert probe: a
	// transport failure means the handshake never completed.
	const api = (path: string) => `/apostille/api/v1${path}`;

	let account = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);
	let me = $state<{ user_id: string; role: string } | null>(null);
	const webauthnOk = typeof navigator !== 'undefined' && !!navigator.credentials?.get;

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
				error = loginErrorCopy(check.status, '');
				return;
			}
			me = await check.json();
			window.location.href = resolve('/(admin)/review');
		} catch (err) {
			if (err instanceof DOMException && err.name === 'NotAllowedError') {
				error = CANCELLED_COPY;
			} else if (isTransportFailure(err)) {
				error =
					'Could not connect — this page needs your staff client certificate. Install it, then reload.';
			} else {
				error = 'Sign-in failed — try again.';
			}
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>Staff login — Apostille Handbook</title>
</svelte:head>

<h1>Staff login</h1>

{#if !webauthnOk}
	<Notice variant="warning" heading="Passkeys unavailable.">
		<p>This browser cannot do passkeys. Use a current Safari, Chrome, Edge, or Firefox.</p>
	</Notice>
{:else if me}
	<Notice variant="success" heading="Signed in.">
		<p>Continuing to the dashboard…</p>
	</Notice>
{:else}
	<form
		class="mt-4 grid max-w-[28rem] gap-3"
		onsubmit={(e) => {
			e.preventDefault();
			login();
		}}
	>
		<label class="grid gap-1 font-bold">
			Staff user ID
			<input
				class="rounded-[0.375rem] border border-line p-[0.6rem] text-base"
				type="text"
				bind:value={account}
				autocomplete="username"
				spellcheck={false}
				placeholder="32-character ID from your invite"
				required
			/>
		</label>
		<button
			class="min-h-11 cursor-pointer text-base font-bold disabled:cursor-wait disabled:opacity-70"
			type="submit"
			disabled={busy || account.trim() === ''}
		>
			{busy ? 'Waiting for security key…' : 'Sign in with passkey'}
		</button>
	</form>

	<p class="mt-4">New here? Enrolment is invite-only — ask your admin for an invite link.</p>
{/if}

{#if error}
	<p class="font-bold" role="alert">{error}</p>
{/if}
