// Admin config API client (secure-config.md §1.5). Admin-only: never
// imported from (public) or (customer) routes.
//
// GET  {apiBase}/admin/config        → [{ key, type, value?|set? }]
// PUT  {apiBase}/admin/config/{key}  → { set: true } (secrets) or saved row
// SECRET values are write-only: the server never returns them and this
// module never renders or stores one — set-new-only.

/**
 * Normalized row for the config page. Secret rows carry `set`, never a value.
 * @typedef {object} ConfigRow
 * @property {string} key
 * @property {'bool' | 'text' | 'secret'} type
 * @property {boolean | string} [value]
 * @property {boolean} [set]
 * @property {string} [draft]
 * @property {boolean} dirty
 * @property {boolean} saving
 * @property {string | null} error
 */

/**
 * Normalize one server row into UI state. Returns null for unknown shapes.
 * Pure (tested). Secret rows keep NO value — only the `set` flag.
 * @param {any} row
 * @returns {ConfigRow | null}
 */
export function normalizeRow(row) {
	if (!row || typeof row.key !== 'string' || typeof row.type !== 'string') return null;
	if (row.type === 'bool') {
		if (typeof row.value !== 'boolean') return null;
		return { key: row.key, type: 'bool', value: row.value, dirty: false, saving: false, error: null };
	}
	if (row.type === 'text') {
		if (typeof row.value !== 'string') return null;
		return { key: row.key, type: 'text', value: row.value, dirty: false, saving: false, error: null };
	}
	if (row.type === 'secret') {
		if (typeof row.set !== 'boolean') return null;
		return { key: row.key, type: 'secret', set: row.set, draft: '', dirty: false, saving: false, error: null };
	}
	return null;
}

/**
 * Build the PUT body for a row. Pure (tested).
 * @param {ConfigRow} row
 * @returns {{ value: unknown }}
 */
export function putBody(row) {
	if (row.type === 'bool') return { type: 'bool', value: row.value === true };
	if (row.type === 'text') return { type: 'text', value: row.value };
	return { type: 'secret', value: row.draft };
}

/**
 * @param {string} apiBase
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<ConfigRow[]>}
 */
export async function fetchConfig(apiBase, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/admin/config`, { credentials: 'same-origin' });
	if (res.status === 401 || res.status === 403) {
		throw new Error('Staff session required — log in on the admin host first.');
	}
	if (res.status === 404) {
		throw new Error('Config API not found (404) — the server lane has not landed yet.');
	}
	if (!res.ok) throw new Error(`Config load failed: HTTP ${res.status}.`);
	let data;
	try {
		data = await res.json();
	} catch {
		throw new Error('Config API returned invalid JSON.');
	}
	const rows = Array.isArray(data) ? data : data?.keys;
	if (!Array.isArray(rows)) throw new Error('Config API returned an unexpected shape.');
	return rows.map(normalizeRow).filter((row) => row !== null);
}

/**
 * @param {string} apiBase
 * @param {ConfigRow} row
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<true>}
 */
export async function saveConfigKey(apiBase, row, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/admin/config/${encodeURIComponent(row.key)}`, {
		method: 'PUT',
		headers: { 'content-type': 'application/json' },
		credentials: 'same-origin',
		body: JSON.stringify(putBody(row))
	});
	if (res.status === 401 || res.status === 403) {
		throw new Error('Staff session expired — log in again, then retry.');
	}
	if (res.status === 404) {
		throw new Error(`Unknown key “${row.key}” (404).`);
	}
	if (res.status === 422) {
		throw new Error(`Rejected by the server (422) — check the value for “${row.key}”.`);
	}
	if (!res.ok) throw new Error(`Save failed: HTTP ${res.status}.`);
	return true;
}
