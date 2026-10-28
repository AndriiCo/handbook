// Google OAuth redirect flow (spec customer-auth.md §9).
// Injectable-io like passkey.js: no direct window/fetch references
// so node tests can drive with fakes. Never stores tokens; the
// callback sets HttpOnly session cookies server-side.

/**
 * Server ?error= codes (callback failures) → friendly copy. Never render raw.
 * @type {Record<string, string>}
 */
export const GOOGLE_ERROR_COPY = {
	oauth_denied: 'Google sign-in was cancelled — try again.',
	oauth_expired: 'The sign-in expired — try again.',
	oauth_failed: 'Google sign-in failed — try again.',
	oauth_unavailable: 'Google sign-in is unavailable — use your passkey.',
	oauth_rate_limited: 'Too many attempts — wait a minute and try again.',
	link_required: 'Email already in use — log in with your passkey, then link Google.',
	already_linked: 'That Google account is already linked to another user.',
	account_locked: 'This account is locked — contact support.'
};
export const GOOGLE_FALLBACK_COPY = 'Google sign-in failed — try again.';

/**
 * Map the callback ?error= code to friendly copy (null when absent).
 * @param {URLSearchParams} params
 */
export function googleErrorCopy(params) {
	const code = params.get('error');
	if (!code) return null;
	return GOOGLE_ERROR_COPY[code] ?? GOOGLE_FALLBACK_COPY;
}

/**
 * Start Google flow: POST start endpoint, then redirect to auth_url.
 * The auth_url is https-gated: a non-https URL short-circuits
 * instead of navigating (defense in depth against open redirects).
 * @param {any} io {post, redirect(url)}
 * @param {'login'|'link'} mode
 */
export async function startGoogle(io, mode) {
	const res = await io.post('/auth/google/start', { mode });
	if (!res.ok) return res;
	if (typeof res.body?.auth_url !== 'string' || !res.body.auth_url.startsWith('https://')) {
		return { ok: false, status: 502, body: { error: 'bad auth_url' } };
	}
	io.redirect(res.body.auth_url);
	return { ok: true, status: 200, body: {} };
}

/**
 * Friendly copy for start-endpoint failures (status-mapped, never raw).
 * @param {number} status
 */
export function googleStartError(status) {
	if (status === 401) return 'Log in first to link Google.';
	if (status === 403) return 'Log in again first (linking needs a fresh session).';
	if (status === 429) return GOOGLE_ERROR_COPY.oauth_rate_limited;
	if (status === 503) return GOOGLE_ERROR_COPY.oauth_unavailable;
	return GOOGLE_FALLBACK_COPY;
}
