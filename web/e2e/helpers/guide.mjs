// Shared mocks for the customer guide specs (cuj-dossier, print,
// gp2-page): the dossier fixtures in web/e2e/fixtures/dossier/ (the same
// files the presenter's node tests read) served the way the public API
// answers them, plus the live bus through page.routeWebSocket.
//
// The API mock answers every read the guide makes with 200, as
// production does (/plan, /changes, /p2/orders/options and
// /notify/status always answer 200), and anything else with 404, so a
// spec can assert which reads happened and that each one succeeded.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { factsDisagree } from '../../src/lib/public-ui/compare.js';
import { publicFacts } from '../../../xtask/ops/gp2-page.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));

/** The public build the specs serve (`npm run build`). */
export const BUILD = join(HERE, '..', '..', 'build');

/** Wednesday 2026-11-04 10:00 PT, the moment wa-guide.json is read at. */
export const NOW = new Date('2026-11-04T18:00:00Z');

/**
 * One dossier fixture, parsed fresh (specs may change their copy).
 * @param {'wa-guide' | 'vietnam-prod' | 'empty'} name
 */
export function dossier(name) {
	return JSON.parse(readFileSync(join(HERE, '..', 'fixtures', 'dossier', `${name}.json`), 'utf8'));
}

/**
 * R1's field-level conflict flags (compare.js is the server rule's
 * twin): a public fact is flagged when another public fact of the
 * authority disagrees with it. Production fixtures read before R1
 * carry the old topic-level flags; this models the same facts after R1.
 * @param {any[]} facts
 */
export function withR1Flags(facts) {
	const live = publicFacts(facts);
	return facts.map((f) => ({
		...f,
		conflict: live.some((g) => g !== f && factsDisagree(f.topic, f.statement, g.topic, g.statement))
	}));
}

/**
 * The fixture's authority row, wherever the fixture keeps it.
 * @param {any} f
 */
export function rowOf(f) {
	return f.plan?.authority ?? f.authority ?? f.row ?? null;
}

/**
 * Route-mock one office's public reads.
 *
 * @param {import('@playwright/test').Page} page
 * @param {any} f a dossier fixture (`dossier(name)`), possibly changed
 * @param {{
 *   plan?: any,
 *   options?: any,
 *   changes?: any[],
 *   notify?: boolean,
 *   index?: any[],
 * }} [opts]
 *   - plan: the `/plan` body (default the fixture's, or an empty one);
 *   - options: the `/p2/orders/options` body (default the fixture's, or
 *     no options);
 *   - changes: the `/changes` notes (default none);
 *   - notify: `/notify/status` enabled (default false);
 *   - index: the `/authorities` rows (default the fixture's row).
 * @returns {Promise<{ reads: { path: string, status: number }[], setFacts: (body: any) => void, setPlan: (body: any) => void }>}
 */
export async function mockGuide(page, f, opts = {}) {
	const reads = [];
	const row = rowOf(f);
	let facts = f.facts;
	let plan =
		opts.plan ??
		f.plan ?? {
			slug: f.slug,
			authority: null,
			plan: null,
			facts: [],
			offices: [],
			forms: [],
			provenance: null,
			evidence: []
		};
	const index = opts.index ?? (row ? [row] : []);
	const options = opts.options ?? f.options ?? { slug: f.slug, options: [] };
	const changes = opts.changes ?? [];
	await page.route('**/apostille/api/v1/**', (route) => {
		const url = new URL(route.request().url());
		const path = url.pathname.replace(/^.*\/api\/v1/, '');
		const answer = (json, status = 200) => {
			reads.push({ path, status });
			return route.fulfill({ status, json });
		};
		if (route.request().method() !== 'GET') return answer({ error: 'mock' }, 404);
		if (path === `/authorities/${f.slug}/facts`) return answer(facts);
		if (path === `/authorities/${f.slug}/plan`) return answer(plan);
		if (path === '/authorities') return answer({ authorities: index });
		if (path === `/authorities/${f.slug}/changes`) return answer({ authority: f.slug, changes });
		if (path === `/authorities/${f.slug}/translations`) {
			return answer({ authority: f.slug, lang: url.searchParams.get('lang'), translations: [], count: 0 });
		}
		if (path === '/p2/orders/options') return answer(options);
		if (path === '/notify/status') return answer({ enabled: opts.notify === true });
		// The shell's anonymous session probes: 200 {"user": null}, like
		// the real API.
		if (path === '/me' || path === '/auth/me') return answer({ user: null });
		if (path === '/authorities/resolve') {
			const want = (url.searchParams.get('path') ?? '').replace(/^\/+|\/+$/g, '');
			const canonical = row?.path ?? f.slug;
			if (want !== canonical && want !== f.slug) return answer({ error: 'no authority at this address' }, 404);
			const parts = canonical.split('/');
			const crumbs = parts.map((part, i) => {
				const prefix = parts.slice(0, i + 1).join('/');
				const own = prefix === canonical;
				return {
					path: prefix,
					label: own ? (row?.name ?? f.slug) : i === 0 && part === 'usa' ? 'United States' : part,
					slug: own ? f.slug : null,
					location: own ? null : prefix
				};
			});
			return answer({ slug: f.slug, canonical, aliases: [], crumbs });
		}
		return answer({ error: 'mock' }, 404);
	});
	return {
		reads,
		setFacts: (body) => {
			facts = body;
		},
		setPlan: (body) => {
			plan = body;
		}
	};
}

/**
 * Hold the live bus open: every `authority:{slug}` socket the page opens
 * is kept, answered with `subscribed`, and returned so a spec can push
 * ids-only events into it.
 * @param {import('@playwright/test').Page} page
 * @returns {Promise<{ urls: string[], sockets: import('@playwright/test').WebSocketRoute[], send: (event: object) => void }>}
 */
export async function liveBus(page) {
	const urls = [];
	const sockets = [];
	await page.routeWebSocket('**/apostille/ws/bus*', (ws) => {
		urls.push(ws.url());
		sockets.push(ws);
		const topic = new URL(ws.url()).searchParams.get('topic') ?? '';
		ws.send(JSON.stringify({ type: 'subscribed', topic }));
	});
	return {
		urls,
		sockets,
		send(event) {
			for (const ws of sockets) ws.send(JSON.stringify(event));
		}
	};
}

/**
 * Light or dark, the way the app picks it: `localStorage.theme` read by
 * the blocking init script in app.html, plus the media feature.
 * @param {import('@playwright/test').Page} page
 * @param {'light' | 'dark'} theme
 */
export async function useTheme(page, theme) {
	await page.emulateMedia({ colorScheme: theme });
	await page.addInitScript((t) => {
		try {
			localStorage.setItem('theme', t);
		} catch {
			// Storage blocked: the media feature still applies.
		}
	}, theme);
}

/**
 * Collect console errors and uncaught page errors.
 * @param {import('@playwright/test').Page} page
 */
export function consoleErrors(page) {
	const errors = [];
	page.on('console', (m) => {
		if (m.type() === 'error') errors.push(m.text());
	});
	page.on('pageerror', (e) => errors.push(String(e)));
	return errors;
}
