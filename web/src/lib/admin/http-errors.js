// Shared admin auth-failure copy (F3). Every admin-branch 403
// carries a machine `code` plus human `error` text that already
// names the detected IP — show the server's answer verbatim
// instead of guessing. "Logged in" appears only for
// staff_role_required: an IP/CSRF/host denial runs before any
// session check, so claiming a login state there is false.
// Admin-only: never imported from (public) or (customer) routes.

export const SESSION_EXPIRED_COPY =
	'Staff session missing or expired — log in on the admin host again.';

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
