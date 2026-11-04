<script lang="ts">
	import { resolve } from '$app/paths';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import { STRINGS } from '../../../lib/i18n';
	import { site } from '../../../lib/locale.svelte.js';
	import { LOGIN_COPY } from '../../../lib/customer/login-copy.js';
	import {
		cookieValue,
		credentialRequestArgs,
		deviceLabel,
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
	import { rememberNext, safeNext, takeNext } from '../../../lib/customer/next.js';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import Guilloche from '../../../lib/components/brand/Guilloche.svelte';
	import ApostilleCertificate from '../../../lib/components/brand/ApostilleCertificate.svelte';
	import CheckIcon from '@lucide/svelte/icons/check';

	let fullName = $state('');
	let email = $state('');
	let account = $state(
		typeof localStorage === 'undefined' ? '' : (localStorage.getItem('hb_last_user') ?? '')
	);
	let error = $state<string | null>(null);
	let busy = $state(false);
	let me = $state<{ user_id: string } | null>(null);
	// The callback's ?error= (read once, then dropped from the address).
	let oauthParams = $state<URLSearchParams | null>(null);
	let providers = $state<string[] | null>(null);
	let magicEmail = $state('');
	let magicSent = $state(false);
	let magicBusy = $state(false);
	let magicStarted = false;
	let magicOpen = $state(false);
	let tab = $state<'signin' | 'create'>('signin');
	let conditionalStarted = false;
	// Where to go once signed in: `?next=` (a same-origin /apostille/
	// path, lib/customer/next.js), else one this tab remembered before an
	// OAuth or email-link round trip. Null stays on this page.
	let next: string | null = null;
	let leaving = $state(false);

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
	const t = $derived(STRINGS[site.locale]);
	const c = $derived(LOGIN_COPY[site.locale]);
	const oauthError = $derived(oauthParams ? oauthErrorCopy(oauthParams, site.locale) : null);
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
		if (status === 400) return body.error ?? c.checkForm;
		if (status === 401) return c.unknownAccount;
		if (status === 404) return c.passkeyGone;
		if (status === 409) return c.passkeyTaken;
		if (status === 410) return c.expired;
		if (status === 422) return c.verifyFailed;
		if (status === 429) return c.rateLimited;
		return body.error ?? c.requestFailed(status);
	}

	/** This tab's session storage, or null where storage is blocked. */
	function tabStore(): Storage | null {
		try {
			return window.sessionStorage;
		} catch {
			return null;
		}
	}

	/** Back to the page that sent the visitor here; /login leaves history. */
	function goNext() {
		if (!next || leaving) return;
		rememberNext(tabStore(), null);
		leaving = true;
		window.location.replace(next);
	}

	function signedInAs(userId: string) {
		localStorage.setItem('hb_last_user', userId);
		me = { user_id: userId };
		goNext();
	}

	async function register(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		busy = true;
		try {
			// The key is named for this site and device, e.g.
			// "andrii.ai - macOS Safari" (the Security page lists it).
			const label = deviceLabel(location.hostname, navigator.userAgent, navigator.maxTouchPoints);
			const res = await registerPasskey(
				{ displayName: fullName.trim(), label, email: email.trim() },
				{ credentials: navigator.credentials, post }
			);
			if (!res.ok) {
				error = friendly(res.status, res.body);
				return;
			}
			signedInAs(res.body.user_id);
		} catch (e) {
			error = e instanceof DOMException && e.name === 'AbortError'
				? c.registerCancelled
				: c.registerFailed;
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
				? c.loginCancelled
				: c.loginFailed;
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
		// The provider returns to /login with no query: this tab keeps
		// `next` for the landing (an unsafe or absent one clears it).
		rememberNext(tabStore(), next);
		const res = await startOAuth(
			{ post, redirect: (u: string) => (window.location.href = u) },
			provider,
			'login'
		);
		// On ok the browser is navigating away; on failure the page
		// stays and the buttons re-enable.
		busy = false;
		if (!res.ok) {
			rememberNext(tabStore(), null);
			error = oauthStartError(res.status, OAUTH_LABELS[provider] ?? provider, site.locale);
		}
	}

	async function magicRequest(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		busy = true;
		try {
			const res = await requestMagic(magicEmail.trim(), { post });
			if (!res.ok) {
				error = magicError(res.status, 'request', site.locale);
				return;
			}
			// Identical for unknown addresses (anti-enumeration:
			// the server answers 200 either way).
			magicSent = true;
			// The emailed link lands on /login without a query; opened in
			// this tab, it returns to `next` (another tab or device cannot).
			rememberNext(tabStore(), next);
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
		// Read before any sign-in below can finish. The remembered value
		// is read once either way, so a stale one never lingers.
		const remembered = takeNext(tabStore());
		next = safeNext(new URLSearchParams(window.location.search).get('next')) ?? remembered;
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
						error = magicError(res.status, 'consume', site.locale);
						return;
					}
					signedInAs(res.body.user_id);
				})
				.catch(() => {
					error = magicError(500, 'consume', site.locale);
				})
				.finally(() => {
					magicBusy = false;
				});
		}
		fetch(api('/auth/me'), { credentials: 'same-origin' })
			.then((res) => (res.ok ? res.json() : null))
			.then((body) => {
				if (!body?.user_id) return;
				me = { user_id: body.user_id };
				// Already signed in, or back from an OAuth provider.
				goNext();
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
		oauthParams = params;
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
		class="relative order-2 overflow-clip rounded-xl p-8 text-white sm:p-10 lg:order-1"
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
			{c.tagline}
		</p>
		<ul class="relative mt-6 flex list-none flex-col gap-3 p-0 text-[0.95rem]">
			{#each c.points as point (point)}
				<li class="flex items-center gap-2">
					<CheckIcon class="size-5 flex-none" aria-hidden="true" />
					<span>{point}</span>
				</li>
			{/each}
		</ul>
	</div>

	<div class="order-1 mx-auto w-full max-w-[25rem] lg:order-2">
		<div class="rounded-xl border border-line bg-bg p-6 shadow-md">
			{#if !webauthnOk}
				<Heading level={1}>{t.loginLabel}</Heading>
				<p class="rounded-md border border-line bg-error-bg px-4 py-3" role="alert">
					{c.noPasskeys}
				</p>
			{:else if me && leaving}
				<Heading level={1}>{t.loginLabel}</Heading>
				<p class="rounded-md border border-line px-4 py-3" role="status">{c.signedInBack}</p>
			{:else if me}
				<Heading level={1}>{t.loginLabel}</Heading>
				<p class="rounded-md border border-line px-4 py-3" role="status">
					{c.loggedIn} <a href={resolve('/(customer)/orders')}>{c.continueToOrders}</a>
					· <a href={resolve('/(customer)/security')}>{c.manageSignIn}</a>
					· <button
						type="button"
						class="min-h-0 cursor-pointer border-0 bg-transparent p-0 font-normal text-accent-strong underline"
						onclick={logout}>{t.logoutLabel}</button
					>
				</p>
			{:else}
				<Heading level={1}>{tab === 'signin' ? t.loginLabel : c.createAccount}</Heading>
				<div class="mt-4 flex rounded-md border border-line bg-surface p-1" role="tablist" aria-label={c.tabsLabel}>
					{#each [{ v: 'signin', label: t.loginLabel }, { v: 'create', label: c.createAccount }] as opt (opt.v)}
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
					<p class="rounded-md border border-line px-4 py-3" role="status">{c.signingIn}</p>
				{/if}

				{#if tab === 'signin'}
					<form class="mt-4 flex flex-col gap-2" onsubmit={login}>
						<label class="font-bold" for="account">{c.account}</label>
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
							disabled={busy}>{c.passkeySignIn}</button
						>
					</form>

					<div class="my-4 flex items-center gap-3 text-sm text-ink-muted" aria-hidden="true">
						<span class="h-px flex-1 bg-line"></span>
						<span>{c.or}</span>
						<span class="h-px flex-1 bg-line"></span>
					</div>

					{#if providers === null}
						<div role="status" aria-label={c.checkingOptions}>
							<span class="sr-only">{c.checkingOptions}</span>
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
								{c.continueWith(OAUTH_LABELS[provider] ?? provider)}
							</button>
						{/each}
					{/if}

					<button
						type="button"
						aria-expanded={magicOpen}
						onclick={() => (magicOpen = !magicOpen)}
						class="mt-2 inline-flex min-h-11 cursor-pointer items-center font-medium text-accent-strong underline decoration-dotted underline-offset-4"
					>
						{magicOpen ? c.magicHide : c.magicShow}
					</button>
					{#if magicOpen}
						<form class="mt-2 flex flex-col gap-2" onsubmit={magicRequest}>
							<label class="font-bold" for="magic-email">{c.email}</label>
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
								disabled={busy}>{c.sendLink}</button
							>
						</form>
						{#if magicSent}
							<p class="rounded-md border border-line px-4 py-3" role="status">
								{c.magicSent}
							</p>
						{/if}
					{/if}
				{:else}
					<form class="mt-4 flex flex-col gap-2" onsubmit={register}>
						<label class="font-bold" for="full-name">{c.fullName}</label>
						<input
							class="field"
							id="full-name"
							name="full_name"
							type="text"
							bind:value={fullName}
							required
							maxlength="128"
							autocomplete="name"
						/>
						<label class="font-bold" for="reg-email">{c.email}</label>
						<input
							class="field"
							id="reg-email"
							name="email"
							type="email"
							bind:value={email}
							required
							autocomplete="email"
							aria-describedby="reg-email-hint"
						/>
						<p class="m-0 text-sm text-ink-muted" id="reg-email-hint">
							{c.emailHint}
						</p>
						<button
							class="mt-2 min-h-12 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
							type="submit"
							disabled={busy}>{c.registerPasskey}</button
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
