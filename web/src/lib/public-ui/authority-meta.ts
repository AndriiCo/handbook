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
	/** ISO 3166-1 alpha-2 (`US`), empty when the pack names none. */
	iso2?: string | null;
	/** `federal | state | regional | local | consular`, or empty. */
	level?: string | null;
	/** English display name when `name` is in another language. */
	name_en?: string | null;
	/** Other search terms ("Bayern", "Deutschland"). */
	alt_names?: string[] | null;
};

// Lanes, kind labels and flags live in plain JS (node-tested).
export {
	countryCode,
	flagFor,
	isUs,
	kindLabel,
	laneOf,
	LANES,
	LANE_LABEL,
	type Lane
} from './lanes.js';

/** The address to link a dossier by: its readable path, else its slug. */
export function addressOf(row: { slug: string; path?: string | null }): string {
	return typeof row.path === 'string' && row.path ? row.path : row.slug;
}

function squash(s: string): string {
	return s
		.normalize('NFKD')
		.replace(/[̀-ͯ]/g, '')
		.toLowerCase()
		.replace(/&/g, 'and')
		.replace(/[^a-z]+/g, '');
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
