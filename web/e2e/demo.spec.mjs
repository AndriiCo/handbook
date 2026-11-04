// Demo gate: walks GP1→GP4 public scenes, records
// `web/shots/demo/`, fails on console errors or banned strings.
// Fixture mode is the default (`npm run demo`): the local `build`
// output plus route mocks, so the gate is data-independent.
// Live mode: DEMO_BASE_URL=https://andrii.ai/apostille replays the
// same public scenes against real data (the recorded take).
// Recording: DEMO_HEADED=1 shows the browser; PW_SLOWMO sets the
// pacing between steps (default 250ms); DEMO_CAPTIONS=0 hides the
// scene captions.
import { chromium, expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { mkdirSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const fixture = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', name), 'utf8'));
const planHero = fixture('plan-usa-wa-sos.json');
const planEmpty = fixture('plan-empty.json');
const modelCalls = fixture('admin-model-calls.json');
const SHOTS = join(HERE, '..', 'shots', 'demo');
const LIVE = process.env.DEMO_BASE_URL || null;
// Scene URLs append `/apostille/...`, so a live base naming the mount
// (`.../apostille`) is trimmed to the origin root. Without this every
// live scene doubled the prefix and the SPA fallback passed vacuously.
const LIVE_ROOT = LIVE?.replace(/\/+$/, '').replace(/\/apostille$/, '');
const HEADED = process.env.DEMO_HEADED === '1';
const SLOWMO = Number(process.env.PW_SLOWMO ?? 250);
const CAPTIONS = process.env.DEMO_CAPTIONS === '1';
const BANNED = ['placeholder', 'not wired', 'skeleton', 'lorem', 'TODO'];

// HERO dossier for the recorded takes (docs/demo-script.md; the
// operator names vietnam or usa-wa-sos — default is the assumed one).
const HERO = process.env.DEMO_HERO || 'usa-wa-sos';
const authorities = JSON.parse(
	readFileSync(join(HERE, 'fixtures', 'authorities.json'), 'utf8')
);
const factsHero = JSON.parse(
	readFileSync(join(HERE, 'fixtures', `facts-${HERO}.json`), 'utf8')
);
const samples = JSON.parse(
	readFileSync(join(HERE, 'fixtures', 'precheck-samples.json'), 'utf8')
);
const providers = JSON.parse(
	readFileSync(join(HERE, 'fixtures', 'providers.json'), 'utf8')
);
// 1x1 transparent PNG so synthetic sample thumbs never 404.
const PIXEL = Buffer.from(
	'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
	'base64'
);

// Captions render through an init script (present before any page
// script, one evaluate per scene) and only when DEMO_CAPTIONS=1,
// the recording take. The plain gate run stays caption-free.
async function caption(page, text) {
	if (!CAPTIONS) return;
	await page.evaluate((label) => window.__demoCaption(label), text);
	await page.waitForTimeout(900);
}

async function mockApi(page) {
	await page.route('**/apostille/api/v1/authorities', (route) =>
		route.fulfill({ json: authorities })
	);
	await page.route(`**/apostille/api/v1/authorities/${HERO}/facts`, (route) =>
		route.fulfill({ json: factsHero })
	);
	// Meta-composed page plan (SA-22 /plan read): the hero shows the
	// staff-accepted plan; any other slug reads as no plan (guide).
	await page.route('**/apostille/api/v1/authorities/*/plan', (route) => {
		const slug = new URL(route.request().url()).pathname.split('/').at(-2);
		return route.fulfill({
			json: slug === 'usa-wa-sos' ? planHero : { ...planEmpty, slug }
		});
	});
	await page.route('**/apostille/api/v1/auth/providers', (route) =>
		route.fulfill({ json: providers })
	);
	await page.route('**/apostille/api/v1/demo/precheck/samples', (route) =>
		route.fulfill({ json: samples })
	);
	await page.route('**/apostille/api/v1/demo/precheck', (route) =>
		route.fulfill({
			json: {
				sample: 'syn-doc-clean',
				verdict: 'pass',
				issues: [],
				boxes: [],
				models: 'muse-spark-1.3 + sam-3.1 (fixture)',
				recorded_at: '2026-10-31T00:00:00Z'
			}
		})
	);
	await page.route('**/apostille/api/v1/quality', (route) =>
		route.fulfill({
			json: { drafted_records: 25, ready_records: 18, quote_found_rate: 0.72 }
		})
	);
	await page.route('**/apostille/demo/*', (route) =>
		route.fulfill({ contentType: 'image/png', body: PIXEL })
	);
}

async function checkClean(page, problems, where) {
	const text = await page.evaluate(() => document.body?.innerText ?? '');
	for (const word of BANNED) {
		if (text.includes(word)) problems.push(`${where}: banned string "${word}"`);
	}
	const scrollWidth = await page.evaluate(() => document.documentElement.scrollWidth);
	const clientWidth = await page.evaluate(() => document.documentElement.clientWidth);
	if (scrollWidth > clientWidth + 1) problems.push(`${where}: horizontal scroll`);
}

function cleanStale(outDir) {
	mkdirSync(outDir, { recursive: true });
	for (const f of readdirSync(outDir))
		if (f.startsWith('page@') && f.endsWith('.webm')) rmSync(join(outDir, f));
}

async function launch(outDir, size) {
	// Stable takes: drop stale hash-named videos so each pass leaves one take.
	cleanStale(outDir);
	const browser = await chromium.launch({ headless: !HEADED, slowMo: SLOWMO });
	const context = await browser.newContext({
		viewport: { width: size.width, height: size.height },
		recordVideo: { dir: outDir, size }
	});
	await context.addInitScript(() => {
		window.__demoCaption = (label) => {
			document.querySelector('#demo-caption')?.remove();
			const bar = document.createElement('div');
			bar.id = 'demo-caption';
			bar.setAttribute('aria-hidden', 'true');
			bar.textContent = label;
			bar.style.cssText =
				'position:fixed;left:50%;bottom:24px;transform:translateX(-50%);' +
				'z-index:9999;max-width:min(90vw,64ch);padding:10px 18px;' +
				'border-radius:12px;background:rgba(11,74,117,.92);color:#fff;' +
				'font:600 15px/1.4 system-ui,sans-serif;text-align:center;';
			document.body.appendChild(bar);
		};
	});
	const problems = [];
	context.on('console', (msg) => {
		if (msg.type() === 'error') problems.push(`console: ${msg.text()}`);
	});
	context.on('pageerror', (err) => problems.push(`pageerror: ${err.message}`));
	return { browser, context, problems };
}

test('demo walks GP1 to GP4 and records web/shots/demo', async () => {
	test.setTimeout(180_000);
	let srv = null;
	const base = LIVE_ROOT ?? (srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' })).url;
	const { browser, context, problems } = await launch(SHOTS, { width: 1440, height: 900 });
	try {
		const page = await context.newPage();
		if (!LIVE) await mockApi(page);

		const scene = async (name, url) => {
			console.log(`demo scene: ${name}`);
			await page.goto(url);
		};

		await scene('home', `${base}/apostille`);
		await caption(page, 'Tell us where your document is going');
		await expect(page.getByRole('heading', { level: 1 }).first()).toBeVisible();
		await checkClean(page, problems, 'home');

		await scene('dossier', `${base}/apostille/authority/${HERO}`);
		await caption(page, 'Muse Spark drafts every fact from official pages');
		await expect(page.getByRole('heading', { level: 1 }).first()).toBeVisible();
		// Citations beat (script scene 3): the first "Citation" button is
		// the guide's Citations toggle, and the first blockquote is its
		// first endnote; guarded so a facts-empty host still passes.
		if ((await page.getByRole('button', { name: 'Citation' }).count()) > 0) {
			await page.getByRole('button', { name: 'Citation' }).first().click();
			await expect(page.locator('blockquote').first()).toBeVisible({ timeout: 15_000 });
			await caption(page, 'Every fact shows its quote, source and date');
		}
		await checkClean(page, problems, 'dossier');

		await scene('login', `${base}/apostille/login`);
		await caption(page, 'Passkey-first sign-in');
		await expect(page.getByRole('heading', { level: 1 }).first()).toBeVisible();
		await checkClean(page, problems, 'login');

		await scene('precheck', `${base}/apostille/precheck`);
		await caption(page, 'Precheck replay: SAM 3.1 + Muse Spark');
		await expect(page.getByRole('button', { name: 'Run precheck' })).toBeVisible();
		if (!LIVE) {
			await page.getByRole('button', { name: /Clean scan/ }).click();
			await page.getByRole('button', { name: 'Run precheck' }).click();
			await expect(page.getByText('Pass — good to submit')).toBeVisible({
				timeout: 15_000
			});
		}
		await checkClean(page, problems, 'precheck');

		await scene('order', `${base}/apostille/order`);
		await caption(page, 'Draft your order in minutes');
		await expect(page.getByRole('heading', { level: 1 }).first()).toBeVisible();
		await checkClean(page, problems, 'order');

		await context.close();
		await page.video()?.saveAs(join(SHOTS, 'desktop.webm'));
		cleanStale(SHOTS);
		expect(problems).toEqual([]);
	} finally {
		await browser.close();
		await srv?.close();
	}
});

const queueItem = {
	id: 7,
	authority_slug: 'vietnam',
	topic: 'fees',
	statement: 'Legalization at the Consular Department costs 300,000 VND per document.',
	status: 'proposed',
	created_by: 'muse-spark-1.3 via analysis-agent/aa-prompt-v1',
	created_at: 1792012800,
	diff: null,
	citations: [
		{
			capture_id: 3,
			start_byte: 120,
			end_byte: 220,
			quote:
				'The Consular Department charges 300,000 VND for each legalized document copy presented at the counter.'
		}
	],
	reasoning_untrusted: null,
	blast_radius: { conflict: false, conflicts: [], open_tasks: 0 },
	provenance: {
		capture_id: 3,
		capture_kind: 'fetch',
		trust: 'verified-source',
		authenticity: 'tls-fetch',
		sidecar: 'vietnam-consular-fees.sidecar.json'
	}
};

async function mockAdmin(page) {
	await page.route('**/apostille/api/v1/queue*', (route) => {
		if (route.request().method() !== 'GET') return route.continue();
		return route.fulfill({ json: { items: [queueItem], count: 1 } });
	});
	await page.route('**/apostille/api/v1/facts/*/accept', (route) =>
		route.fulfill({ json: { status: 'accepted' } })
	);
	await page.route('**/apostille/api/v1/suppress-rules*', (route) =>
		route.fulfill({ json: { rules: [], count: 0 } })
	);
	await page.route('**/apostille/api/v1/suppressions*', (route) =>
		route.fulfill({ json: { suppressions: [], count: 0 } })
	);
	// Verifier-blocked list on /review: empty, like a clean run (the
	// SPA fallback would otherwise answer HTML and the section would
	// show a parse error on camera).
	await page.route('**/apostille/api/v1/draft-runs/recent*', (route) =>
		route.fulfill({ json: { rejections: [], count: 0 } })
	);
	await page.route('**/apostille/api/v1/admin/model-calls*', (route) =>
		route.fulfill({ json: modelCalls })
	);
	// Staff authority index (P358): /review mounts the compose pickers
	// (every tab's content renders), which read it on the admin host.
	await page.route('**/apostille/api/v1/admin/authorities', (route) =>
		route.fulfill({ json: authorities })
	);
	await page.route('**/apostille/api/v1/admin/version', (route) =>
		route.fulfill({ json: { version: '0.1.0+demo', env: 'staging', started_at: 0, schema: 11, p2_schema: 2 } })
	);
	await page.route('**/apostille/api/v1/workers/captures*', (route) =>
		route.fulfill({
			json: {
				captures: [
					{
						id: 3,
						authority_slug: 'vietnam',
						source_uri: 'https://www.hcch.net/en/instruments/conventions/status-table/?cid=41',
						captured_at: 1792012800,
						kind: 'fetch',
						bytes_raw: 48210,
						sha256: '9f2c4a1bd6e07f3a8c5d2e190a4b6c7d8e9f0a1b2c3d4e5f60718293a4b5c6d7e8f',
						trust: 'verified-source',
						excerpt: 'Viet Nam deposited its instrument of accession…'
					}
				]
			}
		})
	);

	// Trace Draft POST: record the CSRF double-submit pair (asserted in
	// the trace scene) and hand back a run id for the live `run:{id}`
	// socket below.
	await page.route('**/apostille/api/v1/captures/*/draft', (route) => {
		const headers = route.request().headers();
		draftCsrf.header = headers['x-csrf'] ?? null;
		draftCsrf.cookie = headers['cookie'] ?? '';
		return route.fulfill({ status: 202, json: { run_id: 9, topic: 'run:9' } });
	});

	// Admin bus: `run:{id}` delivers one finished step and closes (the
	// trace scene asserts the live numbers); `proposals` holds open so
	// the review scene's live feed logs no console error.
	await page.routeWebSocket('**/apostille/ws/bus*', (ws) => {
		const topic = new URL(ws.url()).searchParams.get('topic') ?? '';
		if (topic === 'ops') {
			ws.send(JSON.stringify({ type: 'subscribed', topic }));
			return;
		}
		if (!topic.startsWith('run:')) return;
		const runId = Number(topic.slice(4));
		ws.send(
			JSON.stringify({
				type: 'run_finished',
				topic,
				run_id: runId,
				capture_id: 3,
				slug: 'vietnam',
				proposed: 2,
				rejected: 1,
				model: 'muse-spark-1.3',
				input_tokens: 100,
				output_tokens: 50
			})
		);
		ws.close();
	});
}

// CSRF pair seen on the intercepted Draft POST (one admin take).
const draftCsrf = { header: null, cookie: '' };

test('demo admin pass approves a fact and opens trace', async () => {
	test.setTimeout(120_000);
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	const { browser, context, problems } = await launch(join(SHOTS, 'admin'), {
		width: 1440,
		height: 900
	});
	try {
		const page = await context.newPage();
		await mockAdmin(page);
		const scene = async (name, url) => {
			console.log(`demo scene: ${name}`);
			await page.goto(url);
		};

		await scene('review', `${srv.url}/review`);
		await caption(page, 'A human approves every Muse Spark draft');
		await expect(page.getByRole('heading', { name: 'Review queue' })).toBeVisible();
		await page.getByRole('button', { name: 'Accept', exact: true }).click();
		await expect(page.getByText('Fact #7 decided: accepted.')).toBeVisible({
			timeout: 15_000
		});
		await page.screenshot({ path: join(SHOTS, '..', 'gp5-review-1440x900-light.png') });
		await checkClean(page, problems, 'review');

		await scene('trace', `${srv.url}/trace`);
		await caption(page, 'Muse Spark drafts live — watch the tokens');
		await expect(page.getByRole('heading', { name: 'Trace console' })).toBeVisible();
		await expect(page.getByText('9f2c4a1b', { exact: false }).first()).toBeVisible();
		// Live Draft: the POST must carry the `hb_csrf` cookie value in
		// `x-csrf` (double-submit), and the `run:{id}` socket reports
		// the finished numbers with no polling.
		await page.evaluate(() => {
			document.cookie = 'hb_csrf=demo-csrf-1; Path=/; SameSite=Lax';
		});
		await page.getByRole('button', { name: 'Draft from this capture' }).click();
		await expect(page.getByText('Done — 2 proposed, 1 rejected')).toBeVisible({
			timeout: 15_000
		});
		const csrfCookie = (/hb_csrf=([^;]+)/.exec(draftCsrf.cookie) ?? [])[1] ?? null;
		expect(draftCsrf.header).toBe('demo-csrf-1');
		expect(draftCsrf.header).toBe(csrfCookie);
		await page.screenshot({ path: join(SHOTS, '..', 'gp5-trace-1440x900-light.png') });
		await checkClean(page, problems, 'trace');

		await context.close();
		await page.video()?.saveAs(join(SHOTS, 'admin.webm'));
		cleanStale(join(SHOTS, 'admin'));
		expect(problems).toEqual([]);
	} finally {
		await browser.close();
		await srv.close();
	}
});

test('demo mobile pass records home and dossier', async () => {
	test.setTimeout(120_000);
	let srv = null;
	const base = LIVE_ROOT ?? (srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' })).url;
	const { browser, context, problems } = await launch(join(SHOTS, 'mobile'), {
		width: 390,
		height: 844
	});
	try {
		const page = await context.newPage();
		if (!LIVE) await mockApi(page);

		await page.goto(`${base}/apostille`);
		await caption(page, 'Find your apostille authority');
		await page.screenshot({ path: join(SHOTS, 'mobile-home.png') });
		await checkClean(page, problems, 'mobile home');

		await page.goto(`${base}/apostille/authority/${HERO}`);
		await caption(page, 'Every value sourced, citations on request');
		await page.screenshot({ path: join(SHOTS, 'mobile-dossier.png') });
		await checkClean(page, problems, 'mobile dossier');

		await context.close();
		await page.video()?.saveAs(join(SHOTS, 'mobile.webm'));
		cleanStale(join(SHOTS, 'mobile'));
		expect(problems).toEqual([]);
	} finally {
		await browser.close();
		await srv?.close();
	}
});
