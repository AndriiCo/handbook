// Admin setup-wizard API client (secure-config.md §9). Admin-only:
// never imported from (public) or (customer) routes.
//
// GET  {apiBase}/admin/setup/status          → { setup_required }
// GET  {apiBase}/admin/setup/steps           → { keys: [...] } (x-setup-key)
// POST {apiBase}/admin/setup/keys/{key}      → { key, type, set } (x-setup-key)
//
// The setup key rides a custom header per request (memory only,
// never stored): no cookies or session exist before enrolment.
// Rows reuse the config-client shape; secrets stay write-only.

import { normalizeRow, putBody } from './config-client.js';
import { validSetupKeyShape } from './staff-auth.js';

/**
 * Headers carrying the setup key. Pure (tested). Returns null when
 * the shape is wrong — the page shows the reason instead of sending.
 * @param {unknown} key
 * @returns {Record<string, string> | null}
 */
export function setupKeyHeaders(key) {
	if (typeof key !== 'string' || !validSetupKeyShape(key)) return null;
	return { 'x-setup-key': key };
}

/**
 * @param {Response} res
 * @param {string} what
 * @returns {Promise<any>}
 */
async function readJson(res, what) {
	let data;
	try {
		data = await res.json();
	} catch {
		throw new Error(`${what} returned invalid JSON.`);
	}
	return data;
}

/**
 * Server error detail for 422s (the API vocab lives in the body's
 * `error` field).
 * @param {Response} res
 * @returns {Promise<string | null>}
 */
async function errorDetail(res) {
	try {
		const data = await res.clone().json();
		return typeof data?.error === 'string' ? data.error : null;
	} catch {
		return null;
	}
}

/**
 * Shared status mapping for the key-gated endpoints.
 * @param {Response} res
 * @param {string} what
 * @returns {Promise<void>}
 */
async function throwForSetupStatus(res, what) {
	if (res.status === 401) {
		throw new Error('Setup key required — paste the key minted by `xtask setup-key-mint`.');
	}
	if (res.status === 403) {
		throw new Error(
			'Setup key invalid, or the window already closed (a passkey was enrolled).'
		);
	}
	if (res.status === 404) throw new Error(`${what} not found (404).`);
	if (res.status === 422) {
		const detail = await errorDetail(res);
		throw new Error(detail ? `${what}: ${detail}.` : `${what} refused the request (422).`);
	}
	if (res.status === 429) {
		throw new Error('Too many attempts — wait a minute and try again.');
	}
	if (res.status === 503) {
		throw new Error('The store is unavailable on this server (503).');
	}
	if (!res.ok) throw new Error(`${what} failed: HTTP ${res.status}.`);
}

/**
 * @param {string} apiBase
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ setup_required: boolean }>}
 */
export async function fetchSetupStatus(apiBase, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/admin/setup/status`, { credentials: 'same-origin' });
	if (res.status === 503) throw new Error('The store is unavailable on this server (503).');
	if (!res.ok) throw new Error(`Setup status failed: HTTP ${res.status}.`);
	const data = await readJson(res, 'Setup status');
	if (typeof data?.setup_required !== 'boolean') {
		throw new Error('Setup status returned an unexpected shape.');
	}
	return { setup_required: data.setup_required };
}

/**
 * @param {string} apiBase
 * @param {string} key
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<import('./config-client.js').ConfigRow[]>}
 */
export async function fetchSetupSteps(apiBase, key, fetchFn = fetch) {
	const headers = setupKeyHeaders(key);
	if (!headers) throw new Error('That setup key is malformed — check `setup_` + 64 hex chars.');
	const res = await fetchFn(`${apiBase}/admin/setup/steps`, {
		headers,
		credentials: 'same-origin'
	});
	await throwForSetupStatus(res, 'Setup steps');
	const data = await readJson(res, 'Setup steps');
	const rows = Array.isArray(data) ? data : data?.keys;
	if (!Array.isArray(rows)) throw new Error('Setup steps returned an unexpected shape.');
	return rows.map(normalizeRow).filter((row) => row !== null);
}

/**
 * @param {string} apiBase
 * @param {string} key
 * @param {import('./config-client.js').ConfigRow} row
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<true>}
 */
export async function saveSetupKey(apiBase, key, row, fetchFn = fetch) {
	const headers = setupKeyHeaders(key);
	if (!headers) throw new Error('That setup key is malformed — check `setup_` + 64 hex chars.');
	const res = await fetchFn(`${apiBase}/admin/setup/keys/${encodeURIComponent(row.key)}`, {
		method: 'POST',
		headers: { 'content-type': 'application/json', ...headers },
		credentials: 'same-origin',
		body: JSON.stringify(putBody(row))
	});
	await throwForSetupStatus(res, 'Setup save');
	return true;
}
