import silhouettes from '../geo/silhouettes.json';

// Authority display helpers. Runtime-only (MISSION WIN W2b): nothing
// here knows a slug. Names, countries, regions, kinds and counts come
// from `GET /api/v1/authorities`; flags and silhouettes are looked up
// from those runtime values against the bundled assets.

export type AuthorityRow = {
	slug: string;
	name?: string | null;
	country?: string | null;
	region?: string | null;
	kind?: string | null;
	fact_count?: number | null;
	verified_at?: string | number | null;
	/** Canonical readable address (`usa/washington`); the slug when none. */
	path?: string | null;
	/** Other addresses that open the same dossier (`us/wa`, the slug). */
	aliases?: string[] | null;
};

/** The address to link a dossier by: its readable path, else its slug. */
export function addressOf(row: { slug: string; path?: string | null }): string {
	return typeof row.path === 'string' && row.path ? row.path : row.slug;
}

// Flag SVGs bundled in static/flags/ (flag-icons, MIT). An <img> for a
// missing file would 404 (a console error), so only these render.
const FLAG_ASSETS = new Set(['nl', 'us', 'vn']);

let byName: Map<string, string> | null = null;

function squash(s: string): string {
	return s
		.normalize('NFKD')
		.replace(/[̀-ͯ]/g, '')
		.toLowerCase()
		.replace(/&/g, 'and')
		.replace(/[^a-z]+/g, '');
}

/**
 * ISO 3166-1 alpha-2 for a country name, built from the browser's own
 * CLDR region names (Intl.DisplayNames) — no country table here.
 */
export function countryCode(country: unknown): string | null {
	const key = squash(String(country ?? ''));
	if (!key) return null;
	if (!byName) {
		byName = new Map();
		try {
			const names = new Intl.DisplayNames(['en'], { type: 'region' });
			const A = 65;
			for (let i = 0; i < 26; i++) {
				for (let j = 0; j < 26; j++) {
					const code = String.fromCharCode(A + i, A + j);
					let name: string | undefined;
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

/** Flag asset code for `authority.country`, or null (no asset → no flag). */
export function flagFor(country: unknown): string | null {
	const code = countryCode(country);
	return code && FLAG_ASSETS.has(code) ? code : null;
}

type SilKey = keyof typeof silhouettes;

/** Outline for the authority's region (a US state) or else its country. */
export function silhouetteFor(
	a: Pick<AuthorityRow, 'region' | 'country'> | null | undefined
): (typeof silhouettes)[SilKey] | null {
	if (!a) return null;
	for (const name of [a.region, a.country]) {
		const key = squash(String(name ?? '')) as SilKey;
		if (key && key in silhouettes) return silhouettes[key];
	}
	return null;
}

export type Lane = 'hague' | 'legalization' | 'states' | 'other';

/**
 * Destination grouping from `authority.kind` (staff data). No kind
 * yet → the neutral lane; the slug is never read.
 */
export function laneOf(a: Pick<AuthorityRow, 'kind'> | null | undefined): Lane {
	const kind = String(a?.kind ?? '').toLowerCase();
	if (!kind) return 'other';
	if (/state/.test(kind)) return 'states';
	if (/legali[sz]ation|consular/.test(kind)) return 'legalization';
	if (/hague|apostille|member|contracting/.test(kind)) return 'hague';
	return 'other';
}

export const LANE_LABEL: Record<Lane, string> = {
	hague: 'Hague members',
	legalization: 'Legalization',
	states: 'US states',
	other: 'Destinations'
};

/** One-word status for an index row: its kind's label, else nothing. */
export function kindLabel(a: Pick<AuthorityRow, 'kind'> | null | undefined): string {
	const lane = laneOf(a);
	if (lane === 'hague') return 'Hague member';
	if (lane === 'states') return 'US state';
	if (lane === 'legalization') return 'Legalization';
	const raw = String(a?.kind ?? '').replaceAll('_', ' ').trim();
	return raw ? raw.charAt(0).toUpperCase() + raw.slice(1) : '';
}

/** Newest citation capture across accepted facts (ISO string or ''). */
export function latestCapture(facts: unknown): string {
	let latest = '';
	for (const f of Array.isArray(facts) ? facts : []) {
		for (const c of Array.isArray(f?.citations) ? f.citations : []) {
			if (typeof c?.captured_at === 'string' && c.captured_at > latest) latest = c.captured_at;
		}
	}
	return latest;
}

/** `authority.verified_at` (ISO or epoch seconds) as an ISO string, or ''. */
export function verifiedAtOf(a: Pick<AuthorityRow, 'verified_at'> | null | undefined): string {
	const v = a?.verified_at;
	if (typeof v === 'number' && Number.isFinite(v)) return new Date(v * 1000).toISOString();
	if (typeof v === 'string' && !Number.isNaN(Date.parse(v))) return v;
	return '';
}

// First $amount in a statement + the words before it as its label.
export function parseFee(statement: unknown): { amount: string; label: string } | null {
	const text = String(statement ?? '');
	const m = text.match(/\$[\d,]+(?:\.\d{2})?/);
	if (!m || m.index === undefined) return null;
	const label = text
		.slice(0, m.index)
		.replace(/^[^a-zA-Z]+/, '')
		.trim();
	return { amount: m[0], label: label || 'Fee' };
}

// ETA from accepted facts: same-day mention wins, else the first
// business-day count, else unpublished. Never invented.
export function parseEta(facts: Array<{ statement?: unknown }>): string {
	for (const f of facts ?? []) {
		if (/same-day/i.test(String(f?.statement ?? ''))) return 'Same-day (in person)';
	}
	for (const f of facts ?? []) {
		const m = String(f?.statement ?? '').match(/(\d+)\s*(?:-|–|to)?\s*(\d+)?\s*business days/i);
		if (m) return m[2] ? `${m[1]}–${m[2]} business days` : `${m[1]} business days`;
	}
	return 'Unpublished';
}
