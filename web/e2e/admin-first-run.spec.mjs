// A judge's first minutes in the staff console (P1-1…P1-6, P1-4): a
// fresh boot sends /review to the /config wizard, one setup-key paste
// opens it and enrols the first passkey from there, Sign out ends the
// session, and sign-in comes back with no user ID (discoverable
// passkey). An expired session mid-use lands on /staff-login?next=…
// and returns to the page after sign-in. API is
// route-mocked with a tiny server state; navigator.credentials is a
// stub (create/get resolve fake credentials). W-1: enrolment names the
// passkey by email, and the sign-in email field offers it in autofill:
// the stub keeps an autofill (conditional) request pending, like a
// browser, until a test picks the passkey or the page aborts it.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const fx = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', name), 'utf8'));
const KEY = `setup_${'ab12'.repeat(16)}`;
const USER = 'c0ffee00c0ffee00c0ffee00c0ffee00';
const STEPS = {
	keys: [
		{ key: 'auth.rp_id', type: 'text', value: 'localhost' },
		{ key: 'auth.origin', type: 'text', value: 'http://localhost:18319' }
	]
};

async function stubPasskeys(page) {
	await page.addInitScript(() => {
		const buf = (n) => new Uint8Array([n, n, n]).buffer;
		if (window.PublicKeyCredential) {
			Object.defineProperty(PublicKeyCredential, 'isConditionalMediationAvailable', {
				value: async () => true,
				configurable: true
			});
		}
		window.__autofillRequests = 0;
		Object.defineProperty(navigator, 'credentials', {
			value: {
				create: async () => ({
					id: 'cred',
					rawId: buf(1),
					type: 'public-key',
					response: { attestationObject: buf(2), clientDataJSON: buf(3) }
				}),
				get: async (args) => {
					if (args?.mediation === 'conditional') {
						window.__autofillRequests += 1;
						await new Promise((resolve, reject) => {
							window.__pickAutofill = resolve;
							args.signal?.addEventListener('abort', () =>
								reject(new DOMException('aborted', 'AbortError'))
							);
						});
					} else if (window.__holdModal) {
						// A passkey sheet the user has not answered yet.
						await new Promise((resolve) => (window.__releaseModal = resolve));
					}
					return {
						id: 'cred',
						rawId: buf(1),
						type: 'public-key',
						response: {
							authenticatorData: buf(2),
							clientDataJSON: buf(3),
							signature: buf(4),
							userHandle: null
						}
					};
				}
			}
		});
	});
}

/** Mock admin API over a small mutable server state. */
async function mockServer(page, server) {
	const calls = [];
	// Registered first = matched last: anything unmocked is a 404, or a
	// 401 once the session is gone (what an expired session answers).
	await page.route('**/apostille/api/v1/**', (route) =>
		route.fulfill(server.session ? { status: 404, json: { error: 'not found' } } : { status: 401, json: { error: 'unauthorized' } })
	);
	await page.route('**/apostille/api/v1/setup/status', (route) =>
		route.fulfill({ json: { setup_required: server.setupRequired } })
	);
	await page.route('**/apostille/api/v1/setup/steps', async (route) => {
		const key = await route.request().headerValue('x-setup-key');
		return key === KEY ? route.fulfill({ json: STEPS }) : route.fulfill({ status: 403, json: { error: 'setup key invalid or window closed' } });
	});
	await page.route('**/apostille/api/v1/me', (route) =>
		server.session
			? route.fulfill({ json: { user_id: USER, role: 'admin' } })
			: route.fulfill({ status: 401, json: { error: 'unauthorized' } })
	);
	await page.route('**/apostille/api/v1/admin/version', (route) =>
		server.session ? route.fulfill({ json: fx('admin-version.json') }) : route.fulfill({ status: 401, json: {} })
	);
	await page.route('**/apostille/api/v1/config', (route) =>
		server.session ? route.fulfill({ json: fx('admin-config.json') }) : route.fulfill({ status: 401, json: {} })
	);
	await page.route('**/apostille/api/v1/auth/passkey/enrol/start', (route) => {
		calls.push({ step: 'enrol/start', body: route.request().postDataJSON() });
		return route.fulfill({
			json: {
				challenge_id: 'e1',
				options: { publicKey: { challenge: 'AQID', user: { id: 'AQID', name: USER, displayName: 'Judge' } } },
				role: 'admin',
				cn: 'setup'
			}
		});
	});
	await page.route('**/apostille/api/v1/auth/passkey/enrol/finish', (route) => {
		const body = route.request().postDataJSON();
		calls.push({ step: 'enrol/finish', body });
		if (body.setup_key !== KEY) return route.fulfill({ status: 410, json: { error: 'invite invalid or expired' } });
		server.setupRequired = false;
		server.session = 'enrolled';
		return route.fulfill({ json: { user_id: USER, role: 'admin', methods: ['passkey'] } });
	});
	await page.route('**/apostille/api/v1/auth/logout', (route) => {
		calls.push({ step: 'logout' });
		server.session = null;
		return route.fulfill({ json: { ok: true } });
	});
	await page.route('**/apostille/api/v1/auth/passkey/login/start', (route) => {
		calls.push({ step: 'login/start', body: route.request().postDataJSON() });
		return route.fulfill({ json: { challenge_id: 'l1', options: { publicKey: { challenge: 'AQID' } } } });
	});
	await page.route('**/apostille/api/v1/auth/passkey/login/finish', (route) => {
		calls.push({ step: 'login/finish' });
		server.session = 'signed-in';
		return route.fulfill({ json: { user_id: USER, methods: ['passkey'] } });
	});
	return calls;
}

test('fresh boot: /review → /config, one paste enrols, sign out, sign back in with no user ID', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await stubPasskeys(page);
		const server = { setupRequired: true, session: null };
		const calls = await mockServer(page, server);

		await page.goto(`${srv.url}/review`);
		await expect(page).toHaveURL(/\/config$/, { timeout: 15_000 });
		await expect(page.getByRole('heading', { name: 'First-time setup' })).toBeVisible();

		// The only paste: the wizard keeps the key for the enrolment.
		await page.locator('#setup-key').fill(KEY);
		await page.getByRole('button', { name: 'Start / resume setup' }).click();
		await expect(page.getByRole('heading', { name: 'Enrol the first staff passkey' })).toBeVisible();
		await page.locator('#enrol-display-name').fill('Judge');
		const enrol = page.getByRole('button', { name: 'Enrol passkey and sign in' });
		await expect(enrol).toBeDisabled(); // the email names the passkey: required
		await page.locator('#enrol-email').fill('judge@example.test');
		await enrol.click();

		await expect(page.getByText('Enrolled and signed in.')).toBeVisible();
		await expect(page.getByText(USER)).toHaveCount(0);
		await expect(page.getByRole('heading', { name: 'Configuration' })).toBeVisible();
		const start = calls.find((c) => c.step === 'enrol/start');
		const finish = calls.find((c) => c.step === 'enrol/finish');
		expect(start.body.setup_key).toBe(KEY);
		expect(start.body.email).toBe('judge@example.test');
		expect(finish.body.setup_key).toBe(KEY);

		await page.getByRole('button', { name: 'Sign out' }).click();
		await expect(page).toHaveURL(/\/staff-login$/);
		expect(calls.some((c) => c.step === 'logout')).toBe(true);
		expect(server.session).toBeNull();

		// No user ID anywhere: the passkey names the account.
		await page.getByRole('button', { name: 'Sign in with passkey' }).click();
		await expect(page).toHaveURL(/\/dashboard$/);
		expect(calls.find((c) => c.step === 'login/start').body).toEqual({ account: '' });
	} finally {
		await srv.close();
	}
});

test('an expired session mid-use signs in again and returns to the page', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await stubPasskeys(page);
		const server = { setupRequired: false, session: 'live' };
		await mockServer(page, server);
		// /me still answers (the shell loads) but the queue says the
		// session is gone, as it does once the 30-minute idle timeout hits.
		await page.route('**/apostille/api/v1/queue**', (route) =>
			server.session === 'live'
				? route.fulfill({ status: 401, json: { error: 'unauthorized' } })
				: route.fulfill({ json: { items: [], total: 0 } })
		);

		await page.goto(`${srv.url}/review`);
		await expect(page).toHaveURL(/\/staff-login\?next=%2Freview$/, { timeout: 15_000 });
		await page.getByRole('button', { name: 'Sign in with passkey' }).click();
		await expect(page).toHaveURL(/\/review$/);
	} finally {
		await srv.close();
	}
});

test('sign-in fits a 390 px phone with a real button', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.setViewportSize({ width: 390, height: 844 });
		await stubPasskeys(page);
		await mockServer(page, { setupRequired: false, session: null });
		await page.goto(`${srv.url}/staff-login`);
		const button = page.getByRole('button', { name: 'Sign in with passkey' });
		await expect(button).toBeVisible();
		const look = await button.evaluate((el) => {
			const css = getComputedStyle(el);
			const box = el.getBoundingClientRect();
			return { bg: css.backgroundColor, left: box.left, right: box.right, height: box.height };
		});
		expect(look.bg).not.toBe('rgba(0, 0, 0, 0)');
		expect(look.left).toBeGreaterThanOrEqual(16);
		expect(look.right).toBeLessThanOrEqual(390 - 16);
		expect(look.height).toBeGreaterThanOrEqual(44);
		expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
	} finally {
		await srv.close();
	}
});

test('setup still open: sign-in points to first-time setup', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await stubPasskeys(page);
		await mockServer(page, { setupRequired: true, session: null });
		await page.goto(`${srv.url}/staff-login`);
		await page.getByRole('link', { name: 'First-time setup →' }).click();
		await expect(page).toHaveURL(/\/config$/);
		await expect(page.getByRole('heading', { name: 'First-time setup' })).toBeVisible();
	} finally {
		await srv.close();
	}
});

test('the user-ID fallback still signs in a passkey enrolled before discoverable keys', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await stubPasskeys(page);
		const calls = await mockServer(page, { setupRequired: false, session: null });
		await page.goto(`${srv.url}/staff-login?next=%2Fconfig`);
		await page.getByText('Passkey not offered? Sign in with your user ID').click();
		await page.locator('#staff-account').fill(USER);
		await page.getByRole('button', { name: 'Sign in with user ID' }).click();
		await expect(page).toHaveURL(/\/config$/);
		// The first start is the email field's autofill offer; the
		// fallback's own start names the user ID.
		const starts = calls.filter((c) => c.step === 'login/start');
		expect(starts.at(-1).body).toEqual({ account: USER });
	} finally {
		await srv.close();
	}
});

test('W-1: the email field offers the passkey in autofill, and a pick signs in', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await stubPasskeys(page);
		const calls = await mockServer(page, { setupRequired: false, session: null });
		await page.goto(`${srv.url}/staff-login?next=%2Freview`);
		const field = page.getByLabel('Email');
		await expect(field).toHaveAttribute('autocomplete', 'username webauthn');
		await expect(field).toHaveAttribute('type', 'email');
		// The autofill request is pending while the field is on the page.
		await page.waitForFunction(() => window.__autofillRequests === 1);
		expect(calls.filter((c) => c.step === 'login/finish')).toHaveLength(0);
		await field.fill('judge@example.test');
		await page.evaluate(() => window.__pickAutofill());
		await expect(page).toHaveURL(/\/review$/);
		const starts = calls.filter((c) => c.step === 'login/start');
		// The typed email is never sent: the passkey picks the account.
		expect(starts.map((c) => c.body)).toEqual([{ account: '' }]);
		expect(calls.filter((c) => c.step === 'login/finish')).toHaveLength(1);
	} finally {
		await srv.close();
	}
});

test('W-1: the button takes over from autofill and stays busy until the passkey answers', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await stubPasskeys(page);
		await page.addInitScript(() => {
			window.__holdModal = true;
		});
		await mockServer(page, { setupRequired: false, session: null });
		await page.goto(`${srv.url}/staff-login?next=%2Freview`);
		await page.waitForFunction(() => window.__autofillRequests === 1);
		await page.getByRole('button', { name: 'Sign in with passkey' }).click();
		const waiting = page.getByRole('button', { name: 'Waiting for your passkey…' });
		await expect(waiting).toBeDisabled();
		// The sheet is open and the cancelled autofill request has settled:
		// the button must still say it is waiting.
		await page.waitForFunction(() => typeof window.__releaseModal === 'function');
		await expect(waiting).toBeDisabled();
		await page.evaluate(() => window.__releaseModal());
		await expect(page).toHaveURL(/\/review$/);
	} finally {
		await srv.close();
	}
});
