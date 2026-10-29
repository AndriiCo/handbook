// Setup-wizard save round trip (the click→fetch seam unit tests
// cannot see): key entry → steps render → edit → Save fires one
// POST with the key header and typed body → "Saved." notice.
// API is route-mocked; the page is the real built admin SPA.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const KEY = `setup_${'ab12'.repeat(16)}`;
const STEPS = {
	keys: [
		{ key: 'net.public_host', type: 'text', value: '' },
		{ key: 'telnyx.api_key', type: 'secret', set: false }
	]
};

test('wizard save posts the edited value with the setup key', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.route('**/apostille/api/v1/setup/status', (route) =>
			route.fulfill({ json: { setup_required: true } })
		);
		let stepsKey = null;
		await page.route('**/apostille/api/v1/setup/steps', async (route) => {
			stepsKey = await route.request().headerValue('x-setup-key');
			return route.fulfill({ json: STEPS });
		});
		let saved = null;
		await page.route('**/apostille/api/v1/setup/keys/*', async (route) => {
			saved = {
				url: route.request().url(),
				key: await route.request().headerValue('x-setup-key'),
				body: route.request().postDataJSON()
			};
			return route.fulfill({ json: { key: 'net.public_host', type: 'text', set: false } });
		});

		await page.goto(`${srv.url}/config`);
		await expect(page.getByRole('heading', { name: 'First-time setup' })).toBeVisible();

		await page.locator('#setup-key').fill(KEY);
		await page.getByRole('button', { name: 'Start / resume setup' }).click();

		await expect(page.getByRole('button', { name: /Site & network/ })).toBeVisible();
		expect(stepsKey).toBe(KEY);

		const card = page.locator('li', { has: page.locator('input[id="setup-net.public_host"]') });
		await card.locator('input[id="setup-net.public_host"]').fill('andrii.ai');
		await card.getByRole('button', { name: 'Save', exact: true }).click();

		await expect
			.poll(() => saved, { message: 'expected one setup-key save POST' })
			.not.toBeNull();
		expect(saved.url).toMatch(/\/apostille\/api\/v1\/setup\/keys\/net\.public_host$/);
		expect(saved.key).toBe(KEY);
		expect(saved.body).toEqual({ type: 'text', value: 'andrii.ai' });
		await expect(card.getByText('“net.public_host” updated and audited.')).toBeVisible();
	} finally {
		await srv.close();
	}
});
