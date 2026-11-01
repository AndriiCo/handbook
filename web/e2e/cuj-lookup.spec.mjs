// CUJ J1 — anonymous handbook lookup: open the index, filter,
// open a dossier, read topic tabs. Budget: ≤2 actions + ≤1 input
// (arrival, one filter term, one dossier click — the irreducible
// minimum for "find my authority").
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { cuj } from './helpers/cuj.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));

test('J1 lookup costs at most one search plus one click', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
	const journey = cuj(page);
	try {
		await page.route('**/apostille/api/v1/authorities', (route) =>
			route.fulfill({
				json: {
					authorities: [
						{ slug: 'example-a', name: 'Example Authority', country: 'Exampleland', fact_count: 2 },
						{ slug: 'other-b', name: 'Other Office', country: 'Elsewhere', fact_count: 1 }
					]
				}
			})
		);
		await page.route('**/apostille/api/v1/authorities/example-a/facts', (route) =>
			route.fulfill({
				json: {
					facts: [
						{ topic: 'fees', statement: 'Eight dollars per document.' },
						{ topic: 'hours', statement: 'Walk-ins nine to five.' }
					]
				}
			})
		);

		await journey.goto(`${srv.url}/apostille`);
		await expect(page.getByRole('heading', { name: 'Apostille rules' })).toBeVisible();
		await journey.fill(page.locator('#hero-q'), 'exam');
		await journey.click(page.getByRole('button', { name: /Example Authority/ }));
		// The dossier guide quotes the same statement in its steps, so
		// scope to the Fees topic: the journey proves the fact is found.
		await expect(
			page.getByRole('region', { name: 'Fees' }).getByText('Eight dollars per document.')
		).toBeVisible();

		expect(journey.summary()).toEqual({ actions: 2, inputs: 1 });
	} finally {
		await srv.close();
	}
});
