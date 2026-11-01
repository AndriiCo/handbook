<script lang="ts">
	import { resolve } from '$app/paths';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import { STRINGS, resolveLocale, type Locale } from '../../../lib/i18n';
	import {
		cookieValue,
		credentialRequestArgs,
		loginPasskey,
		registerPasskey,
		serializeAuthentication
	} from '../../../lib/customer/passkey.js';
	import {
		OAUTH_LABELS,
		OAUTH_PROVIDERS,
		oauthErrorCopy,
		oauthStartError,
		startOAuth
	} from '../../../lib/customer/oauth.js';
	import { consumeMagic, magicError, requestMagic } from '../../../lib/customer/magic.js';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import Guilloche from '../../../lib/components/brand/Guilloche.svelte';
	import ApostilleCertificate from '../../../lib/components/brand/ApostilleCertificate.svelte';
	import CheckIcon from '@lucide/svelte/icons/check';

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
	let providers = $state<string[] | null>(null);
	let magicEmail = $state('');
	let magicSent = $state(false);
	let magicBusy = $state(false);
	let magicStarted = false;
	let magicOpen = $state(false);
	let tab = $state<'signin' | 'create'>('signin');
	let conditionalStarted = false;

	// Official provider artwork, dropped in by the operator
	// (see lib/assets/brand/README.md). Missing file → text button.
	const brandUrls = import.meta.glob(
		'../../../lib/assets/brand/*.svg',
		{ eager: true, query: '?url', import: 'default' }
	) as Record<string, string>;
	const brandFor = (provider: string): string | null => {
		const key = Object.keys(brandUrls).find((k) => k.endsWith(`/${provider}.svg`));
		return key ? brandUrls[key] : null;
	};

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

	function signedInAs(userId: string) {
		localStorage.setItem('hb_last_user', userId);
		me = { user_id: userId };
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
			signedInAs(res.body.user_id);
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
			signedInAs(res.body.user_id);
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

	// Passkey autofill: offer the account's passkey through conditional
	// mediation when the browser supports it. Silent by design — any
	// failure falls through to the explicit button below.
	async function tryConditionalMediation() {
		try {
			if (conditionalStarted || typeof PublicKeyCredential === 'undefined') return;
			if (!PublicKeyCredential.isConditionalMediationAvailable) return;
			if (!(await PublicKeyCredential.isConditionalMediationAvailable())) return;
			conditionalStarted = true;
			const started = await post('/auth/passkey/login/start', { account: '' });
			if (!started.ok) return;
			const cred = await navigator.credentials.get({
				...credentialRequestArgs(started.body.options),
				mediation: 'conditional'
			} as CredentialRequestOptions);
			if (!cred) return;
			const finished = await post('/auth/passkey/login/finish', {
				challenge_id: started.body.challenge_id,
				credential: serializeAuthentication(cred)
			});
			if (finished.ok && typeof finished.body?.user_id === 'string') {
				signedInAs(finished.body.user_id);
			}
		} catch {
			// Autofill unavailable — the button path still works.
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
					signedInAs(res.body.user_id);
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
		// OAuth buttons render only for providers this deployment
		// configured (unknown ids are dropped, never rendered).
		fetch(api('/auth/providers'), { credentials: 'same-origin' })
			.then((res) => (res.ok ? res.json() : null))
			.then((body) => {
				const list = body?.providers;
				if (Array.isArray(list)) {
					providers = list.filter(
						(p) => typeof p === 'string' && OAUTH_PROVIDERS.includes(p)
					);
				} else {
					providers = [];
				}
			})
			.catch(() => {
				providers = [];
			});
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

<section
	class="grid items-stretch gap-8 py-10 lg:grid-cols-2 xl:py-16"
	aria-label={t.loginLabel}
>
	<div
		class="relative overflow-clip rounded-xl p-8 text-white sm:p-10"
		style="background: linear-gradient(135deg, #0e3a5c 0%, #07121c 70%);"
	>
		<Guilloche
			seed="login-panel"
			class="pointer-events-none absolute -top-16 -right-16 size-80 text-white opacity-20 [mask-image:radial-gradient(circle,black,transparent_70%)]"
		/>
		<div class="relative w-36 rotate-3 rounded-lg p-2 shadow-xl" style="background: var(--surface);">
			<ApostilleCertificate highlight={2} />
		</div>
		<p class="font-brand text-h2 relative mt-8 max-w-[20ch] font-medium text-balance">
			Your documents, verified quote by quote.
		</p>
		<ul class="relative mt-6 flex list-none flex-col gap-3 p-0 text-[0.95rem]">
			{#each ['Muse Spark drafts from official pages', 'Every quote checked byte-exact', 'Staff approve before anything publishes'] as point (point)}
				<li class="flex items-center gap-2">
					<CheckIcon class="size-5 flex-none" aria-hidden="true" />
					<span>{point}</span>
				</li>
			{/each}
		</ul>
	</div>

	<div class="mx-auto w-full max-w-[25rem]">
		<div class="rounded-xl border border-line bg-bg p-6 shadow-md">
			{#if !webauthnOk}
				<Heading level={1}>{t.loginLabel}</Heading>
				<p class="rounded-md border border-line bg-error-bg px-4 py-3" role="alert">
					This browser cannot do passkeys. Use a current Safari, Chrome, Edge, or Firefox.
				</p>
			{:else if me}
				<Heading level={1}>{t.loginLabel}</Heading>
				<p class="rounded-md border border-line px-4 py-3" role="status">
					Logged in. <a href={resolve('/(customer)/orders')}>Continue to {t.orderLabel}s</a>
					· <a href={resolve('/(customer)/security')}>Manage sign-in methods</a>
					· <button
						type="button"
						class="min-h-0 cursor-pointer border-0 bg-transparent p-0 font-normal text-accent-strong underline"
						onclick={logout}>{t.logoutLabel}</button
					>
				</p>
			{:else}
				<Heading level={1}>{tab === 'signin' ? t.loginLabel : 'Create account'}</Heading>
				<div class="mt-4 flex rounded-md border border-line bg-surface p-1" role="tablist" aria-label="Sign in or create account">
					{#each [{ v: 'signin', label: t.loginLabel }, { v: 'create', label: 'Create account' }] as opt (opt.v)}
						<button
							type="button"
							role="tab"
							aria-selected={tab === opt.v}
							onclick={() => (tab = opt.v as 'signin' | 'create')}
							class={`min-h-11 flex-1 cursor-pointer rounded-sm px-3 text-sm font-medium transition-colors ${
								tab === opt.v ? 'bg-bg text-ink shadow-sm' : 'text-ink-muted hover:text-ink'
							}`}
						>
							{opt.label}
						</button>
					{/each}
				</div>

				{#if magicBusy}
					<p class="rounded-md border border-line px-4 py-3" role="status">Signing you in…</p>
				{/if}

				{#if tab === 'signin'}
					<form class="mt-4 flex flex-col gap-2" onsubmit={login}>
						<label class="font-bold" for="account">User ID or email</label>
						<input
							class="field"
							id="account"
							name="account"
							type="text"
							bind:value={account}
							required
							autocomplete="username webauthn"
							onfocus={() => void tryConditionalMediation()}
						/>
						<button
							class="mt-2 min-h-12 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
							type="submit"
							disabled={busy}>Sign in with passkey</button
						>
					</form>

					<div class="my-4 flex items-center gap-3 text-sm text-ink-muted" aria-hidden="true">
						<span class="h-px flex-1 bg-line"></span>
						<span>or</span>
						<span class="h-px flex-1 bg-line"></span>
					</div>

					{#if providers === null}
						<div role="status" aria-label="Checking sign-in options">
							<span class="sr-only">Checking sign-in options</span>
							<Skeleton class="h-11 w-full" />
						</div>
					{:else}
						{#each providers as provider (provider)}
							{@const logo = brandFor(provider)}
							<button
								class="mb-2 flex min-h-11 w-full cursor-pointer items-center justify-center gap-2 rounded-sm border border-line bg-bg px-4 font-bold text-ink disabled:cursor-wait disabled:opacity-60"
								type="button"
								disabled={busy}
								onclick={() => oauthLogin(provider)}
							>
								{#if logo}
									<img src={logo} alt="" width="18" height="18" class="size-[18px] flex-none" />
								{/if}
								Continue with {OAUTH_LABELS[provider] ?? provider}
							</button>
						{/each}
					{/if}

					<button
						type="button"
						aria-expanded={magicOpen}
						onclick={() => (magicOpen = !magicOpen)}
						class="mt-2 inline-flex min-h-11 cursor-pointer items-center font-medium text-accent-strong underline decoration-dotted underline-offset-4"
					>
						{magicOpen ? 'Hide the email link option' : 'Use an email link instead'}
					</button>
					{#if magicOpen}
						<form class="mt-2 flex flex-col gap-2" onsubmit={magicRequest}>
							<label class="font-bold" for="magic-email">Email</label>
							<input
								class="field"
								id="magic-email"
								name="magic_email"
								type="email"
								bind:value={magicEmail}
								required
								autocomplete="email"
								disabled={busy}
							/>
							<button
								class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink disabled:cursor-wait disabled:opacity-60"
								type="submit"
								disabled={busy}>Send sign-in link</button
							>
						</form>
						{#if magicSent}
							<p class="rounded-md border border-line px-4 py-3" role="status">
								Check your inbox — if that address has an account, a sign-in link is on its way.
							</p>
						{/if}
					{/if}
				{:else}
					<form class="mt-4 flex flex-col gap-2" onsubmit={register}>
						<label class="font-bold" for="display-name">Display name</label>
						<input
							class="field"
							id="display-name"
							name="display_name"
							type="text"
							bind:value={displayName}
							required
						/>
						<label class="font-bold" for="key-label">Key label (optional)</label>
						<input
							class="field"
							id="key-label"
							name="label"
							type="text"
							bind:value={label}
							placeholder="MacBook Pro"
						/>
						<label class="font-bold" for="reg-email">Email (optional, for account recovery)</label>
						<input
							class="field"
							id="reg-email"
							name="email"
							type="email"
							bind:value={email}
							autocomplete="email"
						/>
						<button
							class="mt-2 min-h-12 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
							type="submit"
							disabled={busy}>Register passkey</button
						>
					</form>
				{/if}

				{#if error}
					<p class="rounded-md border border-line bg-error-bg px-4 py-3" role="alert">{error}</p>
				{/if}
			{/if}
		</div>
	</div>
</section>

{#if oauthError}
	<p class="max-w-[28rem] rounded-md border border-line bg-error-bg px-4 py-3" role="alert">{oauthError}</p>
{/if}
