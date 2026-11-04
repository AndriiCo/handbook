// Office helpers for "In person" (plan §3 step 5, D4): office names,
// closure dates, the printed cut-off rule, and "Open now" with today's
// cut-off. Live status is computed only from an accepted office record
// (OfficeSetV1) whose timezone is quoted or staff-confirmed; closures
// for today override its hours. Everywhere else the page shows the
// office's rules as printed. Pure: no window/fetch.

/** @typedef {import('./contract.js').Item} Item */

/**
 * The accepted office record fields this module reads (plan.ts Office).
 * @typedef {{
 *   id: number,
 *   name: string,
 *   hours: { days: string[], open: string, close: string, fact_id?: number }[],
 *   timezone?: string|null,
 *   timezone_source?: string|null,
 *   address?: { city?: string|null } | null,
 *   fact_ids: number[]
 * }} OfficeRecord
 */

/** Closure dates and "today" are read in this zone unless an office names its own. */
export const DEFAULT_ZONE = 'America/Los_Angeles';

const DAYS = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'];
const MONTHS = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'];
const MONTH_WORDS =
	'january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|nov|dec';

/** @param {number} n */
const pad = (n) => String(n).padStart(2, '0');

/**
 * `YYYY-MM-DD` for a calendar-shaped date, else null.
 * @param {number} y
 * @param {number} m 1-12
 * @param {number} d
 */
function ymd(y, m, d) {
	if (!(m >= 1 && m <= 12 && d >= 1 && d <= 31 && y >= 2000 && y <= 2100)) return null;
	const at = new Date(Date.UTC(y, m - 1, d));
	return at.getUTCMonth() === m - 1 ? `${y}-${pad(m)}-${pad(d)}` : null;
}

/**
 * The wall-clock date, weekday and minute of `now` in `timeZone`.
 * @param {Date} now
 * @param {string} timeZone
 * @returns {{ date: string, day: string, minutes: number }|null}
 */
export function zonedNow(now, timeZone) {
	try {
		const parts = new Intl.DateTimeFormat('en-US', {
			timeZone,
			year: 'numeric',
			month: '2-digit',
			day: '2-digit',
			weekday: 'short',
			hour: '2-digit',
			minute: '2-digit',
			hourCycle: 'h23'
		}).formatToParts(now);
		/** @param {string} t */
		const get = (t) => parts.find((p) => p.type === t)?.value ?? '';
		const day = get('weekday').toLowerCase().slice(0, 3);
		return {
			date: `${get('year')}-${get('month')}-${get('day')}`,
			day,
			minutes: (Number(get('hour')) % 24) * 60 + Number(get('minute'))
		};
	} catch {
		return null;
	}
}

/**
 * `YYYY-MM-DD` plus `days`.
 * @param {string} date
 * @param {number} days
 */
export function addDays(date, days) {
	const [y, m, d] = date.split('-').map(Number);
	const at = new Date(Date.UTC(y, m - 1, d + days));
	return `${at.getUTCFullYear()}-${pad(at.getUTCMonth() + 1)}-${pad(at.getUTCDate())}`;
}

/**
 * Weekday key (`mon`) of a `YYYY-MM-DD` date.
 * @param {string} date
 */
function weekdayOf(date) {
	const [y, m, d] = date.split('-').map(Number);
	return DAYS[new Date(Date.UTC(y, m - 1, d)).getUTCDay()];
}

// A date connector: a list ("26 and 27", "26, 27") or a range ("26–30",
// "November 26 – December 1", "through", "until"). A range covers every
// day in it.
const LIST_WORDS = String.raw`,|&|\band\b`;
const RANGE_WORDS = String.raw`-|‐|–|—|\bthrough\b|\bthru\b|\bto\b|\buntil\b`;
// A day number that is really a clock time ("until 2 p.m.").
const NOT_A_TIME = String.raw`(?!\s*(?::\d{2}|[ap]\.?\s?m\b|noon|o['’]clock))`;
const NAMED_DATE = new RegExp(
	String.raw`\b(${MONTH_WORDS})\.?\s+(\d{1,2})(?!\d)(?:st|nd|rd|th)?${NOT_A_TIME}(?:,?\s+(\d{4})(?!\d))?` +
		String.raw`(?:\s*(${LIST_WORDS}|${RANGE_WORDS})\s*(?:(${MONTH_WORDS})\.?\s+)?(\d{1,2})(?!\d)(?:st|nd|rd|th)?${NOT_A_TIME}(?:,?\s+(\d{4})(?!\d))?)?`,
	'gi'
);
const NUMERIC_DATE = /\b(\d{1,2})\/(\d{1,2})\/(\d{4})\b|\b(\d{4})-(\d{2})-(\d{2})\b/g;
const RANGE_GAP = new RegExp(String.raw`^\s*(?:${RANGE_WORDS})\s*$`, 'i');
/** The longest range a closure may span; a longer one is a misreading. */
const MAX_RANGE_DAYS = 62;

// A clause that reopens the office ("Reopens Thursday, November 12")
// closes nothing on its dates.
const OPENS_RE = /\b(?:re-?opens?|re-?opening|resumes?|resuming|(?:will\s+be|is|are)\s+open\b|open\s+(?:as\s+usual|normally|regular))/i;
const CLOSES_RE = /\bclos(?:e|es|ed|ing|ure|ures)\b/i;
// The one closing time a clause may state: "closes at 2:00 p.m.",
// "closing early at noon", "open until 1 p.m.".
const EARLY_RE = /\b(?:clos(?:e|es|ed|ing)\s+(?:early\s+)?(?:at\s+)?|until\s+|till\s+)(?:\d{1,2}(?::\d{2})?\s*[ap]\.?\s?m\b|noon\b)/i;

/**
 * @param {string} word a month name or abbreviation
 */
const monthOf = (word) => MONTHS.findIndex((x) => word.toLowerCase().startsWith(x)) + 1;

/**
 * `YYYY-MM-DD` of an ISO instant in `timeZone` (default Pacific: a
 * capture at 10 p.m. Pacific is that day, not the next).
 * @param {unknown} iso
 * @param {string} [timeZone]
 * @returns {string|null}
 */
export function zonedDay(iso, timeZone = DEFAULT_ZONE) {
	if (typeof iso !== 'string' || !/^\d{4}-\d{2}-\d{2}/.test(iso)) return null;
	// A bare day is already a calendar day.
	if (!/T\d{2}:\d{2}/.test(iso)) return iso.slice(0, 10);
	const at = new Date(iso);
	if (Number.isNaN(at.getTime())) return iso.slice(0, 10);
	return zonedNow(at, timeZone)?.date ?? iso.slice(0, 10);
}

/**
 * Split a text into clauses with their offsets: sentences, lines and
 * `;`. "a.m. on" and "Nov. 26" stay inside their clause.
 * @param {string} text
 * @returns {{ text: string, start: number, end: number }[]}
 */
function clauseSpans(text) {
	/** @type {{ text: string, start: number, end: number }[]} */
	const out = [];
	let from = 0;
	const push = (/** @type {number} */ to) => {
		if (text.slice(from, to).trim()) out.push({ text: text.slice(from, to), start: from, end: to });
		from = to + 1;
	};
	for (let i = 0; i < text.length; i++) {
		const c = text[i];
		if (c === ';' || c === '\n') push(i);
		else if ((c === '.' || c === '!' || c === '?') && /^\s+[A-Z(]/.test(text.slice(i + 1))) push(i);
	}
	push(text.length);
	return out;
}

/**
 * @typedef {{ m: number, d: number, y: number|null }} DatePart
 * @typedef {{ start: number, end: number, parts: DatePart[], join: 'one'|'list'|'range' }} Mention
 */

/**
 * Date mentions in a text, in order: "November 11, 2026", "Nov. 11",
 * "November 26 and 27, 2026", "November 26 – December 1, 2026",
 * "11/11/2026", "2026-11-11", and two dates joined by a range word.
 * @param {string} text
 * @returns {Mention[]}
 */
function mentionsIn(text) {
	/** @type {Mention[]} */
	const found = [];
	for (const m of text.matchAll(NAMED_DATE)) {
		const m1 = monthOf(m[1]);
		const y2 = m[7] ? Number(m[7]) : null;
		/** @type {DatePart[]} */
		const parts = [{ m: m1, d: Number(m[2]), y: m[3] ? Number(m[3]) : null }];
		/** @type {'one'|'list'|'range'} */
		let join = 'one';
		if (m[6]) {
			parts.push({ m: m[5] ? monthOf(m[5]) : m1, d: Number(m[6]), y: y2 });
			join = new RegExp(`^(?:${LIST_WORDS})$`, 'i').test(m[4].trim()) ? 'list' : 'range';
		}
		found.push({ start: m.index ?? 0, end: (m.index ?? 0) + m[0].length, parts, join });
	}
	for (const m of text.matchAll(NUMERIC_DATE)) {
		const part = m[1] ? { m: Number(m[1]), d: Number(m[2]), y: Number(m[3]) } : { m: Number(m[5]), d: Number(m[6]), y: Number(m[4]) };
		found.push({ start: m.index ?? 0, end: (m.index ?? 0) + m[0].length, parts: [part], join: 'one' });
	}
	found.sort((a, b) => a.start - b.start);
	// "11/26/2026 – 12/01/2026": two single dates joined by a range word.
	/** @type {Mention[]} */
	const out = [];
	for (const cur of found) {
		const prev = out[out.length - 1];
		if (prev && prev.join === 'one' && cur.join === 'one' && RANGE_GAP.test(text.slice(prev.end, cur.start))) {
			out[out.length - 1] = { start: prev.start, end: cur.end, parts: [prev.parts[0], cur.parts[0]], join: 'range' };
		} else {
			out.push(cur);
		}
	}
	return out;
}

/**
 * The year of a date part: its own; else that of the date it runs to
 * or is listed with (a year earlier when that date's month comes
 * before its own: "December 28 – January 2, 2027"); else the capture's
 * year, rolled forward when the day had already passed at capture;
 * else null.
 * @param {DatePart} part
 * @param {{ m: number, y: number }|null} next
 * @param {string|null} captureDay
 * @returns {number|null}
 */
function yearOf(part, next, captureDay) {
	if (part.y !== null) return part.y;
	if (next) return part.m > next.m ? next.y - 1 : next.y;
	if (!captureDay) return null;
	const y = Number(captureDay.slice(0, 4));
	const d = ymd(y, part.m, part.d);
	return d && d < captureDay ? y + 1 : y;
}

/**
 * The days a mention names, as `YYYY-MM-DD`; a range names every day
 * from its first to its last. A part printed without a year takes the
 * year printed after it (in the mention, else later in the text), else
 * the capture's.
 * @param {Mention} mention
 * @param {Mention[]} all every mention of the text, in order
 * @param {string|null} captureDay
 * @returns {string[]}
 */
function mentionDays(mention, all, captureDay) {
	const printed = all
		.filter((x) => x.start >= mention.end)
		.flatMap((x) => x.parts)
		.find((p) => p.y !== null);
	const later = printed && printed.y !== null ? { m: printed.m, y: printed.y } : null;
	const [a, b] = mention.parts;
	const yb = b ? yearOf(b, later, captureDay) : null;
	const ya = yearOf(a, b && yb !== null ? { m: b.m, y: yb } : later, captureDay);
	const first = ya !== null ? ymd(ya, a.m, a.d) : null;
	const last = b && yb !== null ? ymd(yb, b.m, b.d) : null;
	if (mention.join !== 'range' || !first || !last || last < first) {
		return [first, last].filter((d) => typeof d === 'string');
	}
	/** @type {string[]} */
	const days = [];
	for (let d = first; d <= last && days.length <= MAX_RANGE_DAYS; d = addDays(d, 1)) days.push(d);
	return days[days.length - 1] === last ? days : [first, last];
}

/**
 * @typedef {{ text: string, dates: string[], mentions: number, times: number[], opens: boolean }} ClosureClause
 */

/**
 * A closure value clause by clause: the days each clause names, how
 * many date mentions it has, its clock times, and whether it reopens
 * the office rather than closing it.
 * @param {Item} item
 * @returns {ClosureClause[]}
 */
export function closureClauses(item) {
	const text = String(item?.display ?? '');
	const captured = (item?.cites ?? []).map((c) => c?.source?.capturedAt).find((v) => typeof v === 'string') ?? null;
	const captureDay = zonedDay(captured);
	const all = mentionsIn(text);
	return clauseSpans(text).map((clause) => {
		const own = all.filter((m) => m.start >= clause.start && m.start < clause.end);
		return {
			text: clause.text,
			dates: [...new Set(own.flatMap((m) => mentionDays(m, all, captureDay)))].sort(),
			mentions: own.length,
			times: timesIn(clause.text),
			opens: OPENS_RE.test(clause.text) && !CLOSES_RE.test(clause.text)
		};
	});
}

/**
 * The instance date of a closure (`closure__2026_11_11`), or null.
 * @param {Item} item
 */
function instanceDay(item) {
	const inst = /^(\d{4})_(\d{2})_(\d{2})$/.exec(String(item?.instance ?? ''));
	return inst ? ymd(Number(inst[1]), Number(inst[2]), Number(inst[3])) : null;
}

/**
 * Every day a closure value closes the office, as `YYYY-MM-DD`: its
 * instance (`closure__2026_11_11`), then the dates in its text
 * ("November 11, 2026", "Nov. 11", "November 26 – December 1, 2026"
 * (every day of it), "11/11/2026", "2026-11-11"), except days a clause
 * says the office reopens. A date printed without a year takes a year
 * printed after it, else the year of the capture that cites it (in
 * Pacific time), rolled forward when that day had already passed at
 * capture.
 * @param {Item} item
 * @returns {string[]}
 */
export function closureDates(item) {
	/** @type {Set<string>} */
	const out = new Set();
	const inst = instanceDay(item);
	if (inst) out.add(inst);
	for (const clause of closureClauses(item)) {
		if (clause.opens) continue;
		for (const d of clause.dates) out.add(d);
	}
	return [...out].sort();
}

/**
 * What one closure value says about one day: null (not a closure day),
 * closed all day, closing early at a time, or unclear (a clock time
 * this text doesn't tie to that day alone, or a time that isn't a
 * closing time), so nothing is calculated from it.
 * @param {Item} item
 * @param {string} date `YYYY-MM-DD`
 * @returns {null|{ allDay: true }|{ closesAt: number }|{ unclear: true }}
 */
export function closureOn(item, date) {
	const clauses = closureClauses(item);
	const hits = clauses.filter((c) => !c.opens && c.dates.includes(date));
	if (hits.length === 0) {
		if (instanceDay(item) !== date) return null;
		const times = clauses.flatMap((c) => c.times);
		if (times.length === 0) return { allDay: true };
		// The text names no date of its own: its time is the instance day's.
		if (clauses.every((c) => c.mentions === 0)) {
			return times.length === 1 && clauses.some((c) => EARLY_RE.test(c.text)) ? { closesAt: times[0] } : { unclear: true };
		}
		return { unclear: true };
	}
	if (hits.length > 1) return hits.some((c) => c.times.length > 0) ? { unclear: true } : { allDay: true };
	const [hit] = hits;
	if (hit.times.length === 0) return { allDay: true };
	if (hit.mentions > 1 || hit.times.length > 1 || !EARLY_RE.test(hit.text)) return { unclear: true };
	return { closesAt: hit.times[0] };
}

/**
 * Closures from today on, soonest first, in `timeZone`: the next three
 * and the rest. A closure whose text names no readable date is kept in
 * `undated` (it may be current); one whose dates have all passed is
 * dropped.
 * @param {readonly Item[]|null|undefined} items
 * @param {Date} now
 * @param {string} [timeZone]
 * @returns {{ next: { item: Item, date: string }[], later: { item: Item, date: string }[], undated: Item[], today: string|null }}
 */
export function upcomingClosures(items, now, timeZone = DEFAULT_ZONE) {
	const today = zonedNow(now, timeZone)?.date ?? null;
	/** @type {{ item: Item, date: string }[]} */
	const dated = [];
	/** @type {Item[]} */
	const undated = [];
	for (const item of items ?? []) {
		if (!item) continue;
		const dates = closureDates(item);
		if (dates.length === 0) {
			undated.push(item);
			continue;
		}
		const next = today ? dates.find((d) => d >= today) : dates[0];
		if (next) dated.push({ item, date: next });
	}
	dated.sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0));
	return { next: dated.slice(0, 3), later: dated.slice(3), undated, today };
}

/**
 * "Wed, Nov 11" for a `YYYY-MM-DD` date (the year when it is not the
 * current one).
 * @param {string} date
 * @param {string|null} [today]
 */
export function dayLabel(date, today = null) {
	const [y, m, d] = date.split('-').map(Number);
	const sameYear = today ? today.slice(0, 4) === date.slice(0, 4) : true;
	return new Intl.DateTimeFormat('en-US', {
		weekday: 'short',
		month: 'short',
		day: 'numeric',
		...(sameYear ? {} : { year: 'numeric' }),
		timeZone: 'UTC'
	}).format(Date.UTC(y, m - 1, d, 12));
}

/**
 * Minutes after midnight as the office prints times: "3:30 p.m.",
 * "noon".
 * @param {number} minutes
 */
export function clockText(minutes) {
	const h = Math.floor(minutes / 60) % 24;
	const m = minutes % 60;
	if (h === 12 && m === 0) return 'noon';
	const hour = h % 12 === 0 ? 12 : h % 12;
	return `${hour}:${pad(m)} ${h < 12 ? 'a.m.' : 'p.m.'}`;
}

/**
 * "4:00 p.m." / "4 pm" / "16:00" in a text → minutes; every time, in order.
 * @param {string} text
 * @returns {number[]}
 */
export function timesIn(text) {
	/** @type {{ at: number, minutes: number }[]} */
	const out = [];
	const s = String(text ?? '');
	for (const m of s.matchAll(/\b(\d{1,2})(?::(\d{2}))?\s*([ap])\.?\s?m\b\.?/gi)) {
		const h = Number(m[1]);
		const min = m[2] ? Number(m[2]) : 0;
		if (h >= 1 && h <= 12 && min < 60) out.push({ at: m.index ?? 0, minutes: (h % 12) * 60 + min + (m[3].toLowerCase() === 'p' ? 720 : 0) });
	}
	for (const m of s.matchAll(/\bnoon\b/gi)) out.push({ at: m.index ?? 0, minutes: 720 });
	return out.sort((a, b) => a.at - b.at).map((t) => t.minutes);
}

/**
 * The printed cut-off rule, read for arithmetic: minutes before closing
 * ("at least 30 minutes prior to closing", "one hour before closing"),
 * or a clock time ("must arrive by 3:30 p.m."). Null when the rule
 * states neither.
 * @param {unknown} text
 * @returns {{ before: number }|{ at: number }|null}
 */
export function cutoffRule(text) {
	const s = String(text ?? '');
	const mins = /\b(\d{1,3})\s*(?:minutes?|mins?)\b[^.;]*?\b(?:before|prior\s+to)\s+(?:the\s+office\s+)?clos/i.exec(s);
	if (mins) return { before: Number(mins[1]) };
	const hour = /\b(?:an|one)\s+hour\b[^.;]*?\b(?:before|prior\s+to)\s+(?:the\s+office\s+)?clos/i.exec(s);
	if (hour) return { before: 60 };
	const by = /\b(?:by|before|no\s+later\s+than)\s+(\d{1,2}(?::\d{2})?\s*[ap]\.?\s?m\b\.?|noon)/i.exec(s);
	if (by) {
		const t = timesIn(by[1]);
		if (t.length > 0) return { at: t[0] };
	}
	return null;
}

/**
 * True when the office's timezone is quoted or staff-confirmed.
 * @param {OfficeRecord|null|undefined} office
 */
export function confirmedZone(office) {
	return (
		!!office &&
		typeof office.timezone === 'string' &&
		!!office.timezone &&
		(office.timezone_source === 'quoted' || office.timezone_source === 'staff_confirmed')
	);
}

/** @param {string} hhmm */
const minutesOf = (hhmm) => Number(hhmm.slice(0, 2)) * 60 + Number(hhmm.slice(3, 5));

/**
 * One day's opening spans of an office record.
 * @param {OfficeRecord} office
 * @param {string} day weekday key
 */
function spansOn(office, day) {
	return (office.hours ?? [])
		.filter((h) => Array.isArray(h.days) && h.days.includes(day) && /^\d{2}:\d{2}$/.test(h.open) && /^\d{2}:\d{2}$/.test(h.close))
		.map((h) => ({ open: minutesOf(h.open), close: minutesOf(h.close) }))
		.sort((a, b) => a.open - b.open);
}

/**
 * @typedef {{
 *   open: boolean,
 *   text: string,
 *   cutoff: string|null,
 *   closure: Item|null,
 *   inputs: Item[]
 * }} LiveStatus
 */

// "All offices", "both locations": a closure for every office.
const EVERY_OFFICE_RE = /\b(?:all|both|every)\s+(?:of\s+(?:our|the)\s+)?(?:offices?|locations?|branches|counters?)\b/i;

/**
 * The names a closure text can call an office record by: its city (a
 * city is distinctive; an office's own name often shares words with
 * its sibling's, such as the division's name).
 * @param {OfficeRecord|null|undefined} office
 * @returns {string[]}
 */
export function officeNames(office) {
	const city = office?.address?.city;
	return typeof city === 'string' && city.trim().length >= 3 ? [city.trim()] : [];
}

/**
 * @param {string} text
 * @param {string} name
 */
function names(text, name) {
	const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
	return new RegExp(String.raw`(?<![\p{L}\p{N}])${escaped}(?![\p{L}\p{N}])`, 'iu').test(text);
}

/**
 * Names as plain strings: records give their city, strings stay.
 * @param {readonly (OfficeRecord|string|null|undefined)[]|null|undefined} list
 * @returns {string[]}
 */
function nameList(list) {
	return (list ?? []).flatMap((o) => (typeof o === 'string' ? (o.trim().length >= 3 ? [o.trim()] : []) : officeNames(o)));
}

/**
 * True when a closure concerns an office: its text names the office
 * (`own`), or names none of the page's other offices (`others`), or
 * speaks of every office. "The Cheney office is closed…" does not close
 * Tumwater.
 * @param {Item} closure
 * @param {readonly string[]} own this office's names (city, instance name)
 * @param {readonly string[]} others the other offices' names
 */
export function closureApplies(closure, own, others) {
	const text = String(closure?.display ?? '');
	if (EVERY_OFFICE_RE.test(text)) return true;
	if (own.some((n) => names(text, n))) return true;
	const mine = new Set(own.map((n) => n.toLowerCase()));
	return !others.some((n) => !mine.has(n.toLowerCase()) && names(text, n));
}

/**
 * "Open now" and today's cut-off for an accepted office record with a
 * confirmed timezone, or null. Closures that concern this office
 * override its hours: closed all day, or closing early at the one
 * closing time a clause ties to that day, with the cut-off counted
 * back from it. The next opening skips closure days. Null (nothing
 * calculated; the card shows the office's rules as printed) when
 * today's closure is unclear: a clock time its text doesn't tie to
 * today alone, or one outside today's hours.
 *
 * `others` lists the page's other offices (records, or their names as
 * shown elsewhere on the page) and `aliases` this office's other names
 * (its instance name), so a closure that names one office closes only
 * that one.
 * @param {{ office: OfficeRecord, closures?: readonly Item[], cutoff?: Item|null, now: Date, others?: readonly (OfficeRecord|string)[], aliases?: readonly string[] }} args
 * @returns {LiveStatus|null}
 */
export function liveStatus({ office, closures = [], cutoff = null, now, others = [], aliases = [] }) {
	if (!confirmedZone(office) || (office.hours ?? []).length === 0) return null;
	const zone = /** @type {string} */ (office.timezone);
	const here = zonedNow(now, zone);
	if (!here) return null;
	const own = [...officeNames(office), ...nameList(aliases)];
	const otherNames = nameList((others ?? []).filter((o) => o !== office && (typeof o === 'string' || o?.id !== office.id)));
	const mine = (closures ?? []).filter((c) => !!c && closureApplies(c, own, otherNames));
	const rule = cutoff ? cutoffRule(cutoff.display) : null;
	/** @type {Item[]} */
	const inputs = cutoff && rule ? [cutoff] : [];
	/** @param {number} close */
	const cutoffAt = (close) => {
		if (!rule) return null;
		const at = 'before' in rule ? close - rule.before : Math.min(rule.at, close);
		return at > 0 ? clockText(at) : null;
	};
	/**
	 * What this office's closures do to one day: closed, closing early,
	 * unclear, or nothing.
	 * @param {string} date
	 * @param {{ open: number, close: number }[]} daySpans
	 * @returns {{ state: 'closed'|'unclear'|'open', close: number|null, used: Item[] }}
	 */
	const dayEffect = (date, daySpans) => {
		/** @type {Item[]} */
		const used = [];
		let closed = false;
		let unclear = false;
		/** @type {number|null} */
		let close = null;
		for (const c of mine) {
			const effect = closureOn(c, date);
			if (!effect) continue;
			used.push(c);
			if ('allDay' in effect) closed = true;
			else if ('unclear' in effect) unclear = true;
			else if (daySpans.some((s) => effect.closesAt > s.open && effect.closesAt < s.close)) {
				close = close === null ? effect.closesAt : Math.min(close, effect.closesAt);
			} else unclear = true;
		}
		return { state: closed ? 'closed' : unclear ? 'unclear' : 'open', close, used };
	};

	let spans = spansOn(office, here.day);
	const today = dayEffect(here.date, spans);
	const todayClosure = today.used[0] ?? null;
	if (today.state === 'closed') {
		return { open: false, text: 'Closed today', cutoff: null, closure: todayClosure, inputs: today.used };
	}
	if (today.state === 'unclear') return null;
	if (today.close !== null) {
		const close = today.close;
		spans = spans.filter((s) => s.open < close).map((s) => ({ open: s.open, close: Math.min(s.close, close) }));
	}
	inputs.push(...today.used);
	for (const s of spans) {
		if (here.minutes >= s.open && here.minutes < s.close) {
			return {
				open: true,
				text: `Open now · closes ${clockText(s.close)}`,
				cutoff: cutoffAt(s.close),
				closure: todayClosure,
				inputs
			};
		}
	}
	const later = spans.find((s) => s.open > here.minutes);
	if (later) {
		return {
			open: false,
			text: `Closed now · opens today at ${clockText(later.open)}`,
			cutoff: cutoffAt(later.close),
			closure: todayClosure,
			inputs
		};
	}
	for (let k = 1; k <= 14; k++) {
		const date = addDays(here.date, k);
		const daySpans = spansOn(office, weekdayOf(date));
		const first = daySpans[0];
		if (!first) continue;
		const effect = dayEffect(date, daySpans);
		if (effect.state === 'closed') continue;
		// An unclear closure ahead: say only that it is closed now.
		if (effect.state === 'unclear') break;
		if (effect.close !== null && effect.close <= first.open) continue;
		const when = k === 1 ? 'tomorrow' : dayLabel(date, here.date);
		return { open: false, text: `Closed now · opens ${when} at ${clockText(first.open)}`, cutoff: null, closure: todayClosure, inputs };
	}
	return { open: false, text: 'Closed now', cutoff: null, closure: todayClosure, inputs };
}

/**
 * Office inputs sorted by shape: `page.offices` groups ({instance,
 * name, items}) and accepted office records (OfficeSetV1, as plan.ts
 * parses them: a numeric id and an hours list). Either may arrive
 * under either prop name; anything else is dropped.
 * @param {readonly unknown[]|null|undefined} list
 * @returns {{ groups: { instance: string|null, name?: string|null, items: Item[] }[], records: OfficeRecord[] }}
 */
export function splitOffices(list) {
	/** @type {{ instance: string|null, name?: string|null, items: Item[] }[]} */
	const groups = [];
	/** @type {OfficeRecord[]} */
	const records = [];
	for (const raw of Array.isArray(list) ? list : []) {
		if (!raw || typeof raw !== 'object') continue;
		const o = /** @type {any} */ (raw);
		if (Array.isArray(o.items)) groups.push({ instance: typeof o.instance === 'string' ? o.instance : null, name: o.name ?? null, items: o.items.filter(Boolean) });
		else if (typeof o.id === 'number' && Array.isArray(o.hours)) records.push(/** @type {OfficeRecord} */ (o));
	}
	return { groups, records };
}

/**
 * "tumwater" → "Tumwater"; "new_york_city" → "New York City".
 * @param {unknown} instance
 */
export function instanceName(instance) {
	return String(instance ?? '')
		.split('_')
		.filter(Boolean)
		.map((w) => w.charAt(0).toUpperCase() + w.slice(1))
		.join(' ');
}

/**
 * True when an office record and an office instance name the same
 * place (the instance appears in the record's name or city).
 * @param {OfficeRecord} office
 * @param {string|null} instance
 */
export function sameOffice(office, instance) {
	if (!instance) return false;
	const words = instance.toLowerCase().split('_').join(' ');
	return [office.name, office.address?.city].some((v) => typeof v === 'string' && v.toLowerCase().includes(words));
}
