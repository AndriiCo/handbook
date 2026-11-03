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
			(a.country ?? '').toLowerCase().includes(q) ||
			(a.path ?? '').toLowerCase().includes(q) ||
			(Array.isArray(a.aliases) &&
				a.aliases.some((/** @type {unknown} */ alias) => String(alias).toLowerCase() === q))
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
 * `today` / `yesterday` / `N days ago` in whole CALENDAR days in the
 * viewer's zone — never 24 h periods, so a capture from late last
 * night reads `yesterday` after midnight, not `today`.
 * @param {string} iso
 * @param {number} [nowMs]
 */
export function relativeDay(iso, nowMs = Date.now()) {
	const at = new Date(iso);
	const now = new Date(nowMs);
	if (Number.isNaN(at.getTime())) return '';
	const day = (/** @type {Date} */ d) =>
		new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
	const days = Math.round((day(now) - day(at)) / 86400000);
	if (days <= 0) return 'today';
	if (days === 1) return 'yesterday';
	return `${days} days ago`;
}

/**
 * `created_at` (unix seconds) -> `approved 42s ago` / `approved 5m ago` /
 * `approved 3h ago` / `approved 4d ago`. Empty when the stamp is missing
 * or in the future. The public facts API carries no accept timestamp,
 * so the draft stamp is the approval proxy (facts are accepted shortly
 * after they are drafted).
 * @param {number} createdAtSec
 * @param {number} [nowMs]
 */
export function approvedAgo(createdAtSec, nowMs = Date.now()) {
	if (typeof createdAtSec !== 'number' || !Number.isFinite(createdAtSec) || createdAtSec <= 0)
		return '';
	const secs = Math.floor(nowMs / 1000 - createdAtSec);
	if (secs < 0) return '';
	if (secs < 60) return `approved ${secs}s ago`;
	const mins = Math.floor(secs / 60);
	if (mins < 60) return `approved ${mins}m ago`;
	const hours = Math.floor(mins / 60);
	if (hours < 48) return `approved ${hours}h ago`;
	return `approved ${Math.floor(hours / 24)}d ago`;
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
		const ago = approvedAgo(f?.created_at);
		events.push({
			id: `fact-${parent}`,
			kind: 'verified',
			text: ago ? `${topicLabel(f?.topic)} — verified quote · ${ago}` : `${topicLabel(f?.topic)} — verified quote`,
			at: cites.length > 0 ? (cites[0]?.captured_at ?? '') : '',
			fresh: false
		});
		for (const sib of f?.conflicts ?? []) {
			const underReview = sib?.status !== 'accepted';
			events.push({
				id: `conflict-${parent}-${sib?.id ?? sib?.statement}`,
				kind: 'changed',
				text: underReview
					? `${topicLabel(f?.topic)} — new proposal under review`
					: `${topicLabel(f?.topic)} — conflicting accepted report`,
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
