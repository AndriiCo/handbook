// Free form fill (R3, plan §6, CUJ D6): the /order Form step fills the
// office's own form with no order and no payment, a signed-out visitor
// gets a sign-in link that brings them back to the same step, and /login
// honours only a same-origin /apostille/ `?next=` (also across an OAuth
// round trip, remembered by the tab). API mocked like commerce.spec.mjs.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const fx = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', name), 'utf8'));
const CSRF = 'fill-csrf-1';
const FILL = fx('orders-form-fill.json');
const FORM_STEP = '/apostille/order?destination=usa-wa-sos&path=birth-certificate&step=form';
const PROVIDER = 'https://accounts.example.test';
/** A stand-in for the filled PDF (the real bytes: main.rs `a_free_fill_pdf_opens_for_its_filler_only`). */
const PDF_BYTES = '%PDF-1.7\n% SYNTHETIC filled form\n%%EOF\n';

/**
 * @param {import('@playwright/test').Page} page
 * @param {{ signedIn?: boolean, fill?: 'ok' | 'unauthorized' | 'daily-cap', plan?: object }} opts
 */
async function mock(page, { signedIn = true, fill = 'ok', plan = fx('orders-plan.json') } = {}) {
	const state = { signedIn };
	const calls = [];
	await page.route('**/apostille/api/v1/**', async (route) => {
		const req = route.request();
		const path = new URL(req.url()).pathname.replace(/^.*\/api\/v1/, '');
		const method = req.method();
		const body = method === 'POST' ? (req.postDataJSON() ?? {}) : null;
		calls.push({ method, path, body, csrf: req.headers()['x-csrf'] ?? null });
		const json = (data, status = 200) => route.fulfill({ status, json: data });
		if (method === 'GET') {
			if (path === '/authorities') return json(fx('authorities.json'));
			if (path === '/authorities/usa-wa-sos/facts') return json(fx('facts-usa-wa-sos.json'));
			if (path === '/authorities/usa-wa-sos/plan') return json(plan);
			if (path === '/auth/providers') return json(fx('providers.json'));
			if (path === '/me')
				return json(state.signedIn ? { customer_ref: 'cust_demo01', expires_at: 1893456000 } : { user: null });
			if (path === '/auth/me') return json(state.signedIn ? { user_id: 'usr_demo01' } : { user: null });
			// The filler's own PDF, as get_fill serves it (signed out: 401).
			if (path === `/p2/forms/fills/${FILL.fill_id}.pdf`) {
				if (!state.signedIn) return json({ error: 'unauthorized' }, 401);
				return route.fulfill({
					status: 200,
					headers: { 'content-type': 'application/pdf', 'cache-control': 'private, no-store' },
					body: PDF_BYTES
				});
			}
		}
		if (method === 'POST') {
			if (path === `/p2/forms/${FILL.form_id}/fill`) {
				if (!state.signedIn || fill === 'unauthorized') return json({ error: 'unauthorized' }, 401);
				if (fill === 'daily-cap') return json({ error: 'too many filled forms today' }, 429);
				return json(FILL, 201);
			}
			if (path === '/auth/google/start') return json({ auth_url: `${PROVIDER}/authorize?state=synthetic` });
		}
		return json({ error: 'mock' }, 404);
	});
	await page.route(`${PROVIDER}/**`, (route) =>
		route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>Provider</title><h1>Synthetic provider</h1>' })
	);
	return { calls, state };
}

/** Uncaught exceptions only: mocked 401/404 resource lines are expected. */
function watchErrors(page) {
	const errors = [];
	page.on('pageerror', (e) => errors.push(String(e)));
	return errors;
}

/**
 * Reflow (WCAG 1.4.10, 320 CSS px): no link or button in the free-fill
 * panel runs past its padding, and the page never scrolls sideways.
 * @param {import('@playwright/test').Page} page
 */
async function expectReflow(page) {
	const seen = await page.evaluate(() => {
		const panel = /** @type {HTMLElement} */ (document.querySelector('[data-free-fill]'));
		const style = getComputedStyle(panel);
		const inner =
			panel.getBoundingClientRect().right - parseFloat(style.paddingRight) - parseFloat(style.borderRightWidth);
		const root = document.documentElement;
		return {
			sideScroll: root.scrollWidth - root.clientWidth,
			pastPadding: [...panel.querySelectorAll('a, button')]
				.filter((el) => el.getBoundingClientRect().right > inner + 0.5)
				.map((el) => (el.textContent ?? '').trim())
		};
	});
	expect(seen).toEqual({ sideScroll: 0, pastPadding: [] });
}

async function open(browser, size = { width: 1440, height: 900 }) {
	const srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
	const context = await browser.newContext({ viewport: size });
	await context.addCookies([{ name: 'hb_csrf', value: CSRF, url: srv.url }]);
	const page = await context.newPage();
	return {
		srv,
		page,
		close: async () => {
			await context.close();
			await srv.close();
		}
	};
}

const answer = async (page) => {
	await page.getByLabel(/Your full name/).fill('SYNTHETIC Customer');
	await page.getByLabel(/Country where the document will be used/).fill('SYNTHETIC Country');
	await page.getByLabel(/Number of documents/).fill('2');
};

for (const size of [
	{ width: 320, height: 640 },
	{ width: 390, height: 844 },
	{ width: 1440, height: 900 }
]) {
	test(`signed in: the Form step fills the office form for free, with no order, at ${size.width}`, async ({
		browser
	}) => {
		const { srv, page, close } = await open(browser, size);
		const errors = watchErrors(page);
		const { calls } = await mock(page);
		try {
			await page.goto(`${srv.url}${FORM_STEP}`);
			await expect(page.locator('[aria-current="step"]')).toContainText('Form');
			const panel = page.locator('[data-free-fill]');
			await expect(panel.getByRole('heading', { name: 'Fill it in online — free' })).toBeVisible();
			await expect(panel).toContainText('SYNTHETIC Apostille Request Form');
			// Only what the server does: the fill is kept with the account
			// until a deletion request, its PDF opens for that account only,
			// and nothing lists it later.
			await expect(panel).toContainText(
				'We keep your answers and the filled PDF with your account until you ask us to delete them. Only your account can open the download link. Your account does not list past fills, so save the PDF to your device.'
			);
			await expect(panel).not.toContainText('download it again');
			const fillButton = panel.getByRole('button', { name: 'Download the filled form (free)' });
			await expectReflow(page);

			// Nothing answered or confirmed: the shared validator lists it all
			// and nothing is sent.
			await fillButton.click();
			const problems = panel.getByRole('alert');
			await expect(problems).toContainText('Your full name is required');
			await expect(problems).toContainText('Number of documents is required');
			await expect(problems).toContainText('Tick the box to confirm');
			expect(calls.some((c) => c.path.endsWith('/fill'))).toBe(false);

			// Answers clear their lines as they come; the confirmation is last.
			await answer(page);
			await expect(problems).not.toContainText('Your full name is required');
			await expect(problems).toContainText('Tick the box to confirm');
			await panel.getByRole('checkbox', { name: /general information, not legal advice/ }).check();
			await expect(panel.getByRole('alert')).toHaveCount(0);
			await fillButton.click();

			const pdfLink = panel.getByRole('link', { name: 'Download your filled form (PDF)' });
			await expect(pdfLink).toBeVisible();
			await expect(pdfLink).toBeFocused();
			await expect(pdfLink).toHaveAttribute('href', FILL.pdf_url);
			await expect(
				panel.getByText(
					'Print it, sign it, and send it with your documents and payment — the guide lists the address and fee.'
				)
			).toBeVisible();
			await expect(panel.getByRole('link', { name: /^Open the guide for / })).toHaveAttribute(
				'href',
				'/apostille/authority/usa/washington'
			);
			await expectReflow(page);

			const posted = calls.filter((c) => c.method === 'POST');
			expect(posted).toHaveLength(1);
			expect(posted[0]).toMatchObject({ path: `/p2/forms/${FILL.form_id}/fill`, csrf: CSRF });
			expect(posted[0].body).toEqual({
				values: {
					RequesterName: 'SYNTHETIC Customer',
					Country: 'SYNTHETIC Country',
					DocCount: '2',
					ReturnMail: false
				}
			});

			// The link downloads the server's pdf_url under a readable name.
			// Chromium fetches a download outside page routes, so the same
			// href is fetched in the page to check what the route serves;
			// the owner-only rule itself is main.rs
			// `a_free_fill_pdf_opens_for_its_filler_only`.
			const [file] = await Promise.all([page.waitForEvent('download'), pdfLink.click()]);
			expect(file.url()).toBe(`${srv.url}${FILL.pdf_url}`);
			expect(file.suggestedFilename()).toBe('SYNTHETIC Apostille Request Form (filled).pdf');
			const served = await page.evaluate(async (href) => {
				const r = await fetch(href, { credentials: 'same-origin' });
				return {
					status: r.status,
					type: r.headers.get('content-type'),
					cache: r.headers.get('cache-control'),
					body: await r.text()
				};
			}, (await pdfLink.getAttribute('href')) ?? '');
			expect(served).toEqual({
				status: 200,
				type: 'application/pdf',
				cache: 'private, no-store',
				body: PDF_BYTES
			});

			// No order, quote, checkout or delivery options: the free route
			// never asks for payment, and the visitor stays on the Form step.
			expect(calls.filter((c) => c.path.startsWith('/p2/orders'))).toEqual([]);
			await expect(page.locator('[aria-current="step"]')).toContainText('Form');

			// A changed answer retires the stale PDF until the form is filled again.
			await page.getByLabel(/Number of documents/).fill('3');
			await expect(pdfLink).toHaveCount(0);
			await expect(fillButton).toBeEnabled();
			expect(errors).toEqual([]);
		} finally {
			await close();
		}
	});
}

test('signed out: the Form step links to sign-in with next, and sign-in returns there', async ({ browser }) => {
	const { srv, page, close } = await open(browser);
	const errors = watchErrors(page);
	const { calls, state } = await mock(page, { signedIn: false });
	try {
		// Walk the stepper: the link follows the address the steps write.
		await page.goto(`${srv.url}/apostille/order?destination=usa-wa-sos`);
		await expect(page.locator('[aria-current="step"]')).toContainText('Document');
		await page.getByText('Birth certificate', { exact: true }).click();
		await page.getByRole('button', { name: 'Continue' }).click();
		await expect(page.locator('[aria-current="step"]')).toContainText('Form');

		const panel = page.locator('[data-free-fill]');
		const signIn = panel.getByRole('link', { name: 'Sign in to fill it in' });
		await expect(signIn).toBeVisible();
		await expect(panel).toContainText(
			'The free fill needs a free account. Only your account can open the filled PDF.'
		);
		await expect(panel.getByRole('button', { name: /Download the filled form/ })).toHaveCount(0);
		await expect(panel.getByRole('checkbox')).toHaveCount(0);
		const nextOf = async () => {
			const href = (await signIn.getAttribute('href')) ?? '';
			expect(href).toMatch(/^\/apostille\/login\?next=/);
			return decodeURIComponent(href.split('next=')[1]);
		};
		expect(await nextOf()).toBe(FORM_STEP);

		// The answers so far stay in this tab across the sign-in.
		await page.getByLabel(/Your full name/).fill('SYNTHETIC Customer');
		await signIn.click();
		await expect(page).toHaveURL(`${srv.url}/apostille/login?next=${encodeURIComponent(FORM_STEP)}`);
		await expect(page.getByRole('button', { name: 'Sign in with passkey' })).toBeVisible();

		// OAuth: the provider's callback sets the session and lands on
		// /login with no query; the tab remembered `next` before it left.
		await page.getByRole('button', { name: 'Continue with Google' }).click();
		await expect(page).toHaveURL(new RegExp(`^${PROVIDER}/authorize`));
		state.signedIn = true;
		await page.goto(`${srv.url}/apostille/login`);
		await expect(page).toHaveURL(`${srv.url}${FORM_STEP}`);
		await expect(page.locator('[aria-current="step"]')).toContainText('Form');
		await expect(page.getByLabel(/Your full name/)).toHaveValue('SYNTHETIC Customer');
		await expect(panel.getByRole('button', { name: 'Download the filled form (free)' })).toBeVisible();

		// Landing on the Form step keeps every query parameter in `next`.
		state.signedIn = false;
		await page.goto(`${srv.url}${FORM_STEP.replace('&step=form', '&kind=mail_kit&step=form')}`);
		await expect(signIn).toBeVisible();
		expect(await nextOf()).toBe(
			'/apostille/order?destination=usa-wa-sos&path=birth-certificate&kind=mail_kit&step=form'
		);
		// The only POST was the OAuth start: nothing was filled signed out.
		expect(calls.filter((c) => c.method === 'POST').map((c) => c.path)).toEqual(['/auth/google/start']);
		expect(errors).toEqual([]);
	} finally {
		await close();
	}
});

test('signed out at 320 px: the sign-in link wraps inside the panel', async ({ browser }) => {
	// Opened at 320 rather than resized mid-test: a resize during a
	// navigation aborts the layout's view transition.
	const { srv, page, close } = await open(browser, { width: 320, height: 640 });
	const errors = watchErrors(page);
	await mock(page, { signedIn: false });
	try {
		await page.goto(`${srv.url}${FORM_STEP}`);
		const panel = page.locator('[data-free-fill]');
		await expect(panel.getByRole('link', { name: 'Sign in to fill it in' })).toBeVisible();
		await expectReflow(page);
		expect(errors).toEqual([]);
	} finally {
		await close();
	}
});

test('a fill the server refuses says why; a lapsed session asks to sign in again', async ({ browser }) => {
	for (const [fill, expected] of [
		// forms_map.rs counts the last 24 hours, so the copy never says "tomorrow".
		['daily-cap', 'Your account has reached the limit of 30 filled forms in 24 hours. Try again later.'],
		['unauthorized', null]
	]) {
		const { srv, page, close } = await open(browser);
		const errors = watchErrors(page);
		await mock(page, { fill });
		try {
			await page.goto(`${srv.url}${FORM_STEP}`);
			const panel = page.locator('[data-free-fill]');
			await answer(page);
			await panel.getByRole('checkbox', { name: /general information, not legal advice/ }).check();
			await panel.getByRole('button', { name: 'Download the filled form (free)' }).click();
			if (expected) {
				await expect(panel.getByRole('alert')).toHaveText(expected);
			} else {
				await expect(panel.getByRole('link', { name: 'Sign in to fill it in' })).toBeVisible();
			}
			await expect(panel.getByRole('link', { name: 'Download your filled form (PDF)' })).toHaveCount(0);
			expect(errors).toEqual([]);
		} finally {
			await close();
		}
	}
});

test('a PDF box longer than the server allows still takes at most 200 characters', async ({ browser }) => {
	// forms_map.rs refuses any value over MAX_FILL_VALUE (200) whatever the
	// box's MaxLen, so the page holds the visitor to the tighter limit.
	const plan = fx('orders-plan.json');
	plan.forms[0].fields.find((f) => f.name === 'RequesterName').max_len = 300;
	const { srv, page, close } = await open(browser);
	const errors = watchErrors(page);
	const { calls } = await mock(page, { plan });
	try {
		await page.goto(`${srv.url}${FORM_STEP}`);
		const panel = page.locator('[data-free-fill]');
		await answer(page);
		await page.getByLabel(/Your full name/).fill('x'.repeat(201));
		await panel.getByRole('checkbox', { name: /general information, not legal advice/ }).check();
		await panel.getByRole('button', { name: 'Download the filled form (free)' }).click();
		await expect(panel.getByRole('alert')).toContainText('Your full name must be at most 200 characters');
		expect(calls.some((c) => c.path.endsWith('/fill'))).toBe(false);

		await page.getByLabel(/Your full name/).fill('x'.repeat(200));
		await expect(panel.getByRole('alert')).toHaveCount(0);
		await panel.getByRole('button', { name: 'Download the filled form (free)' }).click();
		await expect(panel.getByRole('link', { name: 'Download your filled form (PDF)' })).toBeVisible();
		expect(calls.filter((c) => c.path.endsWith('/fill'))).toHaveLength(1);
		expect(errors).toEqual([]);
	} finally {
		await close();
	}
});

test('/login follows only a same-origin /apostille/ next, and only once', async ({ browser }) => {
	const { srv, page, close } = await open(browser);
	const errors = watchErrors(page);
	await mock(page, { signedIn: true });
	try {
		for (const bad of [
			'//evil.example/apostille/',
			'https://evil.example/apostille/orders',
			'/other',
			'/apostille/../x',
			'/apostille/%2e%2e/x',
			'/\\evil.example'
		]) {
			await page.goto(`${srv.url}/apostille/login?next=${encodeURIComponent(bad)}`);
			// "Logged in." renders only when no return target was taken.
			await expect(page.getByText('Logged in.')).toBeVisible();
			expect(new URL(page.url()).pathname, bad).toBe('/apostille/login');
		}

		// Already signed in with a good next: straight back. /login leaves
		// history, so Back skips it instead of bouncing forward again.
		await page.goto(`${srv.url}/apostille/login?next=${encodeURIComponent(FORM_STEP)}`);
		await expect(page).toHaveURL(`${srv.url}${FORM_STEP}`);
		await expect(page.locator('[data-free-fill]')).toBeVisible();
		await page.goBack();
		await expect(page).toHaveURL(`${srv.url}/apostille/login?next=${encodeURIComponent('/\\evil.example')}`);
		await expect(page.getByText('Logged in.')).toBeVisible();

		// A remembered round-trip target is read once: a later visit stays put.
		await page.evaluate(
			([key, next]) => sessionStorage.setItem(key, JSON.stringify({ next, at: Date.now() })),
			['hb_login_next', FORM_STEP]
		);
		await page.goto(`${srv.url}/apostille/login`);
		await expect(page).toHaveURL(`${srv.url}${FORM_STEP}`);
		await page.goto(`${srv.url}/apostille/login`);
		await expect(page.getByText('Logged in.')).toBeVisible();
		expect(errors).toEqual([]);
	} finally {
		await close();
	}
});
