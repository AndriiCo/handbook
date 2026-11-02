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
