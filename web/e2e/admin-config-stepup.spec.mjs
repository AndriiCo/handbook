// A config write refused for a stale passkey step-up (403, P409) shows
// the server's x-request-id with a copy button and confirms in place:
// one "Confirm with passkey and save" runs /me → login start → the
// passkey → login finish, then retries the same save. API is
// route-mocked; navigator.credentials.get is a stub.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const CONFIG = JSON.parse(readFileSync(join(HERE, 'fixtures', 'admin-config.json'), 'utf8'));
const REQUEST_ID = '6377942e39bec1dc82efe1b812797868';

test('a stale step-up confirms with the passkey and retries the save', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	const calls = [];
	try {
		await page.addInitScript(() => {
			const buf = (n) => new Uint8Array([n, n, n]).buffer;
			Object.defineProperty(navigator, 'credentials', {
				value: {
					get: async () => ({
						id: 'cred',
						rawId: buf(1),
						type: 'public-key',
						response: {
							authenticatorData: buf(2),
							clientDataJSON: buf(3),
							signature: buf(4),
							userHandle: null
						}
					})
				}
			});
		});
		await page.route('**/apostille/api/v1/setup/status', (route) =>
			route.fulfill({ json: { setup_required: false } })
		);
		await page.route('**/apostille/api/v1/config', (route) => route.fulfill({ json: CONFIG }));
		let puts = 0;
		await page.route('**/apostille/api/v1/config/labels.fedex_client_id', (route) => {
			puts += 1;
			calls.push(`PUT#${puts}`);
			if (puts === 1) {
				return route.fulfill({
					status: 403,
					headers: { 'x-request-id': REQUEST_ID },
					json: {
						error: 'step-up required: log in again first',
						step_up: 'required',
						window_s: 600
					}
				});
			}
			return route.fulfill({ json: { key: 'labels.fedex_client_id', updated_at: 1793750000 } });
		});
		await page.route('**/apostille/api/v1/me', (route) => {
			calls.push('me');
			return route.fulfill({ json: { user_id: 'a'.repeat(32), role: 'admin' } });
		});
		await page.route('**/apostille/api/v1/auth/passkey/login/start', (route) => {
			calls.push('start');
			return route.fulfill({
				json: { challenge_id: 'c1', options: { publicKey: { challenge: 'AQID' } } }
			});
		});
		await page.route('**/apostille/api/v1/auth/passkey/login/finish', (route) => {
			calls.push('finish');
			return route.fulfill({ json: { user_id: 'a'.repeat(32) } });
		});

		await page.goto(`${srv.url}/config?q=fedex_client_id`);
		const input = page.locator('#cfg-labels\\.fedex_client_id');
		await input.fill('production-client-id');
		const row = page.locator('li', { has: input });
		await row.getByRole('button', { name: 'Save', exact: true }).click();

		await expect(row.getByText('Confirm it’s you.')).toBeVisible();
		await expect(row.locator('code', { hasText: REQUEST_ID })).toBeVisible();
		await expect(row.getByText(`request ${REQUEST_ID}`, { exact: false })).toBeVisible();
		await expect(row.getByRole('button', { name: 'Copy request ID' })).toBeVisible();

		await row.getByRole('button', { name: 'Confirm with passkey and save' }).click();
		await expect(row.getByText('Saved and audited')).toBeVisible();
		// The admin shell reads /me on load too; the flow starts at the save.
		expect(calls.slice(calls.indexOf('PUT#1'))).toEqual(['PUT#1', 'me', 'start', 'finish', 'PUT#2']);
	} finally {
		await srv.close();
	}
});
