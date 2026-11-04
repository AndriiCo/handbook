// A refused FedEx quote names the server's reason and its request ID
// (P415): "quote failed (503)" alone sent the operator to the logs.
// The caps panel says which keys switch live buys on.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const REQUEST_ID = 'c6bb2265da2d8900965eb74b2d9bc1dc';

test('a refused quote shows the reason and request ID', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.route('**/apostille/api/v1/setup/status', (route) =>
			route.fulfill({ json: { setup_required: false } })
		);
		await page.route('**/apostille/api/v1/fedex-live/caps', (route) =>
			route.fulfill({ json: { enabled: false, daily_cap: 2, bought_today: 0, max_usd: 60 } })
		);
		await page.route('**/apostille/api/v1/fedex-live/quote', (route) =>
			route.fulfill({
				status: 503,
				headers: { 'x-request-id': REQUEST_ID },
				json: { error: 'live fedex buys are disabled' }
			})
		);
		await page.goto(`${srv.url}/labels`);
		await expect(page.getByText('Live buys are off.')).toBeVisible();
		await expect(page.getByText('labels.live_ship_to_allow')).toBeVisible();

		await page.locator('#fedex-order').fill('vn-demo-001');
		for (const side of ['fedex-from', 'fedex-to']) {
			await page.locator(`#${side}-name`).fill('Synthetic Sender');
			await page.locator(`#${side}-street`).fill('100 Example St');
			await page.locator(`#${side}-city`).fill('Seattle');
			await page.locator(`#${side}-state`).fill('WA');
			await page.locator(`#${side}-zip`).fill('98101');
		}
		await page.getByRole('button', { name: 'Quote' }).click();
		await expect(page.getByText('live fedex buys are disabled (HTTP 503, request')).toBeVisible();
		await expect(page.locator('code', { hasText: REQUEST_ID })).toBeVisible();
	} finally {
		await srv.close();
	}
});
