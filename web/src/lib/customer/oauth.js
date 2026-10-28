// OAuth redirect flows (spec customer-auth.md §§9–11): Google, Apple,
// Meta. Injectable-io like passkey.js: no direct window/fetch
// references so node tests can drive with fakes. Never stores
// tokens; the callback sets HttpOnly session cookies server-side.

/** Providers with a server-side start endpoint. @type {string[]} */
export const OAUTH_PROVIDERS = ['google', 'apple', 'meta'];

/** Button/heading labels per provider. @type {Record<string, string>} */
export const OAUTH_LABELS = { google: 'Google', apple: 'Apple', meta: 'Meta' };

/**
 * Server ?error= codes (callback failures) → friendly copy. One
 * provider-agnostic map (spec §D4): the callback redirect carries
 * no provider, and the page the user lands on shows which button
 * they pressed. Never render raw.
 * @type {Record<string, string>}
 */
export const OAUTH_ERROR_COPY = {
	oauth_denied: 'Sign-in was cancelled — try again.',
	oauth_expired: 'The sign-in expired — try again.',
	oauth_failed: 'Sign-in failed — try again.',
	oauth_unavailable: 'That sign-in method is unavailable — use your passkey.',
	oauth_rate_limited: 'Too many attempts — wait a minute and try again.',
	link_required: 'Email already in use — log in with your passkey, then link from security.',
	already_linked: 'That account is already linked to another user.',
	account_locked: 'This account is locked — contact support.'
};
export const OAUTH_FALLBACK_COPY = 'Sign-in failed — try again.';

/**
 * Map the callback ?error= code to friendly copy (null when absent).
 * @param {URLSearchParams} params
 */
export function oauthErrorCopy(params) {
	const code = params.get('error');
	if (!code) return null;
	return OAUTH_ERROR_COPY[code] ?? OAUTH_FALLBACK_COPY;
}

/**
 * Start an OAuth flow: POST the provider start endpoint, then
 * redirect to auth_url. The auth_url is https-gated: a non-https
 * URL short-circuits instead of navigating (defense in depth
 * against open redirects). Unknown providers are rejected without
 * touching the network (the provider names a URL path).
 * @param {any} io {post, redirect(url)}
 * @param {string} provider 'google'|'apple'|'meta'
 * @param {'login'|'link'} mode
 */
export async function startOAuth(io, provider, mode) {
	if (!OAUTH_PROVIDERS.includes(provider)) {
		return { ok: false, status: 400, body: { error: 'bad provider' } };
	}
	const res = await io.post(`/auth/${provider}/start`, { mode });
	if (!res.ok) return res;
	if (typeof res.body?.auth_url !== 'string' || !res.body.auth_url.startsWith('https://')) {
		return { ok: false, status: 502, body: { error: 'bad auth_url' } };
	}
	io.redirect(res.body.auth_url);
	return { ok: true, status: 200, body: {} };
}

/**
 * Friendly copy for start-endpoint failures (status-mapped, never
 * raw). Provider-aware: the caller knows which button was pressed.
 * @param {number} status
 * @param {string} label OAUTH_LABELS[provider]
 */
export function oauthStartError(status, label) {
	if (status === 400) return `Unknown sign-in method — try again.`;
	if (status === 401) return `Log in first to link ${label}.`;
	if (status === 403) return 'Log in again first (linking needs a fresh session).';
	if (status === 429) return OAUTH_ERROR_COPY.oauth_rate_limited;
	if (status === 503) return `${label} sign-in is unavailable right now.`;
	return `${label} sign-in failed — try again.`;
}
