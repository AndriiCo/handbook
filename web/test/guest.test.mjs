import test from 'node:test';
import assert from 'node:assert/strict';
import { guestDay, guestLogin, guestOffer, isGuestEmail } from '../src/lib/customer/guest.js';
import { LOGIN_COPY } from '../src/lib/customer/login-copy.js';

test('only the guest address counts, however it was typed', () => {
	assert.equal(isGuestEmail('demo@andrii.ai'), true);
	assert.equal(isGuestEmail('  DEMO@andrii.AI '), true);
	assert.equal(isGuestEmail('demo@andrii.ai.evil.test'), false);
	assert.equal(isGuestEmail('someone@andrii.ai'), false);
	assert.equal(isGuestEmail(''), false);
});

test('the note shows only on the server offer', () => {
	assert.deepEqual(
		guestOffer({ providers: [], guest: { email: 'demo@andrii.ai', last_day: '2026-11-20' } }),
		{ email: 'demo@andrii.ai', lastDay: '2026-11-20' }
	);
	// After the deadline the server leaves `guest` out.
	assert.equal(guestOffer({ providers: [] }), null);
	assert.equal(guestOffer({ guest: { email: 1 } }), null);
	assert.equal(guestOffer(null), null);
});

test('the last day reads the same in every timezone', () => {
	assert.equal(guestDay('2026-11-20', 'en'), 'November 20, 2026');
});

test('guest sign-in posts the trimmed address', async () => {
	const calls = [];
	const post = async (path, body) => {
		calls.push([path, body]);
		return { ok: true, status: 200, body: {} };
	};
	await guestLogin(' demo@andrii.ai ', { post });
	assert.deepEqual(calls, [['/auth/guest/login', { email: 'demo@andrii.ai' }]]);
});

test('every language has the note, and it names the address and the day', () => {
	for (const locale of ['en', 'es', 'fr']) {
		const c = LOGIN_COPY[locale];
		const note = c.guestNote('demo@andrii.ai', 'DAY');
		assert.match(note, /demo@andrii\.ai/, locale);
		assert.match(note, /DAY/, locale);
		assert.ok(c.guestContinue && c.guestEnded, locale);
	}
});
