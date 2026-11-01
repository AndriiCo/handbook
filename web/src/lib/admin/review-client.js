// Admin review-queue API client (analysis-agent.md §11). Admin-only:
// never imported from (public) or (customer) routes.
//
// GET  {apiBase}/queue[?authority&topic&limit&after_id] → { items, count }
// GET  {apiBase}/queue/{id}                               → QueueItem
// POST {apiBase}/facts/{id}/accept  {reason?, statement?} → { status }
// POST {apiBase}/facts/{id}/reject  {reason, suppress?}   → { status }
// GET  {apiBase}/suppress-rules[?authority&include_revoked] → { rules, count }
// POST {apiBase}/suppress-rules      {statement, ...}      → { id }
// POST {apiBase}/suppress-rules/{id}/revoke               → { status }
// GET  {apiBase}/suppressions[?authority&limit&after_id]  → { suppressions, count }
//
// The agent's `reasoning` field renders only under an explicit
// do-not-trust label: it is untrusted model output, never evidence.

import { csrfHeaders } from './config-client.js';

/**
 * @typedef {object} QueueDiff
 * @property {string} base_statement
 * @property {number} base_fact_id
 * @property {string} base_status
 */

/**
 * @typedef {object} QueueCitation
 * @property {number} capture_id
 * @property {number} start_byte
 * @property {number} end_byte
 * @property {string} quote
 */

/**
 * @typedef {object} QueueItem
 * @property {number} id
 * @property {string} authority_slug
 * @property {string} topic
 * @property {string} statement
 * @property {string} status
 * @property {string} created_by
 * @property {number} created_at
 * @property {QueueDiff | null} diff
 * @property {QueueCitation[]} citations
 * @property {string | null} reasoning_untrusted
 * @property {{ conflict: boolean, conflicts: { id: number, statement: string, status: string }[], open_tasks: number }} blast_radius
 * @property {{ capture_id: number, capture_kind: string, trust: string, authenticity: string, sidecar: string }} provenance
 */

/**
 * @typedef {object} SuppressRule
 * @property {number} id
 * @property {number | null} authority_id
 * @property {string | null} authority_slug
 * @property {string | null} topic
 * @property {string} statement
 * @property {string} reason
 * @property {string} created_by
 * @property {number} created_at
 * @property {number | null} revoked_at
 */

/**
 * @typedef {object} Suppression
 * @property {number} id
 * @property {number} capture_id
 * @property {string} authority_slug
 * @property {string} topic
 * @property {string} statement
 * @property {string} path
 * @property {number | null} rule_id
 * @property {number} created_at
 */

/**
 * Normalize one queue item. Returns null for unknown shapes. Pure (tested).
 * @param {any} row
 * @returns {QueueItem | null}
 */
export function normalizeQueueItem(row) {
	if (!row || typeof row.id !== 'number' || typeof row.authority_slug !== 'string') return null;
	if (typeof row.topic !== 'string' || typeof row.statement !== 'string') return null;
	if (typeof row.status !== 'string' || typeof row.created_by !== 'string') return null;
	if (typeof row.created_at !== 'number') return null;
	if (!Array.isArray(row.citations)) return null;
	const citations = [];
	for (const c of row.citations) {
		if (!c || typeof c.capture_id !== 'number' || typeof c.quote !== 'string') return null;
		if (typeof c.start_byte !== 'number' || typeof c.end_byte !== 'number') return null;
		citations.push({
			capture_id: c.capture_id,
			start_byte: c.start_byte,
			end_byte: c.end_byte,
			quote: c.quote
		});
	}
	let diff = null;
	if (row.diff !== null && row.diff !== undefined) {
		const d = row.diff;
		if (typeof d.base_statement !== 'string' || typeof d.base_fact_id !== 'number') return null;
		if (typeof d.base_status !== 'string') return null;
		diff = { base_statement: d.base_statement, base_fact_id: d.base_fact_id, base_status: d.base_status };
	}
	const blast = row.blast_radius;
	if (!blast || typeof blast.conflict !== 'boolean' || !Array.isArray(blast.conflicts)) return null;
	if (typeof blast.open_tasks !== 'number') return null;
	const conflicts = [];
	for (const c of blast.conflicts) {
		if (!c || typeof c.id !== 'number' || typeof c.statement !== 'string') return null;
		if (typeof c.status !== 'string') return null;
		conflicts.push({ id: c.id, statement: c.statement, status: c.status });
	}
	const prov = row.provenance;
	if (!prov || typeof prov.capture_id !== 'number' || typeof prov.capture_kind !== 'string') return null;
	if (typeof prov.trust !== 'string' || typeof prov.authenticity !== 'string') return null;
	if (typeof prov.sidecar !== 'string') return null;
	return {
		id: row.id,
		authority_slug: row.authority_slug,
		topic: row.topic,
		statement: row.statement,
		status: row.status,
		created_by: row.created_by,
		created_at: row.created_at,
		diff,
		citations,
		reasoning_untrusted:
			typeof row.reasoning_untrusted === 'string' ? row.reasoning_untrusted : null,
		blast_radius: { conflict: blast.conflict, conflicts, open_tasks: blast.open_tasks },
		provenance: {
			capture_id: prov.capture_id,
			capture_kind: prov.capture_kind,
			trust: prov.trust,
			authenticity: prov.authenticity,
			sidecar: prov.sidecar
		}
	};
}

/**
 * Normalize one suppress rule. Returns null for unknown shapes. Pure (tested).
 * @param {any} row
 * @returns {SuppressRule | null}
 */
export function normalizeRule(row) {
	if (!row || typeof row.id !== 'number' || typeof row.statement !== 'string') return null;
	if (typeof row.reason !== 'string' || typeof row.created_by !== 'string') return null;
	if (typeof row.created_at !== 'number') return null;
	return {
		id: row.id,
		authority_id: typeof row.authority_id === 'number' ? row.authority_id : null,
		authority_slug: typeof row.authority_slug === 'string' ? row.authority_slug : null,
		topic: typeof row.topic === 'string' ? row.topic : null,
		statement: row.statement,
		reason: row.reason,
		created_by: row.created_by,
		created_at: row.created_at,
		revoked_at: typeof row.revoked_at === 'number' ? row.revoked_at : null
	};
}

/**
 * Normalize one suppression-log row. Returns null for unknown shapes. Pure (tested).
 * @param {any} row
 * @returns {Suppression | null}
 */
export function normalizeSuppression(row) {
	if (!row || typeof row.id !== 'number' || typeof row.capture_id !== 'number') return null;
	if (typeof row.authority_slug !== 'string' || typeof row.topic !== 'string') return null;
	if (typeof row.statement !== 'string' || typeof row.path !== 'string') return null;
	if (typeof row.created_at !== 'number') return null;
	return {
		id: row.id,
		capture_id: row.capture_id,
		authority_slug: row.authority_slug,
		topic: row.topic,
		statement: row.statement,
		path: row.path,
		rule_id: typeof row.rule_id === 'number' ? row.rule_id : null,
		created_at: row.created_at
	};
}

/**
 * Build an accept body. Pure (tested). Returns null when invalid —
 * the page shows the reason instead of sending. The server
 * re-validates (statement 1–2000 chars when present).
 * @param {unknown} reason
 * @param {unknown} statement
 * @returns {{ reason?: string, statement?: string } | null}
 */
export function acceptBody(reason, statement) {
	/** @type {{ reason?: string, statement?: string }} */
	const body = {};
	if (typeof reason === 'string' && reason.trim().length > 0) {
		body.reason = reason.trim();
	}
	if (typeof statement === 'string') {
		const text = statement.trim();
		if (text.length === 0 || text.length > 2000) return null;
		body.statement = statement;
	} else if (statement !== undefined && statement !== null && statement !== '') {
		return null;
	}
	return body;
}

/**
 * Build a reject body. Pure (tested). Returns null when invalid —
 * reject requires a 1–1024 char reason; suppress scope is
 * `authority` or `global` only.
 * @param {unknown} reason
 * @param {unknown} suppress
 * @returns {{ reason: string, suppress?: { scope: string, lock_topic?: boolean } } | null}
 */
export function rejectBody(reason, suppress) {
	if (typeof reason !== 'string') return null;
	const text = reason.trim();
	if (text.length === 0 || text.length > 1024) return null;
	/** @type {{ reason: string, suppress?: { scope: string, lock_topic?: boolean } }} */
	const body = { reason: text };
	if (suppress !== undefined && suppress !== null) {
		if (typeof suppress !== 'object') return null;
		const scope = /** @type {{ scope?: unknown, lock_topic?: unknown }} */ (suppress).scope;
		if (scope !== 'authority' && scope !== 'global') return null;
		/** @type {{ scope: string, lock_topic?: boolean }} */
		const rule = { scope };
		const lock = /** @type {{ lock_topic?: unknown }} */ (suppress).lock_topic;
		if (lock !== undefined) {
			if (typeof lock !== 'boolean') return null;
			rule.lock_topic = lock;
		}
		body.suppress = rule;
	}
	return body;
}

/**
 * Build a standalone suppress-rule body. Pure (tested). Returns null
 * when invalid — the server re-validates (statement 1–2000 chars,
 * reason ≤1024, topic against the frozen enum).
 * @param {unknown} statement
 * @param {unknown} authority
 * @param {unknown} topic
 * @param {unknown} reason
 * @returns {{ statement: string, authority?: string, topic?: string, reason?: string } | null}
 */
export function ruleBody(statement, authority, topic, reason) {
	if (typeof statement !== 'string') return null;
	if (statement.trim().length === 0 || statement.length > 2000) return null;
	/** @type {{ statement: string, authority?: string, topic?: string, reason?: string }} */
	const body = { statement };
	if (typeof authority === 'string') {
		if (authority.trim().length > 0) body.authority = authority.trim();
	} else if (authority !== undefined && authority !== null) {
		return null;
	}
	if (typeof topic === 'string') {
		if (topic.trim().length > 0) body.topic = topic.trim();
	} else if (topic !== undefined && topic !== null) {
		return null;
	}
	if (typeof reason === 'string') {
		if (reason.length === 0) {
			// Dropped below: blank means omitted.
		} else if (reason.length > 1024) {
			return null;
		} else {
			body.reason = reason;
		}
	} else if (reason !== undefined && reason !== null) {
		return null;
	}
	return body;
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
 * Server error detail for 409/422 (the API vocab lives in the
 * body's `error` field: `fact is not proposed`, `duplicate rule`,
 * `rule already revoked`, validation messages).
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
 * @param {Response} res
 * @param {string} what
 * @returns {Promise<void>}
 */
async function throwForStatus(res, what) {
	if (res.status === 401) {
		throw new Error('Staff session missing or expired — log in on the admin host again.');
	}
	if (res.status === 403) {
		throw new Error('Logged in, but the admin host refused the request — the browser must present your enrolled client certificate.');
	}
	if (res.status === 404) throw new Error(`${what} not found (404).`);
	if (res.status === 409) {
		const detail = await errorDetail(res);
		throw new Error(detail ? `${what}: ${detail}.` : `${what} conflicts (409).`);
	}
	if (res.status === 422) {
		const detail = await errorDetail(res);
		throw new Error(detail ? `${what}: ${detail}.` : `${what} refused the request (422).`);
	}
	if (res.status === 503) {
		throw new Error('The store is unavailable on this server (503).');
	}
	if (!res.ok) throw new Error(`${what} failed: HTTP ${res.status}.`);
}

/**
 * @param {Record<string, string | number | boolean | undefined>} params
 * @returns {string}
 */
function queryString(params) {
	const search = new URLSearchParams();
	for (const [key, value] of Object.entries(params)) {
		if (value === undefined || value === '') continue;
		search.set(key, String(value));
	}
	const text = search.toString();
	return text.length > 0 ? `?${text}` : '';
}

/**
 * @param {string} apiBase
 * @param {{ authority?: string, topic?: string, limit?: number, after_id?: number }} [params]
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ items: QueueItem[], count: number }>}
 */
export async function fetchQueue(apiBase, params = {}, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/queue${queryString(params)}`, {
		credentials: 'same-origin'
	});
	await throwForStatus(res, 'Review queue');
	const data = await readJson(res, 'Review queue');
	const items = data?.items;
	if (!Array.isArray(items)) throw new Error('Review queue returned an unexpected shape.');
	return {
		items: items.map(normalizeQueueItem).filter((row) => row !== null),
		count: typeof data.count === 'number' ? data.count : 0
	};
}

/**
 * @param {string} apiBase
 * @param {number} id
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<QueueItem>}
 */
export async function fetchQueueItem(apiBase, id, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/queue/${id}`, { credentials: 'same-origin' });
	await throwForStatus(res, 'Queue item');
	const item = normalizeQueueItem(await readJson(res, 'Queue item'));
	if (!item) throw new Error('Queue item returned an unexpected shape.');
	return item;
}

/**
 * @param {string} apiBase
 * @param {string} path
 * @param {unknown} body
 * @param {string} what
 * @param {typeof fetch} fetchFn
 * @returns {Promise<any>}
 */
async function postReview(apiBase, path, body, what, fetchFn) {
	const res = await fetchFn(`${apiBase}${path}`, {
		method: 'POST',
		headers: { 'content-type': 'application/json', ...csrfHeaders() },
		credentials: 'same-origin',
		body: JSON.stringify(body ?? {})
	});
	await throwForStatus(res, what);
	return readJson(res, what);
}

/**
 * @param {string} apiBase
 * @param {number} id
 * @param {{ reason?: string, statement?: string }} body
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ status: string }>}
 */
export async function acceptFact(apiBase, id, body, fetchFn = fetch) {
	const data = await postReview(apiBase, `/facts/${id}/accept`, body, 'Accept', fetchFn);
	if (!data || typeof data.status !== 'string') throw new Error('Accept returned an unexpected shape.');
	return { status: data.status };
}

/**
 * @param {string} apiBase
 * @param {number} id
 * @param {{ reason: string, suppress?: { scope: string, lock_topic?: boolean } }} body
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ status: string }>}
 */
export async function rejectFact(apiBase, id, body, fetchFn = fetch) {
	const data = await postReview(apiBase, `/facts/${id}/reject`, body, 'Reject', fetchFn);
	if (!data || typeof data.status !== 'string') throw new Error('Reject returned an unexpected shape.');
	return { status: data.status };
}

/**
 * @param {string} apiBase
 * @param {{ authority?: string, include_revoked?: boolean }} [params]
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ rules: SuppressRule[], count: number }>}
 */
export async function fetchRules(apiBase, params = {}, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/suppress-rules${queryString(params)}`, {
		credentials: 'same-origin'
	});
	await throwForStatus(res, 'Suppress rules');
	const data = await readJson(res, 'Suppress rules');
	const rules = data?.rules;
	if (!Array.isArray(rules)) throw new Error('Suppress rules returned an unexpected shape.');
	return {
		rules: rules.map(normalizeRule).filter((row) => row !== null),
		count: typeof data.count === 'number' ? data.count : 0
	};
}

/**
 * @param {string} apiBase
 * @param {{ statement: string, authority?: string, topic?: string, reason?: string }} body
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ id: number }>}
 */
export async function createRule(apiBase, body, fetchFn = fetch) {
	const data = await postReview(apiBase, '/suppress-rules', body, 'Create rule', fetchFn);
	if (!data || typeof data.id !== 'number') {
		throw new Error('Create rule returned an unexpected shape.');
	}
	return { id: data.id };
}

/**
 * @param {string} apiBase
 * @param {number} id
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<unknown>}
 */
export function revokeRule(apiBase, id, fetchFn = fetch) {
	return postReview(apiBase, `/suppress-rules/${id}/revoke`, {}, 'Revoke rule', fetchFn);
}

/**
 * @param {string} apiBase
 * @param {{ authority?: string, limit?: number, after_id?: number }} [params]
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ suppressions: Suppression[], count: number }>}
 */
export async function fetchSuppressions(apiBase, params = {}, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/suppressions${queryString(params)}`, {
		credentials: 'same-origin'
	});
	await throwForStatus(res, 'Suppression log');
	const data = await readJson(res, 'Suppression log');
	const skips = data?.suppressions;
	if (!Array.isArray(skips)) {
		throw new Error('Suppression log returned an unexpected shape.');
	}
	return {
		suppressions: skips.map(normalizeSuppression).filter((row) => row !== null),
		count: typeof data.count === 'number' ? data.count : 0
	};
}
