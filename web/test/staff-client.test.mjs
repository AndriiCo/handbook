import test from 'node:test';
import assert from 'node:assert/strict';
import {
	addPasskey,
	createInvite,
	fetchStaff,
	inviteDays,
	normalizeStaffList,
	revokePasskey,
	revokeStaff,
	shortId,
	validCn,
	withdrawInvite
} from '../src/lib/admin/staff-client.js';

const API = '/apostille/api/v1';

/** Records each request and answers from `reply(url, init)`. */
function recorder(reply) {
	const calls = [];
	const fetchFn = async (url, init = {}) => {
		const body = init.body ? JSON.parse(init.body) : undefined;
		calls.push({ url, method: init.method ?? 'GET', body, headers: init.headers ?? {} });
		const [status, json] = reply(url, init);
		return new Response(JSON.stringify(json), {
			status,
			headers: { 'content-type': 'application/json', 'x-request-id': 'req-1' }
		});
	};
	return { calls, fetchFn };
}

test('names, days and short ids follow the server rules', () => {
	for (const ok of ['jdoe', 'ops.bot', 'admin-2', 'a_b', 'x'.repeat(64)]) assert.equal(validCn(ok), true, ok);
	for (const bad of ['', 'j doe', 'jdoe@example.test', 'x'.repeat(65), null, 7]) assert.equal(validCn(bad), false, String(bad));
	assert.equal(inviteDays(7), 7);
	assert.equal(inviteDays(0), 1);
	assert.equal(inviteDays(90), 30);
	assert.equal(inviteDays('2.9'), 2);
	assert.equal(inviteDays('soon'), 1);
	assert.equal(shortId('c0ffee00c0ffee00c0ffee00c0ffee00'), 'c0ffee00');
});

test('normalizeStaffList keeps well-formed rows only', () => {
	const list = normalizeStaffList({
		me: { user_id: 'u1', role: 'admin' },
		accounts: [
			{ user_id: 'u1', role: 'admin', name: 'Judge', created_at: 10, locked_at: null, passkeys: [{ id: 'k1', label: 'Mac', created_at: 11, last_used_at: 12 }, { label: 'no id' }] },
			{ user_id: 'u2', role: 'root', name: 5, created_at: 'x', locked_at: 20 },
			{ role: 'staff' }
		],
		invites: [{ id: 'h1', role: 'staff', cn: 'jdoe', created_by: 'u1', created_at: 1, expires_at: 2 }, { cn: 'no id' }]
	});
	assert.deepEqual(list.me, { user_id: 'u1', role: 'admin' });
	assert.equal(list.accounts.length, 2);
	assert.deepEqual(list.accounts[0].passkeys, [{ id: 'k1', label: 'Mac', created_at: 11, last_used_at: 12 }]);
	assert.equal(list.accounts[1].role, 'staff');
	assert.equal(list.accounts[1].name, '');
	assert.equal(list.accounts[1].locked_at, 20);
	assert.deepEqual(list.accounts[1].passkeys, []);
	assert.equal(list.invites.length, 1);
	assert.deepEqual(normalizeStaffList(null), { me: { user_id: '', role: '' }, accounts: [], invites: [] });
});

test('fetchStaff reads the list; a refusal keeps status and request id', async () => {
	const ok = recorder(() => [200, { me: { user_id: 'u1', role: 'admin' }, accounts: [], invites: [] }]);
	assert.equal((await fetchStaff(API, ok.fetchFn)).me.user_id, 'u1');
	assert.equal(ok.calls[0].url, `${API}/staff`);
	const refused = recorder(() => [403, { error: 'admin role required' }]);
	await assert.rejects(fetchStaff(API, refused.fetchFn), (err) => {
		assert.equal(err.status, 403);
		assert.equal(err.requestId, 'req-1');
		assert.match(err.message, /admin role required/);
		return true;
	});
});

test('mutations post JSON with the CSRF echo and surface step-up refusals', async () => {
	globalThis.document = { cookie: 'hb_csrf=tok' };
	try {
		const { calls, fetchFn } = recorder((url) =>
			url.endsWith('/staff/invites') ? [200, { url: 'http://localhost/enrol?invite=ab', role: 'staff', cn: 'jdoe', expires_at: 9 }] : [200, { ok: true }]
		);
		const invite = await createInvite(API, { role: 'staff', cn: 'jdoe', days: 99 }, fetchFn);
		assert.equal(invite.cn, 'jdoe');
		assert.deepEqual(calls[0].body, { role: 'staff', cn: 'jdoe', days: 30 });
		assert.equal(calls[0].headers['x-csrf'], 'tok');
		await withdrawInvite(API, 'h/1', fetchFn);
		assert.equal(calls[1].url, `${API}/staff/invites/h%2F1/withdraw`);
		await revokeStaff(API, 'u2', fetchFn);
		assert.equal(calls[2].url, `${API}/staff/u2/revoke`);
		await revokePasskey(API, 'k1', fetchFn);
		assert.deepEqual(calls[3].body, { credential_id: 'k1' });
		await assert.rejects(createInvite(API, { role: 'staff', cn: 'j doe', days: 1 }, fetchFn), /Name:/);
		assert.equal(calls.length, 4, 'a bad name never reaches the server');

		const stale = recorder(() => [403, { error: 'step-up required: log in again first', step_up: 'required' }]);
		await assert.rejects(revokeStaff(API, 'u2', stale.fetchFn), (err) => err.stepUp === true && err.status === 403);
	} finally {
		delete globalThis.document;
	}
});

test('addPasskey: start → create → finish with the label', async () => {
	globalThis.document = { cookie: 'hb_csrf=tok' };
	try {
		const { calls, fetchFn } = recorder((url) =>
			url.endsWith('/add/start')
				? [200, { challenge_id: 'c1', options: { publicKey: { challenge: 'AQID', user: { id: 'AQID', name: 'u1', displayName: 'Judge' } } } }]
				: [200, { ok: true, id: 'k2' }]
		);
		const buf = (n) => new Uint8Array([n]).buffer;
		let created = null;
		const credentials = {
			create: async (args) => {
				created = args;
				return { id: 'k2', rawId: buf(1), type: 'public-key', response: { attestationObject: buf(2), clientDataJSON: buf(3) } };
			}
		};
		const out = await addPasskey(API, { label: ' Phone ' }, { credentials, fetchFn });
		assert.equal(out.id, 'k2');
		assert.ok(created.publicKey.challenge instanceof ArrayBuffer);
		assert.deepEqual(calls[0].body, { label: 'Phone' });
		assert.equal(calls[1].body.challenge_id, 'c1');
		assert.equal(calls[1].body.label, 'Phone');
		assert.equal(calls[1].body.credential.id, 'k2');
	} finally {
		delete globalThis.document;
	}
});
