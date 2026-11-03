// By-authority bulk decisions (/triage and /review?view=authority):
// checked verified rows go out as ONE POST /facts/bulk with
// require_verified; a blocked batch (409/422 + per-id results)
// rolls every optimistic row back, tags the refused ones and offers
// the rest; a batch reject needs a reason. API and bus are
// route-mocked with synthetic fixtures; the page is the built admin SPA.
import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { serveDir } from './helpers/serve.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const fixture = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', name), 'utf8'));
const facts = fixture('admin-facts-proposed.json');
const checks = fixture('admin-fact-checks.json');
const authorities = fixture('admin-authorities.json');
const queue = fixture('admin-queue.json');

const at = (path) => (url) => url.pathname === `/apostille/api/v1${path}`;

/** Mocks every read; `bulk` answers POST /facts/bulk and records bodies. */
async function mockApi(page, bulk) {
	const posts = [];
	await page.routeWebSocket('**/ws/bus*', () => {});
	// Fallback first: Playwright runs the newest matching route first.
	await page.route('**/apostille/api/v1/**', (route) => route.fulfill({ json: {} }));
	await page.route(at('/me'), (route) => route.fulfill({ json: { user_id: 'staff', role: 'staff' } }));
	await page.route(at('/facts'), (route) => route.fulfill({ json: facts }));
	await page.route(at('/queue'), (route) => route.fulfill({ json: queue }));
	await page.route(at('/admin/fact-checks'), (route) => route.fulfill({ json: checks }));
	await page.route(at('/admin/authorities'), (route) => route.fulfill({ json: authorities }));
	await page.route(at('/facts/bulk'), async (route) => {
		const body = route.request().postDataJSON();
		posts.push({ body, csrf: route.request().headers()['x-csrf'] ?? null });
		const { status, json } = bulk(body, posts.length);
		return route.fulfill({ status, json });
	});
	return posts;
}

const ok = (body) => ({
	status: 200,
	json: {
		authority: body.authority,
		decision: body.decision,
		count: body.ids.length,
		bulk_audit_id: 900 + body.ids.length,
		results: body.ids.map((id) => ({ id, ok: true, status: body.decision === 'accept' ? 'accepted' : 'rejected' }))
	}
});

const group = (page, name) => page.getByRole('region', { name });
const VIETNAM = 'Vietnam — Ministry of Foreign Affairs';

test('unchecked rows stay out of the batch; one bulk POST accepts the rest', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const posts = await mockApi(page, ok);
		await page.goto(`${srv.url}/triage`);
		const vietnam = group(page, VIETNAM);
		await expect(vietnam.getByRole('button', { name: 'Accept all verified (3)' })).toBeVisible();
		// The operator holds #10 back for the video.
		await vietnam.getByRole('checkbox', { name: 'Select fact #10' }).uncheck();
		await vietnam.getByRole('button', { name: 'Accept 2 of 3 verified' }).click();
		await expect(vietnam.getByText('Fact #7 accepted.')).toBeVisible();
		await expect(vietnam.getByText('Fact #8 accepted.')).toBeVisible();
		await expect(vietnam.getByRole('checkbox', { name: 'Select fact #10' })).not.toBeChecked();
		expect(posts).toHaveLength(1);
		expect(posts[0].body).toEqual({ decision: 'accept', authority: 'vietnam', ids: [7, 8], require_verified: true });
		expect(posts[0].csrf).toBeTruthy();
	} finally {
		await srv.close();
	}
});

test('a blocked batch rolls back every row, tags the refused one, retries the rest', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const posts = await mockApi(page, (body, n) =>
			n === 1
				? {
						status: 409,
						json: {
							error: 'batch blocked; nothing changed',
							results: body.ids.map((id) =>
								id === 8 ? { id, ok: false, code: 409, error: 'fact is not proposed' } : { id, ok: true }
							)
						}
					}
				: ok(body)
		);
		await page.goto(`${srv.url}/triage`);
		const vietnam = group(page, VIETNAM);
		await vietnam.getByRole('button', { name: 'Accept all verified (3)' }).click();
		await expect(vietnam.getByText('Nothing accepted: 1 of 3 blocked.')).toBeVisible();
		await expect(vietnam.getByText('Decided elsewhere')).toBeVisible();
		// Nothing changed on the server, so nothing reads decided here.
		await expect(vietnam.getByText('Fact #7 accepted.')).toHaveCount(0);
		await expect(vietnam.getByRole('checkbox', { name: 'Select fact #7' })).toBeChecked();
		await expect(vietnam.getByRole('checkbox', { name: 'Select fact #8' })).not.toBeChecked();
		await vietnam.getByRole('button', { name: 'Accept the 2 that pass' }).click();
		await expect(vietnam.getByText('Fact #7 accepted.')).toBeVisible();
		await expect(vietnam.getByText('Fact #10 accepted.')).toBeVisible();
		expect(posts.map((p) => p.body.ids)).toEqual([
			[7, 8, 10],
			[7, 10]
		]);
	} finally {
		await srv.close();
	}
});

test('a batch reject needs a reason and sends the unverified rows', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const posts = await mockApi(page, ok);
		await page.goto(`${srv.url}/triage`);
		const vietnam = group(page, VIETNAM);
		await vietnam.getByRole('button', { name: 'Reject…', exact: true }).click();
		const dialog = page.getByRole('dialog');
		const submit = dialog.getByRole('button', { name: 'Reject 1 fact' });
		await expect(submit).toBeDisabled();
		await dialog.getByLabel('Reason (required)').fill('Superseded by the October fee page (synthetic).');
		await submit.click();
		await expect(vietnam.getByText('Fact #9 rejected.')).toBeVisible();
		expect(posts[0].body).toEqual({
			decision: 'reject',
			authority: 'vietnam',
			ids: [9],
			reason: 'Superseded by the October fee page (synthetic).'
		});
	} finally {
		await srv.close();
	}
});

test('the per-capture batch on the queue is one bulk POST and rolls back when refused', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const posts = await mockApi(page, (body, n) =>
			n === 1
				? {
						status: 422,
						json: {
							error: 'batch blocked; nothing changed',
							results: [
								{ id: 7, ok: true },
								{ id: 8, ok: false, code: 422, error: 'accept requires a verified citation' }
							]
						}
					}
				: ok(body)
		);
		await page.goto(`${srv.url}/review`);
		await page.getByRole('button', { name: 'Accept all verified from this capture (2)' }).click();
		const pane = page.getByRole('complementary', { name: 'Selected fact' });
		await expect(pane.getByText('Nothing accepted: 1 of 2 blocked.')).toBeVisible();
		await expect(pane.getByText('Held back from the batch: Needs citation.')).toBeVisible();
		await expect(page.getByText('Fact #7 decided: accepted.')).toHaveCount(0);
		await pane.getByRole('button', { name: 'Accept the 1 that pass' }).click();
		await expect(page.getByText('Fact #7 decided: accepted.')).toBeVisible();
		expect(posts.map((p) => p.body)).toEqual([
			{ decision: 'accept', authority: 'vietnam', ids: [7, 8], require_verified: true },
			{ decision: 'accept', authority: 'vietnam', ids: [7], require_verified: true }
		]);
	} finally {
		await srv.close();
	}
});

test('review by authority: Shift+A accepts the focused group, the table stays one view', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const posts = await mockApi(page, ok);
		await page.goto(`${srv.url}/review?view=authority`);
		await expect(page.getByRole('heading', { name: 'Review queue' })).toBeVisible();
		const vietnam = group(page, VIETNAM);
		await expect(vietnam.getByRole('button', { name: 'Accept all verified (2)' })).toBeVisible();
		// Exactly one view is mounted: no table caption, no exact "Accept".
		await expect(page.getByRole('table')).toHaveCount(0);
		await expect(page.getByRole('button', { name: 'Accept', exact: true })).toHaveCount(0);
		await page.keyboard.press('j');
		await page.keyboard.press('Shift+A');
		await expect(vietnam.getByText('Fact #7 accepted.')).toBeVisible();
		expect(posts[0].body).toEqual({ decision: 'accept', authority: 'vietnam', ids: [7, 8], require_verified: true });
		// Switch back: the table shows the same decisions.
		await page.getByRole('button', { name: 'Riskiest first' }).click();
		await expect(page).toHaveURL(/\/review$/);
		await expect(page.getByText('Fact #7 decided: accepted.')).toBeVisible();
	} finally {
		await srv.close();
	}
});
