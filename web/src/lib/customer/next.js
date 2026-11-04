// Where /login sends a visitor after they sign in (`?next=`). Only a
// same-origin path under /apostille/ is honoured: anything else (//host,
// a scheme, a backslash, a path outside /apostille/, dot segments or
// their percent-encoded forms) is refused, so the parameter can never
// become an open redirect. Pure and injectable like passkey.js, so node
// tests drive it with fakes.

const PREFIX = '/apostille/';
/** A fixed base: parsing never depends on where the page runs. */
const BASE = 'https://next.invalid';
const MAX_LEN = 2048;
/** Remembered across an OAuth or email-link round trip (this tab only). */
export const NEXT_KEY = 'hb_login_next';
/** The email link lives 15 minutes and an OAuth start 5; nothing older counts. */
export const NEXT_TTL_MS = 15 * 60 * 1000;

/**
 * The validated target (normalised path, query and hash), or null.
 * @param {unknown} raw the decoded `?next=` value
 * @returns {string | null}
 */
export function safeNext(raw) {
	if (typeof raw !== 'string' || raw.length > MAX_LEN) return null;
	// Refuses //host, https://host, javascript:, /other and relative paths.
	if (!raw.startsWith(PREFIX)) return null;
	// Backslashes, spaces and control characters: the URL parser drops
	// tabs and newlines and reads `\` as `/`, so `/\t/host` would leave.
	if (/[\\\u0000- \u007f]/.test(raw)) return null;
	// Encoded dots, slashes, backslashes and percent signs in the path
	// (`%2e%2e`, `..%2f`, `%5c`, `%252e`): no real page here needs them.
	const path = raw.split(/[?#]/, 1)[0];
	if (/%(2e|2f|5c|25)/i.test(path)) return null;
	let url;
	try {
		url = new URL(raw, BASE);
	} catch {
		return null;
	}
	// Dot segments resolve before this check: /apostille/../x is /x.
	if (url.origin !== BASE || !url.pathname.startsWith(PREFIX)) return null;
	// Back to /login would only show this page again.
	if (url.pathname === '/apostille/login' || url.pathname.startsWith('/apostille/login/')) {
		return null;
	}
	return url.pathname + url.search + url.hash;
}

/**
 * Remember `next` before a round trip that leaves the page (OAuth, an
 * email link opened in this tab). Unsafe values clear it instead.
 * @param {Pick<Storage, 'setItem' | 'removeItem'> | null | undefined} storage
 * @param {unknown} next
 * @param {number} [now] ms since the epoch
 */
export function rememberNext(storage, next, now = Date.now()) {
	const safe = safeNext(next);
	try {
		if (safe) storage?.setItem(NEXT_KEY, JSON.stringify({ next: safe, at: now }));
		else storage?.removeItem(NEXT_KEY);
	} catch {
		// Blocked storage: the round trip still signs in, on /login.
	}
}

/**
 * Read and forget a remembered `next`: null when absent, older than
 * NEXT_TTL_MS, from the future, malformed or unsafe.
 * @param {Pick<Storage, 'getItem' | 'removeItem'> | null | undefined} storage
 * @param {number} [now] ms since the epoch
 * @returns {string | null}
 */
export function takeNext(storage, now = Date.now()) {
	let raw = null;
	try {
		raw = storage?.getItem(NEXT_KEY) ?? null;
		storage?.removeItem(NEXT_KEY);
	} catch {
		return null;
	}
	if (!raw) return null;
	try {
		const saved = JSON.parse(raw);
		const at = saved?.at;
		if (typeof at !== 'number' || at > now || now - at > NEXT_TTL_MS) return null;
		return safeNext(saved.next);
	} catch {
		return null;
	}
}
