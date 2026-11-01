// Public handbook reads (Slice A, spec D36): live authority index +
// per-slug accepted facts over the anonymous handbook API. Injectable-io
// like customer/magic.js: no direct window/fetch references so node
// tests can drive with fakes. All fetches same-origin, no auth.

/**
 * List the authority index: GET /api/v1/authorities.
 * @param {any} io {get}
 */
export async function listAuthorities(io) {
	return io.get('/authorities');
}

/**
 * Read one dossier: GET /api/v1/authorities/{slug}/facts
 * (accepted-only + conflict banners). Guards the slug so a bad
 * value never becomes a request path.
 * @param {string} slug
 * @param {any} io {get}
 */
export async function readDossier(slug, io) {
	if (typeof slug !== 'string' || !slug || slug.includes('/')) {
		return { ok: false, status: 400, body: { error: 'bad slug' } };
	}
	return io.get('/authorities/' + encodeURIComponent(slug) + '/facts');
}

/**
 * Case-insensitive substring filter over the live index
 * (name/slug/country). Empty query returns the full list.
 * @param {any[]} authorities
 * @param {string} query
 */
export function filterAuthorities(authorities, query) {
	const q = (query ?? '').trim().toLowerCase();
	if (!q) return authorities;
	return authorities.filter(
		(a) =>
			(a.name ?? '').toLowerCase().includes(q) ||
			(a.slug ?? '').toLowerCase().includes(q) ||
			(a.country ?? '').toLowerCase().includes(q)
	);
}

/**
 * Group facts by topic, first-seen topic order.
 * @param {any[]} facts
 * @returns {{ topic: string, facts: any[] }[]}
 */
export function groupFactsByTopic(facts) {
	const groups = [];
	const byTopic = new Map();
	for (const f of facts ?? []) {
		let g = byTopic.get(f.topic);
		if (!g) {
			g = { topic: f.topic, facts: [] };
			byTopic.set(f.topic, g);
			groups.push(g);
		}
		g.facts.push(f);
	}
	return groups;
}

/**
 * `processing_time` -> `Processing time` for tab labels.
 * @param {string} topic
 */
export function topicLabel(topic) {
	const s = (topic ?? '').replaceAll('_', ' ');
	return s.charAt(0).toUpperCase() + s.slice(1);
}

/**
 * `1 fact` / `N facts` (shared literal, both pages).
 * @param {number} n
 */
export function factCount(n) {
	return `${n} ${n === 1 ? 'fact' : 'facts'}`;
}

/**
 * Step plan for the dossier "How to get your apostille" guide:
 * ordered (title, topics). Topics outside the frozen vocabulary
 * (fixtures, future migrations) never match a step — they still
 * render in their dossier sections.
 */
export const GUIDE_PLAN = [
	{ title: 'Confirm this office handles your document', topics: ['jurisdiction', 'authentication', 'legalization_chain'] },
	{ title: 'Prepare your documents', topics: ['notarization', 'translation', 'copies', 'id_requirements'] },
	{ title: 'Get the request form', topics: ['forms'] },
	{ title: 'Pay the fee', topics: ['fees', 'payment'] },
	{ title: 'Send or bring your packet', topics: ['mailing', 'po_box', 'walkin', 'appointment', 'dropbox', 'address', 'hours', 'holiday_hours', 'parking', 'route_photos'] },
	{ title: 'Wait for processing', topics: ['processing_time'] },
	{ title: 'Avoid common rejections', topics: ['rejections'] },
	{ title: 'Still have questions', topics: ['contacts', 'website'] }
];

/**
 * Build guide steps from accepted facts: the first unused fact in
 * each plan group, in plan order. Groups with no fact are skipped,
 * so the guide is exactly as complete as the dossier's evidence.
 * @param {any[]} facts
 * @returns {{ title: string, topic: string, fact: any }[]}
 */
export function buildGuide(facts) {
	const steps = [];
	for (const group of GUIDE_PLAN) {
		const fact = (facts ?? []).find((f) => group.topics.includes(f?.topic));
		if (!fact) continue;
		steps.push({ title: group.title, topic: fact.topic, fact });
	}
	return steps;
}

/**
 * Seed the dossier evidence ledger from accepted facts: one `verified`
 * event per fact, one `changed` event per conflict banner. Sibling ids
 * repeat across parents (conflict cliques reference each other), so
 * conflict ids are parent-qualified — bare sibling ids duplicated and
 * crashed the keyed ledger render (each_key_duplicate). Newest first.
 * @param {any[]} facts
 */
export function seedLedgerEvents(facts) {
	const events = [];
	const list = facts ?? [];
	for (let i = 0; i < list.length; i++) {
		const f = list[i];
		const cites = Array.isArray(f?.citations) ? f.citations : [];
		const parent = f?.id ?? `row-${i}`;
		events.push({
			id: `fact-${parent}`,
			kind: 'verified',
			text: `${topicLabel(f?.topic)} — verified quote`,
			at: cites.length > 0 ? (cites[0]?.captured_at ?? '') : '',
			fresh: false
		});
		for (const sib of f?.conflicts ?? []) {
			events.push({
				id: `conflict-${parent}-${sib?.id ?? sib?.statement}`,
				kind: 'changed',
				text: `${topicLabel(f?.topic)} — new proposal under review`,
				at: '',
				fresh: false
			});
		}
	}
	events.sort((a, b) => (a.at < b.at ? 1 : -1));
	return events;
}

/**
 * Friendly copy for public-read failures (status-mapped, never raw —
 * takes status only so server text can't leak). One mapper shared by
 * search + dossier; the 429 sentence matches magic.js exactly so the
 * chunk compresses (byte-budget trim).
 * @param {number} status
 * @param {'index'|'dossier'} kind
 */
export function publicError(status, kind) {
	if (status === 429) return 'Too many attempts — wait a minute and try again.';
	if (kind === 'dossier') {
		if (status === 404) return 'Unknown authority — it may have been renamed or removed.';
		return 'Could not load this dossier — try again.';
	}
	return 'Could not load the authority index — try again.';
}
