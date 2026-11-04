// Citations mode, the pure half (R2): source numbering, card and
// endnote copy, dates, and the on/off preference.
//
// `numberSources()` gives every source on the page one number, by
// first appearance over the presenter's canonical order (status band,
// glance, then sections top to bottom), and letters each distinct
// quote within a source; calculations are numbered after every real
// source. The marker (doc/Cite.svelte), the shared card
// (doc/CitationLayer.svelte), the endnotes (doc/Endnotes.svelte) and
// print all read its output, so screen, print and every route show the
// same [n].
//
// Shapes: doc/contract.js (Item, Cite, SourceRef, Source, Ref). The
// output is a superset of the contract's Source and Ref: the extra
// fields are listed on each typedef below, and consumers that read only
// the contract fields keep working.
//
// Optional inputs beyond the contract (the presenter may set them):
// - SourceRef.supports: string[], source keys a staff decision rests on;
// - Item.terms (calculated items): { text, itemId }[], the calculation
//   in reading order, e.g. [{ text: '3 × $15.00', itemId: 'fees-per' },
//   { text: '+ $100.00 expedite fee (1–10 documents)', itemId:
//   'fees-expedite' }, { text: '= $145.00', itemId: null }]; the card
//   and the endnote then print each term with its input's [n letter].
//   Without it they print the item's display and list the inputs.
// - SourceRef.channel on an absence note names the pages staff
//   checked (the evidence lane's `channel`, e.g. "FAQ and fee pages").
//
// Pure: no window, document or fetch, so node tests drive it directly.

import { domain } from './guide.js';
import { guideCopy } from './doc/copy/index.js';

/** @typedef {import('./doc/contract.js').Item} Item */
/** @typedef {import('./doc/contract.js').Cite} Cite */
/** @typedef {import('./doc/contract.js').SourceRef} SourceRef */
/** @typedef {import('./doc/contract.js').EvidenceKind} EvidenceKind */

/**
 * One input of a calculated value, as its card and endnote list it.
 * @typedef {{ label: string, display: string, refs: { n: number, letter: string }[] }} CiteInput
 */

/**
 * One term of a calculation (from the item's optional `terms`), with
 * the refs of the input it reads.
 * @typedef {{ text: string, refs: { n: number, letter: string }[] }} CiteTerm
 */

/**
 * One lettered quote (or staff summary) within a numbered source.
 * Contract fields: letter, text, summary, label, itemId, refId (the
 * first marker that shows it: the endnote backlink target).
 * Extra:
 * - value: the first item's display, so `markQuote` can mark the span;
 * - capturedAt / acceptedAt: that citation's own capture and accept;
 * - uses: every marker that shows this quote, first one first;
 * - inputs: calculated values only, the cited inputs and their refs;
 * - terms: calculated values with `Item.terms` only, the calculation.
 * @typedef {{
 *   letter: string,
 *   text: string|null,
 *   summary: string|null,
 *   label: string,
 *   itemId: string,
 *   refId: string,
 *   value: string,
 *   capturedAt: string|null,
 *   acceptedAt: number|null,
 *   uses: { itemId: string, refId: string, label: string }[],
 *   inputs: CiteInput[]|null,
 *   terms: CiteTerm[]|null
 * }} CiteQuote
 */

/**
 * One numbered endnote: the contract's Source plus
 * - capturedMin / acceptedMin: earliest capture and accept across its
 *   quotes (capturedAt / acceptedAt hold the latest);
 * - basedOn: for a staff decision, the other sources it rests on.
 * @typedef {SourceRef & {
 *   n: number,
 *   quotes: CiteQuote[],
 *   capturedMin: string|null,
 *   acceptedMin: number|null,
 *   basedOn: number[]
 * }} NumberedSource
 */

/**
 * A marker for one item: the contract's Ref plus `letters`, every quote
 * of that source the item shows (the card lists them all), and `key`,
 * the source key (an open card finds its marker again by item and key
 * after a live update renumbers the page). One marker per source per
 * item, sorted by number.
 * @typedef {{ n: number, letter: string, refId: string, letters: string[], key: string }} MarkerRef
 */

/** @type {readonly EvidenceKind[]} */
const KINDS = ['web', 'pdf', 'upload', 'call', 'email', 'visit', 'decision', 'absence', 'computed'];

/** @param {unknown} v */
const str = (v) => (typeof v === 'string' ? v.trim() : '');
/** @param {unknown} v @returns {number|null} */
const secs = (v) => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : null);
/** @param {unknown} v @returns {string|null} */
const isoOrNull = (v) => (typeof v === 'string' && Number.isFinite(Date.parse(v)) ? v : null);
/** @param {string} s */
const squash = (s) => s.replace(/\s+/g, ' ').trim();

/**
 * Letters for quotes within a source: a … z, then aa, ab, … (bijective
 * base 26), so any number of quotes stays lettered.
 * @param {number} i zero-based
 */
export function letterFor(i) {
	let n = Math.max(0, Math.floor(i)) + 1;
	let out = '';
	while (n > 0) {
		n -= 1;
		out = String.fromCharCode(97 + (n % 26)) + out;
		n = Math.floor(n / 26);
	}
	return out;
}

/**
 * The canonical form of a source URL: absolute http(s), fragment
 * dropped (a `#:~:text=` highlight or an anchor never splits one page
 * into two sources). Null for anything else: not a URL, or another
 * scheme (javascript:, data:, file:…), which must never become a link.
 * @param {unknown} url
 * @returns {string|null}
 */
export function canonicalUrl(url) {
	const s = str(url);
	if (!s) return null;
	try {
		const u = new URL(s);
		if (u.protocol !== 'https:' && u.protocol !== 'http:') return null;
		u.hash = '';
		return u.href;
	} catch {
		return null;
	}
}

/**
 * The address a card or endnote may link to: the canonical URL when it
 * is https, else null (an http address prints as plain text).
 * @param {unknown} url
 * @returns {string|null}
 */
export function linkHref(url) {
	const u = canonicalUrl(url);
	return u && u.startsWith('https://') ? u : null;
}

/** @param {string} url */
function isSiteRoot(url) {
	try {
		return new URL(url).pathname === '/';
	} catch {
		return false;
	}
}

/** @param {string} host */
const bareHost = (host) => host.toLowerCase().replace(/^www\./, '');

/** @param {unknown} url */
export function isPdfUrl(url) {
	return /\.pdf($|[?#])/i.test(str(url));
}

/**
 * A readable title from a URL's last path segment
 * (`…/apostille-frequently-asked-questions` → "Apostille frequently
 * asked questions"); null at the site root.
 * @param {string|null} url
 * @returns {string|null}
 */
export function titleFromUrl(url) {
	if (!url) return null;
	let path = '';
	try {
		path = new URL(url).pathname;
	} catch {
		return null;
	}
	const last = path.split('/').filter(Boolean).pop() ?? '';
	let seg = last;
	try {
		seg = decodeURIComponent(last);
	} catch {
		// keep the raw segment
	}
	seg = squash(seg.replace(/\.(pdf|html?|aspx?|php)$/i, '').replace(/[-_+]+/g, ' '));
	if (!seg) return null;
	return seg.charAt(0).toUpperCase() + seg.slice(1);
}

/**
 * The numbering key of a source: its `key` (URL keys lose their
 * fragment), else its canonical URL. Null when it has neither.
 * @param {Partial<SourceRef>|null|undefined} ref
 * @returns {string|null}
 */
export function sourceKey(ref) {
	const key = str(ref?.key);
	if (key) return /^https?:\/\//i.test(key) ? (canonicalUrl(key) ?? key) : key;
	return canonicalUrl(ref?.url);
}

/**
 * @param {Partial<SourceRef>} ref
 * @param {string} key
 * @param {string} [lang] the page language (doc/copy)
 * @returns {SourceRef}
 */
function normalizeRef(ref, key, lang = 'en') {
	const url = canonicalUrl(ref.url) ?? canonicalUrl(/^https?:\/\//i.test(key) ? key : null);
	/** @type {EvidenceKind} */
	const kind = KINDS.includes(/** @type {EvidenceKind} */ (ref.kind))
		? /** @type {EvidenceKind} */ (ref.kind)
		: url
			? 'web'
			: 'decision';
	const host = str(ref.host) || (url ? domain(url) : '') || null;
	// A page with no title of its own reads its last path segment; the
	// site root is the home page (never the host twice: "sos.wa.gov —
	// sos.wa.gov").
	const fallback = url ? (titleFromUrl(url) ?? (isSiteRoot(url) ? guideCopy(lang).present.homePage : '')) : '';
	return {
		key,
		kind,
		title: str(ref.title) || fallback,
		url,
		host,
		capturedAt: isoOrNull(ref.capturedAt),
		acceptedAt: secs(ref.acceptedAt),
		channel: str(ref.channel) || null,
		occurredAt: secs(ref.occurredAt),
		recorded: typeof ref.recorded === 'boolean' ? ref.recorded : null,
		dayOnly: ref.dayOnly === true,
		stance: ref.stance === 'confirms' || ref.stance === 'contradicts' || ref.stance === 'context' ? ref.stance : null
	};
}

/**
 * @param {string|null} a
 * @param {string|null} b
 * @param {1|-1} sign 1 = later wins, -1 = earlier wins
 */
function pickIso(a, b, sign) {
	if (!a) return b;
	if (!b) return a;
	return (Date.parse(b) - Date.parse(a)) * sign > 0 ? b : a;
}

/**
 * @param {number|null} a
 * @param {number|null} b
 * @param {1|-1} sign
 */
function pickSecs(a, b, sign) {
	if (a === null) return b;
	if (b === null) return a;
	return (b - a) * sign > 0 ? b : a;
}

/**
 * Number every source the given items cite.
 *
 * - Items are visited in the given order (the presenter's canonical
 *   order: status band, glance, then sections), then each item's
 *   alternatives; a calculated item's inputs right after it. An id
 *   seen once is not numbered again.
 * - One number per source key (a canonical URL, or `note:<id>` for a
 *   staff evidence note, which numbers like any other source).
 * - Real sources take 1, 2, 3… by first appearance; calculations are
 *   numbered after all of them, so a calculator that switches between
 *   a total and "ask the office" never renumbers the office's pages.
 * - Within a source, each distinct quote (or staff summary) gets the
 *   next letter; the same words cited by two items reuse their letter.
 * - An item shows one marker per source, sorted by number: an item
 *   with two sources gets one group, [1, 4].
 * - A calculated item (state `computed`, or with inputs) gets its own
 *   source, kind `computed`, keyed `computed:<item id>` unless it
 *   carries a `computed` cite, whose quote lists every input with its
 *   number and letter (and its `terms`, when the item has them).
 * - A staff decision's `basedOn` lists the other sources cited next to
 *   it, plus any keys in its SourceRef's optional `supports` array.
 *
 * @param {Item[]} items
 * @param {string} [lang] the page language (doc/copy)
 * @returns {{ sources: NumberedSource[], refs: Record<string, MarkerRef[]> }}
 */
export function numberSources(items, lang = 'en') {
	/** @typedef {{ src: NumberedSource, letter: string, letters: string[], refId: string }} Mark */

	/** Every source, in order of first appearance (numbered at the end). @type {NumberedSource[]} */
	const sources = [];
	/** @type {Map<string, NumberedSource>} */
	const byKey = new Map();
	/** Each visited item's markers, in visit order. @type {Map<string, Mark[]>} */
	const marks = new Map();
	/** Every marker in creation order: marker ids follow it. @type {Mark[]} */
	const created = [];
	/** Per quote: the first marker that shows it and every use. @type {Map<CiteQuote, { first: Mark|null, uses: { itemId: string, mark: Mark, label: string }[] }>} */
	const shown = new Map();
	/** Calculated quotes, resolved once everything is numbered. @type {{ q: CiteQuote, inputs: Item[], terms: { text: string, itemId: string|null }[]|null }[]} */
	const calcs = [];
	/** @type {Set<string>} */
	const visited = new Set();
	/** @type {Map<NumberedSource, Set<NumberedSource>>} */
	const decisionPeers = new Map();
	/** @type {Map<NumberedSource, string[]>} */
	const supports = new Map();

	/**
	 * @param {Partial<SourceRef>|null|undefined} raw
	 * @returns {NumberedSource|null}
	 */
	const sourceFor = (raw) => {
		const key = sourceKey(raw);
		if (!raw || !key) return null;
		const ref = normalizeRef(raw, key, lang);
		let s = byKey.get(key);
		if (!s) {
			s = {
				...ref,
				n: 0,
				quotes: [],
				capturedMin: ref.capturedAt,
				acceptedMin: ref.acceptedAt,
				basedOn: []
			};
			byKey.set(key, s);
			sources.push(s);
		} else {
			s.capturedAt = pickIso(s.capturedAt, ref.capturedAt, 1);
			s.capturedMin = pickIso(s.capturedMin, ref.capturedAt, -1);
			s.acceptedAt = pickSecs(s.acceptedAt, ref.acceptedAt, 1);
			s.acceptedMin = pickSecs(s.acceptedMin, ref.acceptedAt, -1);
			if (!s.title && ref.title) s.title = ref.title;
			if (!s.channel && ref.channel) s.channel = ref.channel;
			if (s.occurredAt === null) {
				s.occurredAt = ref.occurredAt;
				s.dayOnly = ref.dayOnly;
			}
			if (s.recorded === null) s.recorded = ref.recorded;
			if (!s.stance && ref.stance) s.stance = ref.stance;
		}
		const extra = /** @type {{ supports?: unknown }} */ (raw).supports;
		if (Array.isArray(extra)) {
			const keys = supports.get(s) ?? [];
			for (const k of extra) if (typeof k === 'string' && k) keys.push(k);
			supports.set(s, keys);
		}
		return s;
	};

	/**
	 * The quote entry for these words, created on first use.
	 * @param {NumberedSource} s
	 * @param {string|null} text
	 * @param {string|null} summary
	 * @param {Item} item
	 * @param {{ capturedAt: string|null, acceptedAt: number|null }} dates
	 * @returns {CiteQuote}
	 */
	const quoteFor = (s, text, summary, item, dates) => {
		const k = `${squash(text ?? '')}\u0000${squash(summary ?? '')}`;
		const found = s.quotes.find(
			(q) => `${squash(q.text ?? '')}\u0000${squash(q.summary ?? '')}` === k
		);
		if (found) return found;
		/** @type {CiteQuote} */
		const q = {
			letter: letterFor(s.quotes.length),
			text,
			summary,
			label: str(item.label),
			itemId: item.id,
			refId: '',
			value: str(item.display),
			capturedAt: dates.capturedAt,
			acceptedAt: dates.acceptedAt,
			uses: [],
			inputs: null,
			terms: null
		};
		s.quotes.push(q);
		shown.set(q, { first: null, uses: [] });
		return q;
	};

	/**
	 * Attach one quote to the item's marker for its source.
	 * @param {Item} item
	 * @param {Mark[]} list
	 * @param {NumberedSource} s
	 * @param {CiteQuote} q
	 * @returns {Mark}
	 */
	const attach = (item, list, s, q) => {
		let mark = list.find((m) => m.src === s);
		if (!mark) {
			mark = { src: s, letter: q.letter, letters: [], refId: '' };
			list.push(mark);
			created.push(mark);
		}
		if (!mark.letters.includes(q.letter)) mark.letters.push(q.letter);
		const seen = /** @type {{ first: Mark|null, uses: { itemId: string, mark: Mark, label: string }[] }} */ (
			shown.get(q)
		);
		if (!seen.first) seen.first = mark;
		if (!seen.uses.some((u) => u.itemId === item.id)) {
			seen.uses.push({ itemId: item.id, mark, label: str(item.label) });
		}
		return mark;
	};

	/** @param {unknown} raw */
	const visit = (raw) => {
		const item = /** @type {Item} */ (raw);
		if (!item || typeof item !== 'object' || typeof item.id !== 'string' || !item.id) return;
		if (visited.has(item.id)) return;
		visited.add(item.id);
		/** @type {Mark[]} */
		const list = [];
		marks.set(item.id, list);
		const cites = Array.isArray(item.cites) ? item.cites : [];
		const inputs = Array.isArray(item.inputs) ? item.inputs : [];
		const calculated = item.state === 'computed' || inputs.length > 0;

		if (calculated) {
			const own = cites.find((c) => c?.source?.kind === 'computed');
			const s = sourceFor(
				own?.source ?? {
					key: `computed:${item.id}`,
					kind: 'computed',
					title: 'Calculated on this page'
				}
			);
			if (s) {
				const summary = str(own?.summary) || str(item.display) || null;
				const q = quoteFor(s, null, summary, item, { capturedAt: null, acceptedAt: null });
				const via = attach(item, list, s, q);
				for (const input of inputs) visit(input);
				const listed = inputs.filter(
					(input) => input && typeof input.id === 'string' && input.id !== ''
				);
				calcs.push({ q, inputs: listed, terms: termsOf(item) });
				// The card of this marker shows the inputs' quotes too, so
				// their endnote backlinks may return here when the input's
				// own marker is not on the page.
				for (const input of listed) {
					for (const m of marks.get(input.id) ?? []) {
						for (const iq of m.src.quotes) {
							if (!m.letters.includes(iq.letter)) continue;
							const seen = shown.get(iq);
							if (!seen || seen.uses.some((u) => u.mark === via)) continue;
							seen.uses.push({ itemId: item.id, mark: via, label: str(item.label) });
						}
					}
				}
			}
		}

		for (const cite of cites) {
			if (!cite || typeof cite !== 'object') continue;
			if (calculated && cite.source?.kind === 'computed') continue;
			const s = sourceFor(cite.source);
			if (!s) continue;
			const text = str(cite.quote) ? String(cite.quote).trim() : null;
			const summary = str(cite.summary) || null;
			const q = quoteFor(s, text, summary, item, {
				capturedAt: isoOrNull(cite.source?.capturedAt),
				acceptedAt: secs(cite.source?.acceptedAt)
			});
			attach(item, list, s, q);
		}

		for (const m of list) {
			if (m.src.kind !== 'decision') continue;
			const peers = decisionPeers.get(m.src) ?? new Set();
			for (const other of list) if (other.src !== m.src) peers.add(other.src);
			decisionPeers.set(m.src, peers);
		}

		const alternatives = Array.isArray(item.alternatives) ? item.alternatives : [];
		for (const alt of alternatives) visit(alt);
	};

	for (const item of Array.isArray(items) ? items : []) visit(item);

	// Number: the office's sources and staff notes first, calculations last.
	const numbered = [
		...sources.filter((s) => s.kind !== 'computed'),
		...sources.filter((s) => s.kind === 'computed')
	];
	numbered.forEach((s, i) => {
		s.n = i + 1;
	});

	/** @type {Set<string>} */
	const usedIds = new Set();
	for (const m of created) {
		const base = `ref-${m.src.n}${m.letter}`;
		let id = base;
		for (let k = 2; usedIds.has(id); k++) id = `${base}-${k}`;
		usedIds.add(id);
		m.refId = id;
	}

	/** @type {Record<string, MarkerRef[]>} */
	const refs = {};
	for (const [id, list] of marks) {
		refs[id] = list
			.map((m) => ({ n: m.src.n, letter: m.letter, refId: m.refId, letters: m.letters, key: m.src.key }))
			.sort((a, b) => a.n - b.n);
	}
	/** @param {string|null} id */
	const refsOf = (id) => (id ? (refs[id] ?? []) : []).map((r) => ({ n: r.n, letter: r.letter }));

	for (const s of numbered) {
		for (const q of s.quotes) {
			const seen = shown.get(q);
			q.refId = seen?.first?.refId ?? '';
			q.uses = (seen?.uses ?? []).map((u) => ({ itemId: u.itemId, refId: u.mark.refId, label: u.label }));
		}
	}

	for (const { q, inputs, terms } of calcs) {
		q.inputs = inputs.map((input) => ({
			label: str(input.label),
			display: str(input.display),
			refs: refsOf(input.id)
		}));
		q.terms = terms ? terms.map((t) => ({ text: t.text, refs: refsOf(t.itemId) })) : null;
	}

	for (const s of numbered) {
		if (s.kind !== 'decision') continue;
		const set = new Set([...(decisionPeers.get(s) ?? [])].map((p) => p.n));
		for (const k of supports.get(s) ?? []) {
			const peer = byKey.get(sourceKey({ key: k }) ?? k);
			if (peer && peer !== s) set.add(peer.n);
		}
		s.basedOn = [...set].sort((a, b) => a - b);
	}

	return { sources: numbered, refs };
}

/**
 * A calculated item's optional `terms` (see the header), cleaned.
 * @param {Item} item
 * @returns {{ text: string, itemId: string|null }[]|null}
 */
function termsOf(item) {
	const raw = /** @type {{ terms?: unknown }} */ (item).terms;
	if (!Array.isArray(raw)) return null;
	/** @type {{ text: string, itemId: string|null }[]} */
	const out = [];
	for (const t of raw) {
		const term = /** @type {{ text?: unknown, itemId?: unknown }|null} */ (t && typeof t === 'object' ? t : null);
		const text = str(term?.text);
		if (!text) continue;
		out.push({ text, itemId: typeof term?.itemId === 'string' && term.itemId ? term.itemId : null });
	}
	return out.length ? out : null;
}

/**
 * Numbered sources that are not calculations: the count the
 * "Citations shown: N sources numbered" announcement reads, so it
 * matches the header's pages and calls.
 * @param {NumberedSource[]|null|undefined} sources
 */
export function sourceCount(sources) {
	return (Array.isArray(sources) ? sources : []).filter((s) => s && s.kind !== 'computed').length;
}

/**
 * Every item on a presented page in citation order:
 * - the status band (active, then resolved notices) first, since it
 *   renders under the h1, above the glance; a status item that
 *   `page.order` lists keeps the order's place instead;
 * - then `page.order` (glance, then sections top to bottom);
 * - then any item both leave out, in page order (glance, sections with
 *   their extra rows, offices, closures, updates), so no cited value
 *   goes unnumbered.
 * Alternatives and inputs are reached through their parents.
 * @param {Partial<import('./doc/contract.js').Page>|null|undefined} page
 * @returns {Item[]}
 */
export function orderedItems(page) {
	if (!page || typeof page !== 'object') return [];
	/** @type {Map<string, Item>} */
	const byId = new Map();
	/** @param {unknown} list @returns {Item[]} */
	const add = (list) => {
		/** @type {Item[]} */
		const added = [];
		if (!Array.isArray(list)) return added;
		for (const it of list) {
			if (!it || typeof it.id !== 'string' || !it.id) continue;
			if (!byId.has(it.id)) byId.set(it.id, it);
			added.push(/** @type {Item} */ (byId.get(it.id)));
		}
		return added;
	};
	const status = [...add(page.status?.active), ...add(page.status?.resolved)];
	add(page.glance);
	for (const s of Array.isArray(page.sections) ? page.sections : []) {
		add(s?.items);
		add(s?.also);
	}
	for (const o of Array.isArray(page.offices) ? page.offices : []) add(o?.items);
	add(page.closures);
	add(page.updates);
	const order = Array.isArray(page.order) ? page.order.filter((id) => typeof id === 'string') : [];
	const listed = new Set(order);
	/** @type {Item[]} */
	const out = [];
	/** @type {Set<string>} */
	const seen = new Set();
	/** @param {Item|undefined} it */
	const push = (it) => {
		if (it && !seen.has(it.id)) {
			out.push(it);
			seen.add(it.id);
		}
	};
	for (const it of status) if (!listed.has(it.id)) push(it);
	for (const id of order) push(byId.get(id));
	for (const it of byId.values()) push(it);
	return out;
}

/**
 * `numberSources(orderedItems(page))`: the sources and marker refs for a
 * whole presented page.
 * @param {Partial<import('./doc/contract.js').Page>|null|undefined} page
 */
export function numberPage(page) {
	return numberSources(orderedItems(page));
}

// ---------- dates ----------

/**
 * @typedef {{ timeZone?: string, now?: number, year?: 'auto'|'always', lang?: string }} DateOpts
 * - timeZone: IANA zone; default the reader's own;
 * - lang: the page language (doc/copy): its words and its `Intl` locale;
 * - now: ms epoch for the "this year" test (default Date.now());
 * - year: 'always' prints the year (paper); 'auto' only when it is not
 *   the current year.
 */

/**
 * ISO string, unix seconds or Date → Date; null when unreadable.
 * @param {unknown} v
 * @returns {Date|null}
 */
export function toDate(v) {
	if (v instanceof Date) return Number.isFinite(v.getTime()) ? v : null;
	if (typeof v === 'number' && Number.isFinite(v) && v > 0) return new Date(v * 1000);
	if (typeof v === 'string' && v) {
		const t = Date.parse(v);
		return Number.isFinite(t) ? new Date(t) : null;
	}
	return null;
}

/** Intl separators (thin and narrow no-break spaces) as plain spaces. @param {string} s */
const plain = (s) => s.replace(/[   ]/g, ' ');

/**
 * @param {Date} d
 * @param {string|undefined} timeZone
 */
function yearIn(d, timeZone) {
	try {
		return new Intl.DateTimeFormat('en-US', { year: 'numeric', timeZone }).format(d);
	} catch {
		return String(d.getUTCFullYear());
	}
}

/**
 * @param {Date[]} dates
 * @param {DateOpts} opts
 */
function showYear(dates, opts) {
	if (opts.year === 'always') return true;
	const now = yearIn(new Date(opts.now ?? Date.now()), opts.timeZone);
	return dates.some((d) => yearIn(d, opts.timeZone) !== now);
}

/**
 * @param {boolean} year
 * @param {string|undefined} timeZone
 * @param {string} [lang]
 */
function dayFormat(year, timeZone, lang) {
	const intl = guideCopy(lang).intl;
	/** @type {Intl.DateTimeFormatOptions} */
	const o = { month: 'short', day: 'numeric', timeZone };
	if (year) o.year = 'numeric';
	try {
		return new Intl.DateTimeFormat(intl, o);
	} catch {
		return new Intl.DateTimeFormat(intl, { ...o, timeZone: undefined });
	}
}

/**
 * "Oct 31" (or "Oct 31, 2025" outside the current year); '' when the
 * value is unreadable.
 * @param {unknown} v
 * @param {DateOpts} [opts]
 */
export function formatDay(v, opts = {}) {
	const d = toDate(v);
	if (!d) return '';
	return plain(dayFormat(showYear([d], opts), opts.timeZone, opts.lang).format(d));
}

/**
 * "Oct 31 – Nov 4" or one day when both fall on the same day; either
 * end alone when the other is missing.
 * @param {unknown} a
 * @param {unknown} b
 * @param {DateOpts} [opts]
 */
export function formatRange(a, b, opts = {}) {
	const da = toDate(a);
	const db = toDate(b);
	if (!da || !db) return formatDay(da ?? db, opts);
	const [lo, hi] = da.getTime() <= db.getTime() ? [da, db] : [db, da];
	const f = dayFormat(showYear([lo, hi], opts), opts.timeZone, opts.lang);
	const left = plain(f.format(lo));
	const right = plain(f.format(hi));
	if (left === right) return left;
	try {
		return plain(f.formatRange(lo, hi)).replace(/\s*[–-]\s*/, ' – ');
	} catch {
		return `${left} – ${right}`;
	}
}

/**
 * Date and time with the zone, "Nov 4, 9:10 a.m. PT".
 * @param {unknown} v
 * @param {DateOpts} [opts]
 */
export function formatMoment(v, opts = {}) {
	const d = toDate(v);
	if (!d) return '';
	/** @type {Intl.DateTimeFormatOptions} */
	const o = {
		month: 'short',
		day: 'numeric',
		hour: 'numeric',
		minute: '2-digit',
		timeZone: opts.timeZone
	};
	if (showYear([d], opts)) o.year = 'numeric';
	const intl = guideCopy(opts.lang).intl;
	let text = '';
	try {
		text = new Intl.DateTimeFormat(intl, { ...o, timeZoneName: 'shortGeneric' }).format(d);
	} catch {
		try {
			text = new Intl.DateTimeFormat(intl, { ...o, timeZoneName: 'short' }).format(d);
		} catch {
			text = new Intl.DateTimeFormat(intl, { ...o, timeZone: undefined }).format(d);
		}
	}
	if (intl !== 'en-US') return plain(text);
	return plain(text)
		.replace(/\s?\bAM\b/, ' a.m.')
		.replace(/\s?\bPM\b/, ' p.m.');
}

/**
 * "Captured Oct 31 · approved Nov 3"; either half alone; '' for none.
 * Ranges show when the ends differ ("Captured Oct 21 – 22").
 * @param {{ capturedMin?: unknown, capturedAt?: unknown, acceptedMin?: unknown, acceptedAt?: unknown }} d
 * @param {DateOpts} [opts]
 */
export function datesLine(d, opts = {}) {
	const captured = formatRange(d.capturedMin ?? d.capturedAt, d.capturedAt, opts);
	const approved = formatRange(d.acceptedMin ?? d.acceptedAt, d.acceptedAt, opts);
	const t = guideCopy(opts.lang).cite;
	const parts = [];
	if (captured) parts.push(t.captured(captured));
	if (approved) parts.push(t.approved(approved, parts.length === 0));
	return parts.join(' · ');
}

/**
 * The pages behind a page's values: "8 official pages" (on the
 * listed official site), "1 other web page" (any other host), or "2
 * web pages" when no page is on the official site; '' for none.
 * @param {{ pages?: number, otherPages?: number }|null|undefined} trust
 * @param {string} [lang] the page language (doc/copy)
 */
export function pagesPhrase(trust, lang = 'en') {
	const t = guideCopy(lang).cite;
	const pages = Number(trust?.pages) || 0;
	const others = Number(trust?.otherPages) || 0;
	const parts = [];
	if (pages > 0) parts.push(t.officialPages(pages));
	if (others > 0) parts.push(pages > 0 ? t.otherPages(others) : t.webPages(others));
	return parts.join(', ');
}

/**
 * The header trust line, "Sources captured Oct 31 – Nov 4 · approved
 * by staff Nov 4 · 8 official pages, 1 phone call"; '' without data.
 * Pages count as official only on the listed official site (see
 * `pagesPhrase`).
 * @param {{ capturedMin?: string|null, capturedMax?: string|null, acceptedMax?: number|null, pages?: number, otherPages?: number, calls?: number, emails?: number }|null|undefined} trust
 * @param {DateOpts} [opts]
 */
export function trustLine(trust, opts = {}) {
	if (!trust) return '';
	const t = guideCopy(opts.lang).cite;
	const parts = [];
	const captured = formatRange(trust.capturedMin, trust.capturedMax, opts);
	if (captured) parts.push(t.sourcesCaptured(captured));
	const approved = formatDay(trust.acceptedMax, opts);
	if (approved) parts.push(t.approvedByStaff(approved));
	const counts = [];
	const calls = Number(trust.calls) || 0;
	const emails = Number(trust.emails) || 0;
	const pages = pagesPhrase(trust, opts.lang);
	if (pages) counts.push(pages);
	if (calls > 0) counts.push(t.calls(calls));
	if (emails > 0) counts.push(t.emails(emails));
	if (counts.length) parts.push(counts.join(', '));
	const line = parts.join(' · ');
	return line ? line.charAt(0).toUpperCase() + line.slice(1) : '';
}

// ---------- copy per evidence kind ----------

/**
 * What the card and the endnote say about one source, by kind:
 * - lead: the card's first line ("Official web page · sos.wa.gov",
 *   "Official PDF · Apostille request form · sos.wa.gov");
 * - head: the endnote's first sentence, without its URL;
 * - title: the card's title line, web pages only (a PDF or an upload
 *   carries its title in the lead); null when it would repeat the host;
 * - url: the canonical http(s) URL (no fragment) for web, PDF and
 *   uploads, printed in the endnote;
 * - href: the same URL when it is https, the only kind either one links;
 * - dates: "Captured … · approved …" for web, PDF and uploads;
 * - link: the card link label ("Open the page"), with `fragment` true
 *   when the link may carry a text fragment (web pages only);
 * - check: a closing line for staff notes ("Check it yourself on the
 *   same number.").
 * A PDF names its host, not "uploaded by staff": the public citation
 * kind `pdf` covers fetched PDFs and staff uploads alike, so only an
 * `upload` says staff uploaded it.
 * Template words only: every date, host and channel comes from data.
 * @param {NumberedSource} s
 * @param {DateOpts} [opts]
 * @returns {{ lead: string, head: string, title: string|null, url: string|null, href: string|null, dates: string, link: string|null, fragment: boolean, check: string|null }}
 */
export function describeSource(s, opts = {}) {
	const host = s.host ?? '';
	const channel = s.channel ?? '';
	const raw = s.title || null;
	// A title that only repeats the host adds nothing.
	const title = raw && !(host && bareHost(raw) === bareHost(host)) ? raw : null;
	/** @param {(string|null|undefined)[]} parts */
	const dot = (parts) => parts.filter(Boolean).join(' · ');
	/** "First. Rest." with the rest's first letter raised. @param {string} first @param {string} rest */
	const sentences = (first, rest) =>
		rest ? `${first}. ${rest.charAt(0).toUpperCase()}${rest.slice(1)}.` : `${first}.`;
	const url = canonicalUrl(s.url);
	const href = linkHref(url);
	// A published note keeps its day only (SourceRef.dayOnly): print the
	// day, read in UTC so no reader's zone moves it.
	/** @param {unknown} v */
	const day = (v) => formatDay(v, s.dayOnly ? { ...opts, timeZone: 'UTC' } : opts);
	const web = { url, href, dates: datesLine(s, opts), check: null };
	const none = { title: null, url: null, href: null, dates: '', link: null, fragment: false };
	const t = guideCopy(opts.lang).cite;
	// A note that contradicts the value it is about says so first.
	const against = s.stance === 'contradicts' ? t.contradicts : null;
	switch (s.kind) {
		case 'web':
			return {
				...web,
				lead: dot([t.webPage, host]),
				head: title ? `${title}${host ? ` — ${host}` : ''}.` : `${host || t.webPage}.`,
				title,
				link: href ? t.openPage : null,
				fragment: true
			};
		case 'pdf':
			return {
				...web,
				lead: dot([t.officialPdf, title, host]),
				head: t.pdfHead(title ?? '', host),
				title: null,
				link: href ? t.openPdf : null,
				fragment: false
			};
		case 'upload': {
			const pdf = isPdfUrl(url);
			const from = t.uploadedBy(host);
			return {
				...web,
				lead: dot([t.officialUpload(pdf), title, from]),
				head: t.uploadHead(title ?? '', pdf, from),
				title: null,
				link: href ? (pdf ? t.openPdf : t.openDocument) : null,
				fragment: false
			};
		}
		case 'call': {
			const notes = s.recorded === true ? t.staffNotes : t.notRecorded;
			const when = s.dayOnly ? day(s.occurredAt) : formatMoment(s.occurredAt, opts);
			return {
				...none,
				lead: dot([t.call, channel, when, notes, against]),
				head: sentences(`${t.call}${channel ? ` — ${channel}` : ''}`, dot([when, notes, against])),
				check: channel ? t.checkNumber : null
			};
		}
		case 'email': {
			const when = day(s.occurredAt);
			const from = channel ? t.emailFrom(channel) : t.email;
			return {
				...none,
				lead: dot([from, when, t.emailNote, against]),
				head: sentences(from, dot([when, t.emailNote, against])),
				check: null
			};
		}
		case 'visit': {
			const when = day(s.occurredAt);
			return {
				...none,
				lead: dot([t.visit, channel, when, t.staffNotes, against]),
				head: sentences(`${t.visit}${channel ? ` — ${channel}` : ''}`, dot([when, t.staffNotes, against])),
				check: null
			};
		}
		case 'absence': {
			// The evidence lane's `channel` names the pages staff checked
			// ("FAQ and fee pages"); without it the card names none.
			const when = s.occurredAt !== null ? day(s.occurredAt) : formatDay(s.acceptedAt, opts);
			const lead = channel ? t.notFoundOn(channel) : t.notFound;
			return {
				...none,
				lead: dot([lead, when, t.checkedByStaff]),
				head: sentences(lead, dot([when, t.checkedByStaff])),
				check: null
			};
		}
		case 'computed':
			return {
				...none,
				lead: t.calculated,
				head: `${t.calculated}.`,
				check: null
			};
		case 'decision':
		default: {
			const when = s.occurredAt !== null ? day(s.occurredAt) : formatDay(s.acceptedAt, opts);
			return {
				...none,
				lead: dot([t.staffNote, when]),
				head: sentences(t.staffNote, when),
				check: null
			};
		}
	}
}

// ---------- the on/off preference ----------

/** URL parameter: `?cite=1` shows citations, `?cite=0` hides them. */
export const CITE_PARAM = 'cite';
/** localStorage key holding the reader's last choice, '1' or '0'. */
export const CITE_STORAGE_KEY = 'hb_cite';

/**
 * Initial state: `?cite=1|0` wins, then the stored choice, then off.
 * @param {string|null|undefined} search location.search
 * @param {string|null|undefined} stored localStorage hb_cite
 */
export function citePreference(search, stored) {
	let param = null;
	try {
		param = new URLSearchParams(search ?? '').get(CITE_PARAM);
	} catch {
		param = null;
	}
	if (param === '1') return true;
	if (param === '0') return false;
	return stored === '1';
}

/**
 * The same address with `cite=1|0` set; path, other parameters and
 * hash kept. Returns path + search + hash (same origin by design).
 * @param {string} href
 * @param {boolean} on
 */
export function citeHref(href, on) {
	const u = new URL(href);
	u.searchParams.set(CITE_PARAM, on ? '1' : '0');
	return `${u.pathname}${u.search}${u.hash}`;
}

/**
 * The polite announcement after a toggle.
 * @param {boolean} on
 * @param {number} count numbered sources on the page
 * @param {string} [lang] the page language (doc/copy)
 */
export function citeAnnouncement(on, count, lang = 'en') {
	const t = guideCopy(lang).cite;
	if (!on) return t.hidden;
	return t.shown(Math.max(0, Math.floor(Number(count) || 0)));
}
