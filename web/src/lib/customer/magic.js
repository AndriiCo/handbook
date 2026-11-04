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

/** Magic-link failure copy per site language. */
const MAGIC_COPY = {
	en: {
		rateLimited: 'Too many attempts — wait a minute and try again.',
		linkExpired: 'Link expired — request a new one.',
		consumeFailed: 'Sign-in failed — request a new link.',
		badEmail: 'Check the email address and try again.',
		unavailable: 'That sign-in method is unavailable — use your passkey.',
		failed: 'Sign-in failed — try again.'
	},
	es: {
		rateLimited: 'Demasiados intentos: espere un minuto e inténtelo de nuevo.',
		linkExpired: 'El enlace caducó: solicite uno nuevo.',
		consumeFailed: 'No se pudo iniciar sesión: solicite un enlace nuevo.',
		badEmail: 'Revise la dirección de correo electrónico e inténtelo de nuevo.',
		unavailable: 'Ese método de inicio de sesión no está disponible: use su llave de acceso.',
		failed: 'No se pudo iniciar sesión: inténtelo de nuevo.'
	},
	fr: {
		rateLimited: 'Trop de tentatives : patientez une minute puis réessayez.',
		linkExpired: 'Le lien a expiré : demandez-en un nouveau.',
		consumeFailed: 'La connexion a échoué : demandez un nouveau lien.',
		badEmail: 'Vérifiez l’adresse e-mail et réessayez.',
		unavailable: 'Cette méthode de connexion est indisponible : utilisez votre clé d’accès.',
		failed: 'La connexion a échoué : réessayez.'
	}
};

/**
 * Friendly copy for magic failures (status-mapped, never raw —
 * takes status only so server text can't leak), in the site language.
 * @param {number} status
 * @param {'request'|'consume'} form
 * @param {'en' | 'es' | 'fr'} [locale]
 */
export function magicError(status, form, locale = 'en') {
	const copy = MAGIC_COPY[locale] ?? MAGIC_COPY.en;
	if (status === 429) return copy.rateLimited;
	if (form === 'consume') {
		if (status === 400 || status === 410) return copy.linkExpired;
		return copy.consumeFailed;
	}
	if (status === 400) return copy.badEmail;
	if (status === 503) return copy.unavailable;
	return copy.failed;
}
