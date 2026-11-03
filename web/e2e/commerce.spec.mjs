// Commerce walk (SA-28, non-gating for lanes): /order Destination →
// Document → Form → Delivery → Pay against the contract fixtures
// (web/e2e/fixtures/orders-*.json), then the cabinet (/orders,
// /orders/[id], /billing, /shipping) live on recorded ids-only WS
// events. Writes web/shots/commerce/*.png (390/1440, light/dark) for
// the states the static shots gate cannot reach: signed-in delivery,
// the quote, the 503 paused state.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { mkdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const SHOTS = join(HERE, '..', 'shots', 'commerce');
mkdirSync(SHOTS, { recursive: true });
const fx = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', name), 'utf8'));
const CSRF = 'commerce-csrf-1';
const CHECKOUT = 'https://checkout.example.test/c/pay/cs_test_synthetic';

/**
 * Route-mock the API like the real server: contract seeds, CSRF pair
 * recorded on every POST, a WS bus that replays the recorded events.
 * @param {import('@playwright/test').Page} page
 * @param {{ signedIn?: boolean, checkout?: 'paused' | 'redirect' }} opts
 */
async function mock(page, { signedIn = true, checkout = 'paused' } = {}) {
	const calls = [];
	await page.route('**/apostille/api/v1/**', async (route) => {
		const req = route.request();
		const path = new URL(req.url()).pathname.replace(/^.*\/api\/v1/, '');
		const method = req.method();
		const body = method === 'POST' ? (req.postDataJSON() ?? {}) : null;
		calls.push({ method, path, body, csrf: req.headers()['x-csrf'] ?? null, cookie: req.headers()['cookie'] ?? '' });
		const json = (data, status = 200) => route.fulfill({ status, json: data });
		if (method === 'GET') {
			if (path === '/authorities') return json(fx('authorities.json'));
			if (path === '/authorities/usa-wa-sos/facts') return json(fx('facts-usa-wa-sos.json'));
			if (path === '/authorities/usa-wa-sos/plan') return json(fx('orders-plan.json'));
			if (path === '/auth/providers') return json(fx('providers.json'));
			if (path === '/me')
				return json(signedIn ? { customer_ref: 'cust_demo01', expires_at: 1893456000 } : { user: null });
			if (path === '/auth/me') return json({ user: null });
			if (path === '/p2/orders/options') return json(fx('orders-options.json'));
			if (path === '/p2/orders') return signedIn ? json(fx('orders-list.json')) : json({ error: 'unauthorized' }, 401);
			if (path === '/p2/orders/ord_b3Nz8wQ1tY') return json(fx('orders-order-free.json'));
			if (/^\/p2\/orders\/ord_[A-Za-z0-9]+$/.test(path)) return json(fx('orders-order.json'));
		}
		if (method === 'POST') {
			if (/^\/p2\/forms\/\d+\/fill$/.test(path)) return json(fx('orders-form-fill.json'), 201);
			if (path === '/p2/orders')
				return body?.kind === 'address_label'
					? json(fx('orders-order-free.json'), 201)
					: json(fx('orders-created.json'), 201);
			if (/\/quote$/.test(path)) return json(fx('orders-quote.json'));
			if (/\/checkout$/.test(path))
				return checkout === 'paused' ? json(fx('orders-paused.json'), 503) : json({ checkout_url: CHECKOUT });
			if (/\/confirm$/.test(path)) return json(fx('orders-order.json'));
		}
		return json({ error: 'mock' }, 404);
	});
	await page.route('https://checkout.example.test/**', (route) =>
		route.fulfill({
			contentType: 'text/html',
			body: '<!doctype html><title>Checkout</title><h1>Synthetic checkout</h1>'
		})
	);
	const billing = fx('orders-ws-billing.json');
	const shipment = fx('orders-ws-shipment.json');
	await page.routeWebSocket('**/ws/bus*', (ws) => {
		const topic = new URL(ws.url()).searchParams.get('topic') ?? '';
		ws.send(JSON.stringify({ type: 'subscribed', topic }));
		if (topic.startsWith('billing:')) for (const ev of billing) ws.send(JSON.stringify(ev));
		if (topic === 'shipment:51') for (const ev of shipment) ws.send(JSON.stringify(ev));
	});
	return calls;
}

/** Settle the step's view transition, then capture from the top. */
async function snap(page, file, fullPage = true) {
	await page.waitForTimeout(450);
	if (fullPage) await page.evaluate(() => window.scrollTo(0, 0));
	await page.screenshot({ path: join(SHOTS, file), fullPage });
}

/** Uncaught exceptions only: the 503/401 resource lines are expected. */
function watchErrors(page) {
	const errors = [];
	page.on('pageerror', (e) => errors.push(String(e)));
	return errors;
}

async function walkToDelivery(page) {
	await expect(page.locator('[aria-current="step"]')).toContainText('Destination');
	await page.getByRole('combobox').click();
	await page.getByRole('option', { name: /Washington Secretary of State/ }).click();
	await expect(page.getByText('From the accepted plan:')).toBeVisible();
	await page.getByRole('button', { name: 'Continue' }).click();

	await expect(page.locator('[aria-current="step"]')).toContainText('Document');
	await page.getByText('Birth certificate', { exact: true }).click();
	await page.getByRole('button', { name: 'Continue' }).click();

	await expect(page.locator('[aria-current="step"]')).toContainText('Form');
	await page.getByLabel(/Your full name/).fill('SYNTHETIC Customer');
	await page.getByLabel(/Country where the document will be used/).fill('SYNTHETIC Country');
	await page.getByLabel(/Number of documents/).fill('1');
	await page.getByRole('button', { name: 'Continue' }).click();
	await expect(page.locator('[aria-current="step"]')).toContainText('Delivery');
	await expect(page.locator('[data-kind]')).toHaveCount(4);
}

async function fillReturnAddress(page) {
	await page.getByLabel('Full name').fill('SYNTHETIC Customer');
	await page.getByLabel('Street address').fill('1 Example Street');
	await page.getByLabel('City').fill('Springfield');
	await page.getByLabel('State or region').fill('IL');
	await page.getByLabel('Postal code').fill('62701');
	await page.getByLabel('Country code').fill('US');
}

for (const size of [
	{ width: 390, height: 844 },
	{ width: 1440, height: 900 }
]) {
	for (const theme of ['light', 'dark']) {
		test(`order → delivery → quote → paused → free label at ${size.width} ${theme}`, async ({ browser }) => {
			test.setTimeout(90_000);
			const srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
			const context = await browser.newContext({ viewport: size, colorScheme: theme });
			await context.addInitScript((t) => {
				try {
					localStorage.setItem('theme', t);
				} catch {}
			}, theme);
			await context.addCookies([{ name: 'hb_csrf', value: CSRF, url: srv.url }]);
			const page = await context.newPage();
			const errors = watchErrors(page);
			const calls = await mock(page);
			const tag = `${size.width}x${size.height}-${theme}`;
			try {
				await page.goto(`${srv.url}/apostille/order`);
				await walkToDelivery(page);

				// Disabled card: reason from the API, citation one tap away.
				const fedex = page.locator('[data-kind="label_fedex"]');
				await expect(fedex.locator('input')).toBeDisabled();
				await expect(fedex).toContainText('PO Box, which FedEx cannot deliver to');
				await fedex.getByRole('button', { name: /cited fact 203/ }).click();
				await expect(page.getByText('Mail requests to PO Box 4000')).toBeVisible();
				await snap(page, `delivery-fact-${tag}.png`, false);
				await page.keyboard.press('Escape');

				await page.getByText('Mailing kit shipped to you').click();
				await fillReturnAddress(page);
				await snap(page, `delivery-${tag}.png`);

				await page.getByRole('button', { name: 'Get quote' }).click();
				await expect(page.getByRole('heading', { name: 'Review and pay' })).toBeVisible();
				await expect(page.locator('[aria-current="step"]')).toContainText('Pay');
				await expect(page.getByRole('button', { name: /Pay \$49\.00/ })).toBeVisible();
				await snap(page, `pay-${tag}.png`);

				const created = calls.find((c) => c.method === 'POST' && c.path === '/p2/orders');
				expect(created?.body).toMatchObject({
					slug: 'usa-wa-sos',
					kind: 'mail_kit',
					path_id: 'birth-certificate',
					form_fill_id: 'fil_9b2c4e7d1a',
					return_to: { name: 'SYNTHETIC Customer', postal_code: '62701', country: 'US' }
				});
				for (const c of calls.filter((x) => x.method === 'POST')) {
					expect(c.csrf, `${c.path} carries x-csrf`).toBe(CSRF);
					expect(c.cookie).toContain(`hb_csrf=${CSRF}`);
				}

				// 503 paid_tiers_paused → the paused state offers the free label.
				await page.getByRole('button', { name: /Pay \$49\.00/ }).click();
				await expect(page.getByRole('heading', { name: 'Paid options are paused' })).toBeVisible();
				await expect(page.getByText(fx('orders-paused.json').reason)).toBeVisible();
				await snap(page, `paused-${tag}.png`);

				await page.getByRole('button', { name: 'Use the free label' }).click();
				await expect(page).toHaveURL(/\/apostille\/orders\/ord_b3Nz8wQ1tY$/);
				await expect(page.getByText('Address label ready to print')).toBeVisible();
				const free = calls.filter((c) => c.method === 'POST' && c.path === '/p2/orders').at(-1);
				expect(free?.body?.kind).toBe('address_label');
				expect(errors).toEqual([]);
			} finally {
				await context.close();
				await srv.close();
			}
		});
	}
}

test('pay redirects to the server-authored checkout_url', async ({ browser }) => {
	test.setTimeout(60_000);
	const srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
	const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
	await context.addCookies([{ name: 'hb_csrf', value: CSRF, url: srv.url }]);
	const page = await context.newPage();
	const errors = watchErrors(page);
	await mock(page, { checkout: 'redirect' });
	try {
		await page.goto(`${srv.url}/apostille/order`);
		await walkToDelivery(page);
		await page.getByText('USPS labels with postage, print at home').click();
		await fillReturnAddress(page);
		await page.getByRole('button', { name: 'Get quote' }).click();
		await page.getByRole('button', { name: /Pay \$49\.00/ }).click();
		await expect(page).toHaveURL(CHECKOUT);
		expect(errors).toEqual([]);
	} finally {
		await context.close();
		await srv.close();
	}
});

test('delivery asks anonymous visitors to sign in and keeps their choices', async ({ browser }) => {
	const srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
	const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
	const page = await context.newPage();
	await mock(page, { signedIn: false });
	try {
		await page.goto(
			`${srv.url}/apostille/order?destination=usa-wa-sos&path=birth-certificate&kind=mail_kit&step=delivery`
		);
		const link = page.getByRole('link', { name: 'Sign in to continue' });
		await expect(link).toBeVisible();
		const href = await link.getAttribute('href');
		expect(href).toMatch(/^\/apostille\/login\?next=/);
		const next = decodeURIComponent(href.split('next=')[1]);
		expect(next).toBe(
			'/apostille/order?destination=usa-wa-sos&path=birth-certificate&kind=mail_kit&step=delivery'
		);
		await expect(page.locator('[data-kind="mail_kit"] input')).toBeChecked();
	} finally {
		await context.close();
		await srv.close();
	}
});

test('cabinet: orders, detail live on billing + shipment, confirm, billing, shipping', async ({ browser }) => {
	test.setTimeout(60_000);
	const srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
	const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
	await context.addCookies([{ name: 'hb_csrf', value: CSRF, url: srv.url }]);
	const page = await context.newPage();
	const errors = watchErrors(page);
	const calls = await mock(page);
	try {
		await page.goto(`${srv.url}/apostille/orders`);
		await expect(page.getByText('2 orders, newest first.')).toBeVisible();
		const cards = page.locator('[data-order]');
		await expect(cards).toHaveCount(2);
		await expect(cards.first()).toHaveAttribute('data-order', 'ord_q7Hm2xR4kP');
		await expect(page.getByText('Your kit is on its way')).toBeVisible();
		await cards.first().click();

		await expect(page).toHaveURL(/\/apostille\/orders\/ord_q7Hm2xR4kP$/);
		await expect(page.getByRole('heading', { name: 'Mailing kit' })).toBeVisible();
		await expect(page.getByText('Accepted at USPS origin facility')).toBeVisible();
		await expect(page.locator('[data-leg]')).toHaveCount(3);
		await expect(page.getByText('9400100000000000000001')).toBeVisible();
		await expect(page.getByRole('status').filter({ hasText: 'Live' })).toBeVisible();
		// Ids-only events → the order is re-read (no polling): the
		// initial GET plus at least one coalesced refetch.
		await expect
			.poll(() => calls.filter((c) => c.method === 'GET' && c.path === '/p2/orders/ord_q7Hm2xR4kP').length)
			.toBeGreaterThanOrEqual(2);
		await snap(page, 'cabinet-detail-1440x900-light.png');

		// Stripe success redirect → POST confirm with the CSRF pair.
		await page.goto(`${srv.url}/apostille/orders/ord_q7Hm2xR4kP?checkout=success`);
		await expect(page.getByText('Payment confirmed.')).toBeVisible();
		const confirm = calls.find((c) => c.method === 'POST' && c.path.endsWith('/confirm'));
		expect(confirm?.csrf).toBe(CSRF);
		await expect(page).toHaveURL(/\/apostille\/orders\/ord_q7Hm2xR4kP$/);

		await page.goto(`${srv.url}/apostille/billing`);
		await expect(page.getByRole('link', { name: /View receipt/ })).toBeVisible();
		await expect(page.getByText('$49.00').first()).toBeVisible();

		await page.goto(`${srv.url}/apostille/shipping`);
		await expect(page.getByText('4 shipments across 2 orders.')).toBeVisible();
		await expect(page.getByRole('link', { name: /Track with USPS/ }).first()).toBeVisible();
		expect(errors).toEqual([]);
	} finally {
		await context.close();
		await srv.close();
	}
});
