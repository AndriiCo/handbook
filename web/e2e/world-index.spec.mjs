// World-scale home index (P395): the committed world packs (~500
// authorities in 130 countries) render as one collapsed row per
// country, clean at 390 and 1440 px, with search capped at 50 rows.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { readdirSync, readFileSync } from 'node:fs';

const HERE = dirname(fileURLToPath(import.meta.url));
const WORLD = join(HERE, '..', '..', 'packs', 'world');

function worldIndex() {
	const rows = [];
	for (const file of readdirSync(WORLD).filter((f) => f.endsWith('.json'))) {
		const pack = JSON.parse(readFileSync(join(WORLD, file), 'utf8'));
		for (const a of pack.authorities) {
			rows.push({ ...a, fact_count: 0, aliases: [], path: a.slug, verified_at: null });
		}
	}
	return rows;
}

for (const width of [390, 1440]) {
	test(`world index at ${width}px: one row per country, no overflow`, async ({ page }) => {
		const srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
		const errors = [];
		page.on('pageerror', (e) => errors.push(String(e)));
		page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
		const rows = worldIndex();
		const countries = new Set(rows.map((a) => a.country || 'International'));
		try {
			await page.setViewportSize({ width, height: 900 });
			await page.route('**/apostille/api/v1/authorities', (route) =>
				route.fulfill({ json: { authorities: rows } })
			);
			await page.route('**/apostille/api/v1/authorities/*/facts', (route) =>
				route.fulfill({ json: { facts: [] } })
			);
			await page.route('**/apostille/api/v1/quality', (route) =>
				route.fulfill({ json: {} })
			);
			await page.goto(`${srv.url}/apostille`);

			const byCountry = page.getByRole('list', { name: 'Authorities by country' });
			await expect(byCountry).toBeVisible();
			await expect(byCountry.locator('summary')).toHaveCount(countries.size);
			// Collapsed: only the featured links, no 500-row DOM.
			expect(await byCountry.locator('a').count()).toBe(0);

			await byCountry.locator('summary', { hasText: 'France' }).click();
			const france = rows.filter((a) => a.country === 'France').length;
			await expect(byCountry.locator('details[open] a')).toHaveCount(france);

			await page.locator('#hero-q').fill('ministry');
			const matches = page.getByRole('list', { name: /Authorities matching/ });
			expect(await matches.locator('a').count()).toBeLessThanOrEqual(50);

			const overflow = await page.evaluate(
				() => document.documentElement.scrollWidth - document.documentElement.clientWidth
			);
			expect(overflow).toBeLessThanOrEqual(0);
			expect(await page.locator('a[href*="/authority/"]').count()).toBeGreaterThanOrEqual(3);
			expect(errors).toEqual([]);
		} finally {
			await srv.close();
		}
	});
}
