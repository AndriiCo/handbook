// Config table of contents: a click marks that group current even
// when the page cannot scroll it to the top (a short last group), and
// First-boot setup leads the list. API is route-mocked; the page is the
// real built admin SPA.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const CONFIG = JSON.parse(readFileSync(join(HERE, 'fixtures', 'admin-config.json'), 'utf8'));

// Wait until the jump's scroll has stopped, so the intersection
// observer has had its say before the highlight is checked.
async function settle(page) {
	let last = -1;
	for (let i = 0; i < 40; i += 1) {
		const y = await page.evaluate(() => window.scrollY);
		if (y === last) break;
		last = y;
		await page.waitForTimeout(150);
	}
	await page.waitForTimeout(300);
}

test('a clicked group is the current one, first to last', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.setViewportSize({ width: 1440, height: 900 });
		await page.route('**/apostille/api/v1/setup/status', (route) =>
			route.fulfill({ json: { setup_required: false } })
		);
		await page.route('**/apostille/api/v1/config', (route) => route.fulfill({ json: CONFIG }));
		await page.goto(`${srv.url}/config`);

		const toc = page.getByRole('navigation', { name: 'Config groups' });
		const links = toc.getByRole('link');
		await expect(links.first()).toContainText('First-boot setup');

		const count = await links.count();
		for (const i of [count - 1, 0, Math.floor(count / 2), count - 2]) {
			const link = links.nth(i);
			await link.click();
			await settle(page);
			await expect(link).toHaveAttribute('aria-current', 'location');
			await expect(toc.locator('[aria-current="location"]')).toHaveCount(1);
		}

		// The reader's own scrolling hands the highlight back to the observer.
		await links.first().click();
		await settle(page);
		await page.mouse.wheel(0, 1200);
		await settle(page);
		await expect(links.first()).not.toHaveAttribute('aria-current', 'location');
	} finally {
		await srv.close();
	}
});
