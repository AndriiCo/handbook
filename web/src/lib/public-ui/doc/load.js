// What the guide page reads from the public API, made safe (R2): the
// `/plan` body's authority row, office records, forms and published
// staff notes, and the index row the page falls back to when `/plan`
// carries no authority. Every reader tolerates a 404, an empty or an
// unreadable body (the SPA fallback answers HTML) and returns nothing
// rather than a guess. Pure: no window/fetch.

/** @param {unknown} v */
const str = (v) => (typeof v === 'string' ? v.trim() : '');
/** @param {unknown} v @returns {any[]} */
const rows = (v) => (Array.isArray(v) ? v.filter((x) => x && typeof x === 'object') : []);

/**
 * The authority row from a `/plan` body (`authority`, R1), or null.
 * @param {unknown} body
 * @returns {{ slug: string, name: string, region: string, country: string, kind: string, path: string, official_url: string|null }|null}
 */
export function planAuthority(body) {
	const a = /** @type {any} */ (body)?.authority;
	if (!a || typeof a !== 'object' || !str(a.slug)) return null;
	return {
		slug: str(a.slug),
		name: str(a.name),
		region: str(a.region),
		country: str(a.country),
		kind: str(a.kind),
		path: str(a.path),
		official_url: str(a.official_url) || null
	};
}

/**
 * One row of the `/authorities` index by slug (the fallback), or null.
 * @param {unknown} body
 * @param {string} slug
 */
export function indexRow(body, slug) {
	const found = rows(/** @type {any} */ (body)?.authorities).find((a) => a.slug === slug);
	if (!found) return null;
	return {
		slug,
		name: str(found.name),
		region: str(found.region),
		country: str(found.country),
		kind: str(found.kind),
		path: str(found.path),
		iso2: str(found.iso2),
		official_url: str(found.official_url) || null
	};
}

/**
 * Accepted office records (OfficeSetV1) as plan.ts reads them: a
 * numeric id, hours with HH:MM times, fact ids.
 * @param {unknown} body
 */
export function officeRecords(body) {
	return rows(/** @type {any} */ (body)?.offices)
		.filter((o) => typeof o.id === 'number' && (!o.status || o.status === 'accepted'))
		.map((o) => ({
			...o,
			name: str(o.name),
			hours: rows(o.hours).filter(
				(h) => Array.isArray(h.days) && /^\d{2}:\d{2}$/.test(h.open) && /^\d{2}:\d{2}$/.test(h.close)
			),
			fact_ids: Array.isArray(o.fact_ids) ? o.fact_ids.filter((/** @type {unknown} */ x) => typeof x === 'number') : []
		}));
}

/**
 * Active form maps from a `/plan` body.
 * @param {unknown} body
 */
export function formRows(body) {
	return rows(/** @type {any} */ (body)?.forms).filter((f) => typeof f.id === 'number' && (!f.status || f.status === 'active'));
}

/**
 * Published staff notes from a `/plan` body (`evidence[]`, R1).
 * @param {unknown} body
 */
export function evidenceRows(body) {
	return rows(/** @type {any} */ (body)?.evidence).filter(
		(n) => typeof n.id === 'number' && typeof n.kind === 'string' && (!n.status || n.status === 'published')
	);
}

/**
 * The `/facts` list of a body: an array of objects, call scripts
 * dropped (defense in depth: the API already leaves them out).
 * @param {unknown} body
 */
export function factRows(body) {
	return rows(/** @type {any} */ (body)?.facts).filter((f) => f.topic !== 'phone_script');
}

/**
 * True when the page has a value to send to (a mailing or courier
 * address resting on facts): only then are order options read.
 * @param {{ sections: { items: { slot: string|null, factIds: number[], state: string }[] }[] }} page
 */
export function hasSendSlot(page) {
	return (page?.sections ?? []).some((s) =>
		s.items.some(
			(i) =>
				(i.slot === 'mailing.usps' || i.slot === 'mailing.courier') &&
				i.factIds.length > 0 &&
				i.state !== 'missing' &&
				i.state !== 'absent'
		)
	);
}
