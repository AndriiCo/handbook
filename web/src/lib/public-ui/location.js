// Place pages (P2-5, P4-1): read and shape `GET /api/v1/locations/{path}`.
// Injectable-io like handbook.js, so node tests drive it with fakes.

export { clampName, factCount, publicError } from './handbook.js';

/**
 * @typedef {{ slug: string, name: string, path: string, kind: string, region: string, level: string, fact_count: number }} Office
 * @typedef {{ level: string, label: string, offices: Office[] }} OfficeGroup
 * @typedef {{ path: string, label: string }} Place
 * @typedef {{
 *   doc_class: string,
 *   office: { slug: string, name: string, path: string },
 *   prereq: string,
 *   source_uri: string,
 *   fact_ids: number[],
 *   verified: boolean,
 *   inherited: boolean
 * }} Route
 * @typedef {{
 *   location: { path: string, label: string, iso2: string, subdivision: string | null },
 *   crumbs: Place[],
 *   groups: OfficeGroup[],
 *   places: Place[],
 *   routes: Route[],
 *   by_place: string[]
 * }} LocationView
 */

/**
 * The document picker's chips, in display order (the closed DocClass
 * list the packs use). Copy lives here, not in the API.
 * @type {{ id: string, label: string }[]}
 */
export const DOC_CLASSES = [
	{ id: 'vital_record', label: 'Birth, marriage or death certificate' },
	{ id: 'education', label: 'Diploma or transcript' },
	{ id: 'notarized_private', label: 'Notarized document (power of attorney, affidavit)' },
	{ id: 'state_court', label: 'State court document' },
	{ id: 'company', label: 'Company document' },
	{ id: 'state_agency', label: 'State agency document' },
	{ id: 'federal_agency', label: 'Federal agency document' },
	{ id: 'fbi_check', label: 'FBI background check' },
	{ id: 'federal_court', label: 'Federal court document' }
];

/**
 * What the picker answers for one document class: the route here, or
 * "pick a state" when this country routes it by state, or nothing.
 * @param {LocationView} view
 * @param {string} docClass
 * @returns {{ kind: 'route', route: Route } | { kind: 'by_place' } | { kind: 'none' }}
 */
export function answerFor(view, docClass) {
	const route = view.routes.find((r) => r.doc_class === docClass);
	if (route) return { kind: 'route', route };
	if (view.by_place.includes(docClass)) return { kind: 'by_place' };
	return { kind: 'none' };
}

/**
 * Place page address of a country: `usa` for the United States, else
 * the lowercase ISO code; null when the rows carry neither.
 * @param {unknown} iso2
 * @param {unknown} [country]
 * @returns {string | null}
 */
export function placePath(iso2, country) {
	const code = typeof iso2 === 'string' ? iso2.trim().toUpperCase() : '';
	const us = /^(usa|us|united states|united states of america)$/i.test(String(country ?? '').trim());
	if (code === 'US' || (!code && us)) return 'usa';
	return /^[A-Z]{2}$/.test(code) ? code.toLowerCase() : null;
}

/**
 * Host of a citation URL for its link text (`hcch.net`).
 * @param {string} uri
 */
export function sourceHost(uri) {
	try {
		return new URL(uri).hostname.replace(/^www\./, '');
	} catch {
		return '';
	}
}

/**
 * GET /api/v1/locations/{path}. Guards the path so a bad value never
 * becomes a request path.
 * @param {string} path
 * @param {any} io {get}
 */
export async function readLocation(path, io) {
	const clean = String(path ?? '')
		.split('/')
		.filter(Boolean);
	if (clean.length === 0 || clean.length > 3 || clean.some((s) => !/^[a-z0-9_-]{1,48}$/i.test(s))) {
		return { ok: false, status: 404, body: { error: 'bad path' } };
	}
	return io.get('/locations/' + clean.map(encodeURIComponent).join('/'));
}

/** @param {unknown} v */
const str = (v) => (typeof v === 'string' ? v : '');

/**
 * Shape an API body into a view, dropping anything malformed.
 * @param {any} body
 * @returns {LocationView | null}
 */
export function parseLocation(body) {
	const loc = body?.location;
	if (!loc || typeof loc.path !== 'string' || typeof loc.label !== 'string') return null;
	/** @param {any} p */
	const place = (p) => ({ path: str(p?.path), label: str(p?.label) });
	return {
		location: {
			path: loc.path,
			label: loc.label,
			iso2: str(loc.iso2),
			subdivision: typeof loc.subdivision === 'string' ? loc.subdivision : null
		},
		crumbs: (Array.isArray(body.crumbs) ? body.crumbs : []).map(place).filter((/** @type {Place} */ p) => p.path && p.label),
		groups: (Array.isArray(body.groups) ? body.groups : [])
			.map((/** @type {any} */ g) => ({
				level: str(g?.level),
				label: str(g?.label),
				offices: (Array.isArray(g?.offices) ? g.offices : [])
					.filter((/** @type {any} */ o) => typeof o?.slug === 'string' && o.slug)
					.map((/** @type {any} */ o) => ({
						slug: o.slug,
						name: str(o.name) || o.slug,
						path: str(o.path) || o.slug,
						kind: str(o.kind),
						region: str(o.region),
						level: str(o.level),
						fact_count: Number(o.fact_count) || 0
					}))
			}))
			.filter((/** @type {OfficeGroup} */ g) => g.level && g.offices.length > 0),
		places: (Array.isArray(body.places) ? body.places : []).map(place).filter((/** @type {Place} */ p) => p.path && p.label),
		routes: (Array.isArray(body.routes) ? body.routes : [])
			.filter(
				(/** @type {any} */ r) =>
					typeof r?.doc_class === 'string' && typeof r?.office?.slug === 'string'
			)
			.map((/** @type {any} */ r) => ({
				doc_class: r.doc_class,
				office: {
					slug: r.office.slug,
					name: str(r.office.name) || r.office.slug,
					path: str(r.office.path) || r.office.slug
				},
				prereq: str(r.prereq),
				source_uri: /^https?:\/\//.test(str(r.source_uri)) ? r.source_uri : '',
				fact_ids: (Array.isArray(r.fact_ids) ? r.fact_ids : []).filter(
					(/** @type {unknown} */ id) => typeof id === 'number'
				),
				verified: r.verified === true,
				inherited: r.inherited === true
			})),
		by_place: (Array.isArray(body.by_place) ? body.by_place : []).filter(
			(/** @type {unknown} */ c) => typeof c === 'string'
		)
	};
}

/**
 * "state" or "province": what the sub-places of a country are called,
 * from the kinds of its state-level offices.
 * @param {LocationView} view
 */
export function placeWord(view) {
	const state = view.groups.find((g) => g.level === 'state');
	return state?.offices.some((o) => o.kind === 'province') ? 'province' : 'state';
}
