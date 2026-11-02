// Admin calls API client (voice-telnyx.md §7). Admin-only: never
// imported from (public) or (customer) routes.
//
// GET  {apiBase}/calls          → { calls: [...] }
// POST {apiBase}/calls/dial     → { call_id, status }
// GET  {apiBase}/calls/{id}     → detail + transcript
// POST {apiBase}/calls/{id}/hangup | /record/start | /record/stop
// POST {apiBase}/calls/{id}/questions/{idx}/asked
// POST {apiBase}/calls/{id}/quote → { fact_id, call_id, topic, status }
//
// Phone numbers never render here: the server stores only a hash,
// and list/detail payloads carry no dial target.

import { csrfHeaders } from './config-client.js';
import { authError, readErrorBody } from './http-errors.js';

/**
 * @typedef {object} CallSummary
 * @property {string} call_id
 * @property {string} status
 * @property {string | null} authority
 * @property {number} started_at
 * @property {number | null} ended_at
 * @property {string} recording_status
 * @property {number} questions_asked
 * @property {number} questions_total
 */

/**
 * E.164 check mirroring the server: `+` plus 7–15 digits.
 * @param {unknown} to
 * @returns {boolean}
 */
export function e164Valid(to) {
	return typeof to === 'string' && /^\+\d{7,15}$/.test(to);
}

// Default authority-validation script (staff working notes, not
// facts — the dial page loads these into the editable box). Every
// line fits the server's 1–280 chars; the count fits 1–20.
const DEFAULT_SCRIPT = [
	'What is the current fee for an apostille per document?',
	'What is the current processing time for a mail-in apostille request?',
	'What is the correct mailing address for apostille requests?',
	'Which payment methods do you accept for mail-in requests?',
	'Is a cover letter or request form required, and which revision?',
	'What are the walk-in hours and address, if different from mailing?',
	'How are completed documents returned, and what return envelope is needed?'
];

/**
 * Fresh copy of the default script (mutating it is safe).
 * @returns {string[]}
 */
export function defaultScript() {
	return [...DEFAULT_SCRIPT];
}

/**
 * Build a dial body. Pure (tested). Returns null when invalid —
 * the page shows the reason instead of sending.
 * @param {unknown} to
 * @param {unknown} authority
 * @param {unknown} questions
 * @param {unknown} consent
 * @returns {{ to: string, authority?: string, questions: string[], consent_confirmed: true } | null}
 */
export function dialBody(to, authority, questions, consent) {
	if (typeof to !== 'string' || !e164Valid(to)) return null;
	if (consent !== true) return null;
	const list = Array.isArray(questions) ? questions : [];
	if (list.length === 0 || list.length > 20) return null;
	const cleaned = [];
	for (const q of list) {
		if (typeof q !== 'string') return null;
		const text = q.trim();
		if (text.length === 0 || text.length > 280) return null;
		cleaned.push(text);
	}
	/** @type {{ to: string, questions: string[], consent_confirmed: true, authority?: string }} */
	const body = { to, questions: cleaned, consent_confirmed: true };
	if (typeof authority === 'string' && authority.trim().length > 0) {
		body.authority = authority.trim();
	}
	return body;
}

/**
 * Normalize one list row. Returns null for unknown shapes. Pure (tested).
 * @param {any} row
 * @returns {CallSummary | null}
 */
export function normalizeCallSummary(row) {
	if (!row || typeof row.call_id !== 'string' || typeof row.status !== 'string') return null;
	if (typeof row.started_at !== 'number') return null;
	return {
		call_id: row.call_id,
		status: row.status,
		authority: typeof row.authority === 'string' ? row.authority : null,
		started_at: row.started_at,
		ended_at: typeof row.ended_at === 'number' ? row.ended_at : null,
		recording_status: typeof row.recording_status === 'string' ? row.recording_status : 'none',
		questions_asked: typeof row.questions_asked === 'number' ? row.questions_asked : 0,
		questions_total: typeof row.questions_total === 'number' ? row.questions_total : 0
	};
}

/**
 * Normalize a detail payload. Returns null for unknown shapes. Pure (tested).
 * @param {any} data
 * @returns {{ call_id: string, status: string, status_detail: string | null, authority: string | null, questions: { text: string, asked: boolean }[], transcript: { seq: number, speaker: string | null, text: string, at: number }[], transcript_status: string, recording_status: string, ended_at: number | null } | null}
 */
export function normalizeCallDetail(data) {
	if (!data || typeof data.call_id !== 'string' || typeof data.status !== 'string') return null;
	if (!Array.isArray(data.questions) || !Array.isArray(data.transcript)) return null;
	const questions = [];
	for (const q of data.questions) {
		if (!q || typeof q.text !== 'string' || typeof q.asked !== 'boolean') return null;
		questions.push({ text: q.text, asked: q.asked });
	}
	const transcript = [];
	for (const s of data.transcript) {
		if (!s || typeof s.seq !== 'number' || typeof s.text !== 'string') return null;
		transcript.push({
			seq: s.seq,
			speaker: typeof s.speaker === 'string' ? s.speaker : null,
			text: s.text,
			at: typeof s.at === 'number' ? s.at : 0
		});
	}
	return {
		call_id: data.call_id,
		status: data.status,
		status_detail: typeof data.status_detail === 'string' ? data.status_detail : null,
		authority: typeof data.authority === 'string' ? data.authority : null,
		questions,
		transcript,
		transcript_status:
			typeof data.transcript_status === 'string' ? data.transcript_status : 'none',
		recording_status: typeof data.recording_status === 'string' ? data.recording_status : 'none',
		ended_at: typeof data.ended_at === 'number' ? data.ended_at : null
	};
}

/**
 * True while the call can still change (the page polls then).
 * @param {string} status
 * @returns {boolean}
 */
export function isLive(status) {
	return status === 'queued' || status === 'ringing' || status === 'in_progress';
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
 * @param {Response} res
 * @param {string} what
 * @returns {Promise<void>}
 */
async function throwForStatus(res, what) {
	// Auth failures show the server's answer verbatim (the 403
	// body names the detected IP) via the shared helper. The guard
	// above makes the `??` fallback unreachable; it only satisfies
	// the helper's nullable type.
	if (res.status === 401 || res.status === 403) {
		throw new Error(
			authError(res.status, await readErrorBody(res)) ?? 'The admin host refused the request.'
		);
	}
	if (res.status === 404) throw new Error(`${what} not found (404).`);
	if (res.status === 422) throw new Error('The server refused the request (422) — check the number and consent.');
	if (res.status === 503) {
		throw new Error('Voice calling is not configured on this server (503).');
	}
	if (!res.ok) throw new Error(`${what} failed: HTTP ${res.status}.`);
}

/**
 * @param {string} apiBase
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<CallSummary[]>}
 */
export async function fetchCalls(apiBase, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/calls`, { credentials: 'same-origin' });
	await throwForStatus(res, 'Call list');
	const data = await readJson(res, 'Call list');
	const calls = data?.calls;
	if (!Array.isArray(calls)) throw new Error('Call list returned an unexpected shape.');
	return calls.map(normalizeCallSummary).filter((row) => row !== null);
}

/**
 * @param {string} apiBase
 * @param {string} id
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<ReturnType<typeof normalizeCallDetail>>}
 */
export async function fetchCall(apiBase, id, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/calls/${encodeURIComponent(id)}`, {
		credentials: 'same-origin'
	});
	await throwForStatus(res, 'Call detail');
	const detail = normalizeCallDetail(await readJson(res, 'Call detail'));
	if (!detail) throw new Error('Call detail returned an unexpected shape.');
	return detail;
}

/**
 * @param {string} apiBase
 * @param {string} path
 * @param {unknown} body
 * @param {string} what
 * @param {typeof fetch} fetchFn
 * @returns {Promise<any>}
 */
async function postCall(apiBase, path, body, what, fetchFn) {
	const res = await fetchFn(`${apiBase}${path}`, {
		method: 'POST',
		headers: { 'content-type': 'application/json', ...csrfHeaders() },
		credentials: 'same-origin',
		body: body === undefined ? undefined : JSON.stringify(body)
	});
	await throwForStatus(res, what);
	return readJson(res, what);
}

/**
 * @param {string} apiBase
 * @param {{ to: string, questions: string[], consent_confirmed: true, authority?: string }} body
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ call_id: string, status: string }>}
 */
export async function dialCall(apiBase, body, fetchFn = fetch) {
	const data = await postCall(apiBase, '/calls/dial', body, 'Dial', fetchFn);
	if (!data || typeof data.call_id !== 'string') throw new Error('Dial returned an unexpected shape.');
	return { call_id: data.call_id, status: data.status };
}

/**
 * @param {string} apiBase
 * @param {string} id
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<unknown>}
 */
export function hangupCall(apiBase, id, fetchFn = fetch) {
	return postCall(apiBase, `/calls/${encodeURIComponent(id)}/hangup`, undefined, 'Hangup', fetchFn);
}

/**
 * @param {string} apiBase
 * @param {string} id
 * @param {boolean} start
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<unknown>}
 */
export function recordCall(apiBase, id, start, fetchFn = fetch) {
	const action = start ? 'start' : 'stop';
	return postCall(
		apiBase,
		`/calls/${encodeURIComponent(id)}/record/${action}`,
		undefined,
		start ? 'Record start' : 'Record stop',
		fetchFn
	);
}

/**
 * @param {string} apiBase
 * @param {string} id
 * @param {number} idx
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<unknown>}
 */
export function markQuestionAsked(apiBase, id, idx, fetchFn = fetch) {
	return postCall(
		apiBase,
		`/calls/${encodeURIComponent(id)}/questions/${idx}/asked`,
		undefined,
		'Mark asked',
		fetchFn
	);
}

/**
 * Build a quote-to-draft body. Pure (tested). Returns null when invalid —
 * the page shows the reason instead of sending. The server re-validates
 * everything (topic against the frozen 25-enum, segment membership).
 * @param {unknown} segmentIds
 * @param {unknown} topic
 * @param {unknown} statement
 * @returns {{ segment_ids: number[], topic: string, statement: string } | null}
 */
export function quoteBody(segmentIds, topic, statement) {
	if (!Array.isArray(segmentIds) || segmentIds.length === 0) return null;
	const seqs = [];
	for (const seq of segmentIds) {
		if (typeof seq !== 'number' || !Number.isInteger(seq) || seq < 0) return null;
		seqs.push(seq);
	}
	if (typeof topic !== 'string' || topic.trim().length === 0) return null;
	if (typeof statement !== 'string') return null;
	const text = statement.trim();
	if (text.length === 0 || text.length > 1024) return null;
	return { segment_ids: seqs, topic: topic.trim(), statement: text };
}

/**
 * @param {string} apiBase
 * @param {string} id
 * @param {{ segment_ids: number[], topic: string, statement: string }} body
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ fact_id: number, topic: string }>}
 */
export async function quoteToDraft(apiBase, id, body, fetchFn = fetch) {
	const data = await postCall(
		apiBase,
		`/calls/${encodeURIComponent(id)}/quote`,
		body,
		'Quote to draft',
		fetchFn
	);
	if (!data || typeof data.fact_id !== 'number') {
		throw new Error('Quote to draft returned an unexpected shape.');
	}
	return { fact_id: data.fact_id, topic: typeof data.topic === 'string' ? data.topic : '' };
}
