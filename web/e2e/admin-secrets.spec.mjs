// Config secrets never look like a login (P410): no password-type
// field on /config, so neither the browser nor a password manager
// offers to save an API key. Secret fields are masked text inputs with
// autofill off, manager opt-outs, a write-only hint, and a Show/Hide
// toggle. API is route-mocked; the page is the real built admin SPA.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const CONFIG = JSON.parse(readFileSync(join(HERE, 'fixtures', 'admin-config.json'), 'utf8'));

test('config secrets are masked text fields password managers skip', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.route('**/apostille/api/v1/setup/status', (route) =>
			route.fulfill({ json: { setup_required: false } })
		);
		await page.route('**/apostille/api/v1/config', (route) => route.fulfill({ json: CONFIG }));
		await page.goto(`${srv.url}/config?q=fedex_client_secret`);

		const input = page.locator('#cfg-labels\\.fedex_client_secret');
		await expect(input).toBeVisible();
		expect(await page.locator('input[type="password"]').count()).toBe(0);

		await expect(input).toHaveAttribute('type', 'text');
		await expect(input).toHaveAttribute('autocomplete', 'off');
		await expect(input).toHaveAttribute('data-1p-ignore', '');
		await expect(input).toHaveAttribute('data-lpignore', 'true');
		await expect(input).toHaveAttribute('data-bwignore', 'true');
		await expect(input).toHaveAttribute('data-form-type', 'other');
		await expect(input).toHaveAccessibleName(/write-only/);
		await expect(input).toHaveAccessibleDescription(/never shown again/);

		const masking = () =>
			input.evaluate((el) => getComputedStyle(el).getPropertyValue('-webkit-text-security'));
		await input.fill('sk-production-secret');
		expect(await masking()).toBe('disc');

		const row = page.locator('li', { has: input });
		const toggle = row.getByRole('button', { name: 'Show value' });
		await expect(toggle).toHaveAttribute('aria-pressed', 'false');
		await toggle.click();
		await expect(row.getByRole('button', { name: 'Hide value' })).toHaveAttribute(
			'aria-pressed',
			'true'
		);
		expect(await masking()).toBe('none');
		await expect(input).toHaveValue('sk-production-secret');
	} finally {
		await srv.close();
	}
});
