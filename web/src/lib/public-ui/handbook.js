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
 * Case-insensitive substring filter over the live index: name, its
 * English name and other names ("Bayern"), slug, country, region
 * ("Washington", "Jalisco") and readable path; an exact path alias
 * (`us/wa`) too. Empty query returns the full list.
 * @param {any[]} authorities
 * @param {string} query
 */
export function filterAuthorities(authorities, query) {
	const q = (query ?? '').trim().toLowerCase();
	if (!q) return authorities;
	return authorities.filter(
		(a) =>
			(a.name ?? '').toLowerCase().includes(q) ||
			(a.name_en ?? '').toLowerCase().includes(q) ||
			(Array.isArray(a.alt_names) &&
				a.alt_names.some((/** @type {unknown} */ n) => String(n).toLowerCase().includes(q))) ||
			(a.slug ?? '').toLowerCase().includes(q) ||
			(a.country ?? '').toLowerCase().includes(q) ||
			(a.region ?? '').toLowerCase().includes(q) ||
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

// Short spellings in curated packs that name the same country as a
// world pack's formal HCCH name; grouping only, never a rename.
const COUNTRY_ALIASES = new Map([['usa', 'United States of America']]);

/**
 * Group the index by `authority.country` for the world-scale home
 * index: countries A→Z (rows with no country last, as
 * `International`), authorities with the most accepted facts first,
 * then by name. Counts are the group's, never a guess; `iso2` is the
 * first one a row carries (for the flag).
 * @param {any[]} authorities
 * @returns {{ country: string, iso2: string, rows: any[], facts: number }[]}
 */
export function groupByCountry(authorities) {
	const byCountry = new Map();
	for (const a of authorities ?? []) {
		const raw = String(a?.country ?? '').trim();
		const country = COUNTRY_ALIASES.get(raw.toLowerCase()) ?? raw;
		let g = byCountry.get(country);
		if (!g) {
			g = { country, iso2: '', rows: [], facts: 0 };
			byCountry.set(country, g);
		}
		if (!g.iso2 && typeof a?.iso2 === 'string') g.iso2 = a.iso2;
		g.rows.push(a);
		g.facts += Number(a?.fact_count) || 0;
	}
	/** @type {{ country: string, iso2: string, rows: any[], facts: number }[]} */
	const groups = [...byCountry.values()];
	for (const g of groups) {
		g.rows.sort(
			(x, y) =>
				(Number(y?.fact_count) || 0) - (Number(x?.fact_count) || 0) ||
				String(x?.name ?? '').localeCompare(String(y?.name ?? ''))
		);
		if (!g.country) g.country = 'International';
	}
	return groups.sort((x, y) => {
		if (x.country === 'International') return 1;
		if (y.country === 'International') return -1;
		return x.country.localeCompare(y.country);
	});
}

/**
 * An index-row name clamped to `max` characters at a word boundary
 * (HCCH designations run to ~1,000 characters); the dossier shows the
 * full text.
 * @param {unknown} name
 * @param {number} [max]
 */
export function clampName(name, max = 120) {
	const text = String(name ?? '').trim();
	if (text.length <= max) return text;
	const cut = text.slice(0, max);
	const space = cut.lastIndexOf(' ');
	return `${(space > max / 2 ? cut.slice(0, space) : cut).replace(/[\s,;:–—-]+$/, '')}…`;
}

/**
 * `1 fact` / `N facts` (shared literal, both pages). Spanish and French
 * use the locale's digits; French counts 0 and 1 as singular.
 * @param {number} n
 * @param {'en'|'es'|'fr'} [locale]
 */
export function factCount(n, locale = 'en') {
	if (locale === 'es') return `${n.toLocaleString('es')} ${n === 1 ? 'dato' : 'datos'}`;
	if (locale === 'fr') return `${n.toLocaleString('fr')} ${n < 2 ? 'fait' : 'faits'}`;
	return `${n} ${n === 1 ? 'fact' : 'facts'}`;
}

// `relativeDay` words per site language (English is the original).
const RELATIVE_DAY = {
	en: { today: 'today', yesterday: 'yesterday', ago: (/** @type {number} */ n) => `${n} days ago` },
	es: { today: 'hoy', yesterday: 'ayer', ago: (/** @type {number} */ n) => `hace ${n.toLocaleString('es')} días` },
	fr: {
		today: 'aujourd’hui',
		yesterday: 'hier',
		ago: (/** @type {number} */ n) => `il y a ${n.toLocaleString('fr')} jours`
	}
};

/**
 * `today` / `yesterday` / `N days ago` in whole CALENDAR days in the
 * viewer's zone — never 24 h periods, so a capture from late last
 * night reads `yesterday` after midnight, not `today`.
 * @param {string} iso
 * @param {number} [nowMs]
 * @param {'en'|'es'|'fr'} [locale]
 */
export function relativeDay(iso, nowMs = Date.now(), locale = 'en') {
	const words = RELATIVE_DAY[locale] ?? RELATIVE_DAY.en;
	const at = new Date(iso);
	const now = new Date(nowMs);
	if (Number.isNaN(at.getTime())) return '';
	const day = (/** @type {Date} */ d) =>
		new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
	const days = Math.round((day(now) - day(at)) / 86400000);
	if (days <= 0) return words.today;
	if (days === 1) return words.yesterday;
	return words.ago(days);
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

// `publicError` sentences per site language. The English 429 sentence
// matches magic.js exactly so the chunk compresses (byte-budget trim);
// the Spanish and French ones match the login page's.
const PUBLIC_ERROR = {
	en: {
		rateLimited: 'Too many attempts — wait a minute and try again.',
		unknownAuthority: 'Unknown authority — it may have been renamed or removed.',
		dossier: 'Could not load this dossier — try again.',
		index: 'Could not load the authority index — try again.'
	},
	es: {
		rateLimited: 'Demasiados intentos: espere un minuto e inténtelo de nuevo.',
		unknownAuthority: 'Autoridad desconocida: puede que se haya renombrado o eliminado.',
		dossier: 'No se pudo cargar este expediente: inténtelo de nuevo.',
		index: 'No se pudo cargar el índice de autoridades: inténtelo de nuevo.'
	},
	fr: {
		rateLimited: 'Trop de tentatives : patientez une minute puis réessayez.',
		unknownAuthority: 'Autorité inconnue : elle a peut-être été renommée ou supprimée.',
		dossier: 'Impossible de charger ce dossier : réessayez.',
		index: 'Impossible de charger le répertoire des autorités : réessayez.'
	}
};

/**
 * Friendly copy for public-read failures (status-mapped, never raw —
 * takes status only so server text can't leak). One mapper shared by
 * search + dossier, in the site language (English by default).
 * @param {number} status
 * @param {'index'|'dossier'} kind
 * @param {'en'|'es'|'fr'} [locale]
 */
export function publicError(status, kind, locale = 'en') {
	const copy = PUBLIC_ERROR[locale] ?? PUBLIC_ERROR.en;
	if (status === 429) return copy.rateLimited;
	if (kind === 'dossier') {
		if (status === 404) return copy.unknownAuthority;
		return copy.dossier;
	}
	return copy.index;
}
