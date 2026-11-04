// One field-level conflict rule (plan §5.1): the JS twin of
// `split_statement`, `value_tokens`, `statements_disagree` and
// `facts_disagree` in crates/handbook-store/src/db.rs. Both twins run
// the golden cases in docs/contracts/conflict-cases.json, so a change
// here lands there too. Pure: no window/fetch, so node tests drive it.

import { parseStatement } from './guide.js';

/**
 * Topics whose facts never conflict: `service_notice` facts are dated
 * announcements that stack, so a second notice is news, not a
 * disagreement.
 */
export const NON_CONFLICTING_TOPICS = ['service_notice'];

/**
 * Topics that share one USPS identity. Two facts here that both name a
 * PO box are compared on box and ZIP whatever their keys or topic, so
 * a real box dispute cannot hide behind ad-hoc keys or the
 * `po_box`/`mailing` split. Courier street facts (no box) never are.
 */
export const USPS_TOPICS = ['mailing', 'po_box'];

/**
 * Split "{field}: {value}" into its key and value (the guide's
 * `parseStatement` grammar); anything else is unkeyed. An instance
 * suffix (`weekday__tumwater`) is part of the key.
 * @param {string} statement
 * @returns {{ key: string|null, value: string }}
 */
export function splitStatement(statement) {
	const { fieldKey, value } = parseStatement(statement);
	return { key: fieldKey, value };
}

/**
 * True when two statements of one topic describe the same field:
 * equal keys (instance suffix included), or either side unkeyed (a
 * legacy pin is compared with everything in its topic).
 * @param {string} a
 * @param {string} b
 */
export function sameField(a, b) {
	return !otherFields(splitStatement(a).key, splitStatement(b).key);
}

/**
 * Both keyed with different keys: different fields.
 * @param {string|null} keyA
 * @param {string|null} keyB
 */
function otherFields(keyA, keyB) {
	return keyA !== null && keyB !== null && keyA !== keyB;
}

/** Weekday names and abbreviations; the first three letters name the day. */
const DAY_NAMES =
	'monday|tuesday|wednesday|thursday|friday|saturday|sunday' +
	'|mon|tues|tue|weds|wed|thurs|thur|thu|fri|sat|sun';

/** Month names and abbreviations; the first three letters name the month. */
const MONTH_NAMES =
	'january|february|march|april|may|june|july|august' +
	'|september|october|november|december' +
	'|jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|nov|dec';

const WEEKDAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];

const MONTHS = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'];

/** Every dash variant the token and text folds turn into `-`. */
const DASH = /^[\u2010-\u2014\u2212]$/;

/**
 * One token pattern over the `tokenText` haystack, the same ASCII
 * regex source as the Rust twin.
 * @param {string} pattern
 */
const tokenRe = (pattern) => new RegExp(pattern, 'g');

const BOX_RE = tokenRe(String.raw`\b(?:p ?\.? ?o ?\.?|post office) ?box ?(?::|#|no\.?|number)? ?([0-9]+)\b`);
const AMT_RE = tokenRe(String.raw`\$ ?([0-9]{1,3}(?:,[0-9]{3})+|[0-9]+)(?:\.([0-9]{1,2}))?`);
const TEL_RE = tokenRe(
	String.raw`(?:^|[^0-9])((?:\+?1[ .-]?)?(?:\(([0-9]{3})\) ?|([0-9]{3})[ .-]?)([0-9]{3})[ .-]?([0-9]{4}))\b`
);
const DATE_MDY_RE = tokenRe(String.raw`\b(${MONTH_NAMES})\.? ([0-9]{1,2})(?:st|nd|rd|th)?,? ([0-9]{4})\b`);
const DATE_DMY_RE = tokenRe(String.raw`\b([0-9]{1,2})(?:st|nd|rd|th)? (${MONTH_NAMES})\.?,? ([0-9]{4})\b`);
const DATE_NUM_RE = tokenRe(String.raw`\b([0-9]{1,2})[/]([0-9]{1,2})[/]([0-9]{4})\b`);
const DATE_ISO_RE = tokenRe(String.raw`\b([0-9]{4})-([0-9]{1,2})-([0-9]{1,2})\b`);
const ZIP_RE = tokenRe(String.raw`\b([0-9]{5})(?:-([0-9]{4}))?\b`);
const TIME_AMPM_RE = tokenRe(String.raw`\b([0-9]{1,2})(?::([0-9]{2}))? ?([ap])\.? ?m\b\.?`);
const TIME_NOON_RE = tokenRe(String.raw`\b(?:12 )?noon\b`);
const TIME_24H_RE = tokenRe(String.raw`\b([0-9]{1,2}):([0-9]{2})\b`);
const DAYS_RE = tokenRe(String.raw`\b([0-9]{1,3})(?: ?(?:-|to) ?([0-9]{1,3}))? (?:business|working) days?\b`);
const DOW_RANGE_RE = tokenRe(
	String.raw`\b(${DAY_NAMES})s?\b\.? ?(?:-|to|through|thru) ?(${DAY_NAMES})s?\b\.?`
);
const DOW_RE = tokenRe(String.raw`\b(${DAY_NAMES})s?\b\.?`);

/**
 * The token haystack: lowercased, every dash variant folded to `-`,
 * every other non-ASCII or non-printing character (NBSP, newline) to
 * one space, runs collapsed and trimmed. Pure ASCII, so JS UTF-16
 * offsets and Rust byte offsets agree.
 * @param {string} v
 */
function tokenText(v) {
	let out = '';
	for (const ch of String(v ?? '').toLowerCase()) {
		const c = DASH.test(ch) ? '-' : ch >= '!' && ch <= '~' ? ch : ' ';
		if (c !== ' ' || !(out === '' || out.endsWith(' '))) out += c;
	}
	return out.endsWith(' ') ? out.slice(0, -1) : out;
}

/**
 * Read every match of `re` in `text`, then blank each match's `body`
 * group to spaces, so later kinds never re-read it (a box number is
 * not also a ZIP). Returns the blanked text. The body group always
 * ends where the match ends.
 * @param {string} text
 * @param {RegExp} re
 * @param {number} body
 * @param {(m: RegExpMatchArray) => void} read
 */
function take(text, re, body, read) {
	/** @type {[number, number][]} */
	const spans = [];
	for (const m of text.matchAll(re)) {
		read(m);
		const end = (m.index ?? 0) + m[0].length;
		spans.push([end - (m[body] ?? '').length, end]);
	}
	let out = text;
	for (const [start, end] of spans) out = out.slice(0, start) + ' '.repeat(end - start) + out.slice(end);
	return out;
}

/**
 * `"007"` -> `"7"`, `"000"` -> `"0"`.
 * @param {string} digits
 */
function trimZeros(digits) {
	return digits.replace(/^0+/, '') || '0';
}

/** @param {number} n */
const pad2 = (n) => String(n).padStart(2, '0');

/**
 * `date:YYYY-MM-DD` for a calendar-shaped month/day, else null.
 * @param {string} year
 * @param {number} month
 * @param {number} day
 */
function dateToken(year, month, day) {
	return month >= 1 && month <= 12 && day >= 1 && day <= 31 ? `date:${year}-${pad2(month)}-${pad2(day)}` : null;
}

/**
 * Month number from a month-name match.
 * @param {string} name
 */
function monthNumber(name) {
	return MONTHS.findIndex((m) => name.startsWith(m)) + 1;
}

/**
 * Weekday index (Monday = 0) from a day-name match.
 * @param {string} name
 */
function weekday(name) {
	return Math.max(0, WEEKDAYS.findIndex((d) => name.startsWith(d)));
}

/**
 * Typed tokens of one statement value, each `kind:value`, sorted:
 * `box:40228` (PO boxes), `amt:1500` (dollars in cents), `tel:3607250377`
 * (10-digit phones), `date:2026-11-11` (full dates), `zip:98501` (a
 * ZIP+4 yields `zip:98504-0228` and also its bare `zip:98504`, so under
 * the identity rule `98504` agrees with `98504-0228` while two different
 * +4 codes still disagree), `time:16:00`
 * (a.m./p.m., noon, 24h; a bare `h:mm` with no a.m./p.m. and an hour of
 * 1-12 written without a leading zero reads either way, as
 * `time:03:30|15:30`, see `timesAgree`), `days:7-10` (business/working days) and
 * `dow:mon` (weekdays, ranges expanded). Kinds read in that order, each
 * blanking what it took.
 * @param {string} v
 * @returns {Set<string>}
 */
export function valueTokens(v) {
	let text = tokenText(v);
	/** @type {Set<string>} */
	const out = new Set();
	/** @param {string|null} t */
	const add = (t) => {
		if (t) out.add(t);
	};
	text = take(text, BOX_RE, 0, (m) => add(`box:${m[1]}`));
	text = take(text, AMT_RE, 0, (m) => {
		const dollars = m[1].replaceAll(',', '');
		const cents = (m[2] ?? '').padEnd(2, '0');
		add(`amt:${trimZeros(dollars + cents)}`);
	});
	text = take(text, TEL_RE, 1, (m) => add(`tel:${m[2] ?? m[3]}${m[4]}${m[5]}`));
	text = take(text, DATE_MDY_RE, 0, (m) => add(dateToken(m[3], monthNumber(m[1]), Number(m[2]))));
	text = take(text, DATE_DMY_RE, 0, (m) => add(dateToken(m[3], monthNumber(m[2]), Number(m[1]))));
	text = take(text, DATE_NUM_RE, 0, (m) => add(dateToken(m[3], Number(m[1]), Number(m[2]))));
	text = take(text, DATE_ISO_RE, 0, (m) => add(dateToken(m[1], Number(m[2]), Number(m[3]))));
	text = take(text, ZIP_RE, 0, (m) => {
		add(`zip:${m[1]}`);
		if (m[2] !== undefined) add(`zip:${m[1]}-${m[2]}`);
	});
	text = take(text, TIME_AMPM_RE, 0, (m) => {
		const hour = Number(m[1]);
		const minute = m[2] === undefined ? 0 : Number(m[2]);
		if (hour >= 1 && hour <= 12 && minute < 60) {
			add(`time:${pad2((hour % 12) + (m[3] === 'p' ? 12 : 0))}:${pad2(minute)}`);
		}
	});
	text = take(text, TIME_NOON_RE, 0, () => add('time:12:00'));
	text = take(text, TIME_24H_RE, 0, (m) => {
		const hour = Number(m[1]);
		const minute = Number(m[2]);
		if (hour >= 1 && hour <= 12 && minute < 60 && !m[1].startsWith('0')) {
			const am = hour % 12;
			add(`time:${pad2(am)}:${pad2(minute)}|${pad2(am + 12)}:${pad2(minute)}`);
		} else if (hour < 24 && minute < 60) {
			add(`time:${pad2(hour)}:${pad2(minute)}`);
		}
	});
	text = take(text, DAYS_RE, 0, (m) =>
		add(m[2] === undefined ? `days:${Number(m[1])}` : `days:${Number(m[1])}-${Number(m[2])}`)
	);
	text = take(text, DOW_RANGE_RE, 0, (m) => {
		const from = weekday(m[1]);
		const to = weekday(m[2]);
		for (let step = 0; step <= (to + 7 - from) % 7; step++) add(`dow:${WEEKDAYS[(from + step) % 7]}`);
	});
	take(text, DOW_RE, 0, (m) => add(`dow:${WEEKDAYS[weekday(m[1])]}`));
	return new Set([...out].sort());
}

/**
 * Token kinds, and how two sides' sets of one kind agree: identities
 * (box, ZIP, phone) when either set contains the other, so a less
 * specific value agrees; measures (amount, time, day count, date,
 * weekday) only when the sets are equal (times once each bare clock
 * time takes the reading the other side names, `timesAgree`).
 * @type {[string, boolean][]}
 */
const TOKEN_KINDS = [
	['box', true],
	['zip', true],
	['tel', true],
	['amt', false],
	['time', false],
	['days', false],
	['date', false],
	['dow', false]
];

/**
 * @param {Set<string>} tokens
 * @param {string} kind
 * @returns {Set<string>}
 */
function ofKind(tokens, kind) {
	return new Set([...tokens].filter((t) => t.slice(0, t.indexOf(':')) === kind));
}

/**
 * @param {Set<string>} a
 * @param {Set<string>} b
 */
const isSubset = (a, b) => [...a].every((t) => b.has(t));

/**
 * Two identity sets disagree when both are present and neither
 * contains the other.
 * @param {Set<string>} a
 * @param {Set<string>} b
 */
function identitiesDisagree(a, b) {
	return a.size > 0 && b.size > 0 && !isSubset(a, b) && !isSubset(b, a);
}

const ALNUM = /^[\p{Alphabetic}\p{N}]$/u;

/**
 * Token-free values compare as text: whitespace-normalized and
 * lowercased (Rust `normalize_statement`) after folding punctuation.
 * Letters, digits and `$` stay; `.`, `:` and `-` stay only between two
 * digits; other periods and apostrophes vanish (`P.O.` = `PO`);
 * anything else breaks words.
 * @param {string} v
 */
function foldText(v) {
	const chars = Array.from(String(v ?? '').toLowerCase(), (c) => (DASH.test(c) ? '-' : c));
	/** @param {number} i */
	const digit = (i) => i >= 0 && i < chars.length && chars[i] >= '0' && chars[i] <= '9';
	let out = '';
	for (let i = 0; i < chars.length; i++) {
		const c = chars[i];
		if (ALNUM.test(c) || c === '$' || ((c === '.' || c === ':' || c === '-') && digit(i - 1) && digit(i + 1))) {
			out += c;
		} else if (c !== '.' && c !== "'" && c !== '\u2019') {
			out += ' ';
		}
	}
	return out.split(/\s+/).filter(Boolean).join(' ').toLowerCase();
}

/**
 * One side's time tokens with each bare clock time (`time:03:30|15:30`)
 * read as the reading the other side names: its p.m. reading if the
 * other side has it, else its a.m. one, else as written (so it agrees
 * only with the same bare time).
 * @param {Set<string>} own
 * @param {Set<string>} other
 * @returns {Set<string>}
 */
function readTimes(own, other) {
	return new Set(
		[...own].map((token) => {
			const bar = token.indexOf('|');
			if (!token.startsWith('time:') || bar < 0 || other.has(token)) return token;
			const am = `time:${token.slice('time:'.length, bar)}`;
			const pm = `time:${token.slice(bar + 1)}`;
			return [pm, am].find((reading) => other.has(reading)) ?? token;
		})
	);
}

/**
 * Two sides' time sets agree when they are equal once each bare clock
 * time takes the reading the other side names (`readTimes`): `3:30`
 * agrees with `3:30 p.m.` and with `03:30`, and `8:00-4:30` with
 * `8 a.m. to 4:30 p.m.`, while `3:30` still disagrees with `4:30 p.m.`.
 * @param {Set<string>} a
 * @param {Set<string>} b
 */
function timesAgree(a, b) {
	const readA = readTimes(a, b);
	const readB = readTimes(b, a);
	return readA.size === readB.size && isSubset(readA, readB);
}

/**
 * The value rule over each side's `valueTokens`: every token kind both
 * sides carry must agree (see `TOKEN_KINDS`); sides that share no kind
 * compare as folded text.
 * @param {string} a
 * @param {Set<string>} tokensA
 * @param {string} b
 * @param {Set<string>} tokensB
 */
function valuesDisagree(a, tokensA, b, tokensB) {
	let shared = false;
	for (const [kind, identity] of TOKEN_KINDS) {
		const setA = ofKind(tokensA, kind);
		const setB = ofKind(tokensB, kind);
		if (setA.size === 0 || setB.size === 0) continue;
		shared = true;
		const agree = identity
			? !identitiesDisagree(setA, setB)
			: kind === 'time'
				? timesAgree(setA, setB)
				: setA.size === setB.size && isSubset(setA, setB);
		if (!agree) return true;
	}
	return !shared && foldText(a) !== foldText(b);
}

/**
 * True when two statements of ONE topic disagree: the same field
 * (`sameField`) with values that disagree (every shared token kind
 * must agree, else folded text must match). Different fields never
 * disagree. Comparing live facts, use `factsDisagree`, which adds the
 * topic rules.
 * @param {string} a
 * @param {string} b
 */
export function statementsDisagree(a, b) {
	const sa = splitStatement(a);
	const sb = splitStatement(b);
	return !otherFields(sa.key, sb.key) && valuesDisagree(sa.value, valueTokens(sa.value), sb.value, valueTokens(sb.value));
}

/**
 * The USPS identity rule: both sides name a PO box, and their box sets
 * differ or their ZIP sets disagree.
 * @param {Set<string>} tokensA
 * @param {Set<string>} tokensB
 */
function boxesDisagree(tokensA, tokensB) {
	const boxA = ofKind(tokensA, 'box');
	const boxB = ofKind(tokensB, 'box');
	if (boxA.size === 0 || boxB.size === 0) return false;
	const sameBoxes = boxA.size === boxB.size && isSubset(boxA, boxB);
	return !sameBoxes || identitiesDisagree(ofKind(tokensA, 'zip'), ofKind(tokensB, 'zip'));
}

/**
 * The conflict rule for two live facts of one authority.
 * `NON_CONFLICTING_TOPICS` never conflict; two `USPS_TOPICS` facts
 * that both name a PO box disagree on box or ZIP whatever their keys
 * or topic; otherwise only facts of one topic compare, by
 * `statementsDisagree`. Each side is tokenized at most once.
 * @param {string} topicA
 * @param {string} a
 * @param {string} topicB
 * @param {string} b
 */
export function factsDisagree(topicA, a, topicB, b) {
	if (NON_CONFLICTING_TOPICS.includes(topicA) || NON_CONFLICTING_TOPICS.includes(topicB)) return false;
	const usps = USPS_TOPICS.includes(topicA) && USPS_TOPICS.includes(topicB);
	const sa = splitStatement(a);
	const sb = splitStatement(b);
	const oneField = topicA === topicB && !otherFields(sa.key, sb.key);
	if (!usps && !oneField) return false;
	const tokensA = valueTokens(sa.value);
	const tokensB = valueTokens(sb.value);
	return (
		(usps && boxesDisagree(tokensA, tokensB)) || (oneField && valuesDisagree(sa.value, tokensA, sb.value, tokensB))
	);
}
