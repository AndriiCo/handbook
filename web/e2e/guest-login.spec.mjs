// The guest login on the real public /login page, API route-mocked:
// every way in, the signed-in header, and the note's end at the deadline.
// The server's own rules are tested in crates/web/src/guest.rs.
import { expect, test } from '@playwright/test';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { serveDir } from './helpers/serve.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const GUEST = 'demo@andrii.ai';
const USER = 'd3300000000040008000000000000001';

/**
 * @param {import('@playwright/test').Page} page
 * @param {{ offered?: boolean, ended?: boolean }} [opts]
 */
async function mock(page, { offered = true, ended = false } = {}) {
	const state = { signedIn: false };
	const guestPosts = [];
	await page.route('**/apostille/api/v1/**', async (route) => {
		const req = route.request();
		const path = new URL(req.url()).pathname.replace(/^.*\/api\/v1/, '');
		const json = (data, status = 200) => route.fulfill({ status, json: data });
		if (path === '/auth/providers') {
			return json({
				providers: [],
				...(offered ? { guest: { email: GUEST, last_day: '2026-11-20' } } : {})
			});
		}
		if (path === '/auth/me') return json(state.signedIn ? { user_id: USER, role: 'customer', methods: [] } : { user: null });
		if (path === '/me') return json({ user: null });
		if (path === '/p2/orders') return json({ orders: [], topic: `billing:${USER}` });
		if (path === '/auth/guest/login') {
			guestPosts.push(req.postDataJSON());
			if (ended) return json({ error: 'the guest login has ended' }, 410);
			state.signedIn = true;
			return json({ user_id: USER, guest: true });
		}
		return json({ error: 'mock' }, 404);
	});
	return { guestPosts, state };
}

/** @param {(srvUrl: string) => Promise<void>} body */
async function withSite(body) {
	const srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
	try {
		await body(srv.url);
	} finally {
		await srv.close();
	}
}

test('the button signs in, returns to next and shows the signed-in header', async ({ page }) => {
	const { guestPosts } = await mock(page);
	await withSite(async (url) => {
		await page.goto(`${url}/apostille/login?next=${encodeURIComponent('/apostille/orders')}`);
		await expect(page.getByTestId('guest-note')).toContainText(GUEST);
		await expect(page.getByTestId('guest-note')).toContainText('November 20, 2026');
		await page.getByRole('button', { name: 'Continue as guest' }).click();
		await expect(page).toHaveURL(/\/apostille\/orders$/);
		expect(guestPosts).toEqual([{ email: GUEST }]);
		await expect(page.getByRole('link', { name: 'Log in' })).toHaveCount(0);
		await expect(page.getByRole('button', { name: USER })).toBeVisible();
	});
});

test('the email-link form takes the address and sends no mail', async ({ page }) => {
	const { guestPosts } = await mock(page);
	await withSite(async (url) => {
		await page.goto(`${url}/apostille/login`);
		await page.getByRole('button', { name: 'Use an email link instead' }).click();
		await page.locator('#magic-email').fill(GUEST);
		await page.locator('#magic-email').press('Enter');
		await expect(page.getByText('Logged in.')).toBeVisible();
		expect(guestPosts).toEqual([{ email: GUEST }]);
	});
});

test('a browser without passkeys can still sign in as guest', async ({ page }) => {
	await page.addInitScript(() => {
		Object.defineProperty(navigator, 'credentials', { value: undefined, configurable: true });
	});
	const { guestPosts } = await mock(page);
	await withSite(async (url) => {
		await page.goto(`${url}/apostille/login`);
		await expect(page.getByRole('alert')).toBeVisible();
		await page.getByRole('button', { name: 'Continue as guest' }).click();
		await expect(page.getByText('Logged in.')).toBeVisible();
		expect(guestPosts).toEqual([{ email: GUEST }]);
	});
});

test('after the deadline the note is gone and the address is refused', async ({ page }) => {
	const { state } = await mock(page, { offered: false, ended: true });
	await withSite(async (url) => {
		await page.goto(`${url}/apostille/login`);
		await expect(page.locator('#account')).toBeVisible();
		await expect(page.getByTestId('guest-note')).toHaveCount(0);
		await page.locator('#account').fill(GUEST);
		await page.locator('#account').press('Enter');
		await expect(page.getByRole('alert')).toHaveText('The guest login has ended.');
		expect(state.signedIn).toBe(false);
	});
});
