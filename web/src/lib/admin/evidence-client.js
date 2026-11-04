// Staff considerations and retire for /review (plan §5.4, §5.5).
// Admin-only: never imported from (public) or (customer) routes.
//
// GET  {apiBase}/admin/evidence?slug=&status=   → { notes: [staff view], count } newest first
// POST {apiBase}/admin/evidence {authority_slug, kind, fact_id?, preferred_fact_id?, slot?,
//      stance, occurred_at, channel, summary, supports?}  → the note (201)
// POST {apiBase}/admin/evidence/{id}/withdraw {reason}  → the note
// POST {apiBase}/admin/facts/{id}/retire {reason}       → { id, authority_slug, status }
// GET  {apiBase}/facts?status=accepted&authority=&limit=200 → { facts: FactView[], count }
//
// A note is public context beside the facts, never a source. The
// server owns every guard (contact details, anchors, a decision's
// basis) and answers a refusal as 422 {"error": "<plain reason>"};
// these helpers mirror only the shape checks, so the form can say
// what is missing before it sends, and pass the server's reason
// through verbatim when it refuses. Pure except the fetch helpers
// (which take an injectable fetch), so node tests drive it directly.

import { parseStatement } from '../public-ui/guide.js';
import { SLOTS, resolveSlot, slotById } from '../public-ui/slots.js';
import { csrfHeaders } from './config-client.js';
import { authError, readErrorBody } from './http-errors.js';

/** The kinds a note can be, in form order, with their staff labels. */
export const NOTE_KINDS = Object.freeze([
	Object.freeze({ value: 'call', label: 'Phone call' }),
	Object.freeze({ value: 'email', label: 'Email' }),
	Object.freeze({ value: 'visit', label: 'Visit report' }),
	Object.freeze({ value: 'decision', label: 'Staff decision' }),
	Object.freeze({ value: 'absence', label: "Not found on the office's pages" })
]);

/** Kinds that report what the office said or showed: they anchor to an accepted fact. */
export const FACT_KINDS = Object.freeze(['call', 'email', 'visit']);

export const STANCES = Object.freeze([
	Object.freeze({ value: 'confirms', label: 'Confirms' }),
	Object.freeze({ value: 'contradicts', label: 'Contradicts' }),
	Object.freeze({ value: 'context', label: 'Context' })
]);

/** The line above the form (plan §5.4: notes are public, calls unrecorded). */
export const PUBLIC_NOTE_LINE =
	'Notes are public on the guide. Write what the office said; never names or personal numbers; calls are not recorded.';

export const SUMMARY_MIN = 8;
export const SUMMARY_MAX = 280;
export const CHANNEL_MAX = 160;
export const MAX_SUPPORTS = 20;
export const RETIRE_MIN = 8;
export const RETIRE_MAX = 500;
export const WITHDRAW_MAX = 1024;
/** Longest slot key the server stores. */
const SLOT_MAX = 64;
/** A typed time may run this far past the browser clock (seconds). */
const FUTURE_GRACE = 300;

/** @type {readonly string[]} */
const KIND_VALUES = NOTE_KINDS.map((k) => k.value);
/** @type {readonly string[]} */
const STANCE_VALUES = STANCES.map((s) => s.value);
const SLUG = /^[a-z0-9-]{1,80}$/;
/** The server's slot grammar: dot-separated lowercase segments. */
const SLOT_KEY = /^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$/;
/** @param {unknown} slot */
const slotOk = (slot) => typeof slot === 'string' && slot.length <= SLOT_MAX && SLOT_KEY.test(slot);
const LOCAL_INPUT = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/;

/**
 * @typedef {'call'|'email'|'visit'|'decision'|'absence'} NoteKind
 * @typedef {'confirms'|'contradicts'|'context'} Stance
 */

/**
 * One note as the staff routes return it.
 * @typedef {object} EvidenceNote
 * @property {number} id
 * @property {string} authority_slug
 * @property {NoteKind} kind
 * @property {number | null} fact_id
 * @property {number | null} preferred_fact_id
 * @property {string} slot
 * @property {Stance} stance
 * @property {number} occurred_at unix seconds
 * @property {string} date YYYY-MM-DD on the Pacific calendar
 * @property {string} channel
 * @property {string} summary
 * @property {number[]} supports
 * @property {'published'|'withdrawn'} status
 * @property {string} created_by
 * @property {number} created_at
 */

/**
 * One accepted fact (`/facts?status=accepted` FactView), as the
 * considerations form reads it.
 * @typedef {object} PublishedFact
 * @property {number} id
 * @property {string} authority_slug
 * @property {string} topic
 * @property {string} statement
 * @property {string} status
 * @property {number} created_at
 * @property {number | null} accepted_at
 * @property {{ quote: string, source_url: string, captured_at: string, kind: string }[]} citations
 * @property {boolean} conflict
 * @property {{ id: number, topic: string, statement: string, status: string }[]} conflicts
 */

/**
 * The fact a drawer shows: a queue proposal or an accepted fact.
 * `related` lists the ids it is compared with (its conflicting
 * siblings and the live fact it would change).
 * @typedef {object} NoteSubject
 * @property {number} id
 * @property {string} authority_slug
 * @property {string} topic
 * @property {string} statement
 * @property {string} status
 * @property {number[]} [related]
 */

/**
 * Form input for one note.
 * @typedef {object} NoteInput
 * @property {string} slug
 * @property {string} kind
 * @property {string} stance
 * @property {number | null} factId the accepted fact a call, email or visit is about
 * @property {string} slot
 * @property {number | null} preferredFactId a decision's preferred fact
 * @property {number[]} supports the notes a decision rests on
 * @property {number | null} when unix seconds
 * @property {string} channel
 * @property {string} summary
 */

/** @param {unknown} kind */
export function kindLabel(kind) {
	return NOTE_KINDS.find((k) => k.value === kind)?.label ?? String(kind ?? '');
}

/** @param {unknown} stance */
export function stanceLabel(stance) {
	return STANCES.find((s) => s.value === stance)?.label ?? String(stance ?? '');
}

/**
 * Staff text as the server stores it: whitespace runs collapse to one
 * space, ends trimmed.
 * @param {unknown} text
 */
export function plainText(text) {
	return String(text ?? '')
		.split(/\s+/)
		.filter(Boolean)
		.join(' ');
}

/**
 * Characters the server counts (code points after `plainText`).
 * @param {unknown} text
 */
export function charCount(text) {
	return [...plainText(text)].length;
}

/**
 * `text` cut to `max` code points with an ellipsis (select labels).
 * @param {unknown} text
 * @param {number} max
 */
export function shorten(text, max) {
	const chars = [...String(text ?? '').replace(/\s+/g, ' ').trim()];
	return chars.length <= max ? chars.join('') : `${chars.slice(0, max - 1).join('').trimEnd()}…`;
}

/**
 * The slot key a fact fills, in the server's grammar: the registry
 * slot (`fees.same_day`), with the instance when there is one
 * (`address.office__tumwater`). A key no slot claims stays
 * `{topic}.{key}`; an unkeyed statement is its topic. Empty for an
 * unknown topic shape.
 * @param {unknown} topic
 * @param {unknown} statement
 * @returns {string}
 */
export function slotFor(topic, statement) {
	const t = String(topic ?? '');
	if (!SLOT_KEY.test(t) || t.includes('.')) return '';
	const key = parseStatement(String(statement ?? '')).fieldKey;
	const hit = key ? resolveSlot(t, key) : null;
	const slot = hit ? (hit.instance ? `${hit.slot.id}__${hit.instance}` : hit.slot.id) : key ? `${t}.${key}` : t;
	if (slotOk(slot)) return slot;
	return hit && slotOk(hit.slot.id) ? hit.slot.id : t;
}

/**
 * Plain label for a slot key: the registry label plus its instance
 * (`Office address (tumwater)`), else the key humanised.
 * @param {unknown} slot
 */
export function slotLabel(slot) {
	const s = String(slot ?? '');
	if (!s) return 'No field';
	const at = s.indexOf('__');
	const base = at > 0 ? s.slice(0, at) : s;
	const instance = at > 0 ? s.slice(at + 2).replaceAll('_', ' ') : '';
	const known = slotById(base);
	const label = known ? known.label : (base.split('.').at(-1) ?? base).replaceAll('_', ' ');
	const text = label.charAt(0).toUpperCase() + label.slice(1);
	return instance ? `${text} (${instance})` : text;
}

/**
 * Fields an absence note can name: `current` first when the registry
 * lacks it (an instance or an unclaimed key), then every registry slot.
 * @param {string} current
 * @returns {{ value: string, label: string }[]}
 */
export function slotChoices(current) {
	const out = SLOTS.map((s) => ({ value: s.id, label: `${s.label} · ${s.id}` }));
	if (current && !out.some((o) => o.value === current)) {
		out.unshift({ value: current, label: `${slotLabel(current)} · ${current}` });
	}
	return out;
}

/**
 * Ids a drawer compares its fact with: the fact, its conflicting
 * siblings and the live fact a proposal would change.
 * @param {NoteSubject} subject
 * @returns {Set<number>}
 */
function subjectIds(subject) {
	const ids = new Set([subject.id]);
	for (const id of subject.related ?? []) if (Number.isInteger(id)) ids.add(id);
	return ids;
}

/**
 * Published notes about a drawer's fact: anchored to it or to a fact
 * it is compared with, preferring one of those, or naming its slot
 * (an unkeyed fact's topic covers every slot of that topic). Newest
 * first, as the staff list serves them.
 * @param {EvidenceNote[]} notes
 * @param {NoteSubject} subject
 * @returns {EvidenceNote[]}
 */
export function notesForFact(notes, subject) {
	const ids = subjectIds(subject);
	const slot = slotFor(subject.topic, subject.statement);
	const topicOnly = slot !== '' && slot === subject.topic;
	return (notes ?? []).filter(
		(n) =>
			n.status === 'published' &&
			n.authority_slug === subject.authority_slug &&
			((n.fact_id !== null && ids.has(n.fact_id)) ||
				(n.preferred_fact_id !== null && ids.has(n.preferred_fact_id)) ||
				(slot !== '' && n.slot === slot) ||
				(topicOnly && n.slot.startsWith(`${slot}.`)))
	);
}

/**
 * The first accepted fact of `facts` filling `slotId` (any instance).
 * @param {PublishedFact[]} facts
 * @param {string} slotId
 */
function factInSlot(facts, slotId) {
	return (facts ?? []).find((f) => {
		const key = parseStatement(f.statement).fieldKey;
		return key !== null && resolveSlot(f.topic, key)?.slot.id === slotId;
	});
}

/**
 * The channel a new note starts with: the office's accepted phone for
 * a call, its accepted email address for an email, else empty. Taken
 * from the accepted statement, so the contact guard finds its digits
 * and address in the office's own facts.
 * @param {string} kind
 * @param {PublishedFact[]} facts the office's accepted facts
 * @returns {string}
 */
export function channelFor(kind, facts) {
	let value = '';
	if (kind === 'call') {
		value = parseStatement(factInSlot(facts, 'contacts.phone')?.statement ?? '').value;
	} else if (kind === 'email') {
		const text = parseStatement(factInSlot(facts, 'contacts.email')?.statement ?? '').value;
		value = /[^\s@<>()]+@[^\s@<>()]+\.[a-z]{2,}/i.exec(text)?.[0] ?? '';
	}
	const plain = plainText(value);
	return charCount(plain) <= CHANNEL_MAX ? plain : '';
}

/**
 * Accepted facts a call, email or visit can be about: the drawer's
 * fact when it is accepted (only that one), else the office's accepted
 * facts with the ones in the same field, then the related ones, then
 * the same topic first.
 * @param {NoteSubject} subject
 * @param {PublishedFact[]} facts
 * @returns {PublishedFact[]}
 */
export function anchorChoices(subject, facts) {
	const all = (facts ?? []).filter((f) => f.authority_slug === subject.authority_slug);
	if (subject.status === 'accepted') {
		const self = all.find((f) => f.id === subject.id);
		return self ? [self] : [];
	}
	const ids = subjectIds(subject);
	const slot = slotFor(subject.topic, subject.statement);
	/** @param {PublishedFact} f */
	const rank = (f) =>
		slot !== '' && slotFor(f.topic, f.statement) === slot ? 0 : ids.has(f.id) ? 1 : f.topic === subject.topic ? 2 : 3;
	return [...all].sort((a, b) => rank(a) - rank(b) || a.id - b.id);
}

/**
 * The anchor a new call, email or visit starts on: the drawer's fact
 * when accepted, else the live fact or a conflicting sibling it is
 * compared with, else the first accepted fact in its field; null when
 * none fits.
 * @param {NoteSubject} subject
 * @param {PublishedFact[]} facts
 * @returns {number | null}
 */
export function defaultAnchor(subject, facts) {
	if (subject.status === 'accepted') return subject.id;
	const choices = anchorChoices(subject, facts);
	for (const id of subject.related ?? []) if (choices.some((f) => f.id === id)) return id;
	const slot = slotFor(subject.topic, subject.statement);
	return choices.find((f) => slot !== '' && slotFor(f.topic, f.statement) === slot)?.id ?? null;
}

/**
 * Whether an accepted fact is in conflict on the public read (with
 * another accepted fact): the only facts a decision may prefer.
 * @param {PublishedFact} fact
 */
export function inPublicConflict(fact) {
	return (fact.conflicts ?? []).some((c) => c.status === 'accepted');
}

/**
 * The conflicting alternatives a decision chooses between: accepted
 * facts in public conflict that are the drawer's fact, compared with
 * it, conflicting with it, or in its field.
 * @param {NoteSubject} subject
 * @param {PublishedFact[]} facts
 * @returns {PublishedFact[]}
 */
export function preferredChoices(subject, facts) {
	const ids = subjectIds(subject);
	const slot = slotFor(subject.topic, subject.statement);
	return (facts ?? []).filter(
		(f) =>
			f.authority_slug === subject.authority_slug &&
			inPublicConflict(f) &&
			(ids.has(f.id) ||
				(f.conflicts ?? []).some((c) => ids.has(c.id)) ||
				(slot !== '' && slotFor(f.topic, f.statement) === slot))
	);
}

/**
 * Notes a decision can rest on: the office's published call, email,
 * visit and absence notes.
 * @param {EvidenceNote[]} notes
 * @param {string} slug
 * @returns {EvidenceNote[]}
 */
export function supportChoices(notes, slug) {
	return (notes ?? []).filter((n) => n.status === 'published' && n.kind !== 'decision' && n.authority_slug === slug);
}

/**
 * `YYYY-MM-DDTHH:MM` on the browser's clock (a datetime-local value).
 * @param {number} unix seconds
 */
export function toLocalInput(unix) {
	const d = new Date(unix * 1000);
	if (Number.isNaN(d.getTime())) return '';
	/** @param {number} n */
	const two = (n) => String(n).padStart(2, '0');
	return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}T${two(d.getHours())}:${two(d.getMinutes())}`;
}

/**
 * Unix seconds of a datetime-local value on the browser's clock, or
 * null when it is not one.
 * @param {unknown} value
 * @returns {number | null}
 */
export function fromLocalInput(value) {
	const m = LOCAL_INPUT.exec(String(value ?? ''));
	if (!m) return null;
	const [y, mo, d, h, mi, s] = m.slice(1).map((v) => Number(v ?? 0));
	const date = new Date(y, mo - 1, d, h, mi, s);
	if (date.getFullYear() !== y || date.getMonth() !== mo - 1 || date.getDate() !== d) return null;
	return Math.floor(date.getTime() / 1000);
}

/**
 * The first thing a note still needs, as a plain sentence, or null
 * when it is ready to send. `now` (unix seconds) also refuses a time
 * in the future; null skips that check.
 * @param {NoteInput} input
 * @param {number | null} now
 * @returns {string | null}
 */
export function noteProblem(input, now) {
	if (!SLUG.test(input.slug ?? '')) return 'This note has no office.';
	if (!KIND_VALUES.includes(input.kind)) return 'Pick a kind.';
	if (!STANCE_VALUES.includes(input.stance)) return 'Pick a stance.';
	const factKind = FACT_KINDS.includes(input.kind);
	if (factKind && !(Number.isInteger(input.factId) && Number(input.factId) > 0)) {
		return 'Pick the accepted fact this note is about.';
	}
	if (input.kind === 'decision') {
		if (!(Number.isInteger(input.preferredFactId) && Number(input.preferredFactId) > 0)) {
			return 'Pick the preferred fact.';
		}
		const supports = input.supports ?? [];
		if (supports.length === 0) return 'Pick at least one note the decision rests on.';
		if (supports.length > MAX_SUPPORTS) return `A decision rests on at most ${MAX_SUPPORTS} notes.`;
	}
	if (!factKind && !slotOk(input.slot)) return 'Pick the field.';
	if (input.when === null || !Number.isFinite(input.when) || input.when <= 0) {
		return 'Pick the date and time it happened.';
	}
	if (now !== null && input.when > now + FUTURE_GRACE) return 'The date and time are in the future.';
	const channel = charCount(input.channel);
	if (factKind && channel === 0) return 'Name the channel: the phone line, email address or place.';
	if (channel > CHANNEL_MAX) return `The channel is at most ${CHANNEL_MAX} characters.`;
	const summary = charCount(input.summary);
	if (summary < SUMMARY_MIN || summary > SUMMARY_MAX) {
		return `The summary needs ${SUMMARY_MIN}-${SUMMARY_MAX} characters.`;
	}
	return null;
}

/**
 * The POST /admin/evidence body, or the first problem. A call, email
 * or visit names its fact (never a preferred fact or supports); an
 * absence names only a slot; a decision names its slot, preferred
 * fact and supports. `recorded` and `capture_id` are never sent.
 * @param {NoteInput} input
 * @param {number | null} now unix seconds
 * @returns {{ body: Record<string, unknown> } | { error: string }}
 */
export function noteBody(input, now) {
	const problem = noteProblem(input, now);
	if (problem) return { error: problem };
	/** @type {Record<string, unknown>} */
	const body = {
		authority_slug: input.slug,
		kind: input.kind,
		stance: input.stance,
		occurred_at: input.when,
		channel: input.kind === 'decision' ? '' : plainText(input.channel),
		summary: plainText(input.summary)
	};
	if (FACT_KINDS.includes(input.kind)) {
		body.fact_id = input.factId;
		if (slotOk(input.slot)) body.slot = input.slot;
	} else {
		body.slot = input.slot;
		if (input.kind === 'decision') {
			body.preferred_fact_id = input.preferredFactId;
			body.supports = [...new Set(input.supports)];
		}
	}
	return { body };
}

/**
 * POST /admin/facts/{id}/retire body: 8-500 characters once trimmed,
 * else null.
 * @param {unknown} reason
 * @returns {{ reason: string } | null}
 */
export function retireBody(reason) {
	const text = String(reason ?? '').trim();
	const n = [...text].length;
	return n >= RETIRE_MIN && n <= RETIRE_MAX ? { reason: text } : null;
}

/**
 * POST /admin/evidence/{id}/withdraw body: a non-empty reason of at
 * most 1024 bytes once trimmed, else null.
 * @param {unknown} reason
 * @returns {{ reason: string } | null}
 */
export function withdrawBody(reason) {
	const text = String(reason ?? '').trim();
	const bytes = new TextEncoder().encode(text).length;
	return bytes > 0 && bytes <= WITHDRAW_MAX ? { reason: text } : null;
}

/**
 * Normalize one staff-view note. Null for unknown shapes. Pure.
 * @param {any} row
 * @returns {EvidenceNote | null}
 */
export function normalizeNote(row) {
	if (!row || !Number.isInteger(row.id) || typeof row.authority_slug !== 'string') return null;
	if (!KIND_VALUES.includes(row.kind) || !STANCE_VALUES.includes(row.stance)) return null;
	if (typeof row.summary !== 'string' || typeof row.occurred_at !== 'number') return null;
	/** @param {unknown} v */
	const id = (v) => (Number.isInteger(v) ? /** @type {number} */ (v) : null);
	return {
		id: row.id,
		authority_slug: row.authority_slug,
		kind: row.kind,
		fact_id: id(row.fact_id),
		preferred_fact_id: id(row.preferred_fact_id),
		slot: typeof row.slot === 'string' ? row.slot : '',
		stance: row.stance,
		occurred_at: row.occurred_at,
		date: typeof row.date === 'string' ? row.date : '',
		channel: typeof row.channel === 'string' ? row.channel : '',
		summary: row.summary,
		supports: Array.isArray(row.supports) ? row.supports.filter((/** @type {unknown} */ s) => Number.isInteger(s)) : [],
		status: row.status === 'withdrawn' ? 'withdrawn' : 'published',
		created_by: typeof row.created_by === 'string' ? row.created_by : '',
		created_at: typeof row.created_at === 'number' ? row.created_at : 0
	};
}

/**
 * Normalize one `/facts` row. Null for unknown shapes. Pure.
 * @param {any} row
 * @returns {PublishedFact | null}
 */
export function normalizeFact(row) {
	if (!row || !Number.isInteger(row.id) || typeof row.authority_slug !== 'string') return null;
	if (typeof row.topic !== 'string' || typeof row.statement !== 'string') return null;
	const citations = [];
	for (const c of Array.isArray(row.citations) ? row.citations : []) {
		if (!c || typeof c.quote !== 'string') continue;
		citations.push({
			quote: c.quote,
			source_url: typeof c.source_url === 'string' ? c.source_url : '',
			captured_at: typeof c.captured_at === 'string' ? c.captured_at : '',
			kind: typeof c.kind === 'string' ? c.kind : 'web'
		});
	}
	const conflicts = [];
	for (const c of Array.isArray(row.conflicts) ? row.conflicts : []) {
		if (!c || !Number.isInteger(c.id) || typeof c.statement !== 'string') continue;
		conflicts.push({
			id: c.id,
			topic: typeof c.topic === 'string' ? c.topic : row.topic,
			statement: c.statement,
			status: typeof c.status === 'string' ? c.status : ''
		});
	}
	return {
		id: row.id,
		authority_slug: row.authority_slug,
		topic: row.topic,
		statement: row.statement,
		status: typeof row.status === 'string' ? row.status : 'accepted',
		created_at: typeof row.created_at === 'number' ? row.created_at : 0,
		accepted_at: typeof row.accepted_at === 'number' ? row.accepted_at : null,
		citations,
		conflict: row.conflict === true || conflicts.length > 0,
		conflicts
	};
}

/**
 * A refused or failed staff request. `reason` is the server's own
 * `error` text (the guard's plain reason on a 422 or 409), else null.
 */
export class EvidenceError extends Error {
	/**
	 * @param {string} message
	 * @param {number} status HTTP status; 0 for a network or non-JSON answer
	 * @param {string | null} reason
	 */
	constructor(message, status, reason = null) {
		super(message);
		this.name = 'EvidenceError';
		this.status = status;
		this.reason = reason;
	}
}

/**
 * @param {Response} res
 * @param {string} what
 * @returns {Promise<never>}
 */
async function fail(res, what) {
	const body = await readErrorBody(res);
	const auth = authError(res.status, body);
	if (auth) throw new EvidenceError(auth, res.status);
	const reason = typeof body?.error === 'string' && body.error ? body.error : null;
	if ((res.status === 409 || res.status === 422) && reason) throw new EvidenceError(reason, res.status, reason);
	if (res.status === 404) throw new EvidenceError(reason ? `${what}: ${reason}.` : `${what} not found (404).`, 404, reason);
	if (res.status === 503) throw new EvidenceError('The store is unavailable on this server (503).', 503, reason);
	throw new EvidenceError(`${what} failed (HTTP ${res.status})${reason ? `: ${reason}` : ''}.`, res.status, reason);
}

/**
 * @param {string} url
 * @param {RequestInit} init
 * @param {string} what
 * @param {typeof fetch} fetchFn
 * @returns {Promise<any>}
 */
async function request(url, init, what, fetchFn) {
	let res;
	try {
		res = await fetchFn(url, { credentials: 'same-origin', ...init });
	} catch {
		throw new EvidenceError(`${what} could not reach the admin host.`, 0);
	}
	if (!res.ok) await fail(res, what);
	try {
		return await res.json();
	} catch {
		// The static demo host answers unknown API paths with the SPA shell.
		throw new EvidenceError(`${what} is unavailable on this host.`, 0);
	}
}

/**
 * @param {string} url
 * @param {unknown} body
 * @param {string} what
 * @param {typeof fetch} fetchFn
 */
function post(url, body, what, fetchFn) {
	return request(
		url,
		{
			method: 'POST',
			headers: { 'content-type': 'application/json', ...csrfHeaders() },
			body: JSON.stringify(body ?? {})
		},
		what,
		fetchFn
	);
}

/**
 * The office's notes from the staff list (newest first).
 * @param {string} apiBase
 * @param {string} slug
 * @param {{ status?: 'published' | 'withdrawn' | 'all' }} [opts]
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<EvidenceNote[]>}
 */
export async function fetchNotes(apiBase, slug, opts = {}, fetchFn = fetch) {
	const query = new URLSearchParams({ slug, status: opts.status ?? 'published' });
	const data = await request(`${apiBase}/admin/evidence?${query}`, {}, 'Staff notes', fetchFn);
	if (!Array.isArray(data?.notes)) throw new EvidenceError('Staff notes returned an unexpected shape.', 0);
	return data.notes.map(normalizeNote).filter((/** @type {EvidenceNote | null} */ n) => n !== null);
}

/**
 * Accepted facts, of one office when `slug` is given (at most 200, id order).
 * @param {string} apiBase
 * @param {string | undefined} slug
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<PublishedFact[]>}
 */
export async function fetchAcceptedFacts(apiBase, slug, fetchFn = fetch) {
	const query = new URLSearchParams({ status: 'accepted', limit: '200' });
	if (slug) query.set('authority', slug);
	const data = await request(`${apiBase}/facts?${query}`, {}, 'Accepted facts', fetchFn);
	if (!Array.isArray(data?.facts)) throw new EvidenceError('Accepted facts returned an unexpected shape.', 0);
	return data.facts.map(normalizeFact).filter((/** @type {PublishedFact | null} */ f) => f !== null);
}

/**
 * Publish one note (201: the staff view).
 * @param {string} apiBase
 * @param {Record<string, unknown>} body from `noteBody`
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<EvidenceNote>}
 */
export async function createNote(apiBase, body, fetchFn = fetch) {
	const note = normalizeNote(await post(`${apiBase}/admin/evidence`, body, 'Publish note', fetchFn));
	if (!note) throw new EvidenceError('Publish note returned an unexpected shape.', 0);
	return note;
}

/**
 * Withdraw one published note.
 * @param {string} apiBase
 * @param {number} id
 * @param {{ reason: string }} body from `withdrawBody`
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<EvidenceNote>}
 */
export async function withdrawNote(apiBase, id, body, fetchFn = fetch) {
	const note = normalizeNote(await post(`${apiBase}/admin/evidence/${id}/withdraw`, body, 'Withdraw note', fetchFn));
	if (!note) throw new EvidenceError('Withdraw note returned an unexpected shape.', 0);
	return note;
}

/**
 * Retire one accepted fact (it turns `superseded`).
 * @param {string} apiBase
 * @param {number} id
 * @param {{ reason: string }} body from `retireBody`
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ id: number, authority_slug: string, status: string }>}
 */
export async function retireFact(apiBase, id, body, fetchFn = fetch) {
	const data = await post(`${apiBase}/admin/facts/${id}/retire`, body, 'Retire', fetchFn);
	if (!data || typeof data.status !== 'string') throw new EvidenceError('Retire returned an unexpected shape.', 0);
	return {
		id: Number.isInteger(data.id) ? data.id : id,
		authority_slug: typeof data.authority_slug === 'string' ? data.authority_slug : '',
		status: data.status
	};
}
