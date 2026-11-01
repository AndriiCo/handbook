import silhouettes from '../geo/silhouettes.json';

// Shared authority display facts the API does not carry yet (flags,
// silhouettes, native names, Convention dates). Real API data (names,
// fact counts) always wins; these only decorate.
export const FLAG: Record<string, string> = {
	vietnam: 'vn',
	'usa-wa-sos': 'us',
	hcch: 'nl'
};

type SilKey = keyof typeof silhouettes;

export const SILHOUETTE: Record<string, SilKey> = {
	vietnam: 'vietnam',
	'usa-wa-sos': 'washington',
	hcch: 'netherlands'
};

export const NATIVE: Record<string, { name: string; lang: string }> = {
	vietnam: { name: 'Việt Nam', lang: 'vi' }
};

export const IN_FORCE: Record<string, string> = {
	vietnam: '2026-09-11'
};

export function silhouetteFor(slug: string): (typeof silhouettes)[SilKey] | null {
	const key = SILHOUETTE[slug];
	return key ? silhouettes[key] : null;
}

export type Lane = 'hague' | 'legalization' | 'states';

// Destination grouping for the order combobox. Assumed: `usa-*` slugs
// are US states, `egypt-*` need legalization, everything else is a
// Hague member (membership is not in the API).
export function laneOf(slug: string): Lane {
	if (slug.startsWith('usa-')) return 'states';
	if (slug.startsWith('egypt-') || slug === 'egypt') return 'legalization';
	return 'hague';
}

export const LANE_LABEL: Record<Lane, string> = {
	hague: 'Hague members',
	legalization: 'Legalization',
	states: 'US states'
};

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
