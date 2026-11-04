// Staff page (P4-3, customer-auth.md §5.6): lists accounts and their
// passkeys, mints a one-time invite link, revokes an account after an
// in-place passkey step-up, and adds a second passkey. API is
// route-mocked over a small server state; navigator.credentials is a
// stub.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const ME = 'a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0';
const JDOE = 'b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1';
const TOKEN = 'cd'.repeat(32);
const NOW = 1_793_750_000;

async function stubPasskeys(page) {
	await page.addInitScript(() => {
		const buf = (n) => new Uint8Array([n, n, n]).buffer;
		Object.defineProperty(navigator, 'credentials', {
			value: {
				create: async () => ({
					id: 'new-key',
					rawId: buf(1),
					type: 'public-key',
					response: { attestationObject: buf(2), clientDataJSON: buf(3) }
				}),
				get: async () => ({
					id: 'cred',
					rawId: buf(1),
					type: 'public-key',
					response: { authenticatorData: buf(2), clientDataJSON: buf(3), signature: buf(4), userHandle: null }
				})
			}
		});
	});
}

function initialState() {
	return {
		stepUpFresh: true,
		accounts: [
			{ user_id: ME, role: 'admin', name: '', created_at: NOW - 86_400, locked_at: null, passkeys: [{ id: 'mac', label: 'Judge', created_at: NOW - 86_400, last_used_at: NOW - 60 }] },
			{
				user_id: JDOE,
				role: 'staff',
				name: 'jdoe',
				created_at: NOW - 3600,
				locked_at: null,
				passkeys: [
					{ id: 'jd-1', label: 'Jordan laptop', created_at: NOW - 3600, last_used_at: null },
					{ id: 'jd-2', label: 'Jordan phone', created_at: NOW - 3000, last_used_at: null }
				]
			}
		],
		invites: []
	};
}

async function mockServer(page, server) {
	const calls = [];
	const api = (path) => `**/apostille/api/v1${path}`;
	const stale = (route) =>
		route.fulfill({ status: 403, headers: { 'x-request-id': 'req-77' }, json: { error: 'step-up required: log in again first', step_up: 'required', window_s: 600 } });
	await page.route('**/apostille/api/v1/**', (route) => route.fulfill({ status: 404, json: { error: 'not found' } }));
	await page.route(api('/setup/status'), (route) => route.fulfill({ json: { setup_required: false } }));
	await page.route(api('/me'), (route) => route.fulfill({ json: { user_id: ME, role: 'admin' } }));
	await page.route(api('/staff'), (route) =>
		route.fulfill({ json: { me: { user_id: ME, role: 'admin' }, accounts: server.accounts, invites: server.invites } })
	);
	await page.route(api('/staff/invites'), (route) => {
		const body = route.request().postDataJSON();
		calls.push({ step: 'invite', body, csrf: route.request().headers()['x-csrf'] ?? null });
		server.invites = [{ id: 'h1', role: body.role, cn: body.cn, created_by: ME, created_at: NOW, expires_at: NOW + 86_400 }];
		return route.fulfill({ json: { url: `http://localhost:18319/enrol?invite=${TOKEN}`, role: body.role, cn: body.cn, expires_at: NOW + 86_400 } });
	});
	await page.route(api(`/staff/${JDOE}/revoke`), (route) => {
		calls.push({ step: 'revoke' });
		if (!server.stepUpFresh) return stale(route);
		server.accounts = server.accounts.map((a) => (a.user_id === JDOE ? { ...a, locked_at: NOW, passkeys: [] } : a));
		return route.fulfill({ json: { ok: true, sessions: 1, passkeys: 2 } });
	});
	await page.route(api('/auth/passkey/login/start'), (route) => {
		calls.push({ step: 'login/start', body: route.request().postDataJSON() });
		return route.fulfill({ json: { challenge_id: 'l1', options: { publicKey: { challenge: 'AQID' } } } });
	});
	await page.route(api('/auth/passkey/login/finish'), (route) => {
		calls.push({ step: 'login/finish' });
		server.stepUpFresh = true;
		return route.fulfill({ json: { user_id: ME, methods: ['passkey'] } });
	});
	await page.route(api('/staff/passkeys/add/start'), (route) => {
		calls.push({ step: 'add/start', body: route.request().postDataJSON() });
		return route.fulfill({
			json: { challenge_id: 'a1', options: { publicKey: { challenge: 'AQID', user: { id: 'AQID', name: ME, displayName: 'Phone' } } } }
		});
	});
	await page.route(api('/staff/passkeys/add/finish'), (route) => {
		const body = route.request().postDataJSON();
		calls.push({ step: 'add/finish', body });
		server.accounts = server.accounts.map((a) =>
			a.user_id === ME ? { ...a, passkeys: [...a.passkeys, { id: 'new-key', label: body.label ?? '', created_at: NOW, last_used_at: null }] } : a
		);
		return route.fulfill({ json: { ok: true, id: 'new-key' } });
	});
	return calls;
}

test('staff page lists accounts, invites once, and revokes after a step-up', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await stubPasskeys(page);
		const server = initialState();
		const calls = await mockServer(page, server);
		await page.goto(`${srv.url}/staff`);

		await expect(page.getByRole('heading', { name: 'Staff', exact: true })).toBeVisible();
		await expect(page.getByRole('heading', { name: 'Accounts (2)' })).toBeVisible();
		const jdoe = page.getByRole('listitem').filter({ hasText: 'jdoe' }).first();
		await expect(jdoe.getByText('Jordan laptop')).toBeVisible();
		await expect(jdoe.getByText('Jordan phone')).toBeVisible();

		// Invite: the link shows once, with a copy button.
		await page.locator('#invite-cn').fill('ops-2');
		await page.getByRole('button', { name: 'Create invite link' }).click();
		await expect(page.getByText(`http://localhost:18319/enrol?invite=${TOKEN}`)).toBeVisible();
		await expect(page.getByRole('button', { name: 'Copy link' })).toBeVisible();
		const invite = calls.find((c) => c.step === 'invite');
		expect(invite.body).toEqual({ role: 'staff', cn: 'ops-2', days: 1 });
		expect(invite.csrf).toBeTruthy();
		await expect(page.getByRole('heading', { name: 'Pending invites' })).toBeVisible();

		// Revoke: two clicks, a stale step-up confirmed in place, then done.
		server.stepUpFresh = false;
		await jdoe.getByRole('button', { name: 'Revoke access' }).click();
		await jdoe.getByRole('button', { name: 'Revoke jdoe' }).click();
		await expect(page.getByText('Confirm it’s you.')).toBeVisible();
		await expect(page.locator('code', { hasText: 'req-77' })).toBeVisible();
		await page.getByRole('button', { name: 'Confirm with passkey and retry' }).click();
		await expect(page.getByText(/revoked/).first()).toBeVisible();
		expect(calls.filter((c) => c.step === 'revoke')).toHaveLength(2);
		expect(calls.find((c) => c.step === 'login/start').body).toEqual({ account: ME });
	} finally {
		await srv.close();
	}
});

test('a staffer adds a second passkey to their own account', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await stubPasskeys(page);
		const server = initialState();
		const calls = await mockServer(page, server);
		await page.goto(`${srv.url}/staff`);
		await page.locator('#passkey-label').fill('Work phone');
		await page.getByRole('button', { name: 'Add a passkey' }).click();
		await expect(page.getByText('Passkey added.')).toBeVisible();
		await expect(page.getByText('Work phone')).toBeVisible();
		expect(calls.find((c) => c.step === 'add/start').body).toEqual({ label: 'Work phone' });
		const finish = calls.find((c) => c.step === 'add/finish').body;
		expect(finish.challenge_id).toBe('a1');
		expect(finish.credential.id).toBe('new-key');
	} finally {
		await srv.close();
	}
});

test('the staff page fits a 390 px phone', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await page.setViewportSize({ width: 390, height: 844 });
		await stubPasskeys(page);
		await mockServer(page, initialState());
		await page.goto(`${srv.url}/staff`);
		await expect(page.getByRole('heading', { name: 'Accounts (2)' })).toBeVisible();
		expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
	} finally {
		await srv.close();
	}
});
