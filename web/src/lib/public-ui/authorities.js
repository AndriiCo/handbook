// Placeholder authority index for the public shell (roadmap item 8).
// Synthetic only — canonical values never leak into facts. Replaced by
// the publish lane (xtask build-handbook) once accepted facts exist.
// EN-only until the Week 3+ i18n pass.

/** @typedef {{ slug: string, name: string, kind: string, verified: string | null, tier: string }} Authority */

export const AUTHORITIES = [
	{
		slug: 'placeholder-one',
		name: 'Placeholder Authority One',
		kind: 'Secretary of State · Apostille',
		verified: null,
		tier: 'Unverified'
	},
	{
		slug: 'placeholder-two',
		name: 'Placeholder Authority Two',
		kind: 'Consular office · Authentication',
		verified: null,
		tier: 'Unverified'
	},
	{
		slug: 'placeholder-three',
		name: 'Placeholder Authority Three',
		kind: 'Secretary of State · Apostille',
		verified: null,
		tier: 'Unverified'
	}
];

export const RECENT_CHANGES = [
	{
		text: 'Placeholder change feed — accepted facts will appear here with citations.',
		date: '2026-10-27',
		source: 'handbook build (pending)'
	}
];

/**
 * Case-insensitive substring search over the placeholder index.
 * @param {string} query
 * @returns {Authority[]}
 */
export function searchAuthorities(query) {
	const q = (query ?? '').trim().toLowerCase();
	if (!q) return [];
	return AUTHORITIES.filter(
		(a) => a.name.toLowerCase().includes(q) || a.kind.toLowerCase().includes(q)
	);
}

/**
 * @param {string} slug
 * @returns {Authority | null}
 */
export function findAuthority(slug) {
	return AUTHORITIES.find((a) => a.slug === slug) ?? null;
}
