// Fees (plan §3 step 4, D2): the per-document fee, the speed tiers and
// the calculator's arithmetic, read only from the office's own words.
//
// A tier is used only when cited text states it. `tierTable(text)`
// reads one passage. `feeModel(items)` runs it over each citation
// quote of a speed's fee item (web, PDF and staff-upload quotes; staff
// notes are never a source), keeping only the clauses that carry the
// item's own amount. Inside such a clause a "first 10 documents" or
// "per 10 documents" phrase counts only when the amount it sits next
// to is the item's own ("an additional $150, on top of $15.00 per
// document, for the first 10 documents" states no same-day tier), and
// the first block must be that amount, so a passage that lists two
// fees never mixes them. Tiers that overlap with different amounts
// refuse the whole speed. The calculator never extrapolates: a count no
// cited tier covers, a count above MAX_DOCS (the office writes "etc."),
// an uncited amount, a disputed input or one a staff note contests
// (the office told staff otherwise) all answer "ask the office".
// Amounts are US dollars as printed (`$15.00`); a page with no `$`
// amount gets no model. Pure: no window/fetch, so node tests drive it.
//
// The calculator's computed Item (`estimateItem`, id ESTIMATE_ID) has
// one owner: FeeBlock builds it from the customer's count and speed
// and hands it to the page (`bind:estimate`). The presenter builds
// none, and the page numbers it after every other item, so a changing
// count or speed never renumbers the office's sources.

import { guideCopy } from './doc/copy/index.js';

/** @typedef {import('./doc/contract.js').Item} Item */

/**
 * One cited tier: the speed fee for any count from `from` to `to`.
 * @typedef {{ from: number, to: number, cents: number }} Tier
 */

/**
 * @typedef {'standard'|'expedite'|'same_day'} Speed
 */

/**
 * Why a fee or speed cannot be used: `uncited` (no quote states the
 * amount), `no-amount` (the value names no `$` amount), `conflict` (the
 * item is disputed, or its cited tiers disagree), `no-tier` (the
 * amount is cited but no count range is), `unclear` (the cited count
 * range sits next to another fee's amount, or its first block is not
 * the item's own amount).
 * @typedef {'uncited'|'no-amount'|'conflict'|'no-tier'|'unclear'} Unusable
 */

/**
 * @typedef {{ speed: Speed, item: Item, itemIds: string[], tiers: Tier[], reason: Unusable|null }} SpeedModel
 */

/**
 * @typedef {{
 *   perDocument: null|{ item: Item, cents: number|null, reason: Unusable|null },
 *   speeds: { standard: SpeedModel|null, expedite: SpeedModel|null, same_day: SpeedModel|null },
 *   currency: 'USD'
 * }} FeeModel
 */

/**
 * @typedef {{ kind: 'per_document', count: number, unitCents: number, cents: number, itemId: string }} PerDocumentLine
 * @typedef {{ kind: 'expedite'|'same_day', from: number, to: number, cents: number, itemId: string }} SpeedLine
 */

/**
 * Why the calculator answers "ask the office" (see `askText`).
 * @typedef {'no-fee'|'count'|'no-speed'|'conflict'|'contested'|'uncited'|'no-amount'|'no-tier'|'unclear'|'over-max'} AskReason
 */

/**
 * @typedef {{ ask: true, reason: AskReason }} Ask
 * @typedef {{
 *   ask: false,
 *   speed: Speed,
 *   n: number,
 *   totalCents: number,
 *   lines: [PerDocumentLine] | [PerDocumentLine, SpeedLine],
 *   inputs: Item[],
 *   route: 'mail'|'counter'
 * }} Estimate
 */

/** Slot ids the fee model reads (slots.json). */
export const FEE_SLOTS = Object.freeze({
	perDocument: 'fees.per_document',
	expedite: 'fees.expedite',
	sameDay: 'fees.same_day'
});

/** Speeds in calculator order. */
export const SPEEDS = /** @type {const} */ (['standard', 'expedite', 'same_day']);

/** The calculator's largest count; above it the office only writes "etc.". */
export const MAX_DOCS = 20;

/** Id of the calculator's computed Item (owned by FeeBlock). */
export const ESTIMATE_ID = 'fees-estimate';

/** Id of the calculator's "ask the office" row: no cites, never numbered. */
export const ASK_ID = 'fees-estimate-ask';

/** What every "ask" answer but a bad count adds after its reason. */
export const ASK_OFFICE = guideCopy('en').fees.askOffice;

/** Evidence kinds whose quote is the office's own text. */
const QUOTE_KINDS = new Set(['web', 'pdf', 'upload']);

// Hyphen, the Unicode dashes and the minus sign: an office writes a
// range as 1-10, 1–10 or 1—10.
const DASH = '[-‐-―−]';
const DOCS = String.raw`(?:documents?|docs?)`;
const AMOUNT = String.raw`\$\s?(\d{1,3}(?:,\d{3})+|\d+)(?:\.(\d{1,2}))?(?!\d)`;

const AMOUNT_RE = new RegExp(AMOUNT, 'g');
// "1-10 documents = $150.00", "11-20 = $250", "1–10 documents: $150".
const RANGE_RE = new RegExp(
	String.raw`(?<![\d.,$])(\d{1,3})\s*(?:${DASH}|\bto\b|\bthrough\b|\bthru\b)\s*(\d{1,3})(?![\d,.]?\d)\s*(?:${DOCS}\b)?\s*(?:=|:|${DASH}|\bis\b|\bcosts?\b)?\s*` +
		AMOUNT,
	'gi'
);
// "for the first 10 documents": the first block.
const FIRST_RE = new RegExp(String.raw`\bfirst\s+(\d{1,3})\s+${DOCS}\b`, 'gi');
// "per 10 documents", "per each 10 documents", "for every 10 documents":
// the first block only ("each additional 10" never matches).
const PER_RE = new RegExp(
	String.raw`\b(?:per|for\s+every|for\s+each|every|each)\s+(?:each\s+)?(\d{1,3})\s+${DOCS}\b`,
	'gi'
);

/**
 * `"1,000"`, `"5"` → 100000, 500.
 * @param {string} dollars
 * @param {string|undefined} fraction
 */
function toCents(dollars, fraction) {
	const whole = Number(String(dollars).replaceAll(',', ''));
	const part = Number(String(fraction ?? '').padEnd(2, '0').slice(0, 2) || '0');
	return whole * 100 + part;
}

/**
 * Every `$` amount in `text`, in order.
 * @param {string} text
 * @returns {{ cents: number, start: number, end: number }[]}
 */
export function amountsIn(text) {
	const s = String(text ?? '');
	return [...s.matchAll(AMOUNT_RE)].map((m) => ({
		cents: toCents(m[1], m[2]),
		start: m.index ?? 0,
		end: (m.index ?? 0) + m[0].length
	}));
}

/**
 * The first `$` amount in `text`, in cents, or null.
 * @param {string} text
 */
export function firstAmount(text) {
	return amountsIn(text)[0]?.cents ?? null;
}

/**
 * Split a passage into clauses: sentences, lines, and `;` outside
 * parentheses. A parenthetical tier list such as "(1–10 docs = $100;
 * 11–20 = $200)" stays whole.
 * @param {string} text
 * @returns {string[]}
 */
export function clauses(text) {
	const s = String(text ?? '');
	/** @type {string[]} */
	const out = [];
	let cur = '';
	let depth = 0;
	const push = () => {
		if (cur.trim()) out.push(cur.trim());
		cur = '';
	};
	for (let i = 0; i < s.length; i++) {
		const c = s[i];
		if (c === '(') depth += 1;
		else if (c === ')') depth = Math.max(0, depth - 1);
		if (depth === 0 && (c === '\n' || c === ';')) {
			push();
			continue;
		}
		cur += c;
		if (depth === 0 && (c === '.' || c === '!' || c === '?') && /^\s+[A-Z(]/.test(s.slice(i + 1))) push();
	}
	push();
	return out;
}

/**
 * The amount nearest a phrase in its clause: the last one before it,
 * else the first one after it.
 * @param {{ cents: number, start: number, end: number }[]} amounts
 * @param {number} start
 * @param {number} end
 */
function nearestAmount(amounts, start, end) {
	const before = amounts.filter((a) => a.end <= start);
	if (before.length > 0) return before[before.length - 1];
	return amounts.find((a) => a.start >= end) ?? null;
}

/**
 * Tiers sorted by range, duplicates dropped; null when two tiers
 * overlap with different amounts (the text disagrees with itself).
 * @param {Tier[]} tiers
 * @returns {Tier[]|null}
 */
function settle(tiers) {
	/** @type {Tier[]} */
	const out = [];
	for (const t of [...tiers].sort((a, b) => a.from - b.from || a.to - b.to || a.cents - b.cents)) {
		for (const kept of out) {
			const overlap = t.from <= kept.to && kept.from <= t.to;
			if (overlap && t.cents !== kept.cents) return null;
		}
		if (!out.some((k) => k.from === t.from && k.to === t.to && k.cents === t.cents)) out.push(t);
	}
	return out;
}

/**
 * Read the tiers one passage states, with a flag when it states two
 * amounts for one count.
 *
 * With `anchor` (a fee item's own amount, in cents), a "first N" or
 * "per N" phrase states a tier only when the amount nearest it is the
 * anchor; one that sits next to another amount belongs to another fee
 * and is counted in `stray` instead. Stated ranges ("11-20 = $250")
 * always count.
 * @param {string} text
 * @param {number|null} [anchor]
 * @returns {{ tiers: Tier[], conflict: boolean, stray: number }}
 */
export function readTiers(text, anchor = null) {
	/** @type {Tier[]} */
	const found = [];
	let stray = 0;
	for (const clause of clauses(text)) {
		const amounts = amountsIn(clause);
		for (const m of clause.matchAll(RANGE_RE)) {
			const from = Number(m[1]);
			const to = Number(m[2]);
			if (from >= 1 && to >= from) found.push({ from, to, cents: toCents(m[3], m[4]) });
		}
		for (const re of [FIRST_RE, PER_RE]) {
			for (const m of clause.matchAll(re)) {
				const to = Number(m[1]);
				const start = m.index ?? 0;
				const amount = nearestAmount(amounts, start, start + m[0].length);
				if (!(to >= 1 && amount)) continue;
				if (anchor !== null && amount.cents !== anchor) {
					stray += 1;
					continue;
				}
				found.push({ from: 1, to, cents: amount.cents });
			}
		}
	}
	const tiers = settle(found);
	return tiers === null ? { tiers: [], conflict: true, stray } : { tiers, conflict: false, stray };
}

/**
 * The tiers a passage states, by count range; empty when it states
 * none or disagrees with itself.
 * - "1-10 documents = $150.00", "11-20 = $250", "1–10 documents: $150";
 * - "for the first 10 documents" covers 1–10;
 * - "per 10 documents" covers only the first block; later blocks need
 *   their own stated range.
 * @param {string} text
 * @returns {Tier[]}
 */
export function tierTable(text) {
	return readTiers(text).tiers;
}

/**
 * The office's own words behind an item: quotes of web, PDF and
 * staff-upload citations (a citation without a kind is a web page).
 * @param {Item} item
 * @returns {string[]}
 */
export function citedQuotes(item) {
	return (item?.cites ?? [])
		.filter((c) => typeof c?.quote === 'string' && c.quote.trim() && (!c.source?.kind || QUOTE_KINDS.has(c.source.kind)))
		.map((c) => /** @type {string} */ (c.quote));
}

/**
 * Disputed by the office's pages, or contested by a staff note.
 * @param {Item|null|undefined} item
 */
function disputed(item) {
	return item?.state === 'conflict' || item?.state === 'worded' || item?.state === 'contested';
}

/**
 * @param {Item} item
 * @returns {{ cents: number|null, reason: Unusable|null }}
 */
function perDocumentOf(item) {
	if (disputed(item)) return { cents: null, reason: 'conflict' };
	const cents = firstAmount(item.display);
	if (cents === null) return { cents: null, reason: 'no-amount' };
	const cited = citedQuotes(item).some((q) => amountsIn(q).some((a) => a.cents === cents));
	return cited ? { cents, reason: null } : { cents: null, reason: 'uncited' };
}

/**
 * Tiers of one speed's fee item, from the clauses of its quotes that
 * carry the item's own amount, with "first N" and "per N" phrases bound
 * to that amount only; the item's value may restate them but never adds
 * or changes one. The first block (from 1) must be the item's own
 * amount: a first block at another amount means the text ties the
 * count range to another fee, so the speed asks. The item's own amount
 * is its first one past a restated per-document fee ("Fee is $15.00
 * per document and an additional $100 expedite fee per 10 documents"
 * anchors on $100), or its first when it states no other.
 * @param {Item} item
 * @param {number|null} perCents the per-document fee, in cents
 * @returns {{ tiers: Tier[], reason: Unusable|null }}
 */
function speedTiers(item, perCents) {
	if (disputed(item)) return { tiers: [], reason: 'conflict' };
	const amounts = amountsIn(item.display);
	const anchor = (amounts.find((a) => a.cents !== perCents) ?? amounts[0])?.cents ?? null;
	if (anchor === null) return { tiers: [], reason: 'no-amount' };
	const quotes = citedQuotes(item);
	/** @type {Tier[]} */
	const found = [];
	let cited = false;
	let stray = 0;
	for (const quote of quotes) {
		for (const clause of clauses(quote)) {
			if (!amountsIn(clause).some((a) => a.cents === anchor)) continue;
			cited = true;
			const read = readTiers(clause, anchor);
			if (read.conflict) return { tiers: [], reason: 'conflict' };
			stray += read.stray;
			found.push(...read.tiers);
		}
	}
	if (!cited) return { tiers: [], reason: 'uncited' };
	const tiers = settle(found);
	if (tiers === null) return { tiers: [], reason: 'conflict' };
	if (tiers.some((t) => t.from === 1 && t.cents !== anchor)) return { tiers: [], reason: 'unclear' };
	const shown = readTiers(item.display, anchor);
	const contradicts =
		shown.conflict ||
		shown.tiers.some((s) => tiers.some((t) => s.from <= t.to && t.from <= s.to && s.cents !== t.cents));
	if (contradicts) return { tiers: [], reason: 'conflict' };
	if (tiers.length > 0) return { tiers, reason: null };
	return { tiers: [], reason: stray > 0 ? 'unclear' : 'no-tier' };
}

/**
 * The fee item of a slot: a real, cited value resting on facts (not an
 * honest "not confirmed", a stated absence such as "same-day service
 * is not offered", or a value with no source on file), so the
 * calculator offers only speeds that have a fee.
 * @param {Item[]} items
 * @param {string} slot
 */
function feeItem(items, slot) {
	return (
		items.find(
			(i) =>
				i?.slot === slot &&
				i.state !== 'missing' &&
				i.state !== 'absent' &&
				i.state !== 'uncited' &&
				(i.factIds?.length ?? 0) > 0
		) ?? null
	);
}

/**
 * The fee model of a page: per-document cents and the speeds whose fee
 * item exists, each with its cited tiers and the item ids it rests on.
 * Pass the "Pay the fees" section's items (other items are ignored).
 * @param {Item[]} items
 * @returns {FeeModel}
 */
export function feeModel(items) {
	const list = (items ?? []).filter(Boolean);
	const per = feeItem(list, FEE_SLOTS.perDocument);
	const expedite = feeItem(list, FEE_SLOTS.expedite);
	const sameDay = feeItem(list, FEE_SLOTS.sameDay);
	const perDocument = per ? { item: per, ...perDocumentOf(per) } : null;
	/**
	 * @param {Speed} speed
	 * @param {Item|null} item
	 * @returns {SpeedModel|null}
	 */
	const speedOf = (speed, item) =>
		item ? { speed, item, itemIds: [per?.id, item.id].filter((id) => typeof id === 'string'), ...speedTiers(item, perDocument?.cents ?? null) } : null;
	return {
		perDocument,
		speeds: {
			standard: per ? { speed: 'standard', item: per, itemIds: [per.id], tiers: [], reason: perDocument?.reason ?? null } : null,
			expedite: speedOf('expedite', expedite),
			same_day: speedOf('same_day', sameDay)
		},
		currency: 'USD'
	};
}

/**
 * The total for `n` documents at `speed`, or why to ask the office.
 * Never extrapolates past the tiers the office states.
 * @param {FeeModel|null|undefined} model
 * @param {number} n
 * @param {Speed} speed
 * @returns {Estimate|Ask}
 */
export function estimate(model, n, speed) {
	const per = model?.perDocument ?? null;
	if (!model || !per) return { ask: true, reason: 'no-fee' };
	const s = model.speeds?.[speed] ?? null;
	if (!s) return { ask: true, reason: 'no-speed' };
	const inputs = speed === 'standard' ? [per.item] : [per.item, s.item];
	if (inputs.some((i) => i?.state === 'contested')) return { ask: true, reason: 'contested' };
	if (inputs.some(disputed) || per.reason === 'conflict' || s.reason === 'conflict') {
		return { ask: true, reason: 'conflict' };
	}
	if (per.cents === null) return { ask: true, reason: per.reason ?? 'uncited' };
	if (!Number.isInteger(n) || n < 1) return { ask: true, reason: 'count' };
	if (n > MAX_DOCS) return { ask: true, reason: 'over-max' };
	/** @type {PerDocumentLine} */
	const first = { kind: 'per_document', count: n, unitCents: per.cents, cents: n * per.cents, itemId: per.item.id };
	if (speed === 'standard') {
		return { ask: false, speed, n, totalCents: first.cents, lines: [first], inputs, route: 'mail' };
	}
	if (s.reason) return { ask: true, reason: s.reason };
	const tier = s.tiers.find((t) => t.from <= n && n <= t.to);
	if (!tier) return { ask: true, reason: 'no-tier' };
	/** @type {SpeedLine} */
	const extra = { kind: speed, from: tier.from, to: tier.to, cents: tier.cents, itemId: s.item.id };
	return {
		ask: false,
		speed,
		n,
		totalCents: first.cents + extra.cents,
		lines: [first, extra],
		inputs,
		route: speed === 'same_day' ? 'counter' : 'mail'
	};
}

/** @type {Map<string, Intl.NumberFormat>} */
const USD = new Map();

/**
 * Cents as printed money in the page language's `Intl` format: 14500 →
 * "$145.00" (English), "145,00 US$" (Spanish), "145,00 $US" (French).
 * @param {number} cents
 * @param {string} [lang] the page language (doc/copy)
 */
export function money(cents, lang = 'en') {
	const intl = guideCopy(lang).intl;
	let f = USD.get(intl);
	if (!f) {
		f = new Intl.NumberFormat(intl, { style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: 2 });
		USD.set(intl, f);
	}
	return f.format(cents / 100).replace(/[\u00a0\u202f]/g, ' ');
}

/**
 * The calculator's sentence: "Total to enclose: $145.00 = 3 × $15.00 +
 * $100.00 expedite fee (1–10 documents)". Every number is the model's.
 * @param {Estimate} result
 * @param {string} [lang] the page language (doc/copy)
 */
export function estimateText(result, lang = 'en') {
	const t = guideCopy(lang).fees;
	const lead = result.route === 'counter' ? t.totalCounter : t.totalEnclose;
	const [first, extra] = result.lines;
	let text = `${lead}: ${money(result.totalCents, lang)} = ${first.count} × ${money(first.unitCents, lang)}`;
	if (extra) {
		const label = extra.kind === 'same_day' ? t.sameDayFee : t.expediteFee;
		text += ` + ${money(extra.cents, lang)} ${label} (${t.range(extra.from, extra.to)})`;
	}
	return text;
}

/**
 * The calculation as terms in reading order, each with the item id of
 * the input it reads, for the card and the endnote ("3 × $15.00 [1a] +
 * $100.00 expedite fee (1–10 documents) [1b] = $145.00"; doc/contract.js
 * Item.terms). Every number is the model's.
 * @param {Estimate} result
 * @param {string} [lang] the page language (doc/copy)
 * @returns {{ text: string, itemId: string|null }[]}
 */
export function estimateTerms(result, lang = 'en') {
	const t = guideCopy(lang).fees;
	const [first, extra] = result.lines;
	/** @type {{ text: string, itemId: string|null }[]} */
	const terms = [{ text: `${first.count} × ${money(first.unitCents, lang)}`, itemId: first.itemId }];
	if (extra) {
		const label = extra.kind === 'same_day' ? t.sameDayFee : t.expediteFee;
		terms.push({ text: `+ ${money(extra.cents, lang)} ${label} (${t.range(extra.from, extra.to)})`, itemId: extra.itemId });
	}
	terms.push({ text: `= ${money(result.totalCents, lang)}`, itemId: null });
	return terms;
}

/**
 * The worst staleness among a computed value's inputs: a warning
 * ("May be out of date…") wins over a muted age line.
 * @param {readonly Item[]} inputs
 * @returns {Item['stale']}
 */
function worstStale(inputs) {
	const marks = inputs.map((i) => i?.stale).filter((s) => !!s && typeof s.text === 'string' && s.text.trim());
	return marks.find((s) => s?.level === 'warning') ?? marks[0] ?? null;
}

/**
 * The estimate as a computed Item (data-state="computed"): its cites
 * are its inputs' cites, and it carries the worst staleness of its
 * inputs, so a total built from an old fee says so. Null when the
 * calculator has to ask.
 * @param {FeeModel|null|undefined} model
 * @param {number} n
 * @param {Speed} speed
 * @param {string} [lang] the page language (doc/copy)
 * @returns {Item|null}
 */
export function estimateItem(model, n, speed, lang = 'en') {
	const result = estimate(model, n, speed);
	if (result.ask) return null;
	return {
		id: ESTIMATE_ID,
		slot: null,
		label: guideCopy(lang).fees.yourTotal,
		display: estimateText(result, lang),
		kind: 'money',
		factIds: [],
		cites: result.inputs.flatMap((i) => i.cites ?? []),
		state: 'computed',
		stale: worstStale(result.inputs),
		alternatives: null,
		inputs: result.inputs,
		copy: false,
		route: result.route,
		instance: null,
		terms: estimateTerms(result, lang)
	};
}

/**
 * Why the calculator cannot answer, in one plain sentence (the page
 * adds "ask the office" with its phone).
 * `over-max` is the calculator's own limit, not a gap in the office's
 * pages: the per-document fee has none.
 * @param {AskReason} reason
 * @param {string} [lang] the page language (doc/copy)
 */
export function askText(reason, lang = 'en') {
	const ask = guideCopy(lang).fees.ask;
	return reason in ask ? ask[/** @type {keyof typeof ask} */ (reason)] : ask.other;
}

/**
 * True when an "ask" answer sends the customer to the office: every
 * reason but a count still to enter (their own input, not a gap in the
 * office's pages).
 * @param {AskReason} reason
 */
export function asksOffice(reason) {
	return reason !== 'count';
}

/**
 * The calculator row's sentence for an "ask" answer: the reason, then
 * "Ask the office before you pay." unless the count is the problem.
 * @param {AskReason} reason
 * @param {string} [lang] the page language (doc/copy)
 */
export function askSentence(reason, lang = 'en') {
	const text = askText(reason, lang);
	return asksOffice(reason) ? `${text} ${guideCopy(lang).fees.askOffice}` : text;
}
