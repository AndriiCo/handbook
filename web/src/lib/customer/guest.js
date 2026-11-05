// The shared guest login (crates/auth/src/guest.rs): /login sends this
// address straight to the guest sign-in from any of its forms. The
// server alone decides whether the login still works.

export const GUEST_EMAIL = 'demo@andrii.ai';

/**
 * @param {string} input
 * @returns {boolean}
 */
export function isGuestEmail(input) {
	return input.trim().toLowerCase() === GUEST_EMAIL;
}

/**
 * The offer `/auth/providers` makes while the login works; anything
 * else is no offer, so the note stays hidden.
 * @param {unknown} body
 * @returns {{ email: string, lastDay: string } | null}
 */
export function guestOffer(body) {
	const guest = /** @type {{ guest?: { email?: unknown, last_day?: unknown } } | null} */ (body)
		?.guest;
	if (typeof guest?.email !== 'string' || typeof guest?.last_day !== 'string') return null;
	return { email: guest.email, lastDay: guest.last_day };
}

/**
 * A `YYYY-MM-DD` day in the visitor's language, the same day everywhere.
 * @param {string} day
 * @param {string} locale
 * @returns {string}
 */
export function guestDay(day, locale) {
	return new Date(`${day}T12:00:00Z`).toLocaleDateString(locale, {
		dateStyle: 'long',
		timeZone: 'UTC'
	});
}

/**
 * @param {string} email
 * @param {{ post: (path: string, body: unknown) => Promise<{ ok: boolean, status: number, body: any }> }} io
 */
export function guestLogin(email, { post }) {
	return post('/auth/guest/login', { email: email.trim() });
}
