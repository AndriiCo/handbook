// Dossier guide (GP2-page): readable, cited, step-by-step facts.
//
// The analyser writes accepted statements as "{field}: {value}"; this
// module parses them into labelled items grouped into a fixed step
// order over the frozen topic vocabulary
// (crates/contracts/src/topics.rs). Only staff-accepted facts enter;
// `phone_script` never renders publicly. Pure: no window/fetch, so
// node tests drive it directly.

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
 * Known analyser field keys get fixed labels; everything else is
 * humanised snake_case. `topic` disambiguates future same-key reuse.
 * @param {string} topic
 * @param {string|null} fieldKey
 */
export function fieldLabel(topic, fieldKey) {
	/** @type {Record<string, string>} */
	const known = {
		per_document: 'Fee per document',
		apostille_main_branch_street: 'Office address',
		standard: 'Standard processing',
		weekday: 'Weekday hours',
		po_box: 'PO Box',
		methods: 'Accepted payment'
	};
	if (typeof fieldKey === 'string' && fieldKey in known) return known[fieldKey];
	const s = String(fieldKey ?? topic ?? '').replaceAll('_', ' ');
	if (!s) return 'Detail';
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
 * Mark the longest statement phrase found in the quote, so the
 * citation shows which span the fact rests on. Returns HTML with
 * one `<mark>` (caller renders with `{@html}`).
 * @param {string} quote
 * @param {string} statement
 */
export function markQuote(quote, statement) {
	const esc = (/** @type {string} */ s) =>
		String(s ?? '')
			.replace(/&/g, '&amp;')
			.replace(/</g, '&lt;')
			.replace(/>/g, '&gt;')
			.replace(/"/g, '&quot;');
	const words = String(statement ?? '')
		.toLowerCase()
		.replace(/[^a-z0-9\s]/g, ' ')
		.split(/\s+/)
		.filter((w) => w.length > 2);
	const ql = String(quote ?? '').toLowerCase();
	for (let len = Math.min(12, words.length); len >= 3; len--) {
		for (let i = 0; i + len <= words.length; i++) {
			const phrase = words.slice(i, i + len).join(' ');
			const idx = ql.indexOf(phrase);
			if (idx >= 0) {
				const q = String(quote ?? '');
				return (
					esc(q.slice(0, idx)) +
					'<mark>' +
					esc(q.slice(idx, idx + phrase.length)) +
					'</mark>' +
					esc(q.slice(idx + phrase.length))
				);
			}
		}
	}
	return esc(String(quote ?? ''));
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
		(f) => f?.status === 'accepted' && f?.topic !== 'phone_script'
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
