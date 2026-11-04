<script lang="ts">
	import { resolve } from '$app/paths';
	import {
		cookieValue,
		credentialRequestArgs,
		deviceLabel,
		registerPasskey,
		serializeAuthentication
	} from '../../../lib/customer/passkey.js';
	import {
		OAUTH_LABELS,
		OAUTH_PROVIDERS,
		oauthStartError,
		startOAuth
	} from '../../../lib/customer/oauth.js';
	import { site } from '../../../lib/locale.svelte.js';
	import { SECURITY_COPY } from '../../../lib/customer/security-copy.js';

	type Method = { method: string; id: string; label: string; disabled: boolean };

	let methods = $state<Method[]>([]);
	let userId = $state('');
	let label = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;
	const c = $derived(SECURITY_COPY[site.locale]);

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
			error = c.loadFailed;
		}
	}

	/** "andrii.ai - macOS Safari": the name a key gets when none is typed. */
	const autoLabel = () =>
		deviceLabel(location.hostname, navigator.userAgent, navigator.maxTouchPoints);

	async function addKey(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		busy = true;
		try {
			const res = await registerPasskey(
				{ displayName: userId, label: label.trim() || autoLabel() },
				{ credentials: navigator.credentials, post }
			);
			if (!res.ok) {
				error =
					res.status === 403
						? c.freshSessionKey
						: ((res.body.error as string) ?? c.requestFailed(res.status));
				return;
			}
			label = '';
			await refresh();
		} catch (e) {
			error =
				e instanceof DOMException && e.name === 'AbortError'
					? c.addCancelled
					: c.ceremonyFailed;
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
				error = c.reauthFailed;
				return;
			}
			const args = credentialRequestArgs(started.body.options);
			const cred = await navigator.credentials.get(args);
			const finished = await post('/auth/passkey/login/finish', {
				challenge_id: started.body.challenge_id,
				credential: serializeAuthentication(cred)
			});
			if (!finished.ok) {
				error = c.reauthFailed;
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
						? c.lastKey
						: ((res.body.error as string) ?? c.requestFailed(res.status));
				return;
			}
			await refresh();
		} catch (e) {
			error =
				e instanceof DOMException && e.name === 'AbortError'
					? c.removeCancelled
					: c.ceremonyFailed;
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
		if (!res.ok) error = oauthStartError(res.status, OAUTH_LABELS[provider] ?? provider, site.locale);
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
						? c.lastMethod
						: res.status === 403
							? c.freshSessionUnlink
							: ((res.body.error as string) ?? c.requestFailed(res.status));
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
	<title>{c.title} — Apostille Handbook</title>
</svelte:head>

<h1>{c.title}</h1>

{#if !userId}
	<p class="max-w-[28rem] rounded-sm border border-line px-4 py-3">
		{c.logInFirst}
	</p>
	<p><a href={resolve('/(customer)/login')}>{c.goToLogin}</a></p>
{:else}
	{#if methods.length === 0}
		<p class="max-w-[28rem] rounded-sm border border-line px-4 py-3">
			{c.none}
		</p>
	{:else}
		<ul class="max-w-[28rem] list-none p-0">
			<!-- Passkey rows use the credential label; OAuth rows
				(google/apple/meta) use the provider label. -->
			{#each methods as method (method.method + ':' + method.id)}
				<li class="mb-2 flex items-baseline gap-3 rounded-sm border border-line px-[0.8rem] py-[0.6rem]">
					<strong>{(OAUTH_LABELS[method.method] ?? method.label) || c.unnamedKey}</strong>
					<span class="text-ink-muted">
						{OAUTH_LABELS[method.method]
							? method.label || c.linkedAccount
							: method.method === 'passkey'
								? c.passkeyKind
								: method.method}
					</span>
					<button
						class="ml-auto min-h-9 cursor-pointer rounded-sm border border-line bg-bg font-normal text-ink disabled:cursor-wait disabled:opacity-60"
						type="button"
						disabled={busy}
						onclick={() =>
							method.method === 'passkey'
								? removeKey(method.id)
								: removeOAuth(method.method, method.id)}
					>
						{c.remove}
					</button>
				</li>
			{/each}
		</ul>
	{/if}

	<form class="my-6 flex max-w-[28rem] flex-col gap-2" onsubmit={addKey}>
		<h2 class="m-0 text-[1.1rem]">{c.addHeading}</h2>
		<label class="font-bold" for="new-label">{c.keyLabel}</label>
		<input
			class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
			id="new-label"
			name="label"
			type="text"
			bind:value={label}
			maxlength="64"
			placeholder={typeof navigator === 'undefined' ? '' : autoLabel()}
		/>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
			type="submit"
			disabled={busy}>{c.addPasskey}</button
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
			<h2 class="m-0 text-[1.1rem]">{c.orLink(OAUTH_LABELS[provider])}</h2>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
				type="submit"
				disabled={busy}
			>
				{c.linkAccount(OAUTH_LABELS[provider])}
			</button>
		</form>
	{/each}

	{#if error}
		<p class="max-w-[28rem] rounded-sm border border-line bg-warning-bg px-4 py-3" role="alert">{error}</p>
	{/if}
{/if}
