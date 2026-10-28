<script lang="ts">
	import { resolve } from '$app/paths';
	import {
		cookieValue,
		credentialRequestArgs,
		registerPasskey,
		serializeAuthentication
	} from '../../../lib/customer/passkey.js';
	import { googleStartError, startGoogle } from '../../../lib/customer/google.js';

	type Method = { method: string; id: string; label: string; disabled: boolean };

	let methods = $state<Method[]>([]);
	let userId = $state('');
	let label = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

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

	async function refresh() {
		error = null;
		try {
			const meRes = await fetch(api('/auth/me'), { credentials: 'same-origin' });
			if (!meRes.ok) {
				userId = '';
				methods = [];
				return;
			}
			const me = await meRes.json();
			userId = me.user_id;
			const list = await fetch(api('/auth/profile/methods'), { credentials: 'same-origin' });
			methods = ((await list.json()) as { methods: Method[] }).methods ?? [];
		} catch {
			error = 'Could not load sign-in methods.';
		}
	}

	async function addKey(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		busy = true;
		try {
			const res = await registerPasskey(
				{ displayName: userId, label },
				{ credentials: navigator.credentials, post }
			);
			if (!res.ok) {
				error =
					res.status === 403
						? 'Log in again first (linking a key needs a fresh session).'
						: ((res.body.error as string) ?? `Failed (HTTP ${res.status}).`);
				return;
			}
			label = '';
			await refresh();
		} catch (e) {
			error =
				e instanceof DOMException && e.name === 'AbortError'
					? 'Cancelled — no key was added.'
					: 'Passkey ceremony failed — try again.';
		} finally {
			busy = false;
		}
	}

	async function removeKey(id: string) {
		error = null;
		busy = true;
		try {
			// Fresh ceremony doubles as the unlink re-auth proof.
			const started = await post('/auth/passkey/login/start', { account: userId });
			if (!started.ok) {
				error = 'Re-authentication failed — try again.';
				return;
			}
			const args = credentialRequestArgs(started.body.options);
			const cred = await navigator.credentials.get(args);
			const finished = await post('/auth/passkey/login/finish', {
				challenge_id: started.body.challenge_id,
				credential: serializeAuthentication(cred)
			});
			if (!finished.ok) {
				error = 'Re-authentication failed — try again.';
				return;
			}
			const res = await post('/auth/profile/unlink', {
				method: 'passkey',
				id,
				reauth_proof: started.body.challenge_id
			});
			if (!res.ok) {
				error =
					res.status === 409
						? 'This is your last sign-in method — add another key first.'
						: ((res.body.error as string) ?? `Failed (HTTP ${res.status}).`);
				return;
			}
			await refresh();
		} catch (e) {
			error =
				e instanceof DOMException && e.name === 'AbortError'
					? 'Cancelled — nothing was removed.'
					: 'Passkey ceremony failed — try again.';
		} finally {
			busy = false;
		}
	}

	async function linkGoogle() {
		error = null;
		busy = true;
		const res = await startGoogle(
			{ post, redirect: (u: string) => (window.location.href = u) },
			'link'
		);
		// On ok the browser is navigating away; on failure the page
		// stays and the buttons re-enable.
		busy = false;
		if (!res.ok) error = googleStartError(res.status);
	}

	async function removeGoogle(id: string) {
		error = null;
		busy = true;
		try {
			// No passkey ceremony: a Google-only user has none. The
			// server accepts a fresh session as the re-auth proof
			// for google unlinks (spec §9.4).
			const res = await post('/auth/profile/unlink', {
				method: 'google',
				id,
				reauth_proof: 'session'
			});
			if (!res.ok) {
				error =
					res.status === 409
						? 'This is your last sign-in method — add another first.'
						: res.status === 403
							? 'Log in again first (unlinking needs a fresh session).'
							: ((res.body.error as string) ?? `Failed (HTTP ${res.status}).`);
				return;
			}
			await refresh();
		} finally {
			busy = false;
		}
	}

	$effect(() => {
		refresh().catch(() => {});
	});
</script>

<svelte:head>
	<title>Sign-in methods — Apostille Handbook</title>
</svelte:head>

<h1>Sign-in methods</h1>

{#if !userId}
	<p class="notice">Log in first to manage your sign-in methods.</p>
	<p><a href={resolve('/(customer)/login')}>Go to log in</a></p>
{:else}
	{#if methods.length === 0}
		<p class="notice">No sign-in methods yet — add your first below.</p>
	{:else}
		<ul>
			<!-- Only passkey + google rows exist today; future methods
				extend the ternaries below (never removeKey). -->
			{#each methods as method (method.method + ':' + method.id)}
				<li>
					<strong>{method.method === 'google' ? 'Google' : method.label || 'Unnamed key'}</strong>
					<span class="muted">
						{method.method === 'google' ? method.label || 'linked account' : method.method}
					</span>
					<button
						type="button"
						disabled={busy}
						onclick={() =>
							method.method === 'google' ? removeGoogle(method.id) : removeKey(method.id)}
					>
						Remove
					</button>
				</li>
			{/each}
		</ul>
	{/if}

	<form onsubmit={addKey}>
		<h2>Add another passkey</h2>
		<label for="new-label">Key label</label>
		<input id="new-label" name="label" type="text" bind:value={label} placeholder="YubiKey" />
		<button type="submit" disabled={busy}>Add passkey</button>
	</form>

	<form
		onsubmit={(e) => {
			e.preventDefault();
			linkGoogle();
		}}
	>
		<h2>Or link Google</h2>
		<button type="submit" disabled={busy}>
			Link Google account
		</button>
	</form>

	{#if error}
		<p class="notice err" role="alert">{error}</p>
	{/if}
{/if}

<style>
	ul {
		list-style: none;
		padding: 0;
		max-width: 28rem;
	}
	li {
		display: flex;
		align-items: baseline;
		gap: 0.75rem;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.6rem 0.8rem;
		margin-bottom: 0.5rem;
	}
	li button {
		margin-left: auto;
	}
	form {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		max-width: 28rem;
		margin: 1.5rem 0;
	}
	h2 {
		font-size: 1.1rem;
		margin: 0;
	}
	label {
		font-weight: 700;
	}
	input {
		min-height: 44px;
		padding: 0.6rem 0.8rem;
		font: inherit;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
	}
	button {
		min-height: 44px;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
	}
	button:disabled {
		opacity: 0.6;
		cursor: wait;
	}
	li button {
		min-height: 36px;
		font-weight: 400;
		color: var(--text);
		background: var(--bg);
		border-color: var(--border);
	}
	:root[data-theme='dark'] form button {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	.notice {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
		max-width: 28rem;
	}
	.notice.err {
		background: var(--warning-bg);
	}
	.muted {
		color: var(--text-muted);
	}
</style>
