// Magic-link request/consume (spec customer-auth.md §12 — Phase 5).
// Injectable-io like oauth.js: no direct window/fetch references
// so node tests can drive with fakes. Never stores tokens: the
// request returns ok:true either way (anti-enumeration) and the
// consume mints HttpOnly session cookies server-side.

/**
 * Request a sign-in link: POST the magic start endpoint. The
 * server answers ok:true for unknown addresses too, so ok only
 * means "if that address has an account, mail is on its way".
 * @param {string} email
 * @param {any} io {post}
 */
export async function requestMagic(email, io) {
	if (typeof email !== 'string' || !email.includes('@')) {
		return { ok: false, status: 400, body: { error: 'bad email' } };
	}
	return io.post('/auth/magic/start', { email });
}

/**
 * Consume a sign-in link: POST the fragment token. The caller
 * read it from location.hash (#t=) and stripped it already.
 * @param {string} token 64-hex from the emailed link
 * @param {any} io {post}
 */
export async function consumeMagic(token, io) {
	if (typeof token !== 'string' || !token) {
		return { ok: false, status: 400, body: { error: 'bad token' } };
	}
	return io.post('/auth/magic/consume', { token });
}

/**
 * Friendly copy for magic failures (status-mapped, never raw —
 * takes status only so server text can't leak). Shared literals
 * match the login page + oauth.js sentences exactly so the chunk
 * compresses (byte-budget trim).
 * @param {number} status
 * @param {'request'|'consume'} form
 */
export function magicError(status, form) {
	if (status === 429) return 'Too many attempts — wait a minute and try again.';
	if (form === 'consume') {
		if (status === 400 || status === 410) return 'Link expired — request a new one.';
		return 'Sign-in failed — request a new link.';
	}
	if (status === 400) return 'Check the email address and try again.';
	if (status === 503) return 'That sign-in method is unavailable — use your passkey.';
	return 'Sign-in failed — try again.';
}
