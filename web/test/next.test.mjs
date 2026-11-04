import test from 'node:test';
import assert from 'node:assert/strict';
import {
	NEXT_KEY,
	NEXT_TTL_MS,
	rememberNext,
	safeNext,
	takeNext
} from '../src/lib/customer/next.js';

test('safeNext keeps same-origin paths under /apostille/', () => {
	for (const good of [
		'/apostille/',
		'/apostille/orders',
		'/apostille/orders/ord_q7Hm2xR4kP',
		'/apostille/order?destination=usa-wa-sos&path=general&step=form',
		'/apostille/order?destination=usa-wa-sos&path=birth-certificate&kind=mail_kit&step=delivery',
		'/apostille/authority/usa/washington#step-3',
		// A query may carry an encoded value; only the path is strict.
		'/apostille/order?note=a%2Fb'
	]) {
		assert.equal(safeNext(good), good, good);
	}
	// Single-dot segments normalise and stay inside.
	assert.equal(safeNext('/apostille/./orders'), '/apostille/orders');
});

test('safeNext refuses other hosts, schemes and protocol-relative targets', () => {
	for (const bad of [
		'//evil.com',
		'//evil.com/apostille/',
		'///evil.com',
		'https://evil.com',
		'https://evil.com/apostille/orders',
		'http://evil.com/apostille/',
		'javascript:alert(1)',
		'data:text/html,hi',
		'apostille/orders',
		'evil.com/apostille/'
	]) {
		assert.equal(safeNext(bad), null, bad);
	}
});

test('safeNext refuses paths outside /apostille/ and dot-segment escapes', () => {
	for (const bad of [
		'/other',
		'/',
		'/apostille',
		'/apostilles/orders',
		'/apostille-evil/x',
		'/apostille/../x',
		'/apostille/..',
		'/apostille/orders/../../admin',
		'/APOSTILLE/orders'
	]) {
		assert.equal(safeNext(bad), null, bad);
	}
});

test('safeNext refuses backslashes, whitespace and control characters', () => {
	for (const bad of [
		'/\\evil.com',
		'/apostille/\\evil.com',
		'/apostille\\..\\x',
		'/apostille/\t/evil.com',
		'/apostille/\n//evil.com',
		'/apostille/ orders',
		'/apostille/\u0000',
		'/apostille/\u007f'
	]) {
		assert.equal(safeNext(bad), null, JSON.stringify(bad));
	}
});

test('safeNext refuses encoded tricks in the path', () => {
	for (const bad of [
		'/apostille/%2e%2e/x',
		'/apostille/%2E%2E/admin',
		'/apostille/.%2e/x',
		'/apostille/%2e./x',
		'/apostille/..%2f..%2fevil.com',
		'/apostille/%2F%2Fevil.com',
		'/apostille/%5c%5cevil.com',
		'/apostille/%252e%252e/x',
		'%2F%2Fevil.com',
		'%2Fapostille%2Forders'
	]) {
		assert.equal(safeNext(bad), null, bad);
	}
});

test('safeNext refuses non-strings, overlong values and a loop back to /login', () => {
	for (const bad of [null, undefined, 42, {}, ['/apostille/orders'], '']) {
		assert.equal(safeNext(bad), null, String(bad));
	}
	assert.equal(safeNext(`/apostille/orders?q=${'a'.repeat(2048)}`), null);
	assert.equal(safeNext('/apostille/login'), null);
	assert.equal(safeNext('/apostille/login?next=/apostille/orders'), null);
	assert.equal(safeNext('/apostille/login/'), null);
	assert.equal(safeNext('/apostille/./login'), null);
});

/** A Storage stand-in; `fail` makes every call throw (blocked storage). */
function fakeStorage({ fail = false } = {}) {
	const map = new Map();
	const guard = () => {
		if (fail) throw new Error('SecurityError');
	};
	return {
		map,
		getItem: (k) => (guard(), map.has(k) ? map.get(k) : null),
		setItem: (k, v) => (guard(), void map.set(k, String(v))),
		removeItem: (k) => (guard(), void map.delete(k))
	};
}

test('rememberNext then takeNext returns the target once', () => {
	const s = fakeStorage();
	const at = 1_000_000;
	rememberNext(s, '/apostille/order?destination=usa-wa-sos&step=form', at);
	assert.equal(JSON.parse(s.map.get(NEXT_KEY)).next, '/apostille/order?destination=usa-wa-sos&step=form');
	assert.equal(takeNext(s, at + 1000), '/apostille/order?destination=usa-wa-sos&step=form');
	assert.equal(takeNext(s, at + 2000), null, 'read once');
	assert.equal(s.map.has(NEXT_KEY), false);
});

test('rememberNext clears instead of storing an unsafe target', () => {
	const s = fakeStorage();
	rememberNext(s, '/apostille/orders', 1);
	rememberNext(s, '//evil.com', 2);
	assert.equal(s.map.has(NEXT_KEY), false);
	assert.equal(takeNext(s, 3), null);
});

test('takeNext drops stale, future, malformed and tampered entries', () => {
	const at = 5_000_000;
	const put = (value) => {
		const s = fakeStorage();
		s.map.set(NEXT_KEY, value);
		return s;
	};
	const entry = (next, when = at) => JSON.stringify({ next, at: when });
	assert.equal(takeNext(put(entry('/apostille/orders')), at + NEXT_TTL_MS), '/apostille/orders');
	assert.equal(takeNext(put(entry('/apostille/orders')), at + NEXT_TTL_MS + 1), null, 'stale');
	assert.equal(takeNext(put(entry('/apostille/orders', at + 60_000)), at), null, 'future');
	assert.equal(takeNext(put('not json'), at), null);
	assert.equal(takeNext(put(JSON.stringify({ next: '/apostille/orders' })), at), null, 'no time');
	assert.equal(takeNext(put(entry('https://evil.com')), at), null, 'tampered');
	assert.equal(takeNext(put(entry('/apostille/../x')), at), null, 'tampered');
	const s = put(entry('//evil.com'));
	takeNext(s, at);
	assert.equal(s.map.has(NEXT_KEY), false, 'a refused entry is forgotten too');
});

test('blocked or missing storage never throws', () => {
	const blocked = fakeStorage({ fail: true });
	assert.doesNotThrow(() => rememberNext(blocked, '/apostille/orders'));
	assert.equal(takeNext(blocked), null);
	assert.doesNotThrow(() => rememberNext(null, '/apostille/orders'));
	assert.equal(takeNext(undefined), null);
});
