// OAuth redirect flows (spec customer-auth.md §§9–11): Google, Apple,
// Meta. Injectable-io like passkey.js: no direct window/fetch
// references so node tests can drive with fakes. Never stores
// tokens; the callback sets HttpOnly session cookies server-side.

/** Providers with a server-side start endpoint. @type {string[]} */
export const OAUTH_PROVIDERS = ['google', 'apple', 'meta'];

/** Button/heading labels per provider. @type {Record<string, string>} */
export const OAUTH_LABELS = { google: 'Google', apple: 'Apple', meta: 'Meta' };

/**
 * Server ?error= codes (callback failures) → friendly copy, per site
 * language. One provider-agnostic map (spec §D4): the callback
 * redirect carries no provider, and the page the user lands on shows
 * which button they pressed. Never render raw.
 * @type {Record<'en' | 'es' | 'fr', { errors: Record<string, string>, fallback: string }>}
 */
const CALLBACK_COPY = {
	en: {
		errors: {
			oauth_denied: 'Sign-in was cancelled — try again.',
			oauth_expired: 'The sign-in expired — try again.',
			oauth_failed: 'Sign-in failed — try again.',
			oauth_unavailable: 'That sign-in method is unavailable — use your passkey.',
			oauth_rate_limited: 'Too many attempts — wait a minute and try again.',
			link_required: 'Email already in use — log in with your passkey, then link from security.',
			already_linked: 'That account is already linked to another user.',
			account_locked: 'This account is locked — contact support.'
		},
		fallback: 'Sign-in failed — try again.'
	},
	es: {
		errors: {
			oauth_denied: 'Se canceló el inicio de sesión: inténtelo de nuevo.',
			oauth_expired: 'El inicio de sesión caducó: inténtelo de nuevo.',
			oauth_failed: 'No se pudo iniciar sesión: inténtelo de nuevo.',
			oauth_unavailable: 'Ese método de inicio de sesión no está disponible: use su llave de acceso.',
			oauth_rate_limited: 'Demasiados intentos: espere un minuto e inténtelo de nuevo.',
			link_required:
				'Ese correo ya está en uso: inicie sesión con su llave de acceso y vincúlelo desde Seguridad.',
			already_linked: 'Esa cuenta ya está vinculada a otro usuario.',
			account_locked: 'Esta cuenta está bloqueada: comuníquese con soporte.'
		},
		fallback: 'No se pudo iniciar sesión: inténtelo de nuevo.'
	},
	fr: {
		errors: {
			oauth_denied: 'La connexion a été annulée : réessayez.',
			oauth_expired: 'La connexion a expiré : réessayez.',
			oauth_failed: 'La connexion a échoué : réessayez.',
			oauth_unavailable: 'Cette méthode de connexion est indisponible : utilisez votre clé d’accès.',
			oauth_rate_limited: 'Trop de tentatives : patientez une minute puis réessayez.',
			link_required:
				'Cette adresse e-mail est déjà utilisée : connectez-vous avec votre clé d’accès, puis associez-la depuis Sécurité.',
			already_linked: 'Ce compte est déjà associé à un autre utilisateur.',
			account_locked: 'Ce compte est verrouillé : contactez l’assistance.'
		},
		fallback: 'La connexion a échoué : réessayez.'
	}
};

/** English callback copy (the tables above, by code). @type {Record<string, string>} */
export const OAUTH_ERROR_COPY = CALLBACK_COPY.en.errors;
export const OAUTH_FALLBACK_COPY = CALLBACK_COPY.en.fallback;

/**
 * Map the callback ?error= code to friendly copy (null when absent).
 * @param {URLSearchParams} params
 * @param {'en' | 'es' | 'fr'} [locale]
 */
export function oauthErrorCopy(params, locale = 'en') {
	const code = params.get('error');
	if (!code) return null;
	const copy = CALLBACK_COPY[locale] ?? CALLBACK_COPY.en;
	return Object.hasOwn(copy.errors, code) ? copy.errors[code] : copy.fallback;
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

/** Start-endpoint failure copy per site language. */
const START_COPY = {
	en: {
		unknown: 'Unknown sign-in method — try again.',
		/** @param {string} label */
		loginFirst: (label) => `Log in first to link ${label}.`,
		fresh: 'Log in again first (linking needs a fresh session).',
		/** @param {string} label */
		unavailable: (label) => `${label} sign-in is unavailable right now.`,
		/** @param {string} label */
		failed: (label) => `${label} sign-in failed — try again.`
	},
	es: {
		unknown: 'Método de inicio de sesión desconocido: inténtelo de nuevo.',
		/** @param {string} label */
		loginFirst: (label) => `Inicie sesión primero para vincular ${label}.`,
		fresh: 'Vuelva a iniciar sesión primero (vincular requiere una sesión reciente).',
		/** @param {string} label */
		unavailable: (label) => `El inicio de sesión con ${label} no está disponible ahora.`,
		/** @param {string} label */
		failed: (label) => `Falló el inicio de sesión con ${label}: inténtelo de nuevo.`
	},
	fr: {
		unknown: 'Méthode de connexion inconnue : réessayez.',
		/** @param {string} label */
		loginFirst: (label) => `Connectez-vous d’abord pour associer ${label}.`,
		fresh: 'Reconnectez-vous d’abord (l’association demande une session récente).',
		/** @param {string} label */
		unavailable: (label) => `La connexion avec ${label} est indisponible pour le moment.`,
		/** @param {string} label */
		failed: (label) => `La connexion avec ${label} a échoué : réessayez.`
	}
};

/**
 * Friendly copy for start-endpoint failures (status-mapped, never
 * raw). Provider-aware: the caller knows which button was pressed.
 * @param {number} status
 * @param {string} label OAUTH_LABELS[provider]
 * @param {'en' | 'es' | 'fr'} [locale]
 */
export function oauthStartError(status, label, locale = 'en') {
	const copy = START_COPY[locale] ?? START_COPY.en;
	if (status === 400) return copy.unknown;
	if (status === 401) return copy.loginFirst(label);
	if (status === 403) return copy.fresh;
	if (status === 429) return (CALLBACK_COPY[locale] ?? CALLBACK_COPY.en).errors.oauth_rate_limited;
	if (status === 503) return copy.unavailable(label);
	return copy.failed(label);
}
