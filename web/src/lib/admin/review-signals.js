// Shared review signals (risk badge, "verified", Wonder chip, diff
// marks, authority grouping, bulk-refusal tags) for the review queue
// table, the by-authority view and /triage — one definition of
// "verified" for every screen. Pure functions: no DOM, no fetch.
// Admin-only: never imported from (public) or (customer) routes.

/**
 * The minimal fact shape every review screen renders. `/queue` items
 * (review-client.js QueueItem) satisfy it as they are; `/facts` rows
 * (FactView) are adapted by `factViewToItem` in review-client.js.
 * @typedef {object} ReviewCitation
 * @property {number} capture_id 0 when the source is a FactView
 * @property {number} start_byte
 * @property {number} end_byte
 * @property {string} quote
 * @property {string} [source_url]
 * @property {string} [captured_at]
 */

/**
 * @typedef {object} ReviewItem
 * @property {number} id
 * @property {string} authority_slug
 * @property {string} topic
 * @property {string} statement
 * @property {string} created_by
 * @property {number} created_at
 * @property {{ base_statement: string, base_fact_id: number, base_status: string } | null} diff
 * @property {ReviewCitation[]} citations
 * @property {{ conflict: boolean, conflicts: { id: number, statement: string, status: string }[] }} blast_radius
 * @property {{ capture_id: number, trust: string }} provenance
 */

/** @typedef {{ fact_id: number, verdict: string, quote?: string, model?: string }} WonderCheck */

export const TRUSTED = new Set(['official', 'staff_verified']);

/**
 * The server's accept gate keeps quotes of 8-400 characters (Unicode
 * scalar values, like Rust's `chars()`); shorter or longer slices
 * never reach the public dossier. Pure (tested).
 * @param {unknown} quote
 * @returns {boolean}
 */
export function quoteOk(quote) {
	if (typeof quote !== 'string') return false;
	const n = [...quote].length;
	return n >= 8 && n <= 400;
}

/**
 * A citation the accept gate would keep (first one wins, as on the
 * server). Pure (tested).
 * @param {ReviewItem} item
 * @returns {boolean}
 */
export function cited(item) {
	return Array.isArray(item.citations) && item.citations.length > 0 && quoteOk(item.citations[0].quote);
}

/**
 * Riskiest first: changed value vs a live sibling, conflict, no usable
 * citation, low-trust source. The one-word badge says why. An unknown
 * trust (a `/facts` row carries none) earns no badge. Pure (tested).
 * @param {ReviewItem} item
 * @returns {{ rank: number, badge: string | null }}
 */
export function riskOf(item) {
	if (item.diff) return { rank: 0, badge: 'Changed' };
	if (item.blast_radius?.conflict) return { rank: 1, badge: 'Conflict' };
	if (!cited(item)) return { rank: 2, badge: 'Uncited' };
	const trust = item.provenance?.trust;
	if (trust && !TRUSTED.has(trust)) return { rank: 3, badge: 'Low-trust' };
	return { rank: 4, badge: null };
}

/**
 * Badge tone for a risk word (full class strings for the Tailwind audit).
 * @param {string} badge
 * @returns {string}
 */
export function riskCls(badge) {
	if (badge === 'Changed') return 'border-changed text-changed';
	if (badge === 'Conflict' || badge === 'Uncited') return 'border-error text-error';
	return 'border-line text-ink-muted';
}

/**
 * "Verified" = a citation the accept gate keeps, no live value
 * changed, no conflicting sibling, and Muse Wonder did not contradict
 * it. Stricter than the server's `require_verified` (which only
 * refuses accepted-sibling changes), so a bulk accept of verified
 * rows is refused only when the data moved underneath. Pure (tested).
 * @param {ReviewItem} item
 * @param {WonderCheck | null | undefined} check
 * @returns {boolean}
 */
export function isVerified(item, check) {
	return cited(item) && !item.diff && !item.blast_radius?.conflict && check?.verdict !== 'contradicted';
}

export const WONDER = /** @type {Record<string, { label: string, cls: string }>} */ ({
	supported: { label: 'Wonder: supported', cls: 'border-verified text-verified' },
	contradicted: { label: 'Wonder: contradicted', cls: 'border-error text-error' },
	not_found: { label: 'Wonder: not found', cls: 'border-line text-ink-muted' }
});

/**
 * @typedef {object} DiffSeg
 * @property {string} text
 * @property {boolean} changed
 */

/**
 * Word-level change marks via common prefix/suffix trim (page display
 * only — the server stores the verbatim statements). Pure (tested).
 * @param {string} base
 * @param {string} next
 * @returns {{ base: DiffSeg[], next: DiffSeg[] }}
 */
export function markChanged(base, next) {
	const a = base.split(/(\s+)/);
	const b = next.split(/(\s+)/);
	let pre = 0;
	while (pre < a.length && pre < b.length && a[pre] === b[pre]) pre++;
	let suf = 0;
	while (suf < a.length - pre && suf < b.length - pre && a[a.length - 1 - suf] === b[b.length - 1 - suf]) suf++;
	/** @param {string[]} words */
	const segs = (words) => words.map((text, i) => ({ text, changed: i >= pre && i < words.length - suf }));
	return { base: segs(a), next: segs(b) };
}

/**
 * @template {ReviewItem} T
 * @typedef {object} AuthorityGroup
 * @property {string} slug
 * @property {string} name
 * @property {T[]} items   every row in view, decided ones included (input order)
 * @property {number} open   rows not decided this session
 * @property {number} verified   open rows that are verified
 * @property {number} oldest   created_at of the oldest open row (0 when none)
 */

/**
 * Stable riskiest-first order (the queue table's order). Pure (tested).
 * @template {ReviewItem} T
 * @param {T[]} items
 * @returns {T[]}
 */
export function riskiestFirst(items) {
	return items
		.map((item, index) => ({ item, index, rank: riskOf(item).rank }))
		.sort((a, b) => a.rank - b.rank || a.index - b.index)
		.map((row) => row.item);
}

/**
 * Group rows per authority. Groups with the most proposals in view
 * come first (ties by name); rows inside a group run riskiest first,
 * like the queue table. Pure (tested).
 * @template {ReviewItem} T
 * @param {T[]} items
 * @param {Record<number, string>} decided
 * @param {Record<number, WonderCheck>} checks
 * @param {Record<string, string>} [names] slug → display name
 * @returns {AuthorityGroup<T>[]}
 */
export function groupByAuthority(items, decided, checks, names = {}) {
	/** @type {Map<string, AuthorityGroup<T>>} */
	const groups = new Map();
	for (const item of items) {
		const slug = item.authority_slug;
		let group = groups.get(slug);
		if (!group) {
			group = { slug, name: names[slug] || slug, items: [], open: 0, verified: 0, oldest: 0 };
			groups.set(slug, group);
		}
		group.items.push(item);
		if (decided[item.id]) continue;
		group.open += 1;
		if (isVerified(item, checks[item.id])) group.verified += 1;
		if (group.oldest === 0 || item.created_at < group.oldest) group.oldest = item.created_at;
	}
	for (const group of groups.values()) group.items = riskiestFirst(group.items);
	// Rows in view (decided ones included), not open rows: the order
	// stays put while staff decide, so a group never jumps from under
	// the pointer the moment its batch is accepted.
	return [...groups.values()].sort((a, b) => b.items.length - a.items.length || a.name.localeCompare(b.name));
}

/**
 * Short inline tag for one refused id of a blocked bulk batch (the
 * server's `results[].error` vocabulary). Pure (tested).
 * @param {number} code
 * @param {string} error
 * @returns {string}
 */
export function blockedTag(code, error) {
	if (code === 409) return 'Decided elsewhere';
	if (code === 404) return 'No longer exists';
	if (/verified citation/.test(error)) return 'Needs citation';
	if (/published fact/.test(error)) return 'Changes a published fact';
	if (/contradicts/.test(error)) return 'Wonder contradicted';
	if (/not in authority/.test(error)) return 'Other authority';
	return error || 'Blocked';
}

/**
 * Split a blocked batch's per-id results into the refused rows and the
 * ids that would pass on their own. Pure (tested).
 * @param {{ id: number, ok: boolean, code?: number, error?: string }[]} results
 * @returns {{ refused: { id: number, code: number, tag: string }[], passing: number[] }}
 */
export function splitBlocked(results) {
	/** @type {{ id: number, code: number, tag: string }[]} */
	const refused = [];
	/** @type {number[]} */
	const passing = [];
	for (const r of Array.isArray(results) ? results : []) {
		if (!r || typeof r.id !== 'number') continue;
		if (r.ok) passing.push(r.id);
		else {
			const code = typeof r.code === 'number' ? r.code : 422;
			refused.push({ id: r.id, code, tag: blockedTag(code, typeof r.error === 'string' ? r.error : '') });
		}
	}
	return { refused, passing };
}

/**
 * "1 fact" / "3 facts". Pure (tested).
 * @param {number} n
 * @param {string} one
 * @param {string} [many]
 * @returns {string}
 */
export function plural(n, one, many = `${one}s`) {
	return `${n} ${n === 1 ? one : many}`;
}
