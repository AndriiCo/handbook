// CUJ-1 (configure slice) — first-boot wizard: paste the key,
// fill step 1, move on. Navigation auto-saves dirty fields, so
// configuring a step costs its fills plus one Next — never one
// Save click per field. Budget: ≤3 actions + ≤3 inputs for key
// entry plus one fully-filled step.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { cuj } from './helpers/cuj.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const KEY = `setup_${'cd34'.repeat(16)}`;

test('wizard step costs fills plus one Next, never per-field Saves', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	const journey = cuj(page);
	try {
		await page.route('**/apostille/api/v1/admin/setup/status', (route) =>
			route.fulfill({ json: { setup_required: true } })
		);
		// net.bind is deliberately absent (never set): the wizard
		// must track edits to unset keys, not just server rows.
		await page.route('**/apostille/api/v1/admin/setup/steps', (route) =>
			route.fulfill({
				json: {
					keys: [{ key: 'net.public_host', type: 'text', value: '' }]
				}
			})
		);
		const posts = [];
		await page.route('**/apostille/api/v1/admin/setup/keys/*', async (route) => {
			posts.push({
				key: route.request().url().split('/').pop(),
				body: route.request().postDataJSON()
			});
			return route.fulfill({ json: { key: 'k', type: 'text', set: false } });
		});

		await journey.goto(`${srv.url}/admin/config`);
		await journey.fill(page.locator('#setup-key'), KEY);
		await journey.click(page.getByRole('button', { name: 'Start / resume setup' }));
		await expect(page.getByRole('button', { name: /Site & network/ })).toBeVisible();

		await journey.fill(page.locator('input[id="setup-net.public_host"]'), 'andrii.ai');
		await journey.fill(page.locator('input[id="setup-net.bind"]'), '0.0.0.0');
		await journey.click(page.getByRole('button', { name: /Certificates.*→/ }));

		await expect
			.poll(() => posts.length, { message: 'Next auto-saves both dirty fields' })
			.toBe(2);
		expect(posts).toEqual([
			{ key: 'net.public_host', body: { type: 'text', value: 'andrii.ai' } },
			{ key: 'net.bind', body: { type: 'text', value: '0.0.0.0' } }
		]);
		await expect(page.getByRole('heading', { name: '2. Certificates (ACME)' })).toBeVisible();

		expect(journey.summary()).toEqual({ actions: 3, inputs: 3 });
	} finally {
		await srv.close();
	}
});
