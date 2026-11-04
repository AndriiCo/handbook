// The guided answer (/route): the home hero's two countries lead to a
// shareable page that names the method from accepted HCCH facts, the
// issuing office from the place API, and offers an order only for a
// document issued in the United States. Clean at 390 and 1440 px.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));

const AUTHORITIES = [
	{ slug: 'usa-wa-sos', name: 'Washington Secretary of State', country: 'USA', region: 'Washington', kind: 'state', fact_count: 5 },
	{ slug: 'mx-sre', name: 'Secretaría de Relaciones Exteriores', country: 'Mexico', kind: 'country', fact_count: 3 },
	{ slug: 'egypt', name: 'Egypt — destination', country: 'Egypt', kind: 'country', fact_count: 2 }
];
const HCCH = [
	{
		id: 1,
		status: 'accepted',
		topic: 'jurisdiction',
		statement: 'Parties to the Apostille Convention include the United States and Mexico.'
	}
];
const EGYPT = [
	{ id: 9, status: 'accepted', topic: 'legalization_chain', statement: 'Authenticate at the U.S. Department of State.' },
	{ id: 10, status: 'accepted', topic: 'legalization_chain', statement: 'Legalize at the Embassy of Egypt.' }
];
const LOCATIONS = {
	usa: {
		location: { path: 'usa', label: 'United States', iso2: 'US', subdivision: null },
		crumbs: [{ path: 'usa', label: 'United States' }],
		groups: [],
		places: [{ path: 'usa/washington', label: 'Washington' }],
		routes: [],
		by_place: ['vital_record']
	},
	'usa/washington': {
		location: { path: 'usa/washington', label: 'Washington', iso2: 'US', subdivision: 'WA' },
		crumbs: [],
		groups: [],
		places: [],
		routes: [
			{
				doc_class: 'vital_record',
				office: { slug: 'usa-wa-sos', name: 'Washington Secretary of State', path: 'usa/washington' },
				prereq: 'a certified copy from the Department of Health',
				source_uri: 'https://www.sos.wa.gov/apostilles',
				fact_ids: [],
				verified: true,
				inherited: false
			}
		],
		by_place: []
	},
	mx: {
		location: { path: 'mx', label: 'Mexico', iso2: 'MX', subdivision: null },
		crumbs: [],
		groups: [],
		places: [],
		routes: [],
		by_place: []
	}
};

async function mock(page) {
	await page.route('**/apostille/api/v1/**', (route) => {
		const path = new URL(route.request().url()).pathname.replace('/apostille/api/v1', '');
		if (path === '/authorities') return route.fulfill({ json: { authorities: AUTHORITIES } });
		if (path === '/authorities/hcch/facts') return route.fulfill({ json: { facts: HCCH } });
		if (path === '/authorities/egypt/facts') return route.fulfill({ json: { facts: EGYPT } });
		if (path.startsWith('/locations/')) {
			const key = decodeURIComponent(path.slice('/locations/'.length));
			return LOCATIONS[key]
				? route.fulfill({ json: LOCATIONS[key] })
				: route.fulfill({ status: 404, json: {} });
		}
		if (path === '/notify/status') return route.fulfill({ json: { enabled: false } });
		return route.fulfill({ json: {} });
	});
}

let srv;
test.beforeAll(async () => {
	srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
});
test.afterAll(async () => {
	await srv?.close();
});

test('home hero: two countries lead to a shareable answer', async ({ page }) => {
	await mock(page);
	await page.goto(`${srv.url}/apostille/?l=en`);
	await page.locator('#hero-q').fill('Mexi');
	await page.getByRole('option', { name: 'Mexico' }).click();
	await page.locator('#hero-to').fill('United St');
	await page.getByRole('option', { name: 'United States' }).click();
	await page.getByRole('button', { name: 'Show me what to do' }).click();
	await expect(page).toHaveURL(/\/apostille\/route\?from=mx&to=us/);
	await expect(page.getByRole('heading', { level: 1 })).toContainText('Mexico');
});

test('US vital record for Mexico: apostille, the state office, an order', async ({ page }) => {
	await mock(page);
	await page.goto(`${srv.url}/apostille/route?from=usa&to=mx&doc=vital_record&l=en`);
	await expect(page.locator('[data-method]')).toHaveAttribute('data-method', 'apostille');
	// The US routes vital records by state: pick Washington.
	await page.getByRole('link', { name: 'Washington' }).click();
	await expect(page).toHaveURL(/from=usa%2Fwashington|from=usa\/washington/);
	await expect(page.getByRole('link', { name: 'Washington Secretary of State' })).toBeVisible();
	await expect(page.getByText('First: a certified copy from the Department of Health')).toBeVisible();
	await expect(page.locator('[data-cta=order]')).toHaveAttribute('href', /order\?destination=usa-wa-sos/);
});

test('Mexican document for the US: guide only, no order', async ({ page }) => {
	await mock(page);
	await page.goto(`${srv.url}/apostille/route?from=mx&to=us&l=en`);
	await expect(page.locator('[data-method]')).toHaveAttribute('data-method', 'apostille');
	await expect(page.getByRole('heading', { name: 'This happens in Mexico' })).toBeVisible();
	await expect(page.locator('[data-cta=order]')).toHaveCount(0);
});

for (const width of [390, 1440]) {
	test(`FBI check for Egypt at ${width}px: legalization with the embassy chain`, async ({ page }) => {
		const errors = [];
		page.on('pageerror', (e) => errors.push(String(e)));
		await page.setViewportSize({ width, height: 900 });
		await mock(page);
		await page.goto(`${srv.url}/apostille/route?from=usa&to=eg&doc=fbi_check&l=en`);
		await expect(page.locator('[data-method]')).toHaveAttribute('data-method', 'legalization');
		await expect(page.getByText('Legalize at the Embassy of Egypt.')).toBeVisible();
		await expect(page.getByRole('link', { name: 'Full guide for Egypt' })).toBeVisible();
		const overflow = await page.evaluate(
			() => document.documentElement.scrollWidth - document.documentElement.clientWidth
		);
		expect(overflow).toBeLessThanOrEqual(0);
		expect(errors).toEqual([]);
	});
}
