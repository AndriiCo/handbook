// "Quote a page" on /review: staff select an exact span of a captured
// official page (its text sidecar) and propose it as a fact with real
// citation offsets. The fact lands as `proposed`; only the Facts tab's
// separate accept publishes it. Admin-only: never imported from
// (public) or (customer) routes.
//
// GET  {apiBase}/admin/authorities/{slug}/captures?limit=1..200
//      → { slug, captures: [{ id, source_id, uri, kind, trust, captured_at, bytes }] } newest first
// GET  {apiBase}/admin/captures/{id}/text → { id, slug, uri, kind, trust, captured_at, text }
// POST {apiBase}/admin/captures/{id}/quote {quote, start?, end?, topic, statement}
//      201 → { fact_id, status, capture_id, start, end, quote, topic, statement, slug }
//      409 { error, fact_id, status } already on file · 409 { error, count } quote appears N times
//
// The server owns every gate (offsets on the sidecar's UTF-8 bytes,
// the 8-400 character quote, the keyed statement, the value inside
// the quote, duplicates); these helpers mirror them so the tab can say
// why Propose is off before anything is sent, and pass the server's
// reason through when it refuses. Pure: no window/fetch, so node tests
// drive it directly (the tab calls the API through admin-api).

import { TOPICS, resolveSlot, splitKey } from '../public-ui/slots.js';
import { LOCKED_TOPIC, topicChoices } from './review-keys.js';

/** The public citation is served only for quotes of 8..=400 characters (handbook_agent::analyze). */
export const MIN_QUOTE_CHARS = 8;
export const MAX_QUOTE_CHARS = 400;
/** A shorter value cannot anchor a verifiable claim (analyze::MIN_VALUE_CHARS). */
export const MIN_VALUE_CHARS = 2;
export const MAX_STATEMENT_CHARS = 2000;
/** Find stops counting here (a one-letter search of a long capture). */
export const MAX_MATCHES = 1000;

// Rust's `char::is_whitespace` (Unicode White_Space): the server's
// trims and `normalize` split on exactly these. JS `\s` differs by two
// characters (U+0085 out, U+FEFF in).
const WS_RUNS = /[\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+/;
const LEADING_WS = /^[\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+/;
const TRAILING_WS = /[\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+$/;
/** @param {string} ch one UTF-16 unit (every White_Space character is one) */
const isWs = (ch) => LEADING_WS.test(ch);

/** A statement key: the guide's field grammar, `field__instance` included (approval.rs edited_value). */
export const KEY = /^[a-z][a-z0-9_]*$/;

/**
 * @typedef {object} CaptureRow
 * @property {number} id
 * @property {number | null} source_id
 * @property {string} uri
 * @property {string} kind
 * @property {string} trust
 * @property {number} captured_at unix seconds
 * @property {number} bytes the text sidecar's length
 */

/**
 * @typedef {object} CaptureText
 * @property {number} id
 * @property {string} slug
 * @property {string} uri
 * @property {string} kind
 * @property {string} trust
 * @property {number} captured_at unix seconds
 * @property {string} text the sidecar as the API sent it (UTF-8, lossy)
 */

/**
 * @typedef {object} QuoteInput
 * @property {string} quote
 * @property {string} topic
 * @property {string} statement
 * @property {number} [start] UTF-8 byte offset into the sidecar
 * @property {number} [end] UTF-8 byte offset, exclusive
 */

/**
 * @typedef {object} QuoteBody
 * @property {string} quote
 * @property {number} [start]
 * @property {number} [end]
 * @property {string} topic
 * @property {string} statement
 */

/**
 * @typedef {object} PreparedQuote
 * @property {number} capture_id
 * @property {string} quote
 * @property {string} topic
 * @property {string} statement
 * @property {number} [start] UTF-8 byte offsets into the capture text,
 *   both or neither: with them the server checks that exact slice
 *   instead of searching for the quote
 * @property {number} [end]
 */

/**
 * @typedef {object} PreparedRow
 * @property {number} line 1-based line in the box
 * @property {string} raw the line, trimmed: keys the row's status
 * @property {PreparedQuote | null} row null when the line is not a row
 * @property {string | null} error why the line is not a row, or the first thing the row still needs
 */

/**
 * @typedef {'proposed'|'duplicate'|'not_found'|'ambiguous'|'error'} OutcomeKind
 * @typedef {{ kind: OutcomeKind, text: string, factId: number | null }} Outcome
 */

/**
 * Trim the server's whitespace (Rust `str::trim`) off both ends.
 * @param {unknown} text
 * @returns {string}
 */
export function trimWs(text) {
	return String(text ?? '').replace(LEADING_WS, '').replace(TRAILING_WS, '');
}

/**
 * Whitespace runs become one space, ends trimmed, case kept: how a
 * value reads in a one-line statement.
 * @param {unknown} text
 * @returns {string}
 */
export function tidy(text) {
	return String(text ?? '')
		.split(WS_RUNS)
		.filter(Boolean)
		.join(' ');
}

/**
 * The analysis validator's normalization (analyze::normalize): runs of
 * whitespace become one space, ends trimmed, lowercased.
 * @param {unknown} text
 * @returns {string}
 */
export function normalize(text) {
	return tidy(text).toLowerCase();
}

/**
 * Characters as the server counts them (Unicode scalar values, not
 * UTF-16 units: an emoji is one).
 * @param {unknown} text
 * @returns {number}
 */
export function charCount(text) {
	return [...String(text ?? '')].length;
}

/**
 * Whether `value` reads inside `quote` once both are normalized: the
 * server's value-in-quote gate (approval.rs edit_refusal). An empty
 * value is never inside.
 * @param {unknown} value
 * @param {unknown} quote
 * @returns {boolean}
 */
export function valueInQuote(value, quote) {
	const v = normalize(value);
	return v !== '' && normalize(quote).includes(v);
}

/**
 * Split a keyed statement the way the server does (approval.rs
 * edited_value) after trimming it: the key before the first colon,
 * then at least one whitespace, then a non-empty value. Null when the
 * statement is unkeyed (`Fee is $8.`, `UPPER: x`, `fee:$8`, `fee:`).
 * @param {unknown} statement
 * @returns {{ key: string, value: string } | null}
 */
export function splitStatement(statement) {
	const s = trimWs(statement);
	const at = s.indexOf(':');
	if (at < 0) return null;
	const key = s.slice(0, at);
	if (!KEY.test(key)) return null;
	const rest = s.slice(at + 1);
	const value = rest.replace(LEADING_WS, '');
	if (value.length === rest.length || value === '') return null;
	return { key, value };
}

/**
 * The statement a key and value make: `key: value`, the value on one
 * line.
 * @param {unknown} key
 * @param {unknown} value
 * @returns {string}
 */
export function statementOf(key, value) {
	return `${String(key ?? '').trim()}: ${tidy(value)}`;
}

/**
 * The key after a key chip click: the chip's key, keeping the current
 * key's instance when the chip's slot lists one value per instance
 * (`weekday__tumwater` stays `weekday__tumwater`; an alias that implies
 * an office carries it over).
 * @param {string} current the key as typed
 * @param {string} chip a canonical key of `topic`
 * @param {string} topic
 * @returns {string}
 */
export function chipKey(current, chip, topic) {
	const key = String(current ?? '').trim();
	const instance = key ? (splitKey(key).instance ?? resolveSlot(topic, key)?.instance ?? null) : null;
	return resolveSlot(topic, chip)?.slot.many === true && instance ? `${chip}__${instance}` : chip;
}

/**
 * Topic choices for a quote: every handbook topic but `phone_script`,
 * which stays staff-only.
 * @returns {{ topic: string, where: string }[]}
 */
export function quoteTopics() {
	return topicChoices().filter((c) => c.topic !== LOCKED_TOPIC);
}

/**
 * UTF-8 length of `text.slice(0, end)`: the byte offset the server
 * reads for a UTF-16 index (a lone surrogate counts 3, as TextEncoder
 * writes U+FFFD). Counted, not encoded: a 4 MiB capture re-counts on
 * every selection change.
 * @param {string} text
 * @param {number} end UTF-16 index
 * @returns {number}
 */
export function utf8Offset(text, end) {
	const stop = Math.max(0, Math.min(end, text.length));
	let bytes = 0;
	for (let i = 0; i < stop; i++) {
		const c = text.charCodeAt(i);
		if (c < 0x80) bytes += 1;
		else if (c < 0x800) bytes += 2;
		else if (c >= 0xd800 && c <= 0xdbff && i + 1 < stop && (text.charCodeAt(i + 1) & 0xfc00) === 0xdc00) {
			bytes += 4;
			i += 1;
		} else bytes += 3;
	}
	return bytes;
}

/**
 * A textarea holds `\r\n` and `\r` as `\n` (its value is normalized),
 * so its selection counts one unit per line break. The text as the
 * textarea shows it, and the map from its indices back to the API's
 * string: only a `\r\n` pair shrinks, so each one before an index
 * adds one unit back.
 * @param {string} text the capture text as the API sent it
 * @returns {{ shown: string, toOriginal: (index: number) => number }}
 */
export function lineBreakMap(text) {
	/** @type {number[]} shown index of each `\r\n` pair, ascending */
	const pairs = [];
	for (let at = text.indexOf('\r\n'); at >= 0; at = text.indexOf('\r\n', at + 2)) pairs.push(at - pairs.length);
	const shown = pairs.length === 0 && !text.includes('\r') ? text : text.replace(/\r\n?/g, '\n');
	/** @param {number} index */
	const toOriginal = (index) => {
		let lo = 0;
		let hi = pairs.length;
		while (lo < hi) {
			const mid = (lo + hi) >> 1;
			if (pairs[mid] < index) lo = mid + 1;
			else hi = mid;
		}
		return index + lo;
	};
	return { shown, toOriginal };
}

/**
 * The range `[start, end)` of `text` with whitespace trimmed off both
 * ends, as the server stores a quote; an end inside a surrogate pair
 * grows to the whole character.
 * @param {string} text
 * @param {number} start UTF-16 index
 * @param {number} end UTF-16 index
 * @returns {{ start: number, end: number }}
 */
export function trimRange(text, start, end) {
	let s = Math.max(0, Math.min(start, end, text.length));
	let e = Math.min(text.length, Math.max(start, end, 0));
	/** @param {number} i a split pair has its low half at `i` */
	const splits = (i) => i > 0 && i < text.length && (text.charCodeAt(i) & 0xfc00) === 0xdc00 && (text.charCodeAt(i - 1) & 0xfc00) === 0xd800;
	if (splits(s)) s -= 1;
	if (splits(e)) e += 1;
	while (s < e && isWs(text[s])) s += 1;
	while (e > s && isWs(text[e - 1])) e -= 1;
	return { start: s, end: e };
}

/**
 * What a textarea selection quotes: the trimmed slice of the API's
 * text, its UTF-8 byte offsets there (end exclusive), and its length
 * in characters.
 * @param {string} text the capture text as the API sent it
 * @param {(index: number) => number} toOriginal from `lineBreakMap`
 * @param {number} selStart textarea.selectionStart
 * @param {number} selEnd textarea.selectionEnd
 * @returns {{ quote: string, start: number, end: number, chars: number }}
 */
export function selectionQuote(text, toOriginal, selStart, selEnd) {
	const range = trimRange(text, toOriginal(selStart), toOriginal(selEnd));
	const quote = text.slice(range.start, range.end);
	const start = utf8Offset(text, range.start);
	return { quote, start, end: start + utf8Offset(quote, quote.length), chars: charCount(quote) };
}

/** @param {string} s */
const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/**
 * Every match of `needle` in `text`, case-insensitive and
 * whitespace-flexible: its words in order, any whitespace between
 * them. At most `limit`.
 * @param {string} text
 * @param {string} needle
 * @param {number} [limit]
 * @returns {{ start: number, end: number }[]}
 */
export function findAll(text, needle, limit = MAX_MATCHES) {
	const words = String(needle ?? '')
		.split(WS_RUNS)
		.filter(Boolean);
	if (words.length === 0) return [];
	const re = new RegExp(words.map(escapeRe).join('\\s+'), 'gi');
	/** @type {{ start: number, end: number }[]} */
	const out = [];
	for (const m of String(text ?? '').matchAll(re)) {
		const start = m.index ?? 0;
		out.push({ start, end: start + m[0].length });
		if (out.length >= limit) break;
	}
	return out;
}

/**
 * The match Enter moves to: the first that starts at or after the
 * selection's end (so a selected match steps to the next), wrapping to
 * the first; backward (Shift+Enter), the last that ends at or before
 * the selection's start, wrapping to the last. -1 without matches.
 * @param {{ start: number, end: number }[]} matches ascending
 * @param {number} selStart
 * @param {number} selEnd
 * @param {boolean} [backward]
 * @returns {number}
 */
export function nextMatch(matches, selStart, selEnd, backward = false) {
	if (matches.length === 0) return -1;
	if (backward) {
		for (let i = matches.length - 1; i >= 0; i--) if (matches[i].end <= selStart) return i;
		return matches.length - 1;
	}
	const at = matches.findIndex((m) => m.start >= selEnd);
	return at >= 0 ? at : 0;
}

/**
 * The first reason the server would refuse a quote, as a plain
 * sentence, or null when it is ready to send. A prepared quote
 * (`located`: no offsets) is placed by the server, so its length is
 * checked there, on the text it finds.
 * @param {QuoteInput & { located?: boolean }} input
 * @returns {string | null}
 */
export function quoteProblem(input) {
	const quote = String(input.quote ?? '');
	if (normalize(quote) === '') return input.located ? 'The quote is empty.' : 'Select the words to quote in the capture text.';
	if (!input.located) {
		const n = charCount(quote);
		if (n < MIN_QUOTE_CHARS || n > MAX_QUOTE_CHARS) {
			return `A quote must be ${MIN_QUOTE_CHARS} to ${MAX_QUOTE_CHARS} characters (this one is ${n}).`;
		}
	}
	const topic = String(input.topic ?? '');
	if (!topic) return 'Pick a topic.';
	if (topic === LOCKED_TOPIC) return 'phone_script stays staff-only: quote into another topic.';
	if (!TOPICS.includes(topic)) return `Unknown handbook topic: ${topic}.`;
	const statement = trimWs(input.statement);
	if (charCount(statement) > MAX_STATEMENT_CHARS) return `The statement is at most ${MAX_STATEMENT_CHARS} characters.`;
	const parts = splitStatement(statement);
	if (!parts) return statementProblem(statement);
	if (charCount(parts.value) < MIN_VALUE_CHARS) return `The value needs at least ${MIN_VALUE_CHARS} characters.`;
	if (!valueInQuote(parts.value, quote)) {
		return 'The value is not in the quote (case and spacing aside): the public page cites the quote for it.';
	}
	return null;
}

/**
 * Why a trimmed statement is not `key: value`.
 * @param {string} statement
 * @returns {string}
 */
function statementProblem(statement) {
	const at = statement.indexOf(':');
	if (at < 0) return "The statement must be 'key: value'.";
	const key = statement.slice(0, at);
	if (!key.trim()) return 'The statement needs a key before the colon.';
	if (!KEY.test(key)) return `The key “${key.trim()}” must be lowercase letters, digits and _, right before the colon.`;
	if (!statement.slice(at + 1).replace(LEADING_WS, '')) return `The value needs at least ${MIN_VALUE_CHARS} characters.`;
	return 'Put a space between the colon and the value.';
}

/**
 * The POST /admin/captures/{id}/quote body, or the first problem.
 * Offsets go only as a pair (UTF-8 bytes into the sidecar, end
 * exclusive); without them the server finds the quote in the text.
 * @param {QuoteInput} input
 * @returns {{ body: QuoteBody } | { error: string }}
 */
export function quoteBody(input) {
	const located = input.start === undefined && input.end === undefined;
	if (!located) {
		const { start, end } = input;
		if (!Number.isInteger(start) || !Number.isInteger(end) || Number(start) < 0 || Number(start) >= Number(end)) {
			return { error: 'The selection lost its place in the text: select it again.' };
		}
	}
	const problem = quoteProblem({ ...input, located });
	if (problem) return { error: problem };
	const statement = trimWs(input.statement);
	return {
		body: located
			? { quote: input.quote, topic: input.topic, statement }
			: { quote: input.quote, start: input.start, end: input.end, topic: input.topic, statement }
	};
}

/**
 * How a propose answer reads: a new fact, a statement already on
 * file, a quote the server could not place in the text, or the
 * server's own reason.
 * @param {number} status HTTP status (0: no answer)
 * @param {any} body the JSON answer, if any
 * @param {string} [fallback] when the answer carries no reason
 * @returns {Outcome}
 */
export function proposeOutcome(status, body, fallback = 'The proposal failed.') {
	/** @param {unknown} v */
	const id = (v) => (Number.isInteger(v) && Number(v) > 0 ? Number(v) : null);
	const error = typeof body?.error === 'string' ? body.error : '';
	if (status >= 200 && status < 300) {
		const factId = id(body?.fact_id);
		return { kind: 'proposed', text: factId ? `Proposed #${factId}.` : 'Proposed.', factId };
	}
	if (status === 409 && id(body?.fact_id)) {
		const factId = id(body.fact_id);
		const state = typeof body.status === 'string' && body.status ? ` (${body.status})` : '';
		return { kind: 'duplicate', text: `Already on file as #${factId}${state}.`, factId };
	}
	if (status === 409 && Number.isInteger(body?.count)) {
		return {
			kind: 'ambiguous',
			text: `Appears ${body.count} times in this capture: select the one you mean in the text.`,
			factId: null
		};
	}
	if (status === 422 && /quote not found/i.test(error)) {
		return { kind: 'not_found', text: "Not found in this capture's text.", factId: null };
	}
	return { kind: 'error', text: error || fallback, factId: null };
}

/**
 * Prepared quotes: JSON Lines, one `{capture_id, quote, topic,
 * statement}` object per line. Blank lines are skipped; a bad line
 * keeps its number and the reason. A row the server would refuse on
 * its words alone (topic, statement, value outside the quote) says so.
 * @param {unknown} text
 * @returns {PreparedRow[]}
 */
export function parsePrepared(text) {
	/** @type {PreparedRow[]} */
	const rows = [];
	String(text ?? '')
		.split(/\r\n|\r|\n/)
		.forEach((source, i) => {
			const line = i + 1;
			const raw = source.trim();
			if (!raw) return;
			/** @param {string} why */
			const bad = (why) => {
				rows.push({ line, raw, row: null, error: `Line ${line}: ${why}` });
			};
			let data;
			try {
				data = JSON.parse(raw);
			} catch (err) {
				bad(`not JSON (${err instanceof Error ? err.message : 'unreadable'}).`);
				return;
			}
			if (!data || typeof data !== 'object' || Array.isArray(data)) return bad('expected one {…} object.');
			if (!Number.isInteger(data.capture_id) || data.capture_id <= 0) return bad('capture_id must be a capture number.');
			for (const field of ['quote', 'topic', 'statement']) {
				if (typeof data[field] !== 'string' || !data[field].trim()) return bad(`${field} must be a non-empty string.`);
			}
			const offsets = data.start !== undefined || data.end !== undefined;
			if (offsets && (!Number.isInteger(data.start) || !Number.isInteger(data.end) || data.start < 0 || data.start >= data.end)) {
				return bad('start and end are byte offsets with start before end, both or neither.');
			}
			/** @type {PreparedQuote} */
			const row = { capture_id: data.capture_id, quote: data.quote, topic: data.topic.trim(), statement: data.statement };
			if (offsets) Object.assign(row, { start: data.start, end: data.end });
			rows.push({ line, raw, row, error: quoteProblem({ ...row, located: !offsets }) });
		});
	return rows;
}

/**
 * One captures-list answer, rows of unknown shape dropped.
 * @param {any} body
 * @returns {CaptureRow[]}
 */
export function parseCaptures(body) {
	const rows = Array.isArray(body?.captures) ? body.captures : [];
	/** @type {CaptureRow[]} */
	const out = [];
	for (const r of rows) {
		if (!r || !Number.isInteger(r.id) || typeof r.uri !== 'string') continue;
		out.push({
			id: r.id,
			source_id: Number.isInteger(r.source_id) ? r.source_id : null,
			uri: r.uri,
			kind: typeof r.kind === 'string' ? r.kind : '',
			trust: typeof r.trust === 'string' ? r.trust : '',
			captured_at: typeof r.captured_at === 'number' ? r.captured_at : 0,
			bytes: typeof r.bytes === 'number' ? r.bytes : 0
		});
	}
	return out;
}

/**
 * One capture-text answer, or null for an unknown shape.
 * @param {any} body
 * @returns {CaptureText | null}
 */
export function parseCaptureText(body) {
	if (!body || !Number.isInteger(body.id) || typeof body.text !== 'string') return null;
	return {
		id: body.id,
		slug: typeof body.slug === 'string' ? body.slug : '',
		uri: typeof body.uri === 'string' ? body.uri : '',
		kind: typeof body.kind === 'string' ? body.kind : '',
		trust: typeof body.trust === 'string' ? body.trust : '',
		captured_at: typeof body.captured_at === 'number' ? body.captured_at : 0,
		text: body.text
	};
}

/**
 * What a capture row names: the URI's path and query, the host for a
 * site's root page, the text itself when it is not a URL.
 * @param {string} uri
 * @returns {string}
 */
export function uriPath(uri) {
	try {
		const url = new URL(uri);
		const path = `${url.pathname}${url.search}`;
		return path === '/' ? url.host : path;
	} catch {
		return String(uri ?? '');
	}
}

/**
 * 940 → "940 B", 27298 → "27 KB", 1572864 → "1.5 MB".
 * @param {number} bytes
 * @returns {string}
 */
export function sizeLabel(bytes) {
	if (typeof bytes !== 'number' || !Number.isFinite(bytes) || bytes < 0) return '—';
	if (bytes < 1024) return `${bytes} B`;
	/** @param {number} n */
	const short = (n) => (n < 10 ? n.toFixed(1) : String(Math.round(n)));
	const kb = bytes / 1024;
	return kb < 1024 ? `${short(kb)} KB` : `${short(kb / 1024)} MB`;
}
