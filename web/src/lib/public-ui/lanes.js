// Index lanes, kind labels and flags for authority rows (P2-4). Plain
// JS so node tests import it directly; `authority-meta.ts` re-exports.
// Runtime-only: every input is a value from `GET /api/v1/authorities`
// (kind, country, iso2), never a slug.

// Flag SVGs bundled in static/flags/ (flag-icons 1x1, MIT): every HCCH
// party with a pack plus Egypt. An <img> for a missing file would 404
// (a console error), so only these render.
const FLAG_ASSETS = new Set(
	(
		'ad ag al am ar at au az ba bb bd be bg bh bi bn bo br bs bw by bz ca ch ck cl cn co cr ' +
		'cv cy cz de dk dm do dz ec ee eg es fi fj fr gb gd ge gr gt gy hn hr hu id ie il in is ' +
		'it jm jp kg kn kr kz lc li lr ls lt lu lv ma mc md me mh mk mn mt mu mw mx na ni nl no ' +
		'nu nz om pa pe ph pk pl pt pw py ro rs ru rw sa sc se sg si sk sm sn sr st sv sz th tj ' +
		'tn to tr tt ua us uy uz vc ve vn vu ws xk za'
	).split(' ')
);

// HCCH and pack spellings the browser's region names do not use
// (squashed keys, see `squash`).
const NAME_ALIASES = new Map([
	['usa', 'us'],
	['us', 'us'],
	['unitedstatesofamerica', 'us'],
	['republicofkorea', 'kr'],
	['russianfederation', 'ru'],
	['republicofmoldova', 'md'],
	['czechrepublic', 'cz'],
	['caboverde', 'cv'],
	['bruneidarussalam', 'bn'],
	['saintkittsandnevis', 'kn'],
	['saintlucia', 'lc'],
	['saintvincentandthegrenadines', 'vc'],
	['vietnam', 'vn'],
	['turkiye', 'tr']
]);

/** @type {Map<string, string> | null} */
let byName = null;

/** @param {string} s */
function squash(s) {
	return s
		.normalize('NFKD')
		.replace(/[̀-ͯ]/g, '')
		.toLowerCase()
		.replace(/\([^)]*\)/g, '')
		.replace(/&/g, 'and')
		.replace(/[^a-z]+/g, '');
}

/**
 * ISO 3166-1 alpha-2 (lowercase) for a country name: the pack aliases
 * above, else the browser's own CLDR region names (Intl.DisplayNames).
 * @param {unknown} country
 * @returns {string | null}
 */
export function countryCode(country) {
	const key = squash(String(country ?? ''));
	if (!key) return null;
	const alias = NAME_ALIASES.get(key);
	if (alias) return alias;
	if (!byName) {
		byName = new Map();
		try {
			const names = new Intl.DisplayNames(['en'], { type: 'region' });
			const A = 65;
			for (let i = 0; i < 26; i++) {
				for (let j = 0; j < 26; j++) {
					const code = String.fromCharCode(A + i, A + j);
					let name;
					try {
						// Deprecated aliases (VD → VN, ZR → CD) share a
						// name with the live code; only the live one counts.
						if (Intl.getCanonicalLocales(`und-${code}`)[0] !== `und-${code}`) continue;
						name = names.of(code);
					} catch {
						continue;
					}
					if (!name || name === code || name === 'Unknown Region') continue;
					const k = squash(name);
					if (!byName.has(k)) byName.set(k, code.toLowerCase());
				}
			}
		} catch {
			// No Intl region names: no flags, never a guess.
		}
	}
	return byName.get(key) ?? null;
}

/**
 * Flag asset code for a row: its `iso2` when the API sends one, else
 * its country name; null when no asset exists (no asset → no flag).
 * @param {unknown} country
 * @param {unknown} [iso2]
 * @returns {string | null}
 */
export function flagFor(country, iso2) {
	const own = typeof iso2 === 'string' && /^[A-Za-z]{2}$/.test(iso2) ? iso2.toLowerCase() : null;
	const code = own ?? countryCode(country);
	return code && FLAG_ASSETS.has(code) ? code : null;
}

/** @typedef {'hague' | 'legalization' | 'states' | 'regions' | 'other'} Lane */

/** Lane order for the index filter chips. */
export const LANES = /** @type {Lane[]} */ (['hague', 'legalization', 'states', 'regions', 'other']);

/** @type {Record<Lane, string>} */
export const LANE_LABEL = {
	hague: 'Hague members',
	legalization: 'Legalization',
	states: 'US states',
	regions: 'States & provinces',
	other: 'Destinations'
};

/**
 * True for a US row: `iso2` when present, else the country name.
 * @param {{ country?: unknown, iso2?: unknown } | null | undefined} a
 */
export function isUs(a) {
	if (typeof a?.iso2 === 'string' && a.iso2) return a.iso2.toUpperCase() === 'US';
	return countryCode(a?.country) === 'us';
}

/**
 * Destination grouping from `authority.kind` plus the country (staff
 * data): a US state is a US state; a Mexican state or a Canadian
 * province is a state or province of its own country. No kind → the
 * neutral lane; the slug is never read.
 * @param {{ kind?: unknown, country?: unknown, iso2?: unknown } | null | undefined} a
 * @returns {Lane}
 */
export function laneOf(a) {
	const kind = String(a?.kind ?? '').toLowerCase();
	if (!kind) return 'other';
	if (/state|province/.test(kind)) return kind.includes('state') && isUs(a) ? 'states' : 'regions';
	if (/legali[sz]ation|consular/.test(kind)) return 'legalization';
	if (/hague|apostille|member|contracting/.test(kind)) return 'hague';
	return 'other';
}

/**
 * One-word status for an index row: its lane's label, else its kind.
 * @param {{ kind?: unknown, country?: unknown, iso2?: unknown } | null | undefined} a
 */
export function kindLabel(a) {
	const lane = laneOf(a);
	if (lane === 'hague') return 'Hague member';
	if (lane === 'states') return 'US state';
	if (lane === 'legalization') return 'Legalization';
	const raw = String(a?.kind ?? '').replaceAll('_', ' ').trim();
	return raw ? raw.charAt(0).toUpperCase() + raw.slice(1) : '';
}
