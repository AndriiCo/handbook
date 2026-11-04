// W-3 fake modes in the staff console: the FAKE ribbon names every
// integration running its local fake (from /admin/version) on every
// screen, and is absent when nothing runs fake. API is route-mocked.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const FAKES = [
	{ slug: 'mail_out', label: 'Outgoing email' },
	{ slug: 'mail_in', label: 'Incoming email' },
	{ slug: 'labels', label: 'FedEx/USPS labels' }
];

/** Signed-in staff shell with the given `fakes` on the version badge. */
async function mockShell(page, fakes) {
	await page.route('**/apostille/api/v1/setup/status', (route) =>
		route.fulfill({ json: { setup_required: false } })
	);
	await page.route('**/apostille/api/v1/me', (route) =>
		route.fulfill({ json: { user_id: 'staff-1', role: 'staff' } })
	);
	await page.route('**/apostille/api/v1/admin/version', (route) =>
		route.fulfill({
			json: { version: '0.1.0+test', env: 'judge', started_at: 1, schema: 13, p2_schema: 2, fakes }
		})
	);
}

test('the FAKE ribbon names every fake integration', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await mockShell(page, FAKES);
		await page.goto(`${srv.url}/dashboard`);
		const ribbon = page.getByTestId('fake-ribbon');
		await expect(ribbon).toBeVisible();
		await expect(ribbon).toContainText('FAKE — Outgoing email, Incoming email, FedEx/USPS labels.');
		await expect(ribbon).toContainText('nothing here sends mail, ships a parcel or charges a card');
	} finally {
		await srv.close();
	}
});

test('no fakes, no ribbon', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await mockShell(page, []);
		await page.goto(`${srv.url}/dashboard`);
		await expect(page.getByText('judge', { exact: true })).toBeVisible();
		await expect(page.getByTestId('fake-ribbon')).toHaveCount(0);
	} finally {
		await srv.close();
	}
});
