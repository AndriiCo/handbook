// Config restart (P393): boot-time keys saved since the last start
// carry a "Restart needed" chip; a pending lockout key (admin.*)
// needs the typed confirmation; Restart shows the overlay, which
// polls /admin/version through the outage until started_at moves,
// then refetches the list in place — no page reload. API is
// route-mocked; the page is the real built admin SPA.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const OLD_START = 1_900_000_000;
const NEW_START = OLD_START + 300;

function configList(startedAt) {
	return {
		keys: [
			// Saved before the running process started: live already.
			{ key: 'net.public_host', type: 'text', value: 'andrii.ai', updated_at: OLD_START - 50 },
			// Saved after it: boot-time keys waiting for a restart.
			{ key: 'labels.fedex_account', type: 'text', value: '123456789', updated_at: OLD_START + 60 },
			{ key: 'labels.fedex_client_id', type: 'secret', set: true, updated_at: OLD_START + 61 },
			{ key: 'admin.allow_ips', type: 'text', value: '203.0.113.0/24', updated_at: OLD_START + 62 },
			// Live key saved after the start: no chip (applies within seconds).
			{
				key: 'labels.fedex_base_url',
				type: 'text',
				value: 'https://apis.fedex.com',
				updated_at: OLD_START + 63
			}
		],
		started_at: startedAt
	};
}

test('restart waits through the outage and refreshes in place', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.setViewportSize({ width: 1280, height: 900 });
		let restarted = false;
		let versionAfter = 0;
		let configGets = 0;
		const posts = [];
		await page.route('**/apostille/api/v1/setup/status', (route) =>
			route.fulfill({ json: { setup_required: false } })
		);
		await page.route('**/apostille/api/v1/config', (route) => {
			configGets += 1;
			return route.fulfill({ json: configList(restarted ? NEW_START : OLD_START) });
		});
		const version = (startedAt) => ({ json: { version: 't', env: 'local', started_at: startedAt } });
		await page.route('**/apostille/api/v1/admin/version', (route) => {
			if (!restarted) return route.fulfill(version(OLD_START));
			versionAfter += 1;
			// Still draining, then down, then the new process.
			if (versionAfter === 1) return route.fulfill(version(OLD_START));
			if (versionAfter === 2) return route.fulfill({ status: 502, body: 'bad gateway' });
			return route.fulfill(version(NEW_START));
		});
		await page.route('**/apostille/api/v1/admin/restart', async (route) => {
			const body = route.request().postDataJSON();
			posts.push(body);
			restarted = true;
			return route.fulfill({
				status: 202,
				json: {
					started_at: OLD_START,
					pending_keys: ['admin.allow_ips', 'labels.fedex_account', 'labels.fedex_client_id'],
					warnings: []
				}
			});
		});

		await page.goto(`${srv.url}/config`);
		await page.evaluate(() => {
			window.__sameDocument = true;
		});

		// Chips: boot keys saved since the start only.
		const chip = (key) => page.locator(`[id="key-${key}"]`).getByText('Restart needed');
		await expect(chip('labels.fedex_account')).toBeVisible();
		await expect(chip('labels.fedex_client_id')).toBeVisible();
		await expect(chip('admin.allow_ips')).toBeVisible();
		await expect(chip('net.public_host')).toHaveCount(0);
		await expect(chip('labels.fedex_base_url')).toHaveCount(0);

		const card = page.getByRole('region', { name: 'Restart the service' });
		await expect(card).toContainText('3 boot-time changes waiting');
		await expect(card).toContainText('admin.allow_ips changed since the last start');

		// admin.* is pending: the button waits for the typed word.
		const button = card.getByRole('button', { name: 'Restart service' });
		await expect(button).toBeDisabled();
		await card.getByLabel('Type “restart” to confirm').fill('restart');
		await expect(button).toBeEnabled();

		const before = configGets;
		await button.click();
		const overlay = page.getByRole('dialog', { name: 'Restarting…' });
		await expect(overlay).toBeVisible();
		expect(posts).toEqual([{ confirm: 'restart' }]);

		// The new start time lands: overlay gone, data refetched in place.
		await expect(overlay).toBeHidden({ timeout: 15_000 });
		await expect(card).toContainText('Restarted.');
		expect(versionAfter).toBeGreaterThanOrEqual(3);
		expect(configGets).toBe(before + 1);
		expect(await page.evaluate(() => window.__sameDocument)).toBe(true);
		await expect(chip('labels.fedex_account')).toHaveCount(0);
		await expect(card).not.toContainText('waiting');
	} finally {
		await srv.close();
	}
});

test('a boot that would fail shows the reason and keeps running', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.route('**/apostille/api/v1/setup/status', (route) =>
			route.fulfill({ json: { setup_required: false } })
		);
		await page.route('**/apostille/api/v1/config', (route) =>
			route.fulfill({
				json: {
					keys: [{ key: 'tls.cert_file', type: 'text', value: '/x.pem', updated_at: OLD_START + 5 }],
					started_at: OLD_START
				}
			})
		);
		await page.route('**/apostille/api/v1/admin/version', (route) =>
			route.fulfill({ json: { version: 't', env: 'local', started_at: OLD_START } })
		);
		await page.route('**/apostille/api/v1/admin/restart', (route) =>
			route.fulfill({
				status: 422,
				json: {
					error: 'restart refused: the next boot would fail: set both',
					reason: 'set both tls.cert_file and tls.key_file, or neither'
				}
			})
		);
		await page.goto(`${srv.url}/config`);
		const card = page.getByRole('region', { name: 'Restart the service' });
		await card.getByLabel('Type “restart” to confirm').fill('restart');
		await card.getByRole('button', { name: 'Restart service' }).click();
		await expect(card).toContainText('set both tls.cert_file and tls.key_file, or neither');
		await expect(page.getByRole('dialog')).toHaveCount(0);
	} finally {
		await srv.close();
	}
});
