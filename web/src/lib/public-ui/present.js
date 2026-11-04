// Presenter (R2, plan §3 and §5.3): one authority's public facts become
// the customer guide Page (doc/contract.js) that doc/DossierGuide
// renders. Pure: no window, document or fetch, so node tests drive it.
//
// The rules, in the order the page reads:
// - Public facts: status absent or `accepted`, topic not `phone_script`,
//   a statement with text. Each one lands in at least one Item's
//   `factIds`, except a resolved notice older than ninety days.
// - Values display verbatim: whitespace and one trailing colon trimmed,
//   never shortened. Raw `key:` text stays in `records` ("How this page
//   was made").
// - Statements map to slots with slots.js `resolveSlot`. A keyed fact no
//   slot claims goes to its topic's section `also`, labelled with
//   guide.js `fieldLabel`; an unkeyed one shows its value alone.
// - A single-valued slot (and each instance of a many-valued one)
//   merges the facts that agree into one Item that keeps every cite.
//   Same key: compare.js `factsDisagree`, the server's field rule.
//   Other keys of one slot (aliases): two USPS values compare on box and
//   ZIP; money values agree when one value's amounts sit inside the
//   other's and their tier tables never contradict; anything else
//   compares as one key (`statementsDisagree`). A fact joins a group
//   only when it agrees with every member, and a vaguer value that
//   agrees with two disagreeing ones stands alone, so a dispute is never
//   hidden under it. Values that disagree make a `conflict` Item when a
//   token disagrees, a `worded` one when only the wording does, each
//   distinct value an alternative with its own cites, most sources
//   first. A fact the server flags `conflict: true` is always a
//   conflict; when its group holds one value, the public facts the
//   server names as disagreeing (`conflicts[]`, a return address naming
//   another PO box) are the other values. A conflict never shows one
//   value: with no other value on the page, the value shows alone,
//   without Copy.
// - A many-valued slot gives one Item per instance, or per distinct
//   value when the facts name no instance.
// - Evidence notes (/plan `evidence[]`) become extra cites keyed
//   `note:<id>`: a call, email or visit on the Item holding its fact; a
//   decision on the value it prefers (else its slot); an absence on its
//   slot, or an `absent` Item when nothing fills the slot. A note is
//   never a display value. A call, email or visit that contradicts its
//   value makes the Item `contested`: shown with the note as a visible
//   line, never with Copy, never in the calculator or as a gate for
//   paid offers; the glance points to its step instead.
// - Service notices pair by id (slots.js `pairNotices`). An unpaired
//   notice is active (the banner). A paired notice is a resolved status
//   line for seven days after its restore was captured, then an update
//   for ninety days, then hidden.
// - us_state profile: the glance (cost, time, pay, send, form), the
//   route cards, numbered steps, "Not confirmed yet" rows for missing
//   critical slots and the remote-notarization row. The glance and the
//   route cards apply the send step's D3 rules (no street by mail on a
//   US page, no PO box by courier). Without a profile, or with fewer
//   than two glance rows, the page is thin: sections stay as regions,
//   unnumbered.
// - Closures whose every date has passed (in the offices' confirmed
//   zone, else Pacific) move from the visit card to the updates.
// - The header names an official site only from the listed https
//   `official_url`; the trust line counts pages on that site as
//   official and the rest as other pages.
// - No public facts: the empty state, with the listed official site.
//
// The page's own words come from doc/copy in the page language (`lang`);
// that copy carries no digits, `$`, addresses or dates: every such token
// comes from a fact value or a computation over values.

import { SECTIONS, SLOTS, TOPIC_SECTION, pairNotices, profileFor, resolveSlot, slotById, splitKey } from './slots.js';
import { NOTICE_TOPIC, domain, fieldLabel, parseStatement, staleMark } from './guide.js';
import { USPS_TOPICS, factsDisagree, statementsDisagree, valueTokens } from './compare.js';
import { canonicalUrl, isPdfUrl, titleFromUrl } from './cite.js';
import { amountsIn, readTiers } from './fees.js';
import { flagFor } from './lanes.js';
import { OFFICE_SLOTS, SLOT, isUS, issuerName, partOf, remoteNotarizationRow } from './doc/items.js';
import { isPoBox, isStreetOnly, phoneIn, postalCity } from './doc/postal.js';
import { DEFAULT_ZONE, confirmedZone, instanceName, sameOffice, upcomingClosures } from './doc/offices.js';
import { guideCopy } from './doc/copy/index.js';

/** @typedef {import('./doc/contract.js').Item} Item */
/** @typedef {import('./doc/contract.js').Cite} Cite */
/** @typedef {import('./doc/contract.js').Section} Section */
/** @typedef {import('./doc/contract.js').Page} Page */
/** @typedef {import('./doc/contract.js').Header} Header */
/** @typedef {import('./slots.js').Slot} Slot */

/**
 * The authority row: `/plan` `authority` (or the index row when /plan
 * has none), plus the breadcrumbs the page resolved.
 * @typedef {{
 *   slug?: string|null,
 *   name?: string|null,
 *   region?: string|null,
 *   country?: string|null,
 *   kind?: string|null,
 *   path?: string|null,
 *   official_url?: string|null,
 *   iso2?: string|null,
 *   crumbs?: { label: string, href: string|null }[]|null
 * }} AuthorityRow
 */

/**
 * One published staff note from `/plan` `evidence[]`.
 * @typedef {{
 *   id: number,
 *   kind: string,
 *   stance?: string|null,
 *   fact_id?: number|null,
 *   preferred_fact_id?: number|null,
 *   slot?: string|null,
 *   date?: string|null,
 *   channel?: string|null,
 *   summary?: string|null,
 *   recorded?: boolean|null,
 *   supports?: number[]|null,
 *   status?: string|null
 * }} EvidenceNote
 */

/**
 * One public fact, parsed.
 * @typedef {{
 *   index: number,
 *   id: number|null,
 *   topic: string,
 *   statement: string,
 *   key: string|null,
 *   value: string,
 *   slot: Slot|null,
 *   instance: string|null,
 *   cites: Cite[],
 *   acceptedAt: number|null,
 *   flagged: boolean,
 *   fact: any
 * }} Row
 */

const DAY_MS = 86_400_000;
/** A resolved notice is a status line this many days after its restore. */
export const STATUS_DAYS = 7;
/** Then an update, until this many days after its restore; then hidden. */
export const UPDATE_DAYS = 90;
/** Copy is offered only on values this short (whole sentences never). */
export const COPY_MAX = 120;

/** Hops of the legalization chain: ordered rows of step one, kind `chain`. */
const CHAIN_TOPIC = 'legalization_chain';
/** One key for comparing values of two alias keys as one field. */
const ONE_KEY = 'value';
const NOTE_KINDS = new Set(['call', 'email', 'visit', 'decision', 'absence']);
const PAGE_KINDS = new Set(['web', 'pdf', 'upload']);

/**
 * "At a glance" rows: which slots fill them, the sub-label of each, and
 * where a pointer to a disputed value leads.
 */
export const GLANCE_ROWS = Object.freeze([
	{ id: 'cost', label: 'Cost', parts: [{ slot: SLOT.perDocument, label: null, anchor: 'fees' }] },
	{
		id: 'time',
		label: 'Time',
		parts: [
			{ slot: 'processing_time.standard', label: 'Standard', anchor: 'after' },
			{ slot: 'processing_time.expedited', label: 'Expedited', anchor: 'after' },
			{ slot: SLOT.sameDayTime, label: 'Same day', anchor: 'after' }
		]
	},
	{
		id: 'pay',
		label: 'Pay',
		parts: [
			{ slot: SLOT.payee, label: 'Payable to', anchor: 'fees' },
			{ slot: SLOT.methodsCounter, label: 'In person', anchor: 'fees' }
		]
	},
	{
		id: 'send',
		label: 'Send',
		parts: [
			{ slot: SLOT.usps, label: 'By mail', anchor: 'send-mail' },
			{ slot: SLOT.courier, label: 'By courier', anchor: 'send-courier' },
			{ slot: SLOT.office, label: 'In person', anchor: 'visit' }
		]
	},
	{ id: 'form', label: 'Form', parts: [{ slot: SLOT.form, label: null, anchor: 'form' }] }
]);

/** @param {unknown} v */
const str = (v) => (typeof v === 'string' ? v.trim() : '');
/** @param {unknown} v @returns {number|null} */
const secs = (v) => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : null);
/** @param {unknown} v @returns {string|null} */
const iso = (v) => (typeof v === 'string' && Number.isFinite(Date.parse(v)) ? v : null);
/** `fees.per_document` → `fees-per_document`. @param {string} id */
const dash = (id) => id.replace(/\./g, '-');

/**
 * A fact's value as the page prints it: the statement's value with
 * surrounding whitespace and one trailing colon trimmed.
 * @param {unknown} value
 */
export function cleanValue(value) {
	return String(value ?? '')
		.trim()
		.replace(/\s*:$/, '')
		.trim();
}

/**
 * Facts the page shows: accepted (no status counts as accepted, as the
 * J1 fixture sends them), not call scripts, with statement text.
 * @param {unknown} facts the `/facts` array
 * @returns {any[]}
 */
export function publicFacts(facts) {
	return (Array.isArray(facts) ? facts : []).filter(
		(f) =>
			!!f &&
			typeof f === 'object' &&
			(f.status === undefined || f.status === null || f.status === 'accepted') &&
			f.topic !== 'phone_script' &&
			typeof f.statement === 'string' &&
			cleanValue(parseStatement(f.statement).value) !== ''
	);
}

/**
 * A page title from its URL: the last path segment, or "Home page".
 * @param {string} url
 * @param {string} lang
 */
function pageTitle(url, lang) {
	const own = titleFromUrl(url);
	if (own) return own;
	try {
		return new URL(url).pathname === '/' ? guideCopy(lang).present.homePage : '';
	} catch {
		return '';
	}
}

/**
 * The cites of one fact, from its `citations` (web, PDF, upload).
 * @param {any} fact
 * @param {number|null} id
 * @param {number|null} acceptedAt
 * @param {string} lang
 * @returns {Cite[]}
 */
function citesOf(fact, id, acceptedAt, lang) {
	/** @type {Cite[]} */
	const out = [];
	for (const c of Array.isArray(fact?.citations) ? fact.citations : []) {
		if (!c || typeof c !== 'object') continue;
		const url = canonicalUrl(c.source_url);
		const capture = typeof c.capture_id === 'number' ? c.capture_id : null;
		const key = url ?? (capture !== null ? `capture:${capture}` : null);
		if (!key) continue;
		/** @type {import('./doc/contract.js').EvidenceKind} */
		const kind = PAGE_KINDS.has(c.kind) ? c.kind : isPdfUrl(url) ? 'pdf' : 'web';
		out.push({
			source: {
				key,
				kind,
				title: url ? pageTitle(url, lang) : '',
				url,
				host: url ? domain(url) : null,
				capturedAt: iso(c.captured_at),
				acceptedAt,
				channel: null,
				occurredAt: null,
				recorded: null
			},
			quote: str(c.quote) ? String(c.quote) : null,
			summary: null,
			factId: id
		});
	}
	return out;
}

/**
 * Two pages whose URLs end alike ("…/about-office/contact-us" and
 * "…/corporations-charities/contact-us") read "Contact us (about
 * office)" and "Contact us (corporations charities)", so a reader can
 * tell them apart wherever a title alone names them.
 * @param {Row[]} rows
 */
function distinctTitles(rows) {
	/** @type {Map<string, Set<string>>} */
	const urls = new Map();
	for (const r of rows) {
		for (const c of r.cites) {
			if (!c.source.url || !c.source.title) continue;
			const seen = urls.get(c.source.title) ?? new Set();
			seen.add(c.source.url);
			urls.set(c.source.title, seen);
		}
	}
	for (const r of rows) {
		for (const c of r.cites) {
			const url = c.source.url;
			if (!url || (urls.get(c.source.title)?.size ?? 0) < 2) continue;
			let parent = '';
			try {
				const u = new URL(url);
				u.pathname = u.pathname.replace(/\/[^/]*\/?$/, '') || '/';
				parent = titleFromUrl(u.href) ?? '';
			} catch {
				parent = '';
			}
			if (parent) c.source.title = `${c.source.title} (${parent.charAt(0).toLowerCase()}${parent.slice(1)})`;
		}
	}
}

/**
 * Parse every public fact once.
 * @param {unknown} facts
 * @param {string} lang
 * @returns {Row[]}
 */
function rowsOf(facts, lang) {
	/** @type {Row[]} */
	const out = [];
	(Array.isArray(facts) ? facts : []).forEach((f, index) => {
		if (publicFacts([f]).length === 0) return;
		const statement = String(f.statement);
		const { fieldKey, value } = parseStatement(statement);
		const id = typeof f.id === 'number' && Number.isFinite(f.id) ? f.id : null;
		const topic = String(f.topic ?? '');
		const hit = topic === NOTICE_TOPIC ? null : resolveSlot(topic, fieldKey);
		const acceptedAt = secs(f.accepted_at);
		out.push({
			index,
			id,
			topic,
			statement,
			key: fieldKey,
			value: cleanValue(value),
			slot: hit?.slot ?? null,
			instance: hit?.instance ?? null,
			cites: citesOf(f, id, acceptedAt, lang),
			acceptedAt,
			flagged: f.conflict === true,
			fact: f
		});
	});
	distinctTitles(out);
	return out;
}

/** @param {Row[]} rows @returns {number[]} */
const idsOf = (rows) => [...new Set(rows.map((r) => r.id).filter((id) => typeof id === 'number'))];

/**
 * Preferred fact first: the slot's canonical key, then the newest
 * accept, then the lowest id.
 * @param {Slot|null} slot
 * @returns {(a: Row, b: Row) => number}
 */
function prefer(slot) {
	/** @param {Row} r */
	const canon = (r) => (slot && r.key && splitKey(r.key).base === slot.keys[0] ? 1 : 0);
	return (a, b) =>
		canon(b) - canon(a) ||
		(b.acceptedAt ?? 0) - (a.acceptedAt ?? 0) ||
		(a.id ?? Number.MAX_SAFE_INTEGER) - (b.id ?? Number.MAX_SAFE_INTEGER) ||
		a.index - b.index;
}

/** @param {Row} a @param {Row} b */
const bothUsps = (a, b) => USPS_TOPICS.includes(a.topic) && USPS_TOPICS.includes(b.topic);

/**
 * True when two money values' tier tables give one count two amounts.
 * @param {string} a
 * @param {string} b
 */
function tiersClash(a, b) {
	const ta = readTiers(a);
	const tb = readTiers(b);
	if (ta.conflict || tb.conflict) return true;
	return ta.tiers.some((x) => tb.tiers.some((y) => x.from <= y.to && y.from <= x.to && x.cents !== y.cents));
}

/**
 * True when two facts of one slot (or one unmapped key) say the same
 * thing (see the header for the rule).
 * @param {Slot|null} slot
 * @param {Row} a
 * @param {Row} b
 */
function agree(slot, a, b) {
	if (a.topic === b.topic && a.key === b.key) return !factsDisagree(a.topic, a.statement, b.topic, b.statement);
	if (bothUsps(a, b) && factsDisagree(a.topic, a.statement, b.topic, b.statement)) return false;
	if (slot && (slot.kind === 'money' || slot.kind === 'money_tier')) {
		const ca = new Set(amountsIn(a.value).map((x) => x.cents));
		const cb = new Set(amountsIn(b.value).map((x) => x.cents));
		if (ca.size > 0 && cb.size > 0) {
			const within = [...ca].every((c) => cb.has(c)) || [...cb].every((c) => ca.has(c));
			return within && !tiersClash(a.value, b.value);
		}
	}
	return !statementsDisagree(`${ONE_KEY}: ${a.value}`, `${ONE_KEY}: ${b.value}`);
}

/** @param {Set<string>} tokens */
const kindsOf = (tokens) => new Set([...tokens].map((t) => t.slice(0, t.indexOf(':'))));

/**
 * True when two disagreeing values disagree on a token (an amount, a
 * box, a date, a time…), not just in their wording.
 * @param {Row} a
 * @param {Row} b
 */
function tokenClash(a, b) {
	if (bothUsps(a, b) && factsDisagree(a.topic, a.statement, b.topic, b.statement)) return true;
	const ka = kindsOf(valueTokens(a.value));
	const kb = kindsOf(valueTokens(b.value));
	return [...ka].some((k) => kb.has(k));
}

/**
 * Facts grouped by agreement, preferred fact first in each group. A
 * fact joins a group only when it agrees with every member, so two
 * values that disagree never merge under a third. A vaguer value that
 * agrees with two values which disagree with each other ("PO Box 40220,
 * Olympia, WA" beside two ZIP+4 codes) stands as its own group: merging
 * it would hide one side of the dispute.
 * @param {Slot|null} slot
 * @param {Row[]} rows
 * @returns {Row[][]}
 */
function clusters(slot, rows) {
	const sorted = [...rows].sort(prefer(slot));
	const n = sorted.length;
	/** @type {boolean[][]} */
	const ok = sorted.map(() => new Array(n).fill(true));
	for (let i = 0; i < n; i++) {
		for (let j = i + 1; j < n; j++) {
			const same = agree(slot, sorted[i], sorted[j]);
			ok[i][j] = same;
			ok[j][i] = same;
		}
	}
	const vague = sorted.map((_, i) => {
		for (let j = 0; j < n; j++) {
			if (j === i || !ok[i][j]) continue;
			for (let k = j + 1; k < n; k++) if (k !== i && ok[i][k] && !ok[j][k]) return true;
		}
		return false;
	});
	/** @type {number[][]} */
	const out = [];
	for (let i = 0; i < n; i++) {
		const home = out.find((c) => vague[c[0]] === vague[i] && c.every((j) => ok[i][j]));
		if (home) home.push(i);
		else out.push([i]);
	}
	return out.map((c) => c.map((i) => sorted[i]));
}

/**
 * True when two groups disagree on a token (an amount, a box, a date…)
 * somewhere between their members, not just in their wording.
 * @param {Slot|null} slot
 * @param {Row[]} a
 * @param {Row[]} b
 */
function groupsClash(slot, a, b) {
	return a.some((x) => b.some((y) => !agree(slot, x, y) && tokenClash(x, y)));
}

/**
 * A slot's label in the page language.
 * @param {Slot} slot
 * @param {string} lang
 */
function slotLabel(slot, lang) {
	const own = /** @type {Record<string, string>} */ (guideCopy(lang).slots);
	return own[slot.id] ?? slot.label;
}

/**
 * The label a fact's own field reads under (its slot's, or its key's).
 * @param {Row} r
 * @param {string} lang
 */
function rowLabel(r, lang) {
	if (r.slot) return slotLabel(r.slot, lang);
	return r.key ? fieldLabel(r.topic, splitKey(r.key).base, lang) : '';
}

/**
 * Public facts the server names as disagreeing with a flagged fact
 * (`conflicts[]` on `/facts`) that the group does not already hold: a
 * value of another field (a return address naming another PO box under
 * the cross-key USPS rule), or one the client merged.
 * @param {Row[]} rows
 * @param {Map<number, Row>} pool every public fact by id
 * @returns {Row[]}
 */
function siblingsOf(rows, pool) {
	const own = new Set(rows.map((r) => r.id));
	/** @type {Row[]} */
	const out = [];
	for (const r of rows) {
		if (!r.flagged) continue;
		for (const c of Array.isArray(r.fact?.conflicts) ? r.fact.conflicts : []) {
			const id = typeof c?.id === 'number' ? c.id : null;
			if (id === null || own.has(id) || (c.status && c.status !== 'accepted')) continue;
			const sib = pool.get(id);
			if (sib && sib.topic !== NOTICE_TOPIC && !out.includes(sib)) out.push(sib);
		}
	}
	return out;
}

/**
 * Freshness of a value from its facts' captures (guide.js staleMark, at
 * ninety and three hundred sixty-five days): the freshest cited fact
 * decides; uncited facts never make a value look fresh.
 * @param {Row[]} rows
 * @param {number} now
 * @param {string} lang
 * @returns {Item['stale']}
 */
function staleOf(rows, now, lang) {
	const marks = rows.filter((r) => r.cites.length > 0).map((r) => staleMark(r.fact, now));
	if (marks.length === 0 || marks.some((m) => m === null)) return null;
	const fresh = /** @type {{ months: number, warning: boolean }[]} */ (marks).reduce((a, b) =>
		a.months <= b.months ? a : b
	);
	const t = guideCopy(lang).present;
	return fresh.warning
		? { level: 'warning', text: t.staleWarning }
		: { level: 'muted', text: t.staleMonths(fresh.months) };
}

/**
 * @typedef {{
 *   id: string,
 *   slot: Slot|null,
 *   label: string,
 *   kind: string,
 *   route: Item['route'],
 *   instance: string|null,
 *   copyable: boolean,
 *   now: number,
 *   lang: string
 * }} ItemSpec
 */

/**
 * One value from one group of agreeing facts.
 * @param {ItemSpec} spec
 * @param {Row[]} rows preferred fact first
 * @returns {Item}
 */
function valueItem(spec, rows) {
	const rep = rows[0];
	const cites = rows.flatMap((r) => r.cites);
	const stale = staleOf(rows, spec.now, spec.lang);
	/** @type {Item['state']} */
	const state = cites.length === 0 ? 'uncited' : stale ? 'stale' : 'ok';
	return {
		id: spec.id,
		slot: spec.slot?.id ?? null,
		label: spec.label,
		display: rep.value,
		kind: spec.kind,
		factIds: idsOf(rows),
		cites,
		state,
		stale,
		alternatives: null,
		inputs: null,
		copy: spec.copyable && (state === 'ok' || state === 'stale') && rep.value.length <= COPY_MAX,
		route: spec.route,
		instance: spec.instance
	};
}

/** Distinct sources behind an item. @param {Item} item */
const sourceCount = (item) => new Set(item.cites.map((c) => c.source.key)).size;

/**
 * The Item for one slot instance (or one unmapped key): merged when
 * its facts agree, disputed otherwise. A fact the server flags whose
 * group holds one value lists the public facts the server names as
 * disagreeing as the other values; when none is on the page, the value
 * shows alone, without Copy (a conflict never shows one value).
 * @param {ItemSpec} spec
 * @param {Row[]} rows
 * @param {Map<number, Row>} [pool] every public fact by id
 * @returns {Item}
 */
function groupItem(spec, rows, pool = new Map()) {
	const groups = clusters(spec.slot, rows);
	const flagged = rows.some((r) => r.flagged);
	const siblings = groups.length === 1 && flagged ? clusters(null, siblingsOf(rows, pool)) : [];
	if (groups.length === 1 && siblings.length === 0) {
		return valueItem(flagged ? { ...spec, copyable: false } : spec, groups[0]);
	}
	const alternatives = [
		...groups
			.map((g, order) => ({ order, item: valueItem({ ...spec, copyable: false }, g) }))
			.sort((a, b) => sourceCount(b.item) - sourceCount(a.item) || a.order - b.order)
			.map(({ item }) => item),
		...siblings.map((g) => valueItem({ ...spec, label: rowLabel(g[0], spec.lang), copyable: false }, g))
	].map((item, k) => ({ ...item, id: `${spec.id}-v${k + 1}` }));
	let clash = flagged;
	for (let i = 0; i < groups.length && !clash; i++) {
		for (let j = i + 1; j < groups.length && !clash; j++) clash = groupsClash(spec.slot, groups[i], groups[j]);
	}
	return {
		id: spec.id,
		slot: spec.slot?.id ?? null,
		label: spec.label,
		display: alternatives[0].display,
		kind: spec.kind,
		factIds: idsOf(rows),
		cites: [],
		state: clash ? 'conflict' : 'worded',
		stale: null,
		alternatives,
		inputs: null,
		copy: false,
		route: spec.route,
		instance: spec.instance
	};
}

/** @param {Item} item */
const disputed = (item) => (item.state === 'conflict' || item.state === 'worded') && (item.alternatives?.length ?? 0) > 0;

/** @param {Item} item */
const real = (item) => item.factIds.length > 0 && item.state !== 'missing' && item.state !== 'absent';

/**
 * The cite of one published staff note.
 * @param {EvidenceNote} note
 * @returns {Cite}
 */
function noteCite(note) {
	const day = /^(\d{4})-(\d{2})-(\d{2})$/.exec(str(note.date));
	const supports = (Array.isArray(note.supports) ? note.supports : [])
		.filter((id) => typeof id === 'number')
		.map((id) => `note:${id}`);
	return {
		source: {
			key: `note:${note.id}`,
			kind: /** @type {import('./doc/contract.js').EvidenceKind} */ (note.kind),
			title: '',
			url: null,
			host: null,
			capturedAt: null,
			acceptedAt: null,
			channel: str(note.channel) || null,
			occurredAt: day ? Date.UTC(Number(day[1]), Number(day[2]) - 1, Number(day[3]), 12) / 1000 : null,
			recorded: note.kind === 'call' ? note.recorded === true : null,
			supports,
			dayOnly: !!day,
			stance: note.stance === 'confirms' || note.stance === 'contradicts' || note.stance === 'context' ? note.stance : null
		},
		quote: null,
		summary: str(note.summary) || null,
		factId: typeof note.fact_id === 'number' ? note.fact_id : null
	};
}

/**
 * Published notes of the evidence lane.
 * @param {unknown} evidence
 * @returns {EvidenceNote[]}
 */
function notesOf(evidence) {
	return (Array.isArray(evidence) ? evidence : []).filter(
		(n) =>
			!!n &&
			typeof n === 'object' &&
			typeof n.id === 'number' &&
			NOTE_KINDS.has(n.kind) &&
			(!n.status || n.status === 'published') &&
			!!str(n.summary)
	);
}

/**
 * The slot (and instance) a note names: `fees.same_day`,
 * `address.office__tumwater`, or a topic and key the registry knows
 * (`mailing.po_box` is the USPS address).
 * @param {unknown} raw
 * @returns {{ slot: Slot, instance: string|null }|null}
 */
function noteSlot(raw) {
	const text = str(raw);
	if (!text) return null;
	const at = text.indexOf('__');
	const id = at > 0 ? text.slice(0, at) : text;
	const instance = at > 0 ? text.slice(at + 2) || null : null;
	const own = slotById(id);
	if (own) return { slot: own, instance };
	const [topic, key] = id.split('.');
	const hit = resolveSlot(topic, key) ?? resolveSlot(key, key);
	return hit ? { slot: hit.slot, instance: instance ?? hit.instance } : null;
}

/** Notes that report what the office itself said. */
const ANSWER_KINDS = new Set(['call', 'email', 'visit']);

/**
 * Attach published notes to the Items they speak to (never as values).
 * A call, email or visit note whose stance is `contradicts` contests
 * the value it is about: the Item turns `contested` (no Copy, no
 * calculator, no paid offers) and lists the note in `contest`, so the
 * page shows it as a visible staff-note line.
 * Returns notes that found no Item (absences then make one).
 * @param {EvidenceNote[]} notes
 * @param {Item[]} items every top-level Item (status and sections)
 * @returns {{ note: EvidenceNote, slot: Slot, instance: string|null }[]}
 */
function attachNotes(notes, items) {
	/** @type {Map<number, Item[]>} */
	const holders = new Map();
	/** @param {Item} item @param {number} id */
	const hold = (item, id) => {
		const list = holders.get(id) ?? [];
		if (!list.includes(item)) list.push(item);
		holders.set(id, list);
	};
	for (const item of items) {
		// A "Not confirmed yet" row carries the phone's facts for its
		// number; a note about the phone is not about that row.
		if (item.state === 'missing') continue;
		if (disputed(item)) {
			for (const alt of item.alternatives ?? []) for (const id of alt.factIds) hold(alt, id);
		} else {
			for (const id of item.factIds) hold(item, id);
		}
	}
	/** @type {{ note: EvidenceNote, slot: Slot, instance: string|null }[]} */
	const unplaced = [];
	for (const note of notes) {
		const cite = noteCite(note);
		/** @type {Item[]} */
		let targets = [];
		if (note.kind === 'call' || note.kind === 'email' || note.kind === 'visit') {
			targets = typeof note.fact_id === 'number' ? (holders.get(note.fact_id) ?? []) : [];
		} else {
			if (note.kind === 'decision' && typeof note.preferred_fact_id === 'number') {
				targets = holders.get(note.preferred_fact_id) ?? [];
			}
			const named = noteSlot(note.slot);
			if (targets.length === 0 && named) {
				targets = items.filter(
					(i) => i.slot === named.slot.id && (!named.instance || i.instance === named.instance) && i.state !== 'missing'
				);
				if (targets.length === 0) unplaced.push({ note, ...named });
			}
		}
		const contests = ANSWER_KINDS.has(note.kind) && note.stance === 'contradicts';
		for (const t of targets) {
			t.cites = [...t.cites, cite];
			if (!contests || !(t.state === 'ok' || t.state === 'stale' || t.state === 'uncited')) continue;
			t.state = 'contested';
			t.copy = false;
			t.contest = [
				...(t.contest ?? []),
				{ id: note.id, kind: note.kind, date: str(note.date) || null, summary: str(note.summary) }
			];
		}
	}
	return unplaced;
}

/**
 * The listed official site (`authority.official_url`, https only): its
 * host and canonical address. The page never names another host as the
 * office's site: a page it cites (the Hague Conference, say) may not
 * be the office's own.
 * @param {string|null} listed
 * @returns {{ host: string|null, url: string|null }}
 */
function officialSite(listed) {
	if (!listed || !listed.startsWith('https://')) return { host: null, url: null };
	return { host: domain(listed), url: canonicalUrl(listed) };
}

/**
 * True when a URL sits on the official site's host (or under it).
 * @param {string} url
 * @param {string|null} host
 */
function onSite(url, host) {
	if (!host) return false;
	const own = domain(url);
	return own === host || own.endsWith(`.${host}`);
}

/**
 * Header trust numbers: capture span, newest accept, distinct https
 * pages on the listed official site and on other hosts, published
 * calls and emails.
 * @param {Row[]} rows
 * @param {EvidenceNote[]} notes
 * @param {string|null} officialHost the listed official site's host
 * @returns {Header['trust']}
 */
function trustOf(rows, notes, officialHost) {
	/** @type {string|null} */
	let capturedMin = null;
	/** @type {string|null} */
	let capturedMax = null;
	/** @type {number|null} */
	let acceptedMax = null;
	/** @type {Set<string>} */
	const pages = new Set();
	/** @type {Set<string>} */
	const others = new Set();
	for (const r of rows) {
		if (r.acceptedAt !== null && (acceptedMax === null || r.acceptedAt > acceptedMax)) acceptedMax = r.acceptedAt;
		for (const c of r.cites) {
			const at = c.source.capturedAt;
			if (at && (capturedMin === null || Date.parse(at) < Date.parse(capturedMin))) capturedMin = at;
			if (at && (capturedMax === null || Date.parse(at) > Date.parse(capturedMax))) capturedMax = at;
			const url = c.source.url;
			if (!url?.startsWith('https://')) continue;
			if (onSite(url, officialHost)) pages.add(url);
			else others.add(url);
		}
	}
	return {
		capturedMin,
		capturedMax,
		acceptedMax,
		pages: pages.size,
		otherPages: others.size,
		calls: notes.filter((n) => n.kind === 'call').length,
		emails: notes.filter((n) => n.kind === 'email').length
	};
}

/**
 * The newest capture of a fact's citations (ms), or its accept.
 * @param {Row} row
 * @returns {number|null}
 */
function readAt(row) {
	const times = row.cites.map((c) => Date.parse(c.source.capturedAt ?? '')).filter((t) => Number.isFinite(t));
	if (times.length > 0) return Math.max(...times);
	return row.acceptedAt !== null ? row.acceptedAt * 1000 : null;
}

/**
 * Service status: active banners, resolved lines, updates and the
 * notice log's last read.
 * @param {Row[]} noticeRows
 * @param {Row[]} rows every public fact
 * @param {number} now
 * @param {(base: string) => string} uid
 * @param {string} lang
 */
function statusOf(noticeRows, rows, now, uid, lang) {
	const sorted = [...noticeRows].sort(
		(a, b) => (a.id ?? Number.MAX_SAFE_INTEGER) - (b.id ?? Number.MAX_SAFE_INTEGER) || a.index - b.index
	);
	/** @type {Item[]} */
	const active = [];
	/** @type {Item[]} */
	const resolved = [];
	/** @type {Item[]} */
	const updates = [];
	/** @type {string|null} */
	let logUrl = null;
	/** @type {number} */
	let logAt = -Infinity;
	/**
	 * @param {Row} row
	 * @param {'notice'|'restored'} which
	 * @param {string|null} pair
	 * @returns {Item}
	 */
	const itemOf = (row, which, pair) => {
		const slot = slotById(`service_notice.${which}`);
		const tag = pair ?? `f${row.id ?? row.index}`;
		return {
			id: uid(`status-${which}-${tag}`),
			slot: slot?.id ?? null,
			label: slot ? slotLabel(slot, lang) : '',
			display: row.value,
			kind: slot?.kind ?? 'notice',
			factIds: idsOf([row]),
			cites: row.cites,
			state: row.cites.length === 0 ? 'uncited' : 'ok',
			stale: null,
			alternatives: null,
			inputs: null,
			copy: false,
			route: null,
			instance: pair
		};
	};
	for (const p of pairNotices(sorted)) {
		if (p.notice && !p.restored) {
			active.push(itemOf(p.notice, 'notice', p.id));
			continue;
		}
		const restored = /** @type {Row} */ (p.restored);
		const at = readAt(restored);
		const age = at === null ? 0 : now - at;
		if (p.notice) {
			const url = restored.cites.find((c) => c.source.url)?.source.url ?? p.notice.cites.find((c) => c.source.url)?.source.url ?? null;
			if (url && (at ?? 0) >= logAt) {
				logUrl = url;
				logAt = at ?? 0;
			}
		}
		const list = age < STATUS_DAYS * DAY_MS ? resolved : age < UPDATE_DAYS * DAY_MS ? updates : null;
		if (!list) continue;
		if (p.notice) list.push(itemOf(p.notice, 'notice', p.id));
		list.push(itemOf(restored, 'restored', p.id));
	}
	/** @type {null|{ capturedAt: string, url: string }} */
	let lastRead = null;
	if (logUrl) {
		/** @type {string|null} */
		let newest = null;
		for (const r of rows) {
			for (const c of r.cites) {
				const at = c.source.capturedAt;
				if (c.source.url === logUrl && at && (newest === null || Date.parse(at) > Date.parse(newest))) newest = at;
			}
		}
		if (newest) lastRead = { capturedAt: newest, url: logUrl };
	}
	return { status: { active, resolved, lastRead }, updates };
}

/**
 * Items of the send section in the order the send block shows them:
 * the mailing address and other mail rows, the courier rows, each
 * office's address and rules, rules for every office, closures (soonest
 * first), other counter rows, then the rest.
 * @param {Item[]} items
 * @param {number} now
 * @param {string} zone the zone closures are dated in
 */
function sendOrder(items, now, zone) {
	/** @type {Item[]} */
	const out = [];
	/** @param {(i: Item) => boolean} test */
	const take = (test) => {
		for (const i of items) if (!out.includes(i) && test(i)) out.push(i);
	};
	take((i) => i.slot === SLOT.usps);
	take((i) => i.route === 'mail');
	take((i) => i.slot === SLOT.courier);
	take((i) => i.slot === SLOT.courierRule);
	take((i) => i.route === 'courier');
	const offices = [...new Set(items.filter((i) => OFFICE_SLOTS.includes(i.slot ?? '') && i.instance).map((i) => i.instance))];
	for (const inst of offices) {
		for (const slot of OFFICE_SLOTS) take((i) => i.slot === slot && i.instance === inst);
	}
	for (const slot of OFFICE_SLOTS) take((i) => i.slot === slot);
	const closures = items.filter((i) => i.slot === SLOT.closure);
	const soon = upcomingClosures(closures, new Date(now), zone);
	for (const c of [...soon.next.map((x) => x.item), ...soon.later.map((x) => x.item), ...soon.undated]) take((i) => i === c);
	take((i) => i.slot === SLOT.closure);
	take((i) => i.route === 'counter');
	take(() => true);
	return out;
}

/**
 * Items of the form section in the order the form block shows them.
 * @param {Item[]} items
 */
function formOrder(items) {
	return [
		...items.filter((i) => i.slot === SLOT.form),
		...items.filter((i) => i.slot === SLOT.enclose),
		...items.filter((i) => i.slot !== SLOT.form && i.slot !== SLOT.enclose)
	];
}

/**
 * A copy of an Item for the glance, under its own id. It keeps the
 * value's own label (the endnotes name a quote by it); the glance
 * prints its row's sub-label (GLANCE_ROWS) instead.
 * @param {Item} item
 * @param {(base: string) => string} uid
 * @returns {Item}
 */
function glanceCopy(item, uid) {
	return {
		...item,
		id: uid(`glance-${item.id}`),
		copy: item.copy && item.slot === SLOT.payee,
		alternatives: item.alternatives ? item.alternatives.map((a) => ({ ...a, id: uid(`glance-${a.id}`), copy: false })) : null
	};
}

/**
 * Build the page.
 * @param {{
 *   authority?: AuthorityRow|null,
 *   facts?: unknown,
 *   plan?: unknown,
 *   offices?: unknown,
 *   forms?: unknown,
 *   evidence?: unknown,
 *   options?: unknown,
 *   now?: number|Date,
 *   lang?: string
 * }} input
 * - authority: `/plan` `authority` or the index row, with `crumbs`;
 * - facts: the `/facts` array; plan, forms, options: the `/plan` plan,
 *   its forms and the `/p2/orders/options` body (the blocks read them;
 *   the page shape does not depend on them);
 * - offices: `/plan` `offices[]` (accepted office records), which name
 *   office groups; evidence: `/plan` `evidence[]`;
 * - now: ms epoch or Date (default the clock);
 * - lang: the page language (doc/copy; default English): the page's own
 *   words (headings, labels, honest rows, pointers). Values stay as the
 *   office's pages write them.
 * @returns {Page}
 */
export function present({ authority = null, facts = [], offices = [], evidence = [], now = Date.now(), lang = 'en' } = {}) {
	const t = guideCopy(lang);
	const nowMs = now instanceof Date ? now.getTime() : typeof now === 'number' && Number.isFinite(now) ? now : Date.now();
	/** @type {AuthorityRow} */
	const row = authority && typeof authority === 'object' ? authority : {};
	const slug = str(row.slug);
	const name = str(row.name) || slug;
	const region = str(row.region);
	const listed = canonicalUrl(row.official_url);
	const profile = profileFor({ country: row.country ?? '', kind: row.kind ?? '', slug });
	const us = profile?.id === 'us_state';
	// The US postal rules (D3: the mail branch shows no street) hold on a
	// US page, profiled or not; SendBlock reads the same flag.
	const usPostal = us || isUS(row.country);
	const rows = rowsOf(facts, lang);
	const notes = notesOf(evidence);
	/** @type {Map<number, Row>} */
	const pool = new Map(rows.filter((r) => r.id !== null).map((r) => [/** @type {number} */ (r.id), r]));
	/** @type {any[]} */
	const records = Array.isArray(offices) ? offices.filter((o) => o && typeof o === 'object') : [];
	// Closures are dated in an office's confirmed zone (OfficeCards reads
	// the same records), else Pacific.
	const zone = records.find((o) => confirmedZone(o))?.timezone ?? DEFAULT_ZONE;

	/** @type {Set<string>} */
	const taken = new Set();
	/** @param {string} base */
	const uid = (base) => {
		let id = base;
		for (let k = 2; taken.has(id); k++) id = `${base}-${k}`;
		taken.add(id);
		return id;
	};

	// The task heading names the region on a profiled page with values;
	// an empty page keeps the office's own name.
	const h1 = us && region && rows.length > 0 ? t.present.h1(region) : name;
	const issuer = issuerName(name, row.country, lang);
	const site = officialSite(listed);
	/** @type {Header} */
	const header = {
		eyebrow: t.present.eyebrow,
		h1,
		lede: name && h1 !== name ? t.present.lede(issuer) : null,
		crumbs: (Array.isArray(row.crumbs) ? row.crumbs : []).filter((c) => c && typeof c.label === 'string' && c.label),
		flag: flagFor(row.country, row.iso2),
		officialHost: rows.length > 0 ? site.host : null,
		officialUrl: rows.length > 0 ? site.url : null,
		trust: trustOf(rows, notes, site.host)
	};

	/** @type {Page} */
	const page = {
		empty: null,
		thin: true,
		profile: profile?.id ?? null,
		header,
		status: { active: [], resolved: [], lastRead: null },
		glance: [],
		routes: [],
		sections: [],
		offices: [],
		closures: [],
		updates: [],
		records: rows
			.map((r) => ({ id: r.id, topic: r.topic, statement: r.statement, acceptedAt: r.acceptedAt }))
			.sort((a, b) => (a.id ?? Number.MAX_SAFE_INTEGER) - (b.id ?? Number.MAX_SAFE_INTEGER)),
		order: []
	};

	if (rows.length === 0) {
		page.empty = {
			name,
			officialHost: listed && listed.startsWith('https://') ? domain(listed) : null,
			officialUrl: listed && listed.startsWith('https://') ? listed : null
		};
		return page;
	}

	// Service status.
	const notices = rows.filter((r) => r.topic === NOTICE_TOPIC);
	const { status, updates } = statusOf(notices, rows, nowMs, uid, lang);
	page.status = status;
	page.updates = updates;

	// Values, grouped: a slot instance, an unmapped key, or one unkeyed fact.
	/** @type {Map<string, { slot: Slot|null, topic: string, key: string|null, instance: string|null, many: boolean, rows: Row[] }>} */
	const groups = new Map();
	for (const r of rows) {
		if (r.topic === NOTICE_TOPIC) continue;
		let at;
		if (r.topic === CHAIN_TOPIC) at = `chain:${r.index}`;
		else if (r.slot) at = r.slot.many && !r.instance ? `many:${r.slot.id}` : `slot:${r.slot.id}:${r.instance ?? ''}`;
		else if (r.key) at = `also:${r.topic}:${r.key}`;
		else at = `row:${r.index}`;
		const g = groups.get(at) ?? {
			slot: r.slot,
			topic: r.topic,
			key: r.slot ? null : r.key,
			instance: r.instance,
			many: !!r.slot?.many && !r.instance,
			rows: []
		};
		g.rows.push(r);
		groups.set(at, g);
	}

	/** @type {Map<string, { items: { item: Item, rank: number, first: number }[], also: { item: Item, first: number }[] }>} */
	const bySection = new Map();
	/** @param {string} id */
	const sectionOf = (id) => {
		const s = bySection.get(id) ?? { items: [], also: [] };
		bySection.set(id, s);
		return s;
	};
	const rankOf = new Map(SLOTS.map((s, i) => [s.id, i]));
	for (const g of groups.values()) {
		const first = Math.min(...g.rows.map((r) => r.index));
		if (g.topic === CHAIN_TOPIC) {
			// A hop of the legalization chain: its own row in step one, in
			// acceptance order, after the registry's rows.
			const r = g.rows[0];
			const keyed = r.key ? splitKey(r.key) : null;
			const item = groupItem(
				{
					id: uid(`chain-${r.id !== null ? `f${r.id}` : `n${r.index}`}`),
					slot: null,
					label: keyed ? fieldLabel(g.topic, keyed.base, lang) : '',
					kind: 'chain',
					route: null,
					instance: null,
					copyable: false,
					now: nowMs,
					lang
				},
				g.rows,
				pool
			);
			sectionOf('before').items.push({ item, rank: SLOTS.length, first: r.id ?? Number.MAX_SAFE_INTEGER - r.index });
			continue;
		}
		if (g.slot) {
			const slot = g.slot;
			const base = {
				slot,
				label: slotLabel(slot, lang),
				kind: slot.kind,
				route: slot.route,
				copyable: slot.copy && (slot.kind === 'payee' || slot.kind === 'postal'),
				now: nowMs,
				lang
			};
			/** @type {Item[]} */
			const made = g.many
				? clusters(slot, g.rows).map((c) => {
						const r = c[0];
						return valueItem({ ...base, id: uid(`${dash(slot.id)}-${r.id !== null ? `f${r.id}` : `n${r.index}`}`), instance: null }, c);
					})
				: [groupItem({ ...base, id: uid(g.instance ? `${dash(slot.id)}-${g.instance}` : dash(slot.id)), instance: g.instance }, g.rows, pool)];
			for (const item of made) sectionOf(slot.section).items.push({ item, rank: rankOf.get(slot.id) ?? 0, first });
		} else {
			const section = TOPIC_SECTION[g.topic] && !SECTIONS.find((s) => s.id === TOPIC_SECTION[g.topic])?.pseudo ? TOPIC_SECTION[g.topic] : 'before';
			const keyed = g.key ? splitKey(g.key) : null;
			const label = keyed ? `${fieldLabel(g.topic, keyed.base, lang)}${keyed.instance ? ` (${instanceName(keyed.instance)})` : ''}` : '';
			const id = uid(g.key ? `also-${g.topic}-${g.key}` : `row-${g.rows[0].id ?? `n${g.rows[0].index}`}`);
			const item = groupItem({ id, slot: null, label, kind: 'text', route: null, instance: keyed?.instance ?? null, copyable: false, now: nowMs, lang }, g.rows, pool);
			sectionOf(section).also.push({ item, first });
		}
	}

	// Sections in registry order; items in slot order, then as the
	// blocks show them.
	/** @type {Section[]} */
	const sections = [];
	for (const s of SECTIONS) {
		if (s.pseudo || !s.heading) continue;
		const own = bySection.get(s.id) ?? { items: [], also: [] };
		let items = own.items.sort((a, b) => a.rank - b.rank || a.first - b.first).map((x) => x.item);
		if (s.id === 'send') items = sendOrder(items, nowMs, zone);
		if (s.id === 'form') items = formOrder(items);
		const also = own.also.sort((a, b) => a.first - b.first).map((x) => x.item);
		const heading = /** @type {Record<string, string>} */ (t.sections)[s.id] ?? s.heading;
		sections.push({ id: s.id, step: s.step, heading, items, also });
	}

	const all = () => [...page.status.active, ...page.status.resolved, ...sections.flatMap((s) => [...s.items, ...s.also]), ...page.updates];

	// us_state: honest rows for critical slots nothing fills, and the
	// remote-notarization row.
	if (us && profile) {
		const phone = sections.flatMap((s) => s.items).find((i) => i.slot === SLOT.phone && real(i) && !disputed(i)) ?? null;
		const number = phone ? (phoneIn(phone.display) ?? phone.display) : null;
		for (const id of profile.critical) {
			const slot = slotById(id);
			if (!slot) continue;
			const target = sections.find((s) => s.id === slot.section);
			if (!target || target.items.some((i) => i.slot === id)) continue;
			/** @type {Item} */
			const missing = {
				id: uid(`${dash(id)}-missing`),
				slot: id,
				label: slotLabel(slot, lang),
				display: number ? `${t.present.notConfirmed}: ${number}` : t.present.notConfirmed,
				kind: slot.kind,
				factIds: phone && number ? phone.factIds : [],
				cites: phone && number ? phone.cites : [],
				state: 'missing',
				stale: null,
				alternatives: null,
				inputs: null,
				copy: false,
				route: slot.route,
				instance: null
			};
			const rank = rankOf.get(id) ?? 0;
			const at = target.items.findIndex((i) => (rankOf.get(i.slot ?? '') ?? Infinity) > rank);
			if (at < 0) target.items.push(missing);
			else target.items.splice(at, 0, missing);
		}
		const prepare = sections.find((s) => s.id === 'prepare');
		const remote = prepare ? remoteNotarizationRow([...prepare.items, ...prepare.also], lang) : null;
		if (prepare && remote) prepare.items.push({ ...remote, id: uid(remote.id) });
	}

	// Staff notes become cites; an absence nothing fills becomes an
	// `absent` row in its slot's section.
	for (const { note, slot, instance } of attachNotes(notes, all())) {
		if (note.kind !== 'absence') continue;
		const target = sections.find((s) => s.id === slot.section);
		if (!target) continue;
		/** @type {Item} */
		const absent = {
			id: uid(`${dash(slot.id)}${instance ? `-${instance}` : ''}-absent`),
			slot: slot.id,
			label: slotLabel(slot, lang),
			display: t.present.absent,
			kind: slot.kind,
			factIds: [],
			cites: [noteCite(note)],
			state: 'absent',
			stale: null,
			alternatives: null,
			inputs: null,
			copy: false,
			route: slot.route,
			instance
		};
		const at = target.items.findIndex((i) => i.slot === slot.id && i.state === 'missing');
		if (at >= 0) target.items.splice(at, 1, absent);
		else target.items.push(absent);
	}

	// Offices (send section rows with an instance) and closures. A
	// closure whose every date has passed leaves the visit card (which
	// lists today's and later ones) for Updates, so every closure the
	// page numbers is one it shows.
	const send = sections.find((s) => s.id === 'send');
	if (send) {
		const closureItems = send.items.filter((i) => i.slot === SLOT.closure);
		const soon = upcomingClosures(closureItems, new Date(nowMs), zone);
		/** @type {Set<Item>} */
		const ahead = new Set([...soon.next, ...soon.later].map((x) => x.item));
		for (const i of soon.undated) ahead.add(i);
		const past = closureItems.filter((i) => !ahead.has(i));
		if (past.length > 0) {
			send.items = send.items.filter((i) => !past.includes(i));
			page.updates = [...page.updates, ...past];
		}
	}
	const sendItems = send?.items ?? [];
	// D3, as the send block applies it: the mail branch shows no street on
	// a US page (unless the office's pages dispute the value), the courier
	// branch no PO box. The glance and the route cards follow the same
	// rule, so they never name an address the step withholds.
	/** @param {Item} i */
	const mailShown = (i) => real(i) && (disputed(i) || !(usPostal && isStreetOnly(i.display)));
	/** @param {Item} i */
	const courierShown = (i) => real(i) && !isPoBox(i.display);
	/** @type {Page['offices']} */
	const officeGroups = [];
	for (const item of sendItems) {
		if (!OFFICE_SLOTS.includes(item.slot ?? '') || !item.instance) continue;
		let group = officeGroups.find((g) => g.instance === item.instance);
		if (!group) {
			const inst = item.instance;
			const named = records.find((o) => typeof o.name === 'string' && sameOffice(o, inst));
			group = { instance: inst, name: named ? String(named.name) : instanceName(inst), items: [] };
			officeGroups.push(group);
		}
		group.items.push(item);
	}
	page.offices = officeGroups;
	page.closures = sendItems.filter((i) => i.slot === SLOT.closure);

	// The glance and the route cards (profiled pages only).
	/** @type {Item[]} */
	const glance = [];
	let glanceRows = 0;
	if (us) {
		const values = sections.flatMap((s) => s.items);
		for (const row of GLANCE_ROWS) {
			let filled = false;
			for (const part of row.parts) {
				const found = values.filter((i) => i.slot === part.slot && real(i));
				if (part.slot === SLOT.office) {
					for (const item of found) {
						if (disputed(item)) continue;
						const city = postalCity(item.display);
						if (!city) continue;
						glance.push({ ...partOf(item, 'city', city), id: uid(`glance-${item.id}-city`) });
						filled = true;
					}
					continue;
				}
				const item = found[0];
				if (!item) continue;
				if (part.slot === SLOT.usps && !mailShown(item)) continue;
				if (part.slot === SLOT.courier && !courierShown(item)) continue;
				filled = true;
				if (item.state === 'conflict' || item.state === 'contested') {
					glance.push({
						...item,
						id: uid(`glance-${item.id}`),
						display:
							item.state === 'contested'
								? t.present.pointerContested
								: part.slot === SLOT.usps
									? t.present.pointerUsps
									: t.present.pointer,
						cites: [],
						alternatives: null,
						copy: false,
						contest: null,
						own: true
					});
				} else {
					glance.push(glanceCopy(item, uid));
				}
			}
			if (filled) glanceRows += 1;
		}
	}
	page.thin = !us || glanceRows < 2;

	if (!page.thin) {
		page.glance = glance;
		const has = (/** @type {string} */ slot) => sendItems.some((i) => i.slot === slot && real(i));
		const sameDay = [SLOT.sameDay, SLOT.sameDayOnly, SLOT.sameDayTime].some((slot) =>
			sections.some((s) => s.items.some((i) => i.slot === slot && real(i)))
		);
		if (sendItems.some((i) => i.slot === SLOT.usps && mailShown(i))) {
			page.routes.push({ id: 'mail', label: t.present.routes.mail, href: '#send-mail' });
		}
		if (sendItems.some((i) => i.slot === SLOT.courier && courierShown(i))) {
			page.routes.push({ id: 'courier', label: t.present.routes.courier, href: '#send-courier' });
		}
		if (has(SLOT.office)) {
			page.routes.push({ id: 'visit', label: sameDay ? t.present.routes.visitSameDay : t.present.routes.visit, href: '#visit' });
		}
	}

	// Rendered sections: those with rows, numbered on profiled pages; the
	// contact section stays when its "Not confirmed yet" list has rows.
	const missingAny = sections.some((s) => s.items.some((i) => i.state === 'missing' && i.slot));
	let step = 0;
	page.sections = sections
		.filter((s) => s.items.length + s.also.length > 0 || (s.id === 'contact' && us && !page.thin && missingAny))
		.map((s) => ({ ...s, step: page.thin || s.step === null ? null : ++step }));

	page.order = [
		...page.status.active,
		...page.status.resolved,
		...page.glance,
		...page.sections.flatMap((s) => [...s.items, ...s.also]),
		...page.updates
	].map((i) => i.id);
	return page;
}

/**
 * Every Item of a page, with alternatives, once each (status, glance,
 * sections, updates).
 * @param {Page} page
 * @returns {Item[]}
 */
export function allItems(page) {
	/** @type {Item[]} */
	const out = [];
	/** @param {Item} item */
	const add = (item) => {
		out.push(item);
		for (const alt of item.alternatives ?? []) add(alt);
	};
	for (const item of [
		...page.status.active,
		...page.status.resolved,
		...page.glance,
		...page.sections.flatMap((s) => [...s.items, ...s.also]),
		...page.updates
	])
		add(item);
	return out;
}

/**
 * The slot labels of rows that read "Not confirmed yet" (the contact
 * section lists them, profiled pages only).
 * @param {Page} page
 * @returns {string[]}
 */
export function notConfirmedLabels(page) {
	if (page.thin) return [];
	return [
		...new Set(
			page.sections.flatMap((s) => s.items.filter((i) => i.state === 'missing' && i.slot).map((i) => i.label))
		)
	];
}

/**
 * Changed rows between two presentations, for the live flash: ids of
 * Items that are new or whose value, state or facts changed, and their
 * labels (once each).
 * @param {Page|null} before
 * @param {Page} after
 * @returns {{ ids: string[], labels: string[] }}
 */
export function changedItems(before, after) {
	/** @param {Item} i */
	const sig = (i) => `${i.display}|${i.state}|${i.factIds.join(' ')}|${(i.alternatives ?? []).map((a) => a.display).join('|')}`;
	const old = new Map((before ? allItems(before) : []).map((i) => [i.id, sig(i)]));
	/** @type {string[]} */
	const ids = [];
	/** @type {string[]} */
	const labels = [];
	for (const item of after ? allItems(after) : []) {
		if (old.get(item.id) === sig(item)) continue;
		ids.push(item.id);
		if (item.label && !labels.includes(item.label)) labels.push(item.label);
	}
	if (before && after) {
		const now = new Set(allItems(after).map((i) => i.id));
		for (const item of allItems(before)) {
			if (!now.has(item.id) && item.label && !labels.includes(item.label)) labels.push(item.label);
		}
	}
	return { ids, labels };
}
