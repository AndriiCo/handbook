<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import {
		CANCELLED_COPY,
		cookieValue,
		enrolErrorCopy,
		enrolPasskey,
		isTransportFailure,
		validInviteShape
	} from '../../../../lib/admin/staff-auth.js';

	// Staff enrolment (spec customer-auth.md §5.2): single-use
	// invite URL + live client cert + UV ceremony. The invite
	// token is never rendered, logged, or stored — it lives in
	// this closure and the two POST bodies only.
	const api = (path: string) => `/apostille/api/v1${path}`;

	let linkState = $state<'checking' | 'invalid' | 'ready'>('checking');
	let grant = $state<{ role: string; cn: string } | null>(null);
	let displayName = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);
	let done = $state(false);
	let invite = '';
	const webauthnOk = typeof navigator !== 'undefined' && !!navigator.credentials?.create;

	onMount(() => {
		const params = new URLSearchParams(window.location.search);
		invite = params.get('invite') ?? '';
		// Strip the token from the URL immediately (history,
		// screenshots, and shoulder-surfers never see it again).
		history.replaceState(null, '', window.location.pathname);
		linkState = validInviteShape(invite) ? 'ready' : 'invalid';
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

	async function enrol() {
		error = null;
		grant = null;
		busy = true;
		try {
			const res = await enrolPasskey(
				{ invite, displayName: displayName.trim() },
				{ post, credentials: navigator.credentials }
			);
			if (!res.ok) {
				error = enrolErrorCopy(res.status, res.body?.error ?? '');
				return;
			}
			grant = { role: res.body.role, cn: res.body.cn };
			done = true;
			window.location.href = resolve('/(admin)/admin');
		} catch (err) {
			if (err instanceof DOMException && err.name === 'NotAllowedError') {
				error = CANCELLED_COPY;
			} else if (isTransportFailure(err)) {
				error =
					'Could not connect — this page needs your staff client certificate. Install it, then reload.';
			} else {
				error = 'Enrolment failed — try again.';
			}
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>Staff enrolment — Apostille Handbook</title>
</svelte:head>

<h1>Staff enrolment</h1>

{#if linkState === 'checking' || !webauthnOk}
	{#if !webauthnOk}
		<Notice variant="warning" heading="Passkeys unavailable.">
			<p>This browser cannot do passkeys. Use a current Safari, Chrome, Edge, or Firefox.</p>
		</Notice>
	{:else}
		<p class="mt-4">Checking invite link…</p>
	{/if}
{:else if linkState === 'invalid'}
	<Notice variant="error" heading="Invalid invite link.">
		<p>This link is malformed or missing its token — ask your admin for a new invite link.</p>
	</Notice>
{:else if done && grant}
	<Notice variant="success" heading="Enrolled.">
		<p>Continuing to the dashboard…</p>
	</Notice>
{:else}
	<p class="mt-4">Create your staff passkey. Single-use link — it stops working after enrolment.</p>
	<form
		class="mt-4 grid max-w-[28rem] gap-3"
		onsubmit={(e) => {
			e.preventDefault();
			enrol();
		}}
	>
		<label class="grid gap-1 font-bold">
			Display name
			<input
				class="rounded-[0.375rem] border border-line p-[0.6rem] text-base"
				type="text"
				bind:value={displayName}
				autocomplete="name"
				placeholder="Jordan Doe"
				required
			/>
		</label>
		<button
			class="min-h-11 cursor-pointer text-base font-bold disabled:cursor-wait disabled:opacity-70"
			type="submit"
			disabled={busy || displayName.trim() === ''}
		>
			{busy ? 'Waiting for security key…' : 'Enrol this device'}
		</button>
	</form>
{/if}

{#if error}
	<p class="font-bold" role="alert">{error}</p>
{/if}
