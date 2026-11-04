import test from 'node:test';
import assert from 'node:assert/strict';
import { authError, readErrorBody, SESSION_EXPIRED_COPY } from '../src/lib/admin/http-errors.js';

test('401 names the expired session', () => {
	assert.equal(authError(401, null), SESSION_EXPIRED_COPY);
	assert.match(authError(401, null), /log in on the admin host again/);
});

test('safeNext keeps same-origin console paths only', async () => {
	const { safeNext, DEFAULT_AFTER_SIGN_IN } = await import('../src/lib/admin/http-errors.js');
	assert.equal(DEFAULT_AFTER_SIGN_IN, '/dashboard');
	assert.equal(safeNext('/review?tab=bulk#row-3'), '/review?tab=bulk#row-3');
	assert.equal(safeNext('/sources/12'), '/sources/12');
	for (const bad of [
		null,
		'',
		'review',
		'//evil.test/x',
		'/\\evil.test',
		'/\tevil',
		'https://evil.test/',
		'javascript:alert(1)',
		'/staff-login',
		'/staff-login?next=/review',
		'/enrol?invite=ab',
		'/' + 'a'.repeat(3000)
	]) {
		assert.equal(safeNext(bad), '/dashboard', String(bad));
	}
});

test('signInHref carries the page to return to', async () => {
	const { signInHref } = await import('../src/lib/admin/http-errors.js');
	assert.equal(signInHref('/review?tab=bulk'), '/staff-login?next=%2Freview%3Ftab%3Dbulk');
	assert.equal(signInHref('/dashboard'), '/staff-login');
	assert.equal(signInHref('/staff-login?next=/x'), '/staff-login');
});

test('isSessionExpiry: admin API 401s only, never the ceremonies or setup', async () => {
	const { isSessionExpiry } = await import('../src/lib/admin/http-errors.js');
	assert.equal(isSessionExpiry('/apostille/api/v1/facts', 401), true);
	assert.equal(isSessionExpiry('http://localhost:18319/apostille/api/v1/me', 401), true);
	assert.equal(isSessionExpiry('/apostille/api/v1/facts', 403), false);
	assert.equal(isSessionExpiry('/apostille/api/v1/auth/passkey/login/finish', 401), false);
	assert.equal(isSessionExpiry('/apostille/api/v1/setup/steps', 401), false);
	assert.equal(isSessionExpiry('/somewhere/else', 401), false);
});

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

/** Fake window: `/facts` answers 401, setup status answers `setup`. */
function fakeWindow(pathname, setup = false) {
	const assigned = [];
	const win = {
		fetch: async (input) => {
			const url = String(input);
			if (url.endsWith('/setup/status')) return Response.json({ setup_required: setup });
			return new Response('{}', { status: url.includes('/facts') || url.endsWith('/me') ? 401 : 200 });
		},
		location: { pathname, search: '?tab=bulk', hash: '', assign: (href) => assigned.push(href) }
	};
	return { win, assigned };
}

test('installSessionGuard sends one 401 to sign-in with next, once', async () => {
	const { installSessionGuard } = await import('../src/lib/admin/http-errors.js');
	const { win, assigned } = fakeWindow('/review');
	const original = win.fetch;
	const uninstall = installSessionGuard(win);
	assert.notEqual(win.fetch, original);
	assert.equal((await win.fetch('/apostille/api/v1/queue')).status, 200);
	await tick();
	assert.deepEqual(assigned, []);
	// The caller still gets the response (its notice renders until the
	// browser leaves).
	assert.equal((await win.fetch('/apostille/api/v1/facts')).status, 401);
	await win.fetch('/apostille/api/v1/facts');
	await tick();
	assert.deepEqual(assigned, ['/staff-login?next=%2Freview%3Ftab%3Dbulk']);
	uninstall();
	assert.equal(win.fetch, original);
});

test('installSessionGuard sends a 401 to /config while setup is open', async () => {
	const { installSessionGuard } = await import('../src/lib/admin/http-errors.js');
	const { win, assigned } = fakeWindow('/review', true);
	installSessionGuard(win);
	await win.fetch('/apostille/api/v1/facts');
	await tick();
	assert.deepEqual(assigned, ['/config']);
	// Already on the wizard: stay.
	const onConfig = fakeWindow('/config', true);
	installSessionGuard(onConfig.win);
	await onConfig.win.fetch('/apostille/api/v1/facts');
	await tick();
	assert.deepEqual(onConfig.assigned, []);
});

test('installSessionGuard stays put on the sign-in pages', async () => {
	const { installSessionGuard } = await import('../src/lib/admin/http-errors.js');
	const { win, assigned } = fakeWindow('/staff-login');
	installSessionGuard(win);
	await win.fetch('/apostille/api/v1/me');
	await tick();
	assert.deepEqual(assigned, []);
});

test('non-auth statuses map to null', () => {
	assert.equal(authError(200, null), null);
	assert.equal(authError(404, null), null);
	assert.equal(authError(422, { error: 'reason required' }), null);
});

test('403 shows the server error verbatim (it carries the IP)', () => {
	const ip = 'admin access is limited to the configured IP range (admin.allow_ips) — your IP is 198.51.100.9; see "Admin IP allowlist" in docs/specs/config-db.md to change it. Detected IPv4 198.51.100.9 over tcp. Add 198.51.100.9 to admin.allow_ips, then restart apostille.';
	assert.equal(authError(403, { error: ip, code: 'ip_not_allowed', ip: '198.51.100.9' }), ip);
	assert.match(authError(403, { error: ip, code: 'ip_not_allowed' }), /198\.51\.100\.9/);
});

test('403 staff_role_required is the only "Logged in" case', () => {
	assert.match(authError(403, { error: 'staff role required', code: 'staff_role_required' }), /staff role required/);
	assert.equal(authError(403, { error: 'x', code: 'staff_role_required' }), 'x');
	assert.match(authError(403, { error: '', code: 'staff_role_required' }), /Logged in/);
	assert.doesNotMatch(authError(403, { error: 'csrf check failed', code: 'csrf_failed' }), /Logged in/);
	assert.doesNotMatch(authError(403, { error: 'admin host required', code: 'admin_host_required' }), /Logged in/);
});

test('403 without a body falls back per code', () => {
	assert.match(authError(403, null), /refused the request \(403\)/);
	assert.match(authError(403, { code: 'ip_not_allowed' }), /admin\.allow_ips/);
	assert.match(authError(403, { code: 'csrf_failed' }), /reload and try again/);
	assert.match(authError(403, { code: 'admin_host_required' }), /admin host only/);
});

test('readErrorBody never throws', async () => {
	const ok = { clone: () => ({ json: async () => ({ error: 'denied', code: 'x' }) }) };
	assert.deepEqual(await readErrorBody(ok), { error: 'denied', code: 'x' });
	const bad = { clone: () => ({ json: async () => { throw new Error('no json'); } }) };
	assert.equal(await readErrorBody(bad), null);
});

test('adminRequestError carries status, request id and step-up', async () => {
	const { adminRequestError, isStepUpRefusal } = await import('../src/lib/admin/http-errors.js');
	const res = new Response(
		JSON.stringify({ error: 'step-up required: log in again first', step_up: 'required' }),
		{ status: 403, headers: { 'x-request-id': '6377942e39bec1dc82efe1b812797868' } }
	);
	const err = await adminRequestError(res, 'fallback');
	assert.equal(err.status, 403);
	assert.equal(err.requestId, '6377942e39bec1dc82efe1b812797868');
	assert.equal(err.stepUp, true);
	assert.match(err.message, /request 6377942e39bec1dc82efe1b812797868/);
	assert.match(err.message, /passkey check from the last 10 minutes/);
	// Legacy body without the machine flag still reads as a step-up.
	assert.equal(isStepUpRefusal(403, { error: 'step-up required: log in again first' }), true);
	assert.equal(isStepUpRefusal(403, { error: 'csrf' }), false);
	assert.equal(isStepUpRefusal(401, { step_up: 'required' }), false);
});

test('adminRequestError keeps server text and works without a request id', async () => {
	const { adminRequestError } = await import('../src/lib/admin/http-errors.js');
	const res = new Response(JSON.stringify({ error: 'value shape does not match type' }), {
		status: 422
	});
	const err = await adminRequestError(res, 'Rejected');
	assert.equal(err.requestId, null);
	assert.equal(err.stepUp, false);
	assert.equal(err.message, 'value shape does not match type (HTTP 422)');
});
