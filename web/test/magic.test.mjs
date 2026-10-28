import test from 'node:test';
import assert from 'node:assert/strict';
import { consumeMagic, magicError, requestMagic } from '../src/lib/customer/magic.js';

function fakeIo(responses) {
	const seen = [];
	return {
		seen,
		async post(path, body) {
			seen.push([path, body]);
			return responses[seen.length - 1] ?? { ok: true, status: 200, body: {} };
		}
	};
}

test('requestMagic posts /auth/magic/start with the email', async () => {
	const io = fakeIo([{ ok: true, status: 200, body: { ok: true } }]);
	const res = await requestMagic('someone@example.test', io);
	assert.equal(res.ok, true);
	assert.deepEqual(io.seen, [['/auth/magic/start', { email: 'someone@example.test' }]]);
});

test('requestMagic rejects malformed email without posting', async () => {
	for (const bad of ['', 'no-at-sign', 42, null, undefined]) {
		const io = fakeIo([]);
		const res = await requestMagic(bad, io);
		assert.equal(res.ok, false);
		assert.equal(res.status, 400);
		assert.deepEqual(io.seen, []);
	}
});

test('requestMagic propagates non-ok untouched', async () => {
	const io = fakeIo([{ ok: false, status: 429, body: { error: 'rate limited' } }]);
	const res = await requestMagic('a@example.test', io);
	assert.equal(res.ok, false);
	assert.equal(res.status, 429);
});

test('consumeMagic posts /auth/magic/consume with the token', async () => {
	const io = fakeIo([{ ok: true, status: 200, body: { user_id: 'u1', fallback: true } }]);
	const res = await consumeMagic('ab'.repeat(32), io);
	assert.equal(res.ok, true);
	assert.deepEqual(io.seen, [['/auth/magic/consume', { token: 'ab'.repeat(32) }]]);
});

test('consumeMagic rejects empty/non-string token without posting', async () => {
	for (const bad of ['', 42, null, undefined]) {
		const io = fakeIo([]);
		const res = await consumeMagic(bad, io);
		assert.equal(res.ok, false);
		assert.equal(res.status, 400);
		assert.deepEqual(io.seen, []);
	}
});

test('magicError maps request statuses to copy without PII', () => {
	assert.equal(magicError(400, 'request'), 'Check the email address and try again.');
	assert.equal(
		magicError(429, 'request'),
		'Too many attempts — wait a minute and try again.'
	);
	assert.equal(
		magicError(503, 'request'),
		'That sign-in method is unavailable — use your passkey.'
	);
	assert.equal(magicError(500, 'request'), 'Sign-in failed — try again.');
});

test('magicError maps consume statuses to link copy', () => {
	assert.equal(magicError(400, 'consume'), 'Link expired — request a new one.');
	assert.equal(magicError(410, 'consume'), 'Link expired — request a new one.');
	assert.equal(magicError(429, 'consume'), 'Too many attempts — wait a minute and try again.');
	assert.equal(magicError(500, 'consume'), 'Sign-in failed — request a new link.');
});

test('magicError takes status only — server text can never leak', () => {
	// Copy contains "link" only, no echoed token/address; the mapper
	// signature has no body parameter by construction.
	for (const form of ['request', 'consume']) {
		for (const status of [400, 410, 429, 500, 503]) {
			const copy = magicError(status, form);
			assert.match(copy, /[a-z]/i);
			assert.doesNotMatch(copy, /<script>|token|@example/i);
		}
	}
});

test('start then consume order through fakes', async () => {
	const io = fakeIo([
		{ ok: true, status: 200, body: { ok: true } },
		{ ok: true, status: 200, body: { user_id: 'u1', fallback: true } }
	]);
	const first = await requestMagic('a@example.test', io);
	const second = await consumeMagic('cd'.repeat(32), io);
	assert.equal(first.ok, true);
	assert.equal(second.ok, true);
	assert.deepEqual(
		io.seen.map(([path]) => path),
		['/auth/magic/start', '/auth/magic/consume']
	);
});
