// Shared staff API helpers for the admin console (MISSION WIN SA-26).
// Admin-only: never imported from (public) or (customer) routes.
// The admin host serves its API under `/apostille` too, so the root
// is literal (the admin build's `$app/paths` base is '').
import { csrfHeaders } from './config-client.js';
import { authError, readErrorBody } from './http-errors.js';

export const API = '/apostille/api/v1';

/** Error carrying the HTTP status (0 = network / unreadable body). */
export class ApiError extends Error {
	status: number;
	constructor(message: string, status: number) {
		super(message);
		this.status = status;
	}
}

async function fail(res: Response, what: string): Promise<never> {
	const body = await readErrorBody(res);
	const auth = authError(res.status, body);
	if (auth) throw new ApiError(auth, res.status);
	const detail = typeof body?.error === 'string' && body.error ? `: ${body.error}` : '';
	throw new ApiError(`${what} failed (HTTP ${res.status})${detail}.`, res.status);
}

async function readBody<T>(res: Response, what: string): Promise<T> {
	try {
		return (await res.json()) as T;
	} catch {
		// The static demo host answers unknown API paths with the SPA
		// shell: treat a non-JSON body as "endpoint unavailable".
		throw new ApiError(`${what} is unavailable on this host.`, 0);
	}
}

/** GET a staff JSON endpoint (path relative to `/apostille/api/v1`). */
export async function getJson<T>(path: string, what: string): Promise<T> {
	let res: Response;
	try {
		res = await fetch(`${API}${path}`, { credentials: 'same-origin' });
	} catch {
		throw new ApiError(`${what} could not reach the admin host.`, 0);
	}
	if (!res.ok) await fail(res, what);
	return readBody<T>(res, what);
}

/** POST JSON with the CSRF double-submit pair the admin guard requires. */
export async function postJson<T>(path: string, body: unknown, what: string): Promise<T> {
	let res: Response;
	try {
		res = await fetch(`${API}${path}`, {
			method: 'POST',
			headers: { 'content-type': 'application/json', ...csrfHeaders() },
			credentials: 'same-origin',
			body: JSON.stringify(body ?? {})
		});
	} catch {
		throw new ApiError(`${what} could not reach the admin host.`, 0);
	}
	if (!res.ok) await fail(res, what);
	return readBody<T>(res, what);
}

export function errorText(err: unknown, fallback: string): string {
	return err instanceof Error && err.message ? err.message : fallback;
}
