<script lang="ts">
	import { resolve } from '$app/paths';
	import { STRINGS, resolveLocale, type Locale } from '../../../lib/i18n';
	import { cookieValue, loginPasskey, registerPasskey } from '../../../lib/customer/passkey.js';
	import {
		OAUTH_LABELS,
		OAUTH_PROVIDERS,
		oauthErrorCopy,
		oauthStartError,
		startOAuth
	} from '../../../lib/customer/oauth.js';
	import { consumeMagic, magicError, requestMagic } from '../../../lib/customer/magic.js';

	let locale = $state<Locale>(resolveLocale());
	let displayName = $state('');
	let label = $state('');
	let email = $state('');
	let account = $state(
		typeof localStorage === 'undefined' ? '' : (localStorage.getItem('hb_last_user') ?? '')
	);
	let error = $state<string | null>(null);
	let busy = $state(false);
	let me = $state<{ user_id: string } | null>(null);
	let oauthError = $state<string | null>(null);
	let magicEmail = $state('');
	let magicSent = $state(false);
	let magicBusy = $state(false);
	let magicStarted = false;

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;
	const t = $derived(STRINGS[locale]);
	const webauthnOk = typeof navigator !== 'undefined' && !!navigator.credentials?.create;

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

	function friendly(status: number, body: { error?: string }): string {
		if (status === 400) return body.error ?? 'Check the form values.';
		if (status === 401) return 'Unknown account or no passkeys on this account.';
		if (status === 404) return 'This device no longer has that passkey — register a new one below.';
		if (status === 409) return 'This passkey is already registered — use log in instead.';
		if (status === 410) return 'The ceremony expired — try again.';
		if (status === 422) return 'Verification failed — try again.';
		if (status === 429) return 'Too many attempts — wait a minute and try again.';
		return body.error ?? `Request failed (HTTP ${status}).`;
	}

	async function register(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		busy = true;
		try {
			const res = await registerPasskey(
				{ displayName, label, email },
				{ credentials: navigator.credentials, post }
			);
			if (!res.ok) {
				error = friendly(res.status, res.body);
				return;
			}
			localStorage.setItem('hb_last_user', res.body.user_id);
			me = { user_id: res.body.user_id };
		} catch (e) {
			error = e instanceof DOMException && e.name === 'AbortError'
				? 'Cancelled — no passkey was created.'
				: 'Passkey ceremony failed — is this browser allowed to create passkeys?';
		} finally {
			busy = false;
		}
	}

	async function login(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		busy = true;
		try {
			const res = await loginPasskey(account.trim(), {
				credentials: navigator.credentials,
				post
			});
			if (!res.ok) {
				error = friendly(res.status, res.body);
				return;
			}
			localStorage.setItem('hb_last_user', res.body.user_id);
			me = { user_id: res.body.user_id };
		} catch (e) {
			error = e instanceof DOMException && e.name === 'AbortError'
				? 'Cancelled — not logged in.'
				: 'Passkey ceremony failed — try again.';
		} finally {
			busy = false;
		}
	}

	async function logout() {
		await post('/auth/logout', {});
		me = null;
	}

	async function oauthLogin(provider: string) {
		error = null;
		busy = true;
		const res = await startOAuth(
			{ post, redirect: (u: string) => (window.location.href = u) },
			provider,
			'login'
		);
		// On ok the browser is navigating away; on failure the page
		// stays and the buttons re-enable.
		busy = false;
		if (!res.ok) error = oauthStartError(res.status, OAUTH_LABELS[provider] ?? provider);
	}

	async function magicRequest(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		busy = true;
		try {
			const res = await requestMagic(magicEmail.trim(), { post });
			if (!res.ok) {
				error = magicError(res.status, 'request');
				return;
			}
			// Identical for unknown addresses (anti-enumeration:
			// the server answers 200 either way).
			magicSent = true;
		} finally {
			busy = false;
		}
	}

	$effect(() => {
		// Magic-link consume (#t= fragment — never a query param, so
		// the token stays out of server logs). Read once, strip
		// immediately, consume exactly once (single-use token: a
		// double-POST would burn it and show a false error).
		const hash = window.location.hash;
		if (hash.startsWith('#t=') && !magicStarted) {
			magicStarted = true;
			magicBusy = true;
			history.replaceState(null, '', window.location.pathname + window.location.search);
			consumeMagic(hash.slice(3), { post })
				.then((res) => {
					if (!res.ok) {
						error = magicError(res.status, 'consume');
						return;
					}
					localStorage.setItem('hb_last_user', res.body.user_id);
					me = { user_id: res.body.user_id };
				})
				.catch(() => {
					error = magicError(500, 'consume');
				})
				.finally(() => {
					magicBusy = false;
				});
		}
		fetch(api('/auth/me'), { credentials: 'same-origin' })
			.then((res) => (res.ok ? res.json() : null))
			.then((body) => {
				if (body?.user_id) me = { user_id: body.user_id };
			})
			.catch(() => {});
		// Callback-failure copy from ?error= (friendly map, never raw
		// server text); the param is dropped so a retry starts clean.
		// $effect runs client-side only, so window is safe (and this
		// avoids pulling $app/state into the chunk).
		const params = new URLSearchParams(window.location.search);
		oauthError = oauthErrorCopy(params);
		if (params.has('error')) {
			history.replaceState(null, '', window.location.pathname);
		}
	});
</script>

<svelte:head>
	<title>{t.loginLabel} — Apostille Handbook</title>
</svelte:head>

<h1>{t.loginLabel}</h1>

{#if !webauthnOk}
	<p class="notice err" role="alert">
		This browser cannot do passkeys. Use a current Safari, Chrome, Edge, or Firefox.
	</p>
{:else if me}
	<p class="notice ok" role="status">
		Logged in. <a href={resolve('/(customer)/orders')}>Continue to {t.orderLabel}s</a>
		· <a href={resolve('/(customer)/security')}>Manage sign-in methods</a>
		· <button type="button" class="linklike" onclick={logout}>{t.logoutLabel}</button>
	</p>
{:else}
	{#if magicBusy}
		<p class="notice" role="status">Signing you in…</p>
	{/if}
	<form onsubmit={login}>
		<h2>Log in with a passkey</h2>
		<label for="account">User ID or email</label>
		<input
			id="account"
			name="account"
			type="text"
			bind:value={account}
			required
			autocomplete="username"
		/>
		<button type="submit" disabled={busy}>Log in</button>
	</form>

	<form onsubmit={register}>
		<h2>New here? Register a passkey</h2>
		<label for="display-name">Display name</label>
		<input id="display-name" name="display_name" type="text" bind:value={displayName} required />
		<label for="key-label">Key label (optional)</label>
		<input
			id="key-label"
			name="label"
			type="text"
			bind:value={label}
			placeholder="MacBook Pro"
		/>
		<label for="reg-email">Email (optional, for account recovery)</label>
		<input id="reg-email" name="email" type="email" bind:value={email} autocomplete="email" />
		<button type="submit" disabled={busy}>Register passkey</button>
	</form>

	{#if error}
		<p class="notice err" role="alert">{error}</p>
	{/if}

	{#each OAUTH_PROVIDERS as provider (provider)}
		<button type="button" class="oauth" disabled={busy} onclick={() => oauthLogin(provider)}>
			Continue with {OAUTH_LABELS[provider]}
		</button>
	{/each}

	<form onsubmit={magicRequest}>
		<h2>Or get an email link</h2>
		<label for="magic-email">Email</label>
		<input
			id="magic-email"
			name="magic_email"
			type="email"
			bind:value={magicEmail}
			required
			autocomplete="email"
			disabled={busy}
		/>
		<button type="submit" disabled={busy}>Send sign-in link</button>
	</form>

	{#if magicSent}
		<p class="notice" role="status">
			Check your inbox — if that address has an account, a sign-in link is on its way.
		</p>
	{/if}
{/if}

{#if oauthError}
	<p class="notice err" role="alert">{oauthError}</p>
{/if}

<style>
	form {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		max-width: 28rem;
		margin-bottom: 2rem;
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
	:root[data-theme='dark'] button {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	.oauth {
		max-width: 28rem;
		margin-bottom: 0.5rem;
		color: var(--text);
		background: var(--bg);
		border-color: var(--border);
	}
	.linklike {
		min-height: 0;
		padding: 0;
		font-weight: 400;
		color: var(--accent);
		background: none;
		border: none;
		text-decoration: underline;
		cursor: pointer;
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
</style>
