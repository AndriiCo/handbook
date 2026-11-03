// Admin restart client (P393). Admin-only: never imported from
// (public) or (customer) routes.
//
// POST {apiBase}/admin/restart  body { confirm? }
//   202 { started_at, pending_keys, warnings }  drain + exit 75; systemd restarts
//   422 { error, reason }                        the next boot would fail; nothing ran
//   428 { error, confirm_word, risky_keys }      lockout keys pending; type the word
//   429 { error, retry_after }                   one restart per two minutes
// GET  {apiBase}/admin/version → { started_at, … } (polled until it moves)

import { authError, readErrorBody } from './http-errors.js';

/** Key families whose change can lock staff out (server: `LOCKOUT_PREFIXES`). */
export const LOCKOUT_PREFIXES = ['net.', 'tls.', 'admin.', 'auth.', 'acme.'];

/** The word staff type to confirm a lockout-risk restart. */
export const CONFIRM_WORD = 'restart';

/**
 * @param {string} key
 * @returns {boolean}
 */
export function isLockoutKey(key) {
	return LOCKOUT_PREFIXES.some((prefix) => key.startsWith(prefix));
}

/**
 * True when a key was saved after the running process started. Pure (tested).
 * @param {number | null | undefined} updatedAt
 * @param {number | null | undefined} startedAt
 * @returns {boolean}
 */
export function savedSinceStart(updatedAt, startedAt) {
	return typeof updatedAt === 'number' && typeof startedAt === 'number' && updatedAt > startedAt;
}

/**
 * The "restart needed" chip: a boot-time key saved since the last start.
 * Pure (tested).
 * @param {string} applies
 * @param {number | null | undefined} updatedAt
 * @param {number | null | undefined} startedAt
 * @returns {boolean}
 */
export function restartNeeded(applies, updatedAt, startedAt) {
	return applies === 'boot' && savedSinceStart(updatedAt, startedAt);
}

/**
 * @typedef {{ kind: 'accepted', startedAt: number, pendingKeys: string[], warnings: string[] }
 *   | { kind: 'refused', message: string }
 *   | { kind: 'confirm', message: string, word: string, riskyKeys: string[] }
 *   | { kind: 'wait', message: string, seconds: number }} RestartAnswer
 */

/**
 * Ask the server to restart. Known refusals resolve to a typed answer;
 * auth failures and anything unexpected throw.
 * @param {string} apiBase
 * @param {string} confirm
 * @param {typeof fetch} [fetchFn]
 * @param {() => Record<string, string>} [headers] CSRF headers
 * @returns {Promise<RestartAnswer>}
 */
export async function requestRestart(apiBase, confirm, fetchFn = fetch, headers = () => ({})) {
	const res = await fetchFn(`${apiBase}/admin/restart`, {
		method: 'POST',
		headers: { 'content-type': 'application/json', ...headers() },
		credentials: 'same-origin',
		body: JSON.stringify(confirm.trim() ? { confirm: confirm.trim() } : {})
	});
	if (res.status === 401 || res.status === 403) {
		throw new Error(
			authError(res.status, await readErrorBody(res)) ?? 'The admin host refused the request.'
		);
	}
	/** @type {any} */
	let data = {};
	try {
		data = await res.json();
	} catch {
		data = {};
	}
	const message = typeof data.error === 'string' ? data.error : `HTTP ${res.status}`;
	if (res.status === 202) {
		return {
			kind: 'accepted',
			startedAt: Number(data.started_at ?? 0),
			pendingKeys: Array.isArray(data.pending_keys) ? data.pending_keys : [],
			warnings: Array.isArray(data.warnings) ? data.warnings : []
		};
	}
	if (res.status === 422) {
		return { kind: 'refused', message: typeof data.reason === 'string' ? data.reason : message };
	}
	if (res.status === 428) {
		return {
			kind: 'confirm',
			message,
			word: typeof data.confirm_word === 'string' ? data.confirm_word : CONFIRM_WORD,
			riskyKeys: Array.isArray(data.risky_keys) ? data.risky_keys : []
		};
	}
	if (res.status === 429) {
		return { kind: 'wait', message, seconds: Number(data.retry_after ?? 120) };
	}
	throw new Error(`Restart failed: ${message}.`);
}

/**
 * The running process's start time, from the env badge endpoint.
 * @param {string} apiBase
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<number>}
 */
export async function fetchStartedAt(apiBase, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/admin/version`, {
		credentials: 'same-origin',
		cache: 'no-store'
	});
	if (!res.ok) throw new Error(`version: HTTP ${res.status}`);
	const data = await res.json();
	const started = Number(data?.started_at);
	if (!Number.isFinite(started)) throw new Error('version: no started_at');
	return started;
}

/**
 * Poll `/admin/version` once per interval until `started_at` differs
 * from `previous`. Failed polls (the server is down mid-restart) are
 * expected and skipped. Resolves the new `started_at`; rejects after
 * `timeoutMs` without a change.
 * @param {string} apiBase
 * @param {number} previous
 * @param {{ fetchFn?: typeof fetch, sleep?: (ms: number) => Promise<void>,
 *   intervalMs?: number, timeoutMs?: number, now?: () => number,
 *   onTick?: (elapsedMs: number) => void }} [opts]
 * @returns {Promise<number>}
 */
export async function waitForRestart(apiBase, previous, opts = {}) {
	const {
		fetchFn = fetch,
		sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
		intervalMs = 1000,
		timeoutMs = 120_000,
		now = Date.now,
		onTick = () => {}
	} = opts;
	const start = now();
	for (;;) {
		await sleep(intervalMs);
		const elapsed = now() - start;
		onTick(elapsed);
		try {
			const started = await fetchStartedAt(apiBase, fetchFn);
			if (started !== previous) return started;
		} catch {
			// Down or draining: keep polling.
		}
		if (elapsed >= timeoutMs) {
			throw new Error('The service has not come back yet.');
		}
	}
}
