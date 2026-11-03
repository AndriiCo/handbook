// Compose round trip on /review (MISSION WIN SA-26 W1e): pick an
// authority → "Compose plan with Muse Spark" posts {slug} with the
// CSRF pair → the run reports over `run:{id}` → the timeline
// finishes, per-call rows load, the new plan is selected → Accept
// posts with CSRF. Also: the 429 cap state, and an office accept that
// carries a staff-confirmed timezone and rolls back on a server error.
// API and bus are route-mocked; the page is the real built admin SPA.
import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { serveDir } from './helpers/serve.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const fixture = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', name), 'utf8'));
const authorities = fixture('authorities.json');
const plans = fixture('admin-plan-proposals.json');
const offices = fixture('admin-office-proposals.json');
const calls = fixture('admin-model-calls.json');
const runEvents = fixture('admin-ws-runs.json');

const seedPlans = { plans: plans.plans.filter((p) => p.run_id !== 94) };

const at = (path) => (url) => url.pathname === `/apostille/api/v1${path}`;

/** Every API read the review page makes, answered from fixtures. */
async function mockReview(page, { plansBody = () => plans, officesBody = () => offices } = {}) {
	// Fallback first: Playwright runs the newest matching route first.
	await page.route('**/apostille/api/v1/**', (route) => route.fulfill({ json: {} }));
	await page.route(at('/authorities'), (route) => route.fulfill({ json: authorities }));
	// The admin host serves the staff index (the picker's source since P358).
	await page.route(at('/admin/authorities'), (route) => route.fulfill({ json: authorities }));
	await page.route(at('/queue'), (route) => route.fulfill({ json: { items: [], count: 0 } }));
	await page.route(at('/admin/plans'), (route) => route.fulfill({ json: plansBody() }));
	await page.route(at('/admin/offices'), (route) => route.fulfill({ json: officesBody() }));
	await page.route(at('/admin/form-maps'), (route) => route.fulfill({ json: { form_maps: [] } }));
	await page.route(at('/admin/model-calls'), (route) => route.fulfill({ json: calls }));
}

test('compose a plan live, see the run finish, accept the new version', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		let composed = false;
		await mockReview(page, { plansBody: () => (composed ? plans : seedPlans) });
		let composeReq = null;
		await page.route(at('/admin/plans/compose'), async (route) => {
			composeReq = {
				body: route.request().postDataJSON(),
				csrf: await route.request().headerValue('x-csrf')
			};
			return route.fulfill({ status: 202, json: { run_id: 94, topic: 'run:94' } });
		});
		let acceptCsrf = null;
		await page.route(at('/admin/plans/14/accept'), async (route) => {
			acceptCsrf = await route.request().headerValue('x-csrf');
			return route.fulfill({ json: { id: 14, authority_slug: 'usa-wa-sos', version: 5, status: 'accepted' } });
		});
		await page.routeWebSocket('**/apostille/ws/bus*', (ws) => {
			const topic = new URL(ws.url()).searchParams.get('topic') ?? '';
			ws.send(JSON.stringify({ type: 'subscribed', topic }));
			if (topic !== 'run:94') return;
			// The plan is stored before `compose.done` is published.
			setTimeout(() => {
				composed = true;
				for (const ev of runEvents.filter((e) => e.topic === topic)) ws.send(JSON.stringify(ev));
			}, 700);
		});

		await page.goto(`${srv.url}/review?tab=plans`);
		await expect(page.getByRole('heading', { name: 'Review queue' })).toBeVisible();
		await expect(page.getByText('usa-wa-sos · page plan v4')).toBeVisible();

		await page.getByRole('combobox', { name: /Authority/ }).click();
		await page.getByRole('option', { name: /Washington Secretary of State/ }).click();
		await page.getByRole('button', { name: 'Compose plan with Muse Spark' }).click();

		await expect(page.getByText('Muse Spark composing')).toBeVisible();
		await expect(page.getByText('Proposal ready')).toBeVisible({ timeout: 10_000 });
		expect(composeReq?.body).toEqual({ slug: 'usa-wa-sos' });
		expect(composeReq?.csrf).toMatch(/\S{16,}/);

		await expect(page.getByRole('heading', { name: 'Calls in run 94' })).toBeVisible();
		await expect(page.getByText('muse-wonder-1.0 · plan · attempt 1')).toBeVisible();
		await expect(page.getByText('usa-wa-sos · page plan v5')).toBeVisible();
		await expect(page.getByText('New · run 94')).toBeVisible();

		await page.getByRole('button', { name: 'Accept plan v5' }).click();
		await expect(page.getByText('Plan v5 accepted.')).toBeVisible();
		expect(acceptCsrf).toBe(composeReq.csrf);
	} finally {
		await srv.close();
	}
});

test('a hit daily cap shows one next action', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await mockReview(page);
		await page.route(at('/admin/plans/compose'), (route) =>
			route.fulfill({ status: 429, json: { error: 'daily model-call cap reached (meta.daily_call_cap)' } })
		);
		await page.routeWebSocket('**/apostille/ws/bus*', (ws) => {
			ws.send(JSON.stringify({ type: 'subscribed', topic: new URL(ws.url()).searchParams.get('topic') }));
		});
		await page.goto(`${srv.url}/review?tab=plans&compose=usa-wa-sos`);
		await page.getByRole('button', { name: 'Compose plan with Muse Spark' }).click();
		await expect(page.getByText('Daily Meta call cap reached')).toBeVisible();
		await expect(page.getByRole('link', { name: 'Open config' })).toBeVisible();
	} finally {
		await srv.close();
	}
});

test('office accept sends the confirmed timezone and rolls back on error', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await mockReview(page);
		await page.routeWebSocket('**/apostille/ws/bus*', (ws) => {
			ws.send(JSON.stringify({ type: 'subscribed', topic: new URL(ws.url()).searchParams.get('topic') }));
		});
		let acceptBody = null;
		await page.route(at('/admin/offices/33/accept'), (route) => {
			acceptBody = route.request().postDataJSON();
			return route.fulfill({ status: 500, json: { error: 'store unavailable' } });
		});

		await page.goto(`${srv.url}/review?tab=offices`);
		const card = page.locator('#office-33');
		await expect(card.getByText('SYNTHETIC Records Annex')).toBeVisible();
		await card.getByRole('combobox', { name: /Timezone/ }).click();
		await page.getByRole('option', { name: /Pacific \(Los Angeles\)/ }).click();
		await expect(card.getByText('Accept records America/Los_Angeles as staff confirmed', { exact: false })).toBeVisible();

		await card.getByRole('button', { name: 'Accept office' }).click();
		await expect(page.getByText('store unavailable', { exact: false })).toBeVisible();
		expect(acceptBody).toEqual({ timezone: 'America/Los_Angeles' });
		// Rolled back: still proposed, still not confirmed.
		await expect(card.getByRole('button', { name: 'Accept office' })).toBeEnabled();
		await expect(card.getByText('Not quoted in any cited fact')).toBeVisible();
	} finally {
		await srv.close();
	}
});
