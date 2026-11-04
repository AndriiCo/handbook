// Shared admin auth-failure copy (F3). Every admin-branch 403
// carries a machine `code` plus human `error` text that already
// names the detected IP — show the server's answer verbatim
// instead of guessing. "Logged in" appears only for
// staff_role_required: an IP/CSRF/host denial runs before any
// session check, so claiming a login state there is false.
// Admin-only: never imported from (public) or (customer) routes.

// Shown for the moment before the session guard below navigates to
// /staff-login?next=… (or where no guard runs).
export const SESSION_EXPIRED_COPY =
	'Staff session missing or expired — log in on the admin host again.';

/** Console pages that run without a session (no redirect from them). */
export const SIGN_IN_PATHS = ['/staff-login', '/enrol'];

/** Where sign-in lands when no usable `next` is given. */
export const DEFAULT_AFTER_SIGN_IN = '/dashboard';

/**
 * A same-origin console path to return to after sign-in, or the
 * default. Only `/…` paths on this origin pass (no `//host`, no
 * backslash or scheme tricks), never the sign-in pages themselves.
 * @param {string | null | undefined} raw the `next` query value
 * @returns {string}
 */
export function safeNext(raw) {
	if (typeof raw !== 'string' || raw.length === 0 || raw.length > 2048) return DEFAULT_AFTER_SIGN_IN;
	if (!raw.startsWith('/') || raw.startsWith('//') || /[\\\s]/.test(raw)) return DEFAULT_AFTER_SIGN_IN;
	let url;
	try {
		url = new URL(raw, 'https://console.invalid');
	} catch {
		return DEFAULT_AFTER_SIGN_IN;
	}
	if (url.origin !== 'https://console.invalid') return DEFAULT_AFTER_SIGN_IN;
	if (SIGN_IN_PATHS.includes(url.pathname)) return DEFAULT_AFTER_SIGN_IN;
	return `${url.pathname}${url.search}${url.hash}`;
}

/**
 * The sign-in URL that returns to `here` (pathname + search + hash).
 * @param {string} here
 * @returns {string}
 */
export function signInHref(here) {
	const next = safeNext(here);
	return next === DEFAULT_AFTER_SIGN_IN ? '/staff-login' : `/staff-login?next=${encodeURIComponent(next)}`;
}

/**
 * True when a response means the staff session is gone: a 401 from
 * the admin API, except the sign-in ceremonies (their 401 is a failed
 * passkey check) and the setup wizard (its 401 is a missing key).
 * @param {string} url request URL (absolute or path)
 * @param {number} status
 */
export function isSessionExpiry(url, status) {
	if (status !== 401) return false;
	let path;
	try {
		path = new URL(url, 'https://console.invalid').pathname;
	} catch {
		return false;
	}
	if (!path.startsWith('/apostille/api/v1/')) return false;
	return !/^\/apostille\/api\/v1\/(auth|setup)\//.test(path);
}

/**
 * Send every expired-session 401 to `/staff-login?next=…`: wraps the
 * page's `fetch` once (the console layout installs it), so the six
 * API clients and the pages that call `fetch` directly all land on
 * the sign-in page with a way back. While the setup window is open no
 * session can exist, so the 401 goes to the `/config` wizard instead.
 * On the sign-in pages it never redirects. The caller still gets its
 * response. Returns the uninstall function.
 * @param {{ fetch: typeof fetch, location: { pathname: string, search: string, hash: string, assign: (href: string) => void } }} win
 */
export function installSessionGuard(win) {
	const original = win.fetch;
	let leaving = false;
	/** @param {string} here */
	async function leave(here) {
		let dest = signInHref(here);
		try {
			const res = await original.call(win, '/apostille/api/v1/setup/status', { credentials: 'same-origin' });
			if (res.ok && (await res.json())?.setup_required === true) dest = '/config';
		} catch {
			// Status unreachable: sign-in is the safe default.
		}
		if (dest === '/config' && win.location.pathname === '/config') {
			leaving = false;
			return;
		}
		win.location.assign(dest);
	}
	/** @type {typeof fetch} */
	const guarded = async (input, init) => {
		const res = await original.call(win, input, init);
		const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
		const { pathname, search, hash } = win.location;
		if (!leaving && isSessionExpiry(url, res.status) && !SIGN_IN_PATHS.includes(pathname)) {
			leaving = true;
			void leave(`${pathname}${search}${hash}`);
		}
		return res;
	};
	win.fetch = guarded;
	return () => {
		if (win.fetch === guarded) win.fetch = original;
	};
}

/**
 * User-facing message for an admin auth failure, or null when the
 * status is not an auth failure.
 * @param {number} status
 * @param {{ error?: unknown, code?: unknown } | null} body parsed JSON body, if any
 * @returns {string | null}
 */
export function authError(status, body) {
	if (status === 401) return SESSION_EXPIRED_COPY;
	if (status !== 403) return null;
	const server = typeof body?.error === 'string' && body.error.length > 0 ? body.error : null;
	if (server) return server;
	switch (body?.code) {
		case 'ip_not_allowed':
			return 'The admin host refused this network address (403) — add it to admin.allow_ips, then restart apostille.';
		case 'staff_role_required':
			return 'Logged in, but the account lacks the staff role.';
		case 'csrf_failed':
			return 'The admin host rejected the request token (403) — reload and try again.';
		case 'admin_host_required':
			return 'This page is served from the admin host only (403).';
		default:
			return 'The admin host refused the request (403).';
	}
}

/**
 * Best-effort JSON body read that never throws (for cloned error responses).
 * @param {Response} res
 * @returns {Promise<{ error?: unknown, code?: unknown } | null>}
 */
export async function readErrorBody(res) {
	try {
		const data = await res.clone().json();
		return data !== null && typeof data === 'object' ? data : null;
	} catch {
		return null;
	}
}

/**
 * A refused admin request, carrying what triage needs: the HTTP
 * status, the server's `error` text, and the `x-request-id` the
 * server logged it under (the same id the browser's network tab
 * shows), plus whether a fresh passkey step-up would let it through.
 */
export class AdminRequestError extends Error {
	/**
	 * @param {string} message
	 * @param {{ status: number, requestId?: string | null, serverError?: string | null, stepUp?: boolean }} info
	 */
	constructor(message, info) {
		super(message);
		this.name = 'AdminRequestError';
		this.status = info.status;
		this.requestId = info.requestId ?? null;
		this.serverError = info.serverError ?? null;
		this.stepUp = info.stepUp === true;
	}
}

/**
 * True when a 403 asks for a fresh passkey step-up (config writes,
 * restarts): the server says `step_up: "required"` or the legacy
 * "step-up required" text.
 * @param {number} status
 * @param {{ error?: unknown, step_up?: unknown } | null} body
 */
export function isStepUpRefusal(status, body) {
	if (status !== 403 || !body) return false;
	if (body.step_up === 'required') return true;
	return typeof body.error === 'string' && body.error.startsWith('step-up required');
}

/**
 * Build the error for a non-ok admin response: the message ends with
 * the status and request id, so a screenshot or a copy-paste of the
 * notice is enough to find the server log line.
 * @param {Response} res
 * @param {string} fallback message when the server sent no `error`
 */
export async function adminRequestError(res, fallback) {
	const body = await readErrorBody(res);
	const requestId = res.headers?.get?.('x-request-id') ?? null;
	const serverError = typeof body?.error === 'string' && body.error ? body.error : null;
	const stepUp = isStepUpRefusal(res.status, body);
	const base = stepUp
		? 'Confirm it’s you: config changes need a passkey check from the last 10 minutes.'
		: (authError(res.status, body) ?? serverError ?? fallback);
	const suffix = requestId ? ` (HTTP ${res.status}, request ${requestId})` : ` (HTTP ${res.status})`;
	return new AdminRequestError(`${base}${suffix}`, {
		status: res.status,
		requestId,
		serverError,
		stepUp
	});
}
