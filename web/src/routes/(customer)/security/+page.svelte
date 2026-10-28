<script lang="ts">
	import { resolve } from '$app/paths';
	import {
		cookieValue,
		credentialRequestArgs,
		registerPasskey,
		serializeAuthentication
	} from '../../../lib/customer/passkey.js';
	import {
		OAUTH_LABELS,
		OAUTH_PROVIDERS,
		oauthStartError,
		startOAuth
	} from '../../../lib/customer/oauth.js';

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

	async function linkOAuth(provider: string) {
		error = null;
		busy = true;
		const res = await startOAuth(
			{ post, redirect: (u: string) => (window.location.href = u) },
			provider,
			'link'
		);
		// On ok the browser is navigating away; on failure the page
		// stays and the buttons re-enable.
		busy = false;
		if (!res.ok) error = oauthStartError(res.status, OAUTH_LABELS[provider] ?? provider);
	}

	async function removeOAuth(method: string, id: string) {
		error = null;
		busy = true;
		try {
			// No passkey ceremony: an OAuth-only user has none. The
			// server accepts a fresh session as the re-auth proof
			// for OAuth unlinks (spec §9.4, all methods).
			const res = await post('/auth/profile/unlink', {
				method,
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
	<p class="max-w-[28rem] rounded-sm border border-line px-4 py-3">
		Log in first to manage your sign-in methods.
	</p>
	<p><a href={resolve('/(customer)/login')}>Go to log in</a></p>
{:else}
	{#if methods.length === 0}
		<p class="max-w-[28rem] rounded-sm border border-line px-4 py-3">
			No sign-in methods yet — add your first below.
		</p>
	{:else}
		<ul class="max-w-[28rem] list-none p-0">
			<!-- Passkey rows use the credential label; OAuth rows
				(google/apple/meta) use the provider label. -->
			{#each methods as method (method.method + ':' + method.id)}
				<li class="mb-2 flex items-baseline gap-3 rounded-sm border border-line px-[0.8rem] py-[0.6rem]">
					<strong>{(OAUTH_LABELS[method.method] ?? method.label) || 'Unnamed key'}</strong>
					<span class="text-ink-muted">
						{OAUTH_LABELS[method.method] ? method.label || 'linked account' : method.method}
					</span>
					<button
						class="ml-auto min-h-9 cursor-pointer rounded-sm border border-line bg-bg font-normal text-ink [font:inherit] disabled:cursor-wait disabled:opacity-60"
						type="button"
						disabled={busy}
						onclick={() =>
							method.method === 'passkey'
								? removeKey(method.id)
								: removeOAuth(method.method, method.id)}
					>
						Remove
					</button>
				</li>
			{/each}
		</ul>
	{/if}

	<form class="my-6 flex max-w-[28rem] flex-col gap-2" onsubmit={addKey}>
		<h2 class="m-0 text-[1.1rem]">Add another passkey</h2>
		<label class="font-bold" for="new-label">Key label</label>
		<input
			class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink [font:inherit]"
			id="new-label"
			name="label"
			type="text"
			bind:value={label}
			placeholder="YubiKey"
		/>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill font-bold text-accent-ink [font:inherit] disabled:cursor-wait disabled:opacity-60"
			type="submit"
			disabled={busy}>Add passkey</button
		>
	</form>

	{#each OAUTH_PROVIDERS as provider (provider)}
		<form
			class="my-6 flex max-w-[28rem] flex-col gap-2"
			onsubmit={(e) => {
				e.preventDefault();
				linkOAuth(provider);
			}}
		>
			<h2 class="m-0 text-[1.1rem]">Or link {OAUTH_LABELS[provider]}</h2>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill font-bold text-accent-ink [font:inherit] disabled:cursor-wait disabled:opacity-60"
				type="submit"
				disabled={busy}
			>
				Link {OAUTH_LABELS[provider]} account
			</button>
		</form>
	{/each}

	{#if error}
		<p class="max-w-[28rem] rounded-sm border border-line bg-warning-bg px-4 py-3" role="alert">{error}</p>
	{/if}
{/if}
