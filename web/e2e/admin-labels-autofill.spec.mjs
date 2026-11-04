// Label autofill (P416): picking an authority fills ship-to from its
// accepted facts (FedEx street address; the PO box is listed as
// USPS-only and cannot be picked), with the fact cited; "Ship from our
// office" fills ship-from; an order fills both. API is route-mocked;
// the page is the built admin SPA.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const at = (path) => (url) => url.pathname === `/apostille/api/v1${path}`;

const WA_FILL = {
	authority: {
		slug: 'usa-wa-sos',
		name: 'Washington Secretary of State — Apostilles & Authentications'
	},
	name: 'Washington Secretary of State',
	addresses: [
		{
			fact_id: 11,
			topic: 'mailing',
			label: 'apostille mailing address',
			street1: 'PO Box 40228',
			street2: null,
			city: 'Olympia',
			state_province: 'WA',
			postal: '98504-0228',
			country: 'US',
			po_box: true,
			fedex_ok: false,
			quote: 'apostille_mailing_address: PO BOX 40228 Olympia, WA 98504-0228'
		},
		{
			fact_id: 10,
			topic: 'address',
			label: 'apostille main branch street',
			street1: '243 Israel Rd SE',
			street2: 'Town Center 3',
			city: 'Tumwater',
			state_province: 'WA',
			postal: '98501',
			country: 'US',
			po_box: false,
			fedex_ok: true,
			quote: 'apostille_main_branch_street: Town Center 3 243 Israel Rd SE Tumwater, WA 98501'
		},
		{
			fact_id: 13,
			topic: 'address',
			label: 'apostille eastern branch street',
			street1: '960 Washington St',
			street2: null,
			city: 'Cheney',
			state_province: 'WA',
			postal: '99004',
			country: 'US',
			po_box: false,
			fedex_ok: true,
			quote: 'apostille_eastern_branch_street: 960 Washington St Cheney, WA 99004'
		}
	],
	recommended: 1,
	phone: { value: '360-725-0377', fact_id: 12, quote: 'apostille_phone: 360-725-0377' },
	reason: null
};

const OFFICE = {
	from: {
		name: 'Synthetic Notary Office',
		street1: '100 Example Ave',
		street2: 'Suite 1',
		city: 'Bothell',
		state_province: 'WA',
		postal: '98011',
		country: 'US',
		phone: '425-555-0100'
	}
};

test('an authority, the office and an order fill the label', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.route('**/apostille/api/v1/**', (route) => route.fulfill({ json: {} }));
		await page.route(at('/setup/status'), (route) =>
			route.fulfill({ json: { setup_required: false } })
		);
		await page.route(at('/fedex-live/caps'), (route) =>
			route.fulfill({ json: { enabled: true, daily_cap: 2, bought_today: 0, max_usd: 60 } })
		);
		await page.route(at('/admin/authorities'), (route) =>
			route.fulfill({
				json: {
					authorities: [
						{
							slug: 'usa-wa-sos',
							name: 'Washington Secretary of State — Apostilles & Authentications',
							country: 'USA',
							region: 'Washington',
							fact_count: 30,
							kind: 'state'
						}
					]
				}
			})
		);
		await page.route(at('/fedex-live/autofill/authority/usa-wa-sos'), (route) =>
			route.fulfill({ json: WA_FILL })
		);
		await page.route(at('/fedex-live/autofill/office'), (route) => route.fulfill({ json: OFFICE }));
		await page.route(at('/fedex-live/autofill/order/ord_synthetic01'), (route) =>
			route.fulfill({
				json: {
					order_id: 'ord_synthetic01',
					kind: 'label_fedex',
					authority_slug: 'usa-wa-sos',
					from: { ...OFFICE.from, name: 'Synthetic Customer', phone: '206-555-0199' },
					to: WA_FILL
				}
			})
		);

		await page.goto(`${srv.url}/labels`);
		await page.getByRole('combobox', { name: /Ship to an authority/ }).click();
		await page.getByRole('option', { name: /Washington Secretary of State/ }).click();

		// Recommended FedEx address (street, not the PO box), cited.
		await expect(page.locator('#fedex-to-name')).toHaveValue('Washington Secretary of State');
		await expect(page.locator('#fedex-to-street')).toHaveValue('243 Israel Rd SE');
		await expect(page.locator('#fedex-to-street2')).toHaveValue('Town Center 3');
		await expect(page.locator('#fedex-to-city')).toHaveValue('Tumwater');
		await expect(page.locator('#fedex-to-zip')).toHaveValue('98501');
		await expect(page.locator('#fedex-to-phone')).toHaveValue('360-725-0377');
		await expect(page.getByText('PO box: USPS only')).toBeVisible();
		const radios = page.getByRole('radio');
		await expect(radios.nth(0)).toBeDisabled();
		await expect(radios.nth(1)).toBeChecked();

		// Another office on file is one click.
		await radios.nth(2).check();
		await expect(page.locator('#fedex-to-city')).toHaveValue('Cheney');
		await expect(page.locator('#fedex-to-street2')).toHaveValue('');

		await page.getByRole('button', { name: 'Ship from our office' }).click();
		await expect(page.locator('#fedex-from-name')).toHaveValue('Synthetic Notary Office');
		await expect(page.locator('#fedex-from-phone')).toHaveValue('425-555-0100');

		await page.locator('#fill-order').fill('ord_synthetic01');
		await page.getByRole('button', { name: 'Fill from order' }).click();
		await expect(page.locator('#fedex-order')).toHaveValue('ord_synthetic01');
		await expect(page.locator('#fedex-from-name')).toHaveValue('Synthetic Customer');
		await expect(page.locator('#fedex-to-street')).toHaveValue('243 Israel Rd SE');
		// Filled values stay editable.
		await page.locator('#fedex-to-street').fill('243 Israel Rd SE Bldg 3');
		await expect(page.locator('#fedex-to-street')).toHaveValue('243 Israel Rd SE Bldg 3');
	} finally {
		await srv.close();
	}
});
