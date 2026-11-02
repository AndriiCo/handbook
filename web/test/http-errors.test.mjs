import test from 'node:test';
import assert from 'node:assert/strict';
import { authError, readErrorBody, SESSION_EXPIRED_COPY } from '../src/lib/admin/http-errors.js';

test('401 names the expired session', () => {
	assert.equal(authError(401, null), SESSION_EXPIRED_COPY);
	assert.match(authError(401, null), /log in on the admin host again/);
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
