// "Updates" (plan §3 section 10): resolved service notices and the
// office pages' change notes from `/changes`, newest first. A change
// too large to list (a re-captured page whose diff runs past
// DEGENERATE_LINES) collapses to one line. Days are Pacific calendar
// days, as the rest of the page prints them: a page read at 10 p.m.
// Pacific is read that day, not the next. Pure: no window/fetch.

import { DEFAULT_ZONE, closureClauses, zonedDay, zonedNow } from './offices.js';

/** @typedef {import('./contract.js').Item} Item */

/** A diff with more changed lines than this is listed as one line. */
export const DEGENERATE_LINES = 50;

/** Entries shown before the rest fold into a details element. */
export const VISIBLE_UPDATES = 3;

/**
 * One change panel (`guide.js` changePanels). `capturedAt` is the raw
 * capture instant (ISO); `date` its UTC day, the fallback.
 * @typedef {{ sourceUrl: string, date: string, capturedAt?: string, since: string, added: string[], removed: string[], more: number }} ChangePanel
 */

/**
 * @typedef {{ key: string, date: string, kind: 'notice', item: Item }
 *   | { key: string, date: string, kind: 'change', panel: ChangePanel, degenerate: boolean }} UpdateEntry
 */

/**
 * `YYYY-MM-DD` of unix seconds in Pacific time, or null.
 * @param {unknown} secs
 */
function secondsDay(secs) {
	if (typeof secs !== 'number' || !Number.isFinite(secs)) return null;
	return zonedNow(new Date(secs * 1000), DEFAULT_ZONE)?.date ?? null;
}

/**
 * The notice's own day, `YYYY-MM-DD` ('' when nothing dates it), in
 * this order:
 * - its instance, when that is a date (`restored__2026_11_03`);
 * - the one date its text states, unless that day comes after we read
 *   it (a date ahead is a plan, not the notice's day);
 * - the first day we read it (capture, or the call or email);
 * - the first day staff approved it.
 * Every day is a Pacific calendar day.
 * @param {Item} item
 */
export function itemDay(item) {
	const inst = /^(\d{4})_(\d{2})_(\d{2})$/.exec(String(item?.instance ?? ''));
	if (inst) return `${inst[1]}-${inst[2]}-${inst[3]}`;
	/** @type {string[]} */
	const seen = [];
	/** @type {string[]} */
	const approved = [];
	for (const c of item?.cites ?? []) {
		const s = c?.source;
		if (!s) continue;
		for (const d of [zonedDay(s.capturedAt), secondsDay(s.occurredAt)]) if (d) seen.push(d);
		const a = secondsDay(s.acceptedAt);
		if (a) approved.push(a);
	}
	seen.sort();
	approved.sort();
	const bound = seen[seen.length - 1] ?? approved[approved.length - 1] ?? null;
	const stated = [...new Set(closureClauses(item).flatMap((c) => c.dates))];
	if (stated.length === 1 && (!bound || stated[0] <= bound)) return stated[0];
	return seen[0] ?? approved[0] ?? '';
}

/**
 * A change panel's day: its capture instant as a Pacific day, else the
 * day the payload gives.
 * @param {ChangePanel} panel
 */
export function panelDay(panel) {
	return zonedDay(panel.capturedAt) ?? panel.date;
}

/**
 * Changed lines in a panel, those the payload lists plus those it only counts.
 * @param {ChangePanel} panel
 */
export function changedLines(panel) {
	return panel.added.length + panel.removed.length + (panel.more > 0 ? panel.more : 0);
}

/**
 * Notices and change panels as one list, newest first (ties keep notices
 * first). Notices without a readable day sort last.
 * @param {{ items?: readonly (Item|null|undefined)[]|null, panels?: readonly ChangePanel[]|null }} input
 * @returns {UpdateEntry[]}
 */
export function updateEntries({ items = [], panels = [] }) {
	/** @type {UpdateEntry[]} */
	const out = [];
	for (const item of items ?? []) {
		if (!item) continue;
		out.push({ key: `notice-${item.id}`, date: itemDay(item), kind: 'notice', item });
	}
	(panels ?? []).forEach((panel, i) => {
		out.push({
			key: `change-${i}-${panel.sourceUrl}`,
			date: panelDay(panel),
			kind: 'change',
			panel,
			degenerate: changedLines(panel) > DEGENERATE_LINES
		});
	});
	return out.sort((a, b) => (a.date === b.date ? (a.kind === b.kind ? 0 : a.kind === 'notice' ? -1 : 1) : a.date < b.date ? 1 : -1));
}

/**
 * `YYYY-MM-DD` as "Nov 3, 2026"; '' when unreadable.
 * @param {unknown} day
 */
export function shortDay(day) {
	const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(String(day ?? ''));
	if (!m) return '';
	return new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' }).format(
		Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]), 12)
	);
}
