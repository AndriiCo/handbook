// Dossier guide (GP2-page): readable, cited, step-by-step facts.
//
// The analyser writes accepted statements as "{field}: {value}"; this
// module parses them into labelled items grouped into a fixed step
// order over the frozen topic vocabulary
// (crates/contracts/src/topics.rs). Only staff-accepted facts enter;
// `phone_script` never renders publicly, and `service_notice` facts
// are a dated banner above the guide (see `serviceNotices`), never a
// step. Pure: no window/fetch, so node tests drive it directly.

import { guideCopy } from './doc/copy/index.js';

/**
 * Fixed guide steps: stable ids, titles, and their topics.
 */
export const GUIDE_STEPS = [
	{
		id: 'office',
		title: 'Is this the right office?',
		topics: ['jurisdiction', 'authentication', 'legalization_chain', 'website']
	},
	{
		id: 'before',
		title: 'Before you start',
		topics: ['notarization', 'translation', 'copies', 'id_requirements']
	},
	{ id: 'prepare', title: 'What to prepare', topics: ['forms'] },
	{ id: 'cost', title: 'What it costs', topics: ['fees', 'payment'] },
	{
		id: 'in-person',
		title: 'Submit in person',
		topics: ['address', 'walkin', 'appointment', 'hours', 'holiday_hours', 'parking', 'route_photos']
	},
	{
		id: 'by-mail',
		title: 'Submit by mail',
		topics: ['mailing', 'po_box', 'dropbox']
	},
	{ id: 'timing', title: 'How long it takes', topics: ['processing_time'] },
	{ id: 'trouble', title: 'If something goes wrong', topics: ['rejections', 'contacts'] }
];

/** Topic of dated service notices (closures, outages): banner only. */
export const NOTICE_TOPIC = 'service_notice';

/**
 * Banner rows for accepted `service_notice` facts, newest capture
 * first (ties: newest fact first): the notice text, its verbatim
 * quote, source and captured date. Uncited notices never render: a
 * banner that changes someone's plans must show where it came from.
 * @param {any[]} facts
 * @returns {{ id: number, text: string, quote: string, sourceUrl: string, capturedAt: string }[]}
 */
export function serviceNotices(facts) {
	return (facts ?? [])
		.filter((f) => f?.status === 'accepted' && f?.topic === NOTICE_TOPIC)
		.map((f) => {
			const cite = Array.isArray(f.citations) ? f.citations[0] : null;
			return {
				id: f.id ?? 0,
				text: parseStatement(f.statement).value,
				quote: String(cite?.quote ?? ''),
				sourceUrl: String(cite?.source_url ?? ''),
				capturedAt: String(cite?.captured_at ?? '')
			};
		})
		.filter((n) => n.quote && n.sourceUrl.startsWith('https://') && n.capturedAt)
		.sort((a, b) =>
			a.capturedAt === b.capturedAt ? b.id - a.id : a.capturedAt < b.capturedAt ? 1 : -1
		);
}

/**
 * "What changed" panels from `GET /authorities/{slug}/changes`: one
 * per source with any added or removed line, newest change first.
 * Malformed rows are dropped, never guessed at.
 * @param {unknown} body
 * @returns {{ sourceUrl: string, date: string, since: string, added: string[], removed: string[], more: number }[]}
 */
export function changePanels(body) {
	/** @type {any} */
	const raw = body;
	/** @type {any[]} */
	const rows = Array.isArray(raw?.changes) ? raw.changes : [];
	/** @param {unknown} v @returns {string[]} */
	const lines = (v) => (Array.isArray(v) ? v.filter((l) => typeof l === 'string' && l) : []);
	/** @param {unknown} v @returns {number} */
	const count = (v) => (typeof v === 'number' && v > 0 ? v : 0);
	/** @type {{ sourceUrl: string, date: string, capturedAt: string, since: string, added: string[], removed: string[], more: number }[]} */
	const panels = [];
	for (const c of rows) {
		if (typeof c?.source_url !== 'string' || !c.source_url.startsWith('https://')) continue;
		if (typeof c?.captured_at !== 'string') continue;
		const panel = {
			sourceUrl: c.source_url,
			date: c.captured_at.split('T')[0],
			capturedAt: c.captured_at,
			since: typeof c.previous_captured_at === 'string' ? c.previous_captured_at.split('T')[0] : '',
			added: lines(c.added),
			removed: lines(c.removed),
			more: count(c.more_added) + count(c.more_removed)
		};
		if (panel.added.length + panel.removed.length > 0) panels.push(panel);
	}
	return panels.sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : 0));
}

/**
 * Split "{field}: {value}" into its key and readable value; anything
 * else is an unkeyed statement shown as-is.
 * @param {string} statement
 * @returns {{ fieldKey: string|null, value: string }}
 */
export function parseStatement(statement) {
	const s = String(statement ?? '');
	const m = /^([a-z][a-z0-9_]*):\s+(.+)$/s.exec(s);
	if (!m) return { fieldKey: null, value: s };
	return { fieldKey: m[1], value: m[2] };
}

/**
 * Known analyser field keys get fixed labels (in the page language,
 * doc/copy `fields`); everything else is humanised snake_case, which
 * stays as the key spells it. `topic` disambiguates future same-key
 * reuse.
 * @param {string} topic
 * @param {string|null} fieldKey
 * @param {string} [lang] the page language (doc/copy)
 */
export function fieldLabel(topic, fieldKey, lang = 'en') {
	const t = guideCopy(lang);
	/** @type {Record<string, string>} */
	const known = t.fields;
	if (typeof fieldKey === 'string' && Object.hasOwn(known, fieldKey)) return known[fieldKey];
	const s = String(fieldKey ?? topic ?? '').replaceAll('_', ' ');
	if (!s) return t.present.detail;
	return s.charAt(0).toUpperCase() + s.slice(1);
}

/**
 * `https://www.sos.wa.gov/x` -> `sos.wa.gov` for citation captions.
 * @param {unknown} url
 */
export function domain(url) {
	try {
		return new URL(String(url)).hostname.replace(/^www\./, '');
	} catch {
		return 'official source';
	}
}

/**
 * Escape text for the citation popover. Source quotes carry the
 * page's line breaks (office addresses especially); HTML would
 * collapse them into one run-on line, so they render as breaks.
 * @param {string} s
 */
function escQuote(s) {
	return String(s ?? '')
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;')
		.replace(/\n/g, '<br>');
}

/**
 * Clean one whitespace-separated token for value matching:
 * lowercase, strip edge punctuation, but keep digits and `$`, `/`,
 * `:`, `-` (fees, phones, hours all hinge on them).
 * @param {string} raw
 */
function cleanToken(raw) {
	return String(raw ?? '')
		.toLowerCase()
		.replace(/^[^a-z0-9$:/-]+|[^a-z0-9$:/-]+$/g, '');
}

/**
 * Mark the exact value span inside the quote, so the citation shows
 * which span the fact rests on. The statement's value (after its
 * `field:` key, when keyed) is tokenized with {@link cleanToken};
 * the first contiguous token run in the quote is wrapped in one
 * `<mark>` at its original offsets (case and punctuation kept).
 * Falls back to the longest 2+ token sub-run, else no mark.
 * Returns HTML (caller renders with `{@html}`).
 * @param {string} quote
 * @param {string} statement
 */
export function markQuote(quote, statement) {
	const q = String(quote ?? '');
	const { value } = parseStatement(statement);
	const wanted = String(value ?? '')
		.split(/\s+/)
		.map(cleanToken)
		.filter(Boolean);
	if (wanted.length === 0) return escQuote(q);
	/** @type {{ t: string, start: number, end: number }[]} */
	const have = [];
	const re = /\S+/g;
	let m;
	while ((m = re.exec(q)) !== null) {
		const t = cleanToken(m[0]);
		if (t) have.push({ t, start: m.index, end: m.index + m[0].length });
	}
	/**
	 * @param {number} at
	 * @param {string[]} run
	 * @returns {boolean}
	 */
	const same = (at, run) => {
		if (at + run.length > have.length) return false;
		for (let k = 0; k < run.length; k++) if (have[at + k].t !== run[k]) return false;
		return true;
	};
	let at = -1;
	let len = wanted.length;
	for (let i = 0; i + wanted.length <= have.length; i++) {
		if (same(i, wanted)) {
			at = i;
			break;
		}
	}
	if (at < 0 && wanted.length > 2) {
		for (let l = wanted.length - 1; l >= 2 && at < 0; l--) {
			for (let s = 0; s + l <= wanted.length && at < 0; s++) {
				const sub = wanted.slice(s, s + l);
				for (let i = 0; i + l <= have.length; i++) {
					if (same(i, sub)) {
						at = i;
						len = l;
						break;
					}
				}
			}
		}
	}
	if (at < 0) return escQuote(q);
	// The span covers the matched raw tokens; trailing edge
	// punctuation (a sentence period, comma) is not part of the
	// value, so it stays outside the mark. Leading characters
	// stay: `(`, `$`, `+` usually belong to the value itself.
	const lastRaw = q.slice(have[at + len - 1].start, have[at + len - 1].end);
	const tail = lastRaw.match(/[^a-z0-9$:/-]+$/);
	const span = [have[at].start, have[at + len - 1].end - (tail ? tail[0].length : 0)];
	return escQuote(q.slice(0, span[0])) + '<mark>' + escQuote(q.slice(span[0], span[1])) + '</mark>' + escQuote(q.slice(span[1]));
}

/**
 * A "Source" link that opens the cited page at our quote via a
 * text fragment (`#:~:text=<start>,<end>`): the first and last five
 * quote words, percent-encoded (`-`, `&` and `,` escaped, since
 * they delimit the fragment). PDFs get the plain URL — most
 * viewers jump to fragments unreliably there. Returns null for a
 * non-https URL or an empty quote (caller renders "unavailable").
 * @param {unknown} url
 * @param {unknown} quote
 * @returns {string|null}
 */
export function sourceUrl(url, quote) {
	const u = String(url ?? '');
	if (!u.startsWith('https://')) return null;
	if (/\.pdf($|[?#])/i.test(u)) return u;
	if (u.includes('#')) return u;
	const words = String(quote ?? '').split(/\s+/).filter(Boolean);
	if (words.length === 0) return null;
	// `encodeURIComponent` already escapes `&` and `,`; `-` needs
	// an explicit escape or the fragment parser splits on it.
	/** @param {string} s */
	const frag = (s) => encodeURIComponent(s).replace(/-/g, '%2D');
	const start = words.slice(0, 5).join(' ');
	if (words.length <= 5) return `${u}#:~:text=${frag(start)}`;
	const end = words.slice(-5).join(' ');
	return `${u}#:~:text=${frag(start)},${frag(end)}`;
}

/**
 * Freshness of a fact's first citation against `now` (ms epoch).
 * Older than 90 days: "checked N months ago"; older than 365 days
 * it is also a warning. Newer (or uncited): null.
 * @param {any} fact
 * @param {number} now
 * @returns {{ months: number, warning: boolean }|null}
 */
export function staleMark(fact, now) {
	const cites = Array.isArray(fact?.citations) ? fact.citations : [];
	const at = cites.length > 0 ? Date.parse(cites[0]?.captured_at) : NaN;
	if (!Number.isFinite(at)) return null;
	const days = Math.floor((now - at) / 86_400_000);
	if (days < 90) return null;
	return { months: Math.max(3, Math.floor(days / 30)), warning: days >= 365 };
}

/**
 * Build guide steps from accepted facts: per step, one item per
 * accepted fact in the step's topics (topic order, then id order).
 * Same topic + same field key with different values is a conflict:
 * both items render, flagged. Unkeyed statements never conflict.
 * @param {any[]} facts
 * @param {number} now ms epoch
 * @returns {{ id: string, title: string, items: any[] }[]}
 */
export function buildGuide(facts, now) {
	const list = (facts ?? []).filter(
		(f) =>
			f?.status === 'accepted' && f?.topic !== 'phone_script' && f?.topic !== NOTICE_TOPIC
	);
	const steps = [];
	for (const step of GUIDE_STEPS) {
		const rows = list
			.filter((f) => step.topics.includes(f.topic))
			.sort((a, b) => {
				const ta = step.topics.indexOf(a.topic) - step.topics.indexOf(b.topic);
				if (ta !== 0) return ta;
				return (a.id ?? 0) - (b.id ?? 0);
			});
		if (rows.length === 0) continue;
		// Conflict cliques: same topic + same key, differing values.
		const seen = new Map();
		for (const f of rows) {
			const { fieldKey, value } = parseStatement(f.statement);
			const k = `${f.topic}::${fieldKey ?? `row:${f.id ?? value}`}`;
			if (!seen.has(k)) seen.set(k, new Set());
			seen.get(k).add(value);
		}
		const items = rows.map((f) => {
			const { fieldKey, value } = parseStatement(f.statement);
			const k = `${f.topic}::${fieldKey ?? `row:${f.id ?? value}`}`;
			return {
				label: fieldLabel(f.topic, fieldKey),
				value,
				topic: f.topic,
				factIds: [f.id],
				citations: Array.isArray(f.citations) ? f.citations : [],
				conflict: fieldKey !== null && (seen.get(k)?.size ?? 0) > 1,
				stale: staleMark(f, now),
				draftedBy: String(f.drafted_by ?? f.created_by ?? 'unknown').split(' ')[0]
			};
		});
		steps.push({ id: step.id, title: step.title, items });
	}
	return steps;
}
