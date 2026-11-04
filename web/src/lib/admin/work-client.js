// Admin work-queue API client (work-queue.md §3). Admin-only: never
// imported from (public) or (customer) routes.
//
// GET  {apiBase}/admin/work-items?status=&limit=&before_id= → { items, workers, kinds, counts }
// POST {apiBase}/admin/work-items {kind, payload, max_attempts?} → 201 { id, status }
// GET  {apiBase}/admin/work-items/{id}        → { item, audit }
// GET  {apiBase}/admin/work-items/{id}/result → result bytes (attachment)
// POST {apiBase}/admin/work-items/{id}/retry  → { id, status } (failed → pending)
// POST {apiBase}/admin/work-items/{id}/cancel → { id, status } (pending or leased → failed)
// POST {apiBase}/admin/workers/token {name}  → { name, token } (shown once; fresh step-up)
// POST {apiBase}/admin/work-items/{id}/stage-packs → { id, dir, packs, count }
//      (done research.nonparty with a result only: 409 otherwise, 422 when
//      the result is not a packs.v1 bundle; re-staging overwrites)
//
// Shapes: docs/contracts/admin-work-items.json. `lease_owner` is the
// worker name only; lease tokens never leave the server. Unknown kinds
// and non-object payloads answer 422, payloads over 64 KiB 413.

import { csrfHeaders } from './config-client.js';
import { authError, readErrorBody } from './http-errors.js';

/** @typedef {'pending' | 'leased' | 'done' | 'failed'} WorkStatus */

/**
 * @typedef {object} WorkResult
 * @property {string} sha256
 * @property {number} bytes
 * @property {string} content_type
 */

/**
 * @typedef {object} WorkItem
 * @property {number} id
 * @property {string} kind
 * @property {string} status
 * @property {number} attempts
 * @property {number} max_attempts
 * @property {string | null} lease_owner
 * @property {number | null} lease_expires_at
 * @property {string | null} created_by
 * @property {number} created_at
 * @property {number} updated_at
 * @property {string | null} error
 * @property {WorkResult | null} result
 * @property {unknown} summary
 * @property {unknown} payload
 * @property {boolean} staged `<store>/packs-inbox/<id>/` exists
 * @property {number | null} retry_after pending, backing off until this unix time
 */

/**
 * @typedef {object} WorkWorker
 * @property {number | null} conn socket id (two sockets may share a name)
 * @property {string} name
 * @property {'shared' | 'named' | null} identity which token it connected with
 * @property {boolean | null} version_matches same build as the server (null: unknown)
 * @property {string[]} capabilities
 * @property {string | null} version
 * @property {number | null} connected_at
 * @property {number | null} last_seen_at
 * @property {number | null} job_id
 */

/**
 * @typedef {object} WorkKind
 * @property {string} kind
 * @property {string} about
 * @property {Record<string, unknown>} example
 * @property {number | null} max_runtime_secs a lease running longer fails
 */

/** @typedef {{ pending: number, leased: number, done: number, failed: number }} WorkCounts */

/**
 * @typedef {object} WorkList
 * @property {WorkItem[]} items
 * @property {WorkWorker[]} workers
 * @property {WorkKind[]} kinds
 * @property {WorkCounts} counts
 * @property {number} offset seconds to add to this browser's clock to read the server's
 */

/**
 * @typedef {object} AuditRow
 * @property {number} at
 * @property {string} actor
 * @property {string} action
 * @property {string | null} reason
 */

/** @typedef {{ item: WorkItem, audit: AuditRow[] }} WorkDetail */

/**
 * @typedef {object} StageResult
 * @property {number} id
 * @property {string} dir store-relative, e.g. `packs-inbox/9`
 * @property {string[]} packs ISO2 codes staged
 * @property {number} count
 */

/** @type {WorkStatus[]} */
export const STATUSES = ['pending', 'leased', 'done', 'failed'];
export const LIST_LIMIT = 50;
export const POLL_MS = 5000;
export const DEFAULT_ATTEMPTS = 3;
export const MAX_ATTEMPTS = 10;
export const MAX_PAYLOAD_BYTES = 64 * 1024;
// The in-page viewer renders at most this many characters (~1 MiB);
// a 16 MiB bundle would stall the tab. Download has the whole file.
export const VIEW_CAP = 1024 * 1024;

/**
 * @param {unknown} value
 * @returns {value is WorkStatus}
 */
export function isStatus(value) {
	return typeof value === 'string' && /** @type {string[]} */ (STATUSES).includes(value);
}

const STATUS_LABEL = { pending: 'Pending', leased: 'Leased', done: 'Done', failed: 'Failed' };

/**
 * @param {string} status
 * @returns {string}
 */
export function statusLabel(status) {
	return isStatus(status) ? STATUS_LABEL[status] : status.replace(/_/g, ' ');
}

/**
 * Tone for a status pill: green once done, accent while a worker holds
 * the lease, red when it failed for good, neutral while it waits.
 * @param {string} status
 * @returns {'done' | 'active' | 'error' | 'muted'}
 */
export function statusTone(status) {
	if (status === 'done') return 'done';
	if (status === 'leased') return 'active';
	if (status === 'failed') return 'error';
	return 'muted';
}

const AUDIT_LABEL = {
	work_enqueue: 'Enqueued',
	work_checkout: 'Checked out',
	work_complete: 'Completed',
	work_fail: 'Failed',
	work_requeue: 'Back in the queue',
	work_expire: 'Lease expired',
	work_retry: 'Retried'
};

/**
 * @param {string} action
 * @returns {string}
 */
export function auditLabel(action) {
	return /** @type {Record<string, string>} */ (AUDIT_LABEL)[action] ?? action.replace(/_/g, ' ');
}

/**
 * List query for the status filter (`all` or anything unknown → no
 * `status`).
 * @param {string} status
 * @param {number} [limit]
 * @returns {string}
 */
export function listPath(status, limit = LIST_LIMIT) {
	const params = new URLSearchParams({ limit: String(limit) });
	if (isStatus(status)) params.set('status', status);
	return `/admin/work-items?${params}`;
}

/**
 * @param {string} apiBase
 * @param {number} id
 * @returns {string}
 */
export function resultHref(apiBase, id) {
	return `${apiBase}/admin/work-items/${encodeURIComponent(String(id))}/result`;
}

/** @param {unknown} v */
const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
/** @param {unknown} v */
const str = (v) => (typeof v === 'string' ? v : null);

/**
 * Normalize one item row. Returns null for unknown shapes. Pure (tested).
 * @param {any} row
 * @returns {WorkItem | null}
 */
export function normalizeItem(row) {
	if (!row || typeof row.id !== 'number' || typeof row.kind !== 'string' || typeof row.status !== 'string') {
		return null;
	}
	const r = row.result;
	return {
		id: row.id,
		kind: row.kind,
		status: row.status,
		attempts: num(row.attempts) ?? 0,
		max_attempts: num(row.max_attempts) ?? DEFAULT_ATTEMPTS,
		lease_owner: str(row.lease_owner),
		lease_expires_at: num(row.lease_expires_at),
		created_by: str(row.created_by),
		created_at: num(row.created_at) ?? 0,
		updated_at: num(row.updated_at) ?? num(row.created_at) ?? 0,
		error: str(row.error),
		result:
			r && typeof r.sha256 === 'string' && typeof r.bytes === 'number'
				? {
						sha256: r.sha256,
						bytes: r.bytes,
						content_type: typeof r.content_type === 'string' ? r.content_type : 'application/octet-stream'
					}
				: null,
		summary: row.summary ?? null,
		payload: row.payload ?? null,
		staged: row.staged === true,
		retry_after: num(row.retry_after)
	};
}

/**
 * @param {any} row
 * @returns {WorkWorker | null}
 */
export function normalizeWorker(row) {
	if (!row || typeof row.name !== 'string') return null;
	return {
		conn: num(row.conn),
		name: row.name,
		identity: row.identity === 'shared' || row.identity === 'named' ? row.identity : null,
		version_matches: typeof row.version_matches === 'boolean' ? row.version_matches : null,
		capabilities: Array.isArray(row.capabilities)
			? row.capabilities.filter((/** @type {unknown} */ c) => typeof c === 'string')
			: [],
		version: str(row.version),
		connected_at: num(row.connected_at),
		last_seen_at: num(row.last_seen_at),
		job_id: num(row.job_id)
	};
}

/**
 * @param {any} row
 * @returns {WorkKind | null}
 */
export function normalizeKind(row) {
	if (!row || typeof row.kind !== 'string' || row.kind === '') return null;
	const ex = row.example;
	return {
		kind: row.kind,
		about: typeof row.about === 'string' ? row.about : '',
		example: ex && typeof ex === 'object' && !Array.isArray(ex) ? ex : {},
		max_runtime_secs: num(row.max_runtime_secs)
	};
}

/**
 * @param {any} counts
 * @returns {WorkCounts}
 */
export function normalizeCounts(counts) {
	/** @param {unknown} v */
	const n = (v) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : 0);
	return { pending: n(counts?.pending), leased: n(counts?.leased), done: n(counts?.done), failed: n(counts?.failed) };
}

/**
 * Normalize the list answer. Returns null when `items` is missing.
 * `offset` is 0 here; `fetchWorkList` fills it from the `Date` header.
 * @param {any} data
 * @returns {WorkList | null}
 */
export function normalizeList(data) {
	if (!data || !Array.isArray(data.items)) return null;
	const arr = (/** @type {unknown} */ v) => (Array.isArray(v) ? v : []);
	return {
		items: data.items.map(normalizeItem).filter((/** @type {WorkItem | null} */ x) => x !== null),
		workers: arr(data.workers).map(normalizeWorker).filter((x) => x !== null),
		kinds: arr(data.kinds).map(normalizeKind).filter((x) => x !== null),
		counts: normalizeCounts(data.counts),
		offset: 0
	};
}

/**
 * Normalize the detail answer. Returns null for unknown shapes.
 * @param {any} data
 * @returns {WorkDetail | null}
 */
export function normalizeDetail(data) {
	const item = normalizeItem(data?.item);
	if (!item) return null;
	/** @type {AuditRow[]} */
	const audit = [];
	for (const a of Array.isArray(data.audit) ? data.audit : []) {
		if (!a || typeof a.at !== 'number' || typeof a.action !== 'string') continue;
		audit.push({
			at: a.at,
			actor: typeof a.actor === 'string' ? a.actor : '?',
			action: a.action,
			reason: typeof a.reason === 'string' && a.reason !== '' ? a.reason : null
		});
	}
	return { item, audit };
}

/**
 * Pretty-printed `example` of a kind, for the payload box (`{}` when the
 * kind is unknown or has none).
 * @param {WorkKind[]} kinds
 * @param {string} kind
 * @returns {string}
 */
export function examplePayload(kinds, kind) {
	return JSON.stringify(kinds.find((k) => k.kind === kind)?.example ?? {}, null, 2);
}

/**
 * Any JSON value as indented text (`null` stays `null`).
 * @param {unknown} value
 * @returns {string}
 */
export function pretty(value) {
	return JSON.stringify(value ?? null, null, 2) ?? 'null';
}

/**
 * 1234 → "1.2 KiB".
 * @param {unknown} bytes
 * @returns {string}
 */
export function fmtBytes(bytes) {
	if (typeof bytes !== 'number' || !Number.isFinite(bytes) || bytes < 0) return '—';
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${+(bytes / 1024).toFixed(1)} KiB`;
	return `${+(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

/** @param {unknown} value */
function describe(value) {
	if (value === null) return 'null';
	if (Array.isArray(value)) return 'an array';
	if (typeof value === 'string') return 'a string';
	if (typeof value === 'number') return 'a number';
	if (typeof value === 'boolean') return `${value}`;
	return typeof value;
}

/**
 * Read the payload box: it must parse to a JSON object of at most
 * 64 KiB (compact), as the server checks. Pure (tested).
 * @param {unknown} text
 * @returns {{ ok: true, payload: Record<string, unknown> } | { ok: false, error: string }}
 */
export function parsePayload(text) {
	if (typeof text !== 'string' || text.trim() === '') {
		return { ok: false, error: 'Enter the payload as a JSON object, for example {}.' };
	}
	let value;
	try {
		value = JSON.parse(text);
	} catch (err) {
		const why = err instanceof Error && err.message ? err.message : 'unreadable';
		return { ok: false, error: `The payload is not valid JSON (${why}).` };
	}
	if (value === null || typeof value !== 'object' || Array.isArray(value)) {
		return { ok: false, error: `The payload must be a JSON object, not ${describe(value)}.` };
	}
	const bytes = new TextEncoder().encode(JSON.stringify(value)).length;
	if (bytes > MAX_PAYLOAD_BYTES) {
		return { ok: false, error: `The payload is ${fmtBytes(bytes)}; the server takes at most 64 KiB.` };
	}
	return { ok: true, payload: value };
}

/**
 * Max attempts from the number box: blank → the default (3), else a
 * whole number 1–10, else null. Pure (tested).
 * @param {unknown} value
 * @returns {number | null}
 */
export function parseMaxAttempts(value) {
	if (value === null || value === undefined) return DEFAULT_ATTEMPTS;
	const text = typeof value === 'number' ? String(value) : typeof value === 'string' ? value.trim() : '?';
	if (text === '') return DEFAULT_ATTEMPTS;
	if (!/^\d+$/.test(text)) return null;
	const n = Number(text);
	return n >= 1 && n <= MAX_ATTEMPTS ? n : null;
}

/**
 * Build the enqueue body, or say which field is wrong. Pure (tested):
 * the page shows the reason instead of sending.
 * @param {unknown} kind
 * @param {unknown} payloadText
 * @param {unknown} maxAttempts
 * @returns {{ ok: true, body: { kind: string, payload: Record<string, unknown>, max_attempts: number } } | { ok: false, field: 'kind' | 'payload' | 'max_attempts', error: string }}
 */
export function enqueueBody(kind, payloadText, maxAttempts) {
	if (typeof kind !== 'string' || kind === '') return { ok: false, field: 'kind', error: 'Choose a job kind.' };
	const parsed = parsePayload(payloadText);
	if (!parsed.ok) return { ok: false, field: 'payload', error: parsed.error };
	const attempts = parseMaxAttempts(maxAttempts);
	if (attempts === null) {
		return { ok: false, field: 'max_attempts', error: `Max attempts is a whole number from 1 to ${MAX_ATTEMPTS}.` };
	}
	return { ok: true, body: { kind, payload: parsed.payload, max_attempts: attempts } };
}

/**
 * "45 s", "4 min", "3 h", "2 d" for a non-negative span in seconds.
 * @param {number} secs
 * @returns {string}
 */
function span(secs) {
	if (secs < 60) return `${secs} s`;
	const mins = Math.round(secs / 60);
	if (mins < 60) return `${mins} min`;
	const hours = Math.floor(mins / 60);
	if (hours < 48) return `${hours} h`;
	return `${Math.floor(hours / 24)} d`;
}

/**
 * Lease expiry relative to `now` (both unix seconds on the server's
 * clock), or null when the item holds no lease. Pure (tested).
 * @param {number | null} expiresAt
 * @param {number} now
 * @returns {string | null}
 */
export function leaseText(expiresAt, now) {
	if (typeof expiresAt !== 'number' || !Number.isFinite(expiresAt)) return null;
	const secs = Math.round(expiresAt - now);
	if (secs > 0) return `expires in ${span(secs)}`;
	if (secs === 0) return 'expires now';
	return `expired ${span(-secs)} ago`;
}

/**
 * Seconds to add to this browser's clock to read the server's, from a
 * response `Date` header (0 when absent, unreadable, or within the
 * header's one-second resolution). A skewed laptop then still counts
 * leases down against the clock that set them. Pure (tested).
 * @param {string | null | undefined} dateHeader
 * @param {number} clientMs Date.now() when the answer arrived
 * @returns {number}
 */
export function clockOffset(dateHeader, clientMs) {
	if (!dateHeader) return 0;
	const server = Date.parse(dateHeader);
	if (!Number.isFinite(server)) return 0;
	const offset = Math.round((server - clientMs) / 1000);
	return Math.abs(offset) <= 2 ? 0 : offset;
}

/**
 * True when a result's content type is JSON (`application/json`, any
 * `+json`, parameters ignored): the viewer pretty-prints only those.
 * @param {unknown} contentType
 * @returns {boolean}
 */
export function isJsonType(contentType) {
	if (typeof contentType !== 'string') return false;
	const base = contentType.split(';')[0].trim().toLowerCase();
	return base === 'application/json' || base.endsWith('+json');
}

/**
 * Text for the in-page result viewer: JSON pretty-printed (text that
 * does not parse is shown as sent), cut at `cap` characters without
 * splitting a surrogate pair. Pure (tested).
 * @param {string} raw
 * @param {number} [cap]
 * @returns {{ text: string, parsed: boolean, truncated: boolean, chars: number }}
 */
export function resultView(raw, cap = VIEW_CAP) {
	let text = raw;
	let parsed = false;
	try {
		text = JSON.stringify(JSON.parse(raw), null, 2);
		parsed = true;
	} catch {
		// Not JSON after all: show it as sent.
	}
	const chars = text.length;
	if (chars <= cap) return { text, parsed, truncated: false, chars };
	let end = cap;
	const code = text.charCodeAt(end - 1);
	if (code >= 0xd800 && code <= 0xdbff) end -= 1;
	return { text: text.slice(0, end), parsed, truncated: true, chars };
}

/**
 * Screen-reader lines for items whose status changed between two
 * reads of the list (items new to the list are not announced: a filter
 * change or a fresh enqueue is the reader's own doing). Pure (tested).
 * @param {Map<number, string>} before id → status from the last read
 * @param {WorkItem[]} items
 * @returns {string[]}
 */
export function statusChanges(before, items) {
	const lines = [];
	for (const item of items) {
		const was = before.get(item.id);
		if (was === undefined || was === item.status) continue;
		lines.push(`Job ${item.id} (${item.kind}) is ${statusLabel(item.status).toLowerCase()}.`);
	}
	return lines;
}

/**
 * True when the server takes `stage-packs` for this item: a finished
 * `research.nonparty` job with a result, or a finished
 * `discovery.bootstrap` job whose summary counts world packs to stage
 * (anything else answers 409 or 422). Pure (tested).
 * @param {Pick<WorkItem, 'kind' | 'status' | 'result'> & { summary?: unknown }} item
 * @returns {boolean}
 */
export function canStagePacks(item) {
	if (item.status !== 'done' || item.result === null) return false;
	if (item.kind === 'research.nonparty') return true;
	const summary = /** @type {{ packs?: unknown } | null | undefined} */ (item.summary);
	return item.kind === 'discovery.bootstrap' && Number(summary?.packs) > 0;
}

/**
 * Normalize a stage answer (falls back to the convention
 * `packs-inbox/<id>` when `dir` is missing). Pure (tested).
 * @param {any} data
 * @param {number} id
 * @returns {StageResult}
 */
export function normalizeStage(data, id) {
	const packs = Array.isArray(data?.packs) ? data.packs.filter((/** @type {unknown} */ p) => typeof p === 'string') : [];
	return {
		id: typeof data?.id === 'number' ? data.id : id,
		dir: typeof data?.dir === 'string' && data.dir !== '' ? data.dir : `packs-inbox/${id}`,
		packs,
		count: typeof data?.count === 'number' && Number.isFinite(data.count) ? data.count : packs.length
	};
}

/**
 * "Staged 1 pack in packs-inbox/9 — load it from the Packs page".
 * Pure (tested).
 * @param {StageResult} out
 * @returns {string}
 */
export function stagedText(out) {
	if (out.count === 0) return `The bundle held no packs; ${out.dir} is empty.`;
	const one = out.count === 1;
	return `Staged ${out.count} pack${one ? '' : 's'} in ${out.dir} — load ${one ? 'it' : 'them'} from the Packs page.`;
}

/**
 * Re-run `run` every `intervalMs` while the tab is visible: one run at
 * a time (the next is timed from when the last settles), paused while
 * `doc.hidden`, run at once when the tab comes back. `now()` runs
 * immediately, or right after the run in flight, and re-times the next.
 * @param {() => Promise<unknown>} run
 * @param {{ intervalMs?: number, doc?: Pick<Document, 'hidden' | 'addEventListener' | 'removeEventListener'> }} [opts]
 * @returns {{ now: () => void, stop: () => void }}
 */
export function poll(run, { intervalMs = POLL_MS, doc = globalThis.document } = {}) {
	/** @type {ReturnType<typeof setTimeout> | null} */
	let timer = null;
	let running = false;
	let again = false;
	let stopped = false;

	const clear = () => {
		if (timer !== null) clearTimeout(timer);
		timer = null;
	};
	const arm = () => {
		clear();
		if (!stopped && !doc?.hidden) timer = setTimeout(tick, intervalMs);
	};
	async function tick() {
		timer = null;
		if (stopped) return;
		if (running) {
			again = true;
			return;
		}
		running = true;
		try {
			await run();
		} catch {
			// The page reports its own errors; polling carries on.
		} finally {
			running = false;
		}
		if (again && !stopped) {
			again = false;
			void tick();
		} else arm();
	}
	const onVisibility = () => {
		if (doc?.hidden) clear();
		else void tick();
	};
	doc?.addEventListener('visibilitychange', onVisibility);
	arm();
	return {
		now() {
			clear();
			void tick();
		},
		stop() {
			stopped = true;
			clear();
			doc?.removeEventListener('visibilitychange', onVisibility);
		}
	};
}

/** Error carrying the HTTP status (0 = the host was not reached). */
export class WorkError extends Error {
	/**
	 * @param {string} message
	 * @param {number} status
	 */
	constructor(message, status) {
		super(message);
		this.name = 'WorkError';
		this.status = status;
	}
}

/**
 * @param {typeof fetch} fetchFn
 * @param {string} url
 * @param {RequestInit} init
 * @param {string} what
 * @returns {Promise<Response>}
 */
async function send(fetchFn, url, init, what) {
	try {
		return await fetchFn(url, { credentials: 'same-origin', ...init });
	} catch {
		throw new WorkError(`${what}: the admin host could not be reached.`, 0);
	}
}

/**
 * Throw the server's own words for a failed answer: auth failures via
 * the shared helper, an `{"error": "..."}` body verbatim otherwise,
 * then the caller's `hints` for a bare status.
 * @param {Response} res
 * @param {string} what
 * @param {Record<number, string>} [hints]
 * @returns {Promise<void>}
 */
async function throwForStatus(res, what, hints = {}) {
	if (res.ok) return;
	const body = await readErrorBody(res);
	const auth = authError(res.status, body);
	if (auth) throw new WorkError(auth, res.status);
	const server = typeof body?.error === 'string' && body.error !== '' ? body.error : null;
	const verb = res.status < 500 ? 'refused' : 'failed';
	if (server) throw new WorkError(`${what} ${verb} (HTTP ${res.status}): ${server}`, res.status);
	if (hints[res.status]) throw new WorkError(`${what} ${verb} (HTTP ${res.status}): ${hints[res.status]}`, res.status);
	if (res.status === 413) throw new WorkError(`${what} refused (HTTP 413): the payload is over 64 KiB.`, 413);
	if (res.status === 404) throw new WorkError(`${what}: not found (HTTP 404).`, 404);
	throw new WorkError(`${what} ${verb}: HTTP ${res.status}.`, res.status);
}

/**
 * @param {Response} res
 * @param {string} what
 * @returns {Promise<any>}
 */
async function readJson(res, what) {
	try {
		return await res.json();
	} catch {
		// The static demo host answers unknown API paths with the SPA
		// shell: a non-JSON body means the endpoint is not there.
		throw new WorkError(`${what} is unavailable on this host.`, res.status);
	}
}

/**
 * @param {string} apiBase
 * @param {string} path
 * @param {unknown} body
 * @param {string} what
 * @param {typeof fetch} fetchFn
 * @param {Record<number, string>} [hints]
 * @returns {Promise<any>}
 */
async function postWork(apiBase, path, body, what, fetchFn, hints) {
	const res = await send(
		fetchFn,
		`${apiBase}${path}`,
		{
			method: 'POST',
			headers: { 'content-type': 'application/json', ...csrfHeaders() },
			body: JSON.stringify(body)
		},
		what
	);
	await throwForStatus(res, what, hints);
	return readJson(res, what);
}

/**
 * The list, connected workers, kinds and counts. A stalled answer is
 * dropped after 20 s so the poller never waits on it for good.
 * @param {string} apiBase
 * @param {string} status `all` or a status
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<WorkList>}
 */
export async function fetchWorkList(apiBase, status, fetchFn = fetch) {
	const res = await send(fetchFn, `${apiBase}${listPath(status)}`, { signal: AbortSignal.timeout(20_000) }, 'Work list');
	await throwForStatus(res, 'Work list');
	const list = normalizeList(await readJson(res, 'Work list'));
	if (!list) throw new WorkError('Work list returned an unexpected shape.', res.status);
	list.offset = clockOffset(res.headers.get('date'), Date.now());
	return list;
}

/**
 * @param {string} apiBase
 * @param {number} id
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<WorkDetail>}
 */
export async function fetchWorkItem(apiBase, id, fetchFn = fetch) {
	const what = `Job ${id}`;
	const res = await send(fetchFn, `${apiBase}/admin/work-items/${encodeURIComponent(String(id))}`, {}, what);
	await throwForStatus(res, what);
	const detail = normalizeDetail(await readJson(res, what));
	if (!detail) throw new WorkError(`${what} returned an unexpected shape.`, res.status);
	return detail;
}

/**
 * The result bytes as text, for the in-page viewer.
 * @param {string} apiBase
 * @param {number} id
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<string>}
 */
export async function fetchResultText(apiBase, id, fetchFn = fetch) {
	const what = `Result of job ${id}`;
	const res = await send(fetchFn, resultHref(apiBase, id), {}, what);
	await throwForStatus(res, what);
	return res.text();
}

/**
 * @param {string} apiBase
 * @param {{ kind: string, payload: Record<string, unknown>, max_attempts: number }} body
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ id: number, status: string }>}
 */
export async function enqueueWork(apiBase, body, fetchFn = fetch) {
	const data = await postWork(apiBase, '/admin/work-items', body, 'Enqueue', fetchFn);
	if (!data || typeof data.id !== 'number') throw new WorkError('Enqueue returned an unexpected shape.', 0);
	return { id: data.id, status: typeof data.status === 'string' ? data.status : 'pending' };
}

/**
 * @param {string} apiBase
 * @param {number} id
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ id: number, status: string }>}
 */
export async function retryWork(apiBase, id, fetchFn = fetch) {
	const path = `/admin/work-items/${encodeURIComponent(String(id))}/retry`;
	const data = await postWork(apiBase, path, {}, `Retry of job ${id}`, fetchFn);
	return {
		id: typeof data?.id === 'number' ? data.id : id,
		status: typeof data?.status === 'string' ? data.status : 'pending'
	};
}

/**
 * Mint a token that admits only the worker `name` (shown once). Rotating
 * `worker.token` in /config revokes every minted token.
 * @param {string} apiBase
 * @param {string} name
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ name: string, token: string }>}
 */
export async function mintWorkerToken(apiBase, name, fetchFn = fetch) {
	const data = await postWork(apiBase, '/admin/workers/token', { name }, `Token for ${name}`, fetchFn, {
		422: 'worker names are 1-32 of a-z, 0-9 and -.'
	});
	if (!data || typeof data.token !== 'string') throw new WorkError('The token reply had an unexpected shape.', 0);
	return { name: typeof data.name === 'string' ? data.name : name, token: data.token };
}

/**
 * End a pending or leased job (it fails as "cancelled by …"; a worker
 * running it loses the lease at its next heartbeat).
 * @param {string} apiBase
 * @param {number} id
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ id: number, status: string }>}
 */
export async function cancelWork(apiBase, id, fetchFn = fetch) {
	const path = `/admin/work-items/${encodeURIComponent(String(id))}/cancel`;
	const data = await postWork(apiBase, path, {}, `Cancel of job ${id}`, fetchFn, {
		404: 'no such job.',
		409: 'only a pending or running job can be cancelled.'
	});
	return {
		id: typeof data?.id === 'number' ? data.id : id,
		status: typeof data?.status === 'string' ? data.status : 'failed'
	};
}

/**
 * Copy a finished research.nonparty result into `<store>/packs-inbox/<id>/`
 * for the Packs page (re-staging overwrites).
 * @param {string} apiBase
 * @param {number} id
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<StageResult>}
 */
export async function stagePacks(apiBase, id, fetchFn = fetch) {
	const path = `/admin/work-items/${encodeURIComponent(String(id))}/stage-packs`;
	const data = await postWork(apiBase, path, {}, `Staging job ${id}`, fetchFn, {
		404: 'no such job.',
		409: 'only a finished research.nonparty job with a result can be staged.',
		422: 'the result is not a packs.v1 bundle.'
	});
	return normalizeStage(data, id);
}
