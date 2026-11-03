// Typed sweep targets for /workers (watch sweep): one free-form entry
// becomes a URL to fetch, an office email or an office phone number to
// save. Detection is deterministic (no model call) and mirrors the
// server gates: the email charset is first_source::valid_email, phones
// end as E.164 like first_source::normalize_phone, URLs are absolute
// http(s) like admin_workers' http_url gate (2048 chars at most).
// Admin-only: never imported from (public) or (customer) routes, and
// the one module that loads libphonenumber-js (admin bundle only).
import {
	parsePhoneNumberFromString,
	findPhoneNumbersInText,
	validatePhoneNumberLength
} from 'libphonenumber-js/min';

/**
 * @typedef {'url' | 'email' | 'phone' | 'invalid'} EntryKind
 * @typedef {object} Entry
 * @property {EntryKind} kind
 * @property {string} raw what staff typed or pasted (pulled back on edit)
 * @property {string} value normalized: URL href, lowercased email, E.164
 * @property {string} display chip text
 * @property {string} details phone extension ("ext. 2"), else ''
 * @property {string | null} country ISO 3166-1 alpha-2 of a phone
 * @property {string | null} warning amber note (still sendable)
 * @property {string | null} error why the entry cannot be sent
 * @property {boolean} sendable
 */

/** The sweep endpoint takes 1-10 items (admin_workers MAX_SWEEP_ITEMS). */
export const MAX_SWEEP_URLS = 10;
const MAX_URL_LEN = 2048;

const EMAIL = /^(?:mailto:)?([a-z0-9._%+-]{1,64})@((?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63})$/i;
const URL_SCHEME = /^https?:\/\/[^\s<>"']+$/i;
const OTHER_SCHEME = /^[a-z][a-z0-9+.-]*:\/\//i;
const PHONE_SHAPE = /^(?:tel:)?\+?[\d\s().‐-―-]{6,}\d(?:\s*(?:x|ext\.?|extension|#)\s*\d{1,6})?$/i;
const PHONE_EXT = /\s*(?:x|ext\.?|extension|#)\s*\d{1,6}$/i;
const BARE_HOST = /^(?:www\.)?(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}(?::\d{2,5})?(?:[/?#]\S*)?$/i;
const DOTTED_QUAD = /^\d{1,3}(?:\.\d{1,3}){3}/;

const URL_SPAN = /\bhttps?:\/\/[^\s<>"']+/gi;
const EMAIL_SPAN = /\b(?:mailto:)?[a-z0-9._%+-]{1,64}@(?:[a-z0-9-]+\.)+[a-z]{2,63}\b/gi;
const SEGMENT_SPLIT = /[\n\r;,\t]+/;
const PHONE_RUN = /\+?\(?\d[\d\s().‐-―-]{5,}\d(?:\s*(?:x|ext\.?|extension|#)\s*\d{1,6})?/gi;

/** @type {Intl.DisplayNames | null | undefined} */
let regionNames;

/**
 * English country name for an ISO2 code ("US" → "United States").
 * @param {string | null | undefined} code
 * @returns {string}
 */
export function countryName(code) {
	if (!code) return '';
	if (regionNames === undefined) {
		try {
			regionNames = new Intl.DisplayNames(['en'], { type: 'region' });
		} catch {
			regionNames = null;
		}
	}
	try {
		return regionNames?.of(code.toUpperCase()) ?? code.toUpperCase();
	} catch {
		return code.toUpperCase();
	}
}

/** @param {string} s */
function squash(s) {
	return s
		.normalize('NFKD')
		.replace(/[̀-ͯ]/g, '')
		.toLowerCase()
		.replace(/&/g, 'and')
		.replace(/[^a-z]+/g, '');
}

/** Pack spellings Intl does not produce ("USA", "Viet Nam" is CLDR's own). */
const COUNTRY_ALIAS = /** @type {Record<string, string>} */ ({
	usa: 'US',
	us: 'US',
	unitedstatesofamerica: 'US',
	uk: 'GB',
	greatbritain: 'GB',
	vietnam: 'VN',
	vietnamsocialistrepublicof: 'VN',
	southkorea: 'KR',
	russia: 'RU'
});

/** @type {Map<string, string> | null} */
let byName = null;

/**
 * ISO2 for a country name, from the runtime's CLDR region names.
 * @param {unknown} country
 * @returns {string | null}
 */
function codeForName(country) {
	const key = squash(String(country ?? ''));
	if (!key) return null;
	if (COUNTRY_ALIAS[key]) return COUNTRY_ALIAS[key];
	if (/^[a-z]{2}$/.test(key) && String(country).trim().length === 2) {
		const code = key.toUpperCase();
		return countryName(code) !== code ? code : null;
	}
	if (!byName) {
		byName = new Map();
		try {
			const names = new Intl.DisplayNames(['en'], { type: 'region' });
			for (let i = 0; i < 26; i++) {
				for (let j = 0; j < 26; j++) {
					const code = String.fromCharCode(65 + i, 65 + j);
					let name;
					try {
						if (Intl.getCanonicalLocales(`und-${code}`)[0] !== `und-${code}`) continue;
						name = names.of(code);
					} catch {
						continue;
					}
					if (!name || name === code || name === 'Unknown Region') continue;
					const k = squash(name);
					if (!byName.has(k)) byName.set(k, code);
				}
			}
		} catch {
			// No Intl region names: phones then need a leading +.
		}
	}
	return byName.get(key) ?? null;
}

/**
 * Default phone country for an authority row: its country name, else
 * the world-roster slug prefix (`us-wa-sos` → US), else null (a
 * number then needs its +country code).
 * @param {{ country?: unknown, slug?: unknown } | null | undefined} row
 * @returns {string | null}
 */
export function defaultCountryFor(row) {
	if (!row) return null;
	const named = codeForName(row.country);
	if (named) return named;
	const slug = typeof row.slug === 'string' ? row.slug : '';
	const usa = /^usa?-/.test(slug) || slug.startsWith('usdos');
	if (usa) return 'US';
	const prefix = /^([a-z]{2})-/.exec(slug);
	if (prefix) {
		const code = prefix[1].toUpperCase();
		if (countryName(code) !== code) return code;
	}
	return null;
}

/**
 * Trim, NFKC, strip wrapping quotes / brackets, lowercase a scheme
 * prefix (mailto:, tel:).
 * @param {string} raw
 * @returns {string}
 */
function tidy(raw) {
	let s = String(raw ?? '')
		.normalize('NFKC')
		.replace(/ /g, ' ')
		.trim();
	for (let i = 0; i < 3; i++) {
		const m = /^(["'<(\[{])([\s\S]*)([>"')\]}])$/.exec(s);
		if (!m) break;
		s = m[2].trim();
	}
	s = s.replace(/^(mailto|tel):/i, (p) => p.toLowerCase());
	return s;
}

/**
 * @param {Partial<Entry> & { kind: EntryKind, raw: string }} e
 * @returns {Entry}
 */
function entry(e) {
	return {
		value: '',
		display: e.raw,
		details: '',
		country: null,
		warning: null,
		error: null,
		sendable: e.kind !== 'invalid',
		...e
	};
}

/** @param {string} raw @param {string} error */
function invalid(raw, error) {
	return entry({ kind: 'invalid', raw, value: raw, display: raw, error, sendable: false });
}

/**
 * Trailing punctuation that is prose, not URL (and one unbalanced ')').
 * @param {string} s
 * @returns {string}
 */
function trimUrlTail(s) {
	let out = s.replace(/[.,;:!?]+$/, '');
	if (out.endsWith(')') && (out.match(/\(/g) ?? []).length < (out.match(/\)/g) ?? []).length) {
		out = out.slice(0, -1).replace(/[.,;:!?]+$/, '');
	}
	return out;
}

/**
 * @param {string} raw
 * @param {string} s tidied
 * @returns {Entry}
 */
function urlEntry(raw, s) {
	let parsed;
	try {
		parsed = new URL(s);
	} catch {
		return invalid(raw, 'Not a valid web address');
	}
	if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
		return invalid(raw, 'Only http and https addresses can be fetched');
	}
	if (!parsed.hostname.includes('.')) return invalid(raw, 'Use a full address with a domain');
	if (parsed.username || parsed.password) return invalid(raw, 'Remove the user name from the address');
	const href = parsed.href;
	if (href.length > MAX_URL_LEN) return invalid(raw, 'Longer than 2048 characters');
	const display = href.replace(/^https:\/\//, '').replace(/\/$/, '');
	return entry({
		kind: 'url',
		raw,
		value: href,
		display: parsed.protocol === 'http:' ? href.replace(/\/$/, '') : display,
		warning: parsed.protocol === 'http:' ? 'http — citations need https' : null
	});
}

/**
 * @param {string} raw
 * @param {string} s tidied
 * @param {string | null | undefined} defaultCountry
 * @returns {Entry}
 */
function phoneEntry(raw, s, defaultCountry) {
	let text = s.replace(/^tel:/, '').trim();
	if (text.startsWith('00')) text = `+${text.slice(2)}`;
	const digits = text.replace(PHONE_EXT, '').replace(/\D/g, '');
	if (digits.length < 7) return invalid(raw, 'Too short for a phone number');
	if (digits.length > 15) return invalid(raw, 'Too long for a phone number');
	const country = defaultCountry ? /** @type {any} */ (defaultCountry.toUpperCase()) : undefined;
	const hasPlus = text.startsWith('+');
	if (!hasPlus && !country) return invalid(raw, 'Add the country code, e.g. +1');
	const pn = parsePhoneNumberFromString(text, country);
	if (!pn) return invalid(raw, 'Not a phone number');
	const where = countryName(pn.country ?? country) || 'this country';
	const length = validatePhoneNumberLength(text, country);
	if (length === 'TOO_SHORT') return invalid(raw, `Too short for ${where}`);
	if (length === 'TOO_LONG') return invalid(raw, `Too long for ${where}`);
	if (!pn.isValid()) return invalid(raw, `Not a valid ${where} number`);
	return entry({
		kind: 'phone',
		raw,
		value: pn.number,
		display: pn.formatInternational(),
		details: pn.ext ? `ext. ${pn.ext}` : '',
		country: pn.country ?? null
	});
}

/**
 * Classify one typed token (first match wins: email, http(s) URL,
 * phone, bare host). Never throws; unknown input is an invalid entry
 * with a reason.
 * @param {string} raw
 * @param {string | null} [defaultCountry] ISO2 for numbers without +
 * @returns {Entry}
 */
export function classifyEntry(raw, defaultCountry = null) {
	const original = String(raw ?? '').trim();
	const s = tidy(original);
	if (!s) return invalid(original, 'Empty entry');
	if (s.includes('@') && !/^https?:\/\//i.test(s)) {
		const m = EMAIL.exec(s);
		if (!m) return invalid(original, 'Not a valid email address');
		const value = `${m[1]}@${m[2]}`.toLowerCase();
		return entry({ kind: 'email', raw: original, value, display: value });
	}
	if (/^https?:\/\//i.test(s)) {
		const t = trimUrlTail(s);
		if (!URL_SCHEME.test(t)) return invalid(original, 'Not a valid web address');
		return urlEntry(original, t);
	}
	if (OTHER_SCHEME.test(s)) return invalid(original, 'Only http and https addresses can be fetched');
	if (PHONE_SHAPE.test(s)) return phoneEntry(original, s, defaultCountry);
	if (DOTTED_QUAD.test(s)) return invalid(original, 'Use a full https:// address');
	const host = trimUrlTail(s);
	if (BARE_HOST.test(host)) return urlEntry(original, `https://${host}`);
	if (/^[+\d][\d\s().-]*$/.test(s)) return invalid(original, 'Too short for a phone number');
	return invalid(original, 'Not a URL, email or phone number');
}

/**
 * Split a pasted block (a contact card, a list) into entries. URL and
 * email spans go first and are blanked, so digits inside them never
 * read as phones; then phones; leftover words become entries only when
 * they look like a host. Label words ("Phone:", "Email") are dropped
 * and counted in `ignored`.
 * @param {string} text
 * @param {string | null} [defaultCountry]
 * @returns {{ entries: Entry[], ignored: number }}
 */
export function extractEntries(text, defaultCountry = null) {
	const src = String(text ?? '');
	const segments = src
		.split(SEGMENT_SPLIT)
		.map((s) => s.trim())
		.filter(Boolean);
	// A single token is classified as typed, so it can come back invalid.
	if (segments.length === 1 && !/\s/.test(segments[0])) {
		return { entries: [classifyEntry(segments[0], defaultCountry)], ignored: 0 };
	}
	/** @type {Entry[]} */
	const entries = [];
	let ignored = 0;
	const country = defaultCountry ? /** @type {any} */ (defaultCountry.toUpperCase()) : undefined;
	for (const segment of segments) {
		// A list item that is one token ("555-01" between commas) is read
		// as typed, so a bad entry shows with its reason; a bare label
		// word ("Email") is dropped.
		if (!/\s/.test(segment)) {
			const e = classifyEntry(segment, defaultCountry);
			if (e.sendable || /[\d@./]/.test(segment)) entries.push(e);
			else ignored += 1;
			continue;
		}
		/** @type {{ at: number, entry: Entry }[]} */
		const found = [];
		let rest = segment;
		const blank = (/** @type {RegExpExecArray} */ m) => {
			rest = rest.slice(0, m.index) + ' '.repeat(m[0].length) + rest.slice(m.index + m[0].length);
		};
		for (const m of [...segment.matchAll(URL_SPAN)]) {
			found.push({ at: m.index ?? 0, entry: classifyEntry(m[0], defaultCountry) });
			blank(/** @type {RegExpExecArray} */ (m));
		}
		for (const m of [...rest.matchAll(EMAIL_SPAN)]) {
			found.push({ at: m.index ?? 0, entry: classifyEntry(m[0], defaultCountry) });
			blank(/** @type {RegExpExecArray} */ (m));
		}
		/** @type {ReturnType<typeof findPhoneNumbersInText>} */
		let phones = [];
		try {
			phones = findPhoneNumbersInText(rest, country ? { defaultCountry: country } : undefined);
		} catch {
			phones = [];
		}
		for (const p of phones) {
			const raw = rest.slice(p.startsAt, p.endsAt);
			found.push({ at: p.startsAt, entry: classifyEntry(raw, defaultCountry) });
			rest = rest.slice(0, p.startsAt) + ' '.repeat(p.endsAt - p.startsAt) + rest.slice(p.endsAt);
		}
		// Digit runs the phone finder skipped (a foreign number without
		// its + code) still become chips, so staff see why they fail.
		for (const m of [...rest.matchAll(PHONE_RUN)]) {
			if (m[0].replace(/\D/g, '').length < 7) continue;
			found.push({ at: m.index ?? 0, entry: classifyEntry(m[0], defaultCountry) });
			blank(/** @type {RegExpExecArray} */ (m));
		}
		for (const m of rest.matchAll(/\S+/g)) {
			const word = trimUrlTail(tidy(m[0]));
			if (word && BARE_HOST.test(word) && !DOTTED_QUAD.test(word)) {
				found.push({ at: m.index ?? 0, entry: classifyEntry(word, defaultCountry) });
			} else {
				ignored += 1;
			}
		}
		found.sort((a, b) => a.at - b.at);
		for (const f of found) entries.push(f.entry);
	}
	return { entries, ignored };
}

/**
 * Identity for de-duplication: same kind and normalized value.
 * @param {Pick<Entry, 'kind' | 'value' | 'raw'>} e
 * @returns {string}
 */
export function entryKey(e) {
	return e.kind === 'invalid' ? `invalid:${e.raw}` : `${e.kind}:${e.value}`;
}

/**
 * Chip text for a contact already on file (`channel` row).
 * @param {{ kind?: unknown, value?: unknown, details?: unknown }} contact
 * @returns {{ kind: 'url' | 'email' | 'phone', display: string, title: string, key: string }}
 */
export function formatSaved(contact) {
	const kind = contact?.kind === 'phone' ? 'phone' : contact?.kind === 'email' ? 'email' : 'url';
	const value = typeof contact?.value === 'string' ? contact.value : '';
	const details = typeof contact?.details === 'string' ? contact.details.trim() : '';
	if (kind === 'phone') {
		const pn = parsePhoneNumberFromString(value);
		const shown = pn ? pn.formatInternational() : value;
		const display = details ? `${shown} ${details}` : shown;
		const where = pn?.country ? ` · ${countryName(pn.country)}` : '';
		return { kind, display, title: `${value}${where}`, key: `phone:${pn?.number ?? value}` };
	}
	if (kind === 'email') return { kind, display: value, title: value, key: `email:${value.toLowerCase()}` };
	return { kind, display: value.replace(/^https:\/\//, ''), title: value, key: `url:${value}` };
}

/**
 * @typedef {'ok' | 'saved' | 'warning' | 'over' | 'invalid'} ChipState
 * @typedef {object} SubmitPlan
 * @property {Entry[]} urls URLs the sweep fetches (at most maxUrls)
 * @property {Entry[]} contacts emails and phones to save
 * @property {Entry[]} onFile entries already on file (not re-sent)
 * @property {Entry[]} overLimit URLs past the sweep limit (kept for the next run)
 * @property {Entry[]} invalid entries that need fixing
 * @property {Map<string, ChipState>} states chip state by entryKey
 */

/**
 * What a submit sends, and each chip's state. Pure (tested): the field
 * counter and the page submit read the same plan.
 * @param {Entry[]} entries
 * @param {Set<string>} savedKeys formatSaved().key of contacts on file
 * @param {number} [maxUrls]
 * @returns {SubmitPlan}
 */
export function planSubmit(entries, savedKeys, maxUrls = MAX_SWEEP_URLS) {
	/** @type {SubmitPlan} */
	const plan = { urls: [], contacts: [], onFile: [], overLimit: [], invalid: [], states: new Map() };
	for (const e of entries) {
		const key = entryKey(e);
		if (!e.sendable) {
			plan.invalid.push(e);
			plan.states.set(key, 'invalid');
		} else if (e.kind === 'url') {
			if (plan.urls.length >= maxUrls) {
				plan.overLimit.push(e);
				plan.states.set(key, 'over');
			} else {
				plan.urls.push(e);
				plan.states.set(key, e.warning ? 'warning' : 'ok');
			}
		} else if (savedKeys.has(key)) {
			plan.onFile.push(e);
			plan.states.set(key, 'saved');
		} else {
			plan.contacts.push(e);
			plan.states.set(key, 'ok');
		}
	}
	return plan;
}

/**
 * Counter line for the field ("3 URLs to fetch · 2 contacts to save").
 * @param {{ urls: number, emails: number, phones: number, onFile: number, invalid: number, overLimit: number }} c
 * @returns {string}
 */
export function countLine(c) {
	const n = (/** @type {number} */ k, /** @type {string} */ one, /** @type {string} */ many) =>
		`${k} ${k === 1 ? one : many}`;
	const parts = [];
	if (c.urls) parts.push(`${n(c.urls, 'URL', 'URLs')} to fetch`);
	const save = [];
	if (c.emails) save.push(n(c.emails, 'email', 'emails'));
	if (c.phones) save.push(n(c.phones, 'phone', 'phones'));
	if (save.length) parts.push(`${save.join(' · ')} to save`);
	if (c.onFile) parts.push(`${c.onFile} already on file`);
	if (c.overLimit) parts.push(`${c.overLimit} over the ${MAX_SWEEP_URLS}-URL limit`);
	if (c.invalid) parts.push(`${c.invalid} ${c.invalid === 1 ? 'needs' : 'need'} fixing`);
	return parts.length ? parts.join(' · ') : 'No entries yet';
}
