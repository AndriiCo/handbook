// Compile card on /dashboard: pick an authority, add an extra official
// URL → "Compile" posts {slug, seed_urls} with the CSRF pair → the five
// steps move over `run:{id}` → the done line links to Review. Also: a
// non-https URL blocks the start, a 409 offers "Watch the running one"
// (which follows that run live), and a 429 names the cap. API and bus
// are route-mocked; the page is the real built admin SPA.
import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { serveDir } from './helpers/serve.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const fixture = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', name), 'utf8'));
const stats = fixture('admin-stats.json');
const authorities = fixture('admin-staff-authorities.json');
const runs = fixture('admin-compile-runs.json');
const run91 = fixture('admin-ws-compile.json');
const done88 = runs.runs.find((r) => r.run_id === 88);

const at = (path) => (url) => url.pathname === `/apostille/api/v1${path}`;

async function mockDashboard(page, { runsBody = () => runs } = {}) {
	// Fallback first: Playwright runs the newest matching route first.
	await page.route('**/apostille/api/v1/**', (route) => route.fulfill({ json: {} }));
	await page.route(at('/admin/stats'), (route) => route.fulfill({ json: stats }));
	await page.route(at('/admin/authorities'), (route) => route.fulfill({ json: authorities }));
	await page.route(at('/admin/compile'), (route) => {
		if (route.request().method() !== 'GET') return route.fallback();
		return route.fulfill({ json: runsBody() });
	});
}

/** Run 92's live report, step by step (compile.rs order). */
const live92 = [
	{ type: 'compile.started', slug: 'usa-wa-sos' },
	{ type: 'compile.step', step: 'capture', status: 'running' },
	{ type: 'compile.step', step: 'capture', status: 'done', counts: { pages: 5, created: 3, duplicate: 2 }, latency_ms: 6100 },
	{ type: 'compile.step', step: 'extract', status: 'running', counts: { captures: 3 } },
	{ type: 'compile.step', step: 'extract', status: 'done', counts: { captures: 3, proposed: 7, rejected: 2, no_text: 0, failed: 0, waiting: 0 }, latency_ms: 48200 },
	{ type: 'compile.step', step: 'cross_check', status: 'running' },
	{ type: 'compile.step', step: 'cross_check', status: 'done', counts: { captures: 3, checked: 3, failed: 0 }, latency_ms: 21000 },
	{ type: 'compile.step', step: 'offices', status: 'running' },
	{ type: 'compile.step', step: 'offices', status: 'running', detail: 'guards' },
	{ type: 'compile.step', step: 'offices', status: 'done', summary: done88.summary.offices.summary, latency_ms: 5300 },
	{ type: 'compile.step', step: 'compose', status: 'skipped', reason: 'needs at least 3 accepted facts (has 2)' },
	{
		type: 'compile.done',
		slug: 'usa-wa-sos',
		status: 'done',
		summary: {
			...done88.summary,
			capture: { pages: 5, created: 3, duplicate: 2, failed: 0, skipped: 0, dropped: 0 },
			extract: { captures: 3, proposed: 7, rejected: 2, waiting: 0 },
			plan: { status: 'skipped', reason: 'needs at least 3 accepted facts (has 2)' }
		}
	}
];

test('compile an authority live and land in Review', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		let started = false;
		await mockDashboard(page, {
			runsBody: () =>
				started
					? { ...runs, today: 3, runs: [{ run_id: 92, slug: 'usa-wa-sos', status: 'running', started_at: 1793750400, finished_at: null, summary: {} }, ...runs.runs] }
					: runs
		});
		let req = null;
		await page.route(at('/admin/compile'), async (route) => {
			if (route.request().method() !== 'POST') return route.fallback();
			req = { body: route.request().postDataJSON(), csrf: await route.request().headerValue('x-csrf') };
			started = true;
			return route.fulfill({ status: 202, json: { run_id: 92, topic: 'run:92' } });
		});
		await page.route(at('/admin/compile/92'), (route) =>
			route.fulfill({ json: { run_id: 92, slug: 'usa-wa-sos', status: 'running', started_at: 1793750400, finished_at: null, summary: {} } })
		);
		let send = null;
		await page.routeWebSocket('**/apostille/ws/bus*', (ws) => {
			const topic = new URL(ws.url()).searchParams.get('topic') ?? '';
			ws.send(JSON.stringify({ type: 'subscribed', topic }));
			if (topic === 'run:92') send = (ev) => ws.send(JSON.stringify({ topic, run_id: 92, at: 1793750400, ...ev }));
		});

		await page.goto(`${srv.url}/dashboard`);
		await expect(page.getByRole('heading', { name: 'Compile an authority' })).toBeVisible();
		await expect(page.getByText('2 of 6 runs today')).toBeVisible();
		await expect(page.getByText('4 pages · 6 facts proposed · 1 office · plan v5')).toBeVisible();

		const card = page.locator('section[aria-labelledby="compile-title"]');
		await card.getByRole('combobox', { name: /Authority/ }).click();
		await page.getByRole('option', { name: /Washington Secretary of State/ }).click();

		// A plain-http URL never leaves the browser.
		const seeds = card.getByLabel(/Extra official URLs/);
		await seeds.fill('http://www.sos.wa.gov/apostille');
		await expect(card.getByText('Line 1 is not an https:// URL.')).toBeVisible();
		await expect(card.getByRole('button', { name: 'Compile', exact: true })).toBeDisabled();
		await seeds.fill('https://www.sos.wa.gov/apostille');
		await card.getByRole('button', { name: 'Compile', exact: true }).click();

		await expect(card.getByText('Run 92', { exact: false }).first()).toBeVisible();
		expect(req?.body).toEqual({ slug: 'usa-wa-sos', seed_urls: ['https://www.sos.wa.gov/apostille'] });
		expect(req?.csrf).toMatch(/\S{16,}/);
		await expect(page.getByText('3 of 6 runs today')).toBeVisible();

		await expect.poll(() => send !== null).toBe(true);
		for (const ev of live92.slice(0, 4)) send(ev);
		const steps = card.getByRole('list', { name: 'Compile run 92 steps' });
		await expect(steps.getByText('5 pages')).toBeVisible();
		await expect(steps.locator('li[aria-current="step"]')).toContainText('Draft facts (Muse Spark)');

		for (const ev of live92.slice(4, 9)) send(ev);
		await expect(steps.getByText('Guards checking address, hours and mail')).toBeVisible();
		for (const ev of live92.slice(9)) send(ev);

		await expect(card.getByText('Run 92 done.')).toBeVisible();
		await expect(card.getByText('5 pages · 7 facts proposed · 1 office · no plan').first()).toBeVisible();
		await expect(steps.getByText('Needs at least 3 accepted facts (has 2)')).toBeVisible();
		await expect(card.getByRole('link', { name: 'Open Review' }).last()).toHaveAttribute('href', '/review?authority=usa-wa-sos');
	} finally {
		await srv.close();
	}
});

test('a run already going offers to watch it', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await mockDashboard(page);
		await page.route(at('/admin/compile'), (route) => {
			if (route.request().method() !== 'POST') return route.fallback();
			return route.fulfill({
				status: 409,
				json: { error: 'a Compile run is already going for this authority', run_id: 91, topic: 'run:91' }
			});
		});
		await page.route(at('/admin/compile/91'), (route) => route.fulfill({ json: runs.runs[0] }));
		await page.routeWebSocket('**/apostille/ws/bus*', (ws) => {
			const topic = new URL(ws.url()).searchParams.get('topic') ?? '';
			if (topic !== 'run:91') return ws.send(JSON.stringify({ type: 'subscribed', topic }));
			for (const ev of run91) ws.send(JSON.stringify(ev));
		});

		await page.goto(`${srv.url}/dashboard`);
		const card = page.locator('section[aria-labelledby="compile-title"]');
		await card.getByRole('combobox', { name: /Authority/ }).click();
		await page.getByRole('option', { name: /Vietnam/ }).click();
		await card.getByRole('button', { name: 'Compile', exact: true }).click();

		await expect(card.getByText('Already compiling Vietnam — Ministry of Foreign Affairs')).toBeVisible();
		await expect(card.getByRole('paragraph').filter({ hasText: 'A Compile run is already going for this authority.' })).toBeVisible();
		await card.getByRole('button', { name: 'Watch the running one' }).click();
		const steps = card.getByRole('list', { name: 'Compile run 91 steps' });
		await expect(steps.getByText('3 pages')).toBeVisible();
		await expect(steps.locator('li[aria-current="step"]')).toContainText('Draft facts (Muse Spark)');
	} finally {
		await srv.close();
	}
});

test('the daily cap names itself', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await mockDashboard(page);
		await page.route(at('/admin/compile'), (route) => {
			if (route.request().method() !== 'POST') return route.fallback();
			return route.fulfill({ status: 429, json: { error: 'daily Compile cap reached (compile.daily_runs = 6)' } });
		});
		await page.route(at('/admin/compile/88'), (route) => route.fulfill({ json: done88 }));
		await page.routeWebSocket('**/apostille/ws/bus*', (ws) => {
			ws.send(JSON.stringify({ type: 'subscribed', topic: new URL(ws.url()).searchParams.get('topic') }));
		});
		await page.goto(`${srv.url}/dashboard?compile=88`);
		const card = page.locator('section[aria-labelledby="compile-title"]');
		// The link opened run 88 from its stored summary.
		await expect(card.getByText('Run 88 done.')).toBeVisible();
		await expect(card.getByRole('list', { name: 'Compile run 88 steps' }).getByText('plan v5')).toBeVisible();

		await card.getByRole('button', { name: 'Compile', exact: true }).click();
		await expect(card.getByText('Daily cap reached')).toBeVisible();
		await expect(card.getByRole('paragraph').filter({ hasText: 'Daily Compile cap reached (compile.daily_runs = 6).' })).toBeVisible();
		await expect(card.getByRole('link', { name: 'Open config' })).toBeVisible();
	} finally {
		await srv.close();
	}
});
