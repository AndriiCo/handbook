// Admin IP-denial rendering (P-ADMIN-IP F3): a refused API call
// shows the server's denial verbatim — the detected IP visible on
// the page. API is route-mocked; the page is the real built admin
// SPA. Synthetic documentation-range IP only.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const PEER = '198.51.100.9';

test('triage shows the refused IP from a 403 denial', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.route('**/apostille/api/v1/me', (route) =>
			route.fulfill({ json: { user_id: 'staff', role: 'staff' } })
		);
		await page.route('**/apostille/api/v1/facts*', (route) =>
			route.fulfill({
				status: 403,
				json: {
					error: `admin access is limited to the configured IP range (admin.allow_ips) — your IP is ${PEER}`,
					code: 'ip_not_allowed',
					ip: PEER
				}
			})
		);
		await page.goto(`${srv.url}/triage`);
		await expect(page.getByText('Queue unavailable.')).toBeVisible();
		// The detected IP reaches the visible message, not a log.
		await expect(page.getByText(PEER).first()).toBeVisible();
		await expect(page.getByText('client certificate')).toHaveCount(0);
	} finally {
		await srv.close();
	}
});
