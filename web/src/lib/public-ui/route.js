// The guided answer (home hero → /route): a document issued in one
// country, to be used in another. Pure and node-tested like lanes.js;
// the pages pass in what they fetched (index rows, Hague members) and
// the browser bits (time zone, region names). Nothing here knows a
// fact: whether a country is a Hague party is read from accepted HCCH
// facts by the caller (HagueMap.members.ts `memberAlpha2FromFacts`).

import { countryCode } from './lanes.js';
import { DOC_CLASSES } from './location.js';

/** @typedef {'apostille' | 'legalization' | 'same' | 'unknown'} Method */
/** @typedef {{ from: string, to: string, doc: string }} RouteQuery */

/** Routing id of the HCCH status-table authority (its facts name the parties). */
export const HCCH_SLUG = 'hcch';

const PLACE = /^[a-z]{2,3}(?:\/[a-z0-9-]{1,40}){0,2}$/;
const DOC_IDS = new Set(DOC_CLASSES.map((d) => d.id));

/**
 * Country code (lowercase alpha-2) of a place address: `usa/washington`
 * → `us`, `mx` → `mx`; '' when it is not one.
 * @param {unknown} place
 */
export function codeOfPlace(place) {
	const head = String(place ?? '')
		.toLowerCase()
		.split('/')[0];
	if (head === 'usa' || head === 'us') return 'us';
	return /^[a-z]{2}$/.test(head) ? head : '';
}

/**
 * Place address of a country code: the place pages say `usa` for the
 * United States and the lowercase code elsewhere (location.js placePath).
 * @param {unknown} code
 */
export function placeOfCode(code) {
	const c = String(code ?? '').toLowerCase();
	if (c === 'us' || c === 'usa') return 'usa';
	return /^[a-z]{2}$/.test(c) ? c : '';
}

/**
 * Apostille when both countries are Hague parties, legalization when
 * either is not, `same` for a document used where it was issued, and
 * `unknown` while no accepted HCCH fact names any party (never a guess).
 * @param {string} from country code
 * @param {string} to country code
 * @param {Iterable<string> | null | undefined} members lowercase alpha-2 codes
 * @returns {Method}
 */
export function methodFor(from, to, members) {
	const a = String(from ?? '').toLowerCase();
	const b = String(to ?? '').toLowerCase();
	if (!a || !b) return 'unknown';
	if (a === b) return 'same';
	const set = new Set([...(members ?? [])].map((m) => String(m).toLowerCase()));
	if (set.size === 0) return 'unknown';
	return set.has(a) && set.has(b) ? 'apostille' : 'legalization';
}

/**
 * `?from=&to=&doc=` read back from an address; anything malformed is
 * dropped rather than echoed into the page.
 * @param {{ get(name: string): string | null }} params
 * @returns {RouteQuery}
 */
export function parseRouteQuery(params) {
	const from = String(params.get('from') ?? '').trim().toLowerCase();
	const to = codeOfPlace(String(params.get('to') ?? '').trim());
	const doc = String(params.get('doc') ?? '').trim();
	return {
		from: PLACE.test(from) && codeOfPlace(from) ? from : '',
		to,
		doc: DOC_IDS.has(doc) ? doc : ''
	};
}

/**
 * The answer's address: `{base}?from=usa&to=mx&doc=vital_record`.
 * @param {string} base resolved `/route` path
 * @param {Partial<RouteQuery>} q
 */
export function routeHref(base, q) {
	const params = new URLSearchParams();
	if (q.from) params.set('from', q.from);
	if (q.to) params.set('to', q.to);
	if (q.doc) params.set('doc', q.doc);
	const qs = params.toString();
	return qs ? `${base}?${qs}` : base;
}

// IANA zones inside the United States (the prefill's only guess).
const US_ZONES =
	/^(?:America\/(?:New_York|Detroit|Kentucky\/.+|Indiana\/.+|Chicago|Menominee|North_Dakota\/.+|Denver|Boise|Phoenix|Los_Angeles|Anchorage|Juneau|Sitka|Metlakatla|Yakutat|Nome|Adak)|Pacific\/Honolulu|US\/.+)$/;

/**
 * Where the visitor probably lives now, from the browser's time zone
 * only (a language tag says where someone is from, not where they
 * are). `us` or ''; the pickers show it and it is one tap to change.
 * @param {{ timeZone?: string } | null | undefined} env
 */
export function guessHomeCountry(env) {
	return US_ZONES.test(String(env?.timeZone ?? '')) ? 'us' : '';
}

/**
 * The picker list: every code from the world map table plus the codes
 * the live index carries, named in the site language (CLDR region
 * names) and sorted by that name. Codes the browser cannot name drop.
 * @param {Iterable<string>} codes lowercase alpha-2
 * @param {{ country?: unknown, iso2?: unknown }[] | null} rows live index
 * @param {(code: string) => string | undefined} nameOf
 * @param {string} [locale]
 * @returns {{ code: string, name: string }[]}
 */
export function countryOptions(codes, rows, nameOf, locale = 'en') {
	const all = new Set([...codes].map((c) => String(c).toLowerCase()));
	for (const r of rows ?? []) {
		const code = rowCode(r);
		if (code) all.add(code);
	}
	const out = [];
	for (const code of all) {
		if (!/^[a-z]{2}$/.test(code)) continue;
		const name = nameOf(code.toUpperCase());
		if (name && name.toUpperCase() !== code.toUpperCase()) out.push({ code, name });
	}
	return out.sort((a, b) => a.name.localeCompare(b.name, locale));
}

/**
 * Lowercase alpha-2 of an index row: its `iso2`, else its country name.
 * @param {{ country?: unknown, iso2?: unknown } | null | undefined} row
 */
export function rowCode(row) {
	if (typeof row?.iso2 === 'string' && /^[A-Za-z]{2}$/.test(row.iso2)) return row.iso2.toLowerCase();
	return countryCode(row?.country) ?? '';
}

/**
 * The destination's own guide: the one country-level row (kind
 * `country`, no region) for that code, like a "receiving authorities"
 * dossier, or among several the one with accepted facts. Several with
 * no single verified one (one row per competent authority), or none:
 * null, and the page links the country's place page instead of picking.
 * @template {{ slug: string, kind?: unknown, region?: unknown, country?: unknown, iso2?: unknown, fact_count?: unknown }} R
 * @param {R[] | null | undefined} rows
 * @param {string} code
 * @returns {R | null}
 */
export function destinationRow(rows, code) {
	const want = String(code ?? '').toLowerCase();
	if (!want) return null;
	const hits = (rows ?? []).filter(
		(r) =>
			rowCode(r) === want &&
			!String(r.region ?? '').trim() &&
			String(r.kind ?? '').toLowerCase() === 'country'
	);
	if (hits.length === 1) return hits[0];
	const verified = hits.filter((r) => Number(r.fact_count ?? 0) > 0);
	return verified.length === 1 ? verified[0] : null;
}

/**
 * True when the index has any row for this country code.
 * @param {{ country?: unknown, iso2?: unknown }[] | null | undefined} rows
 * @param {string} code
 */
export function hasCountry(rows, code) {
	const want = String(code ?? '').toLowerCase();
	return Boolean(want) && (rows ?? []).some((r) => rowCode(r) === want);
}

/**
 * Filter picker options by what the visitor typed (name or code,
 * accents ignored).
 * @param {{ code: string, name: string }[]} options
 * @param {string} query
 */
export function filterCountries(options, query) {
	const q = fold(query);
	if (!q) return options;
	const starts = [];
	const contains = [];
	for (const o of options) {
		const n = fold(o.name);
		if (n.startsWith(q) || o.code === q) starts.push(o);
		else if (n.includes(q)) contains.push(o);
	}
	return [...starts, ...contains];
}

/** @param {unknown} s */
function fold(s) {
	return String(s ?? '')
		.normalize('NFKD')
		.replace(/[̀-ͯ]/g, '')
		.toLowerCase()
		.trim();
}
