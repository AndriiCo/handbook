import { test } from 'node:test';
import assert from 'node:assert/strict';
import { STRINGS } from '../src/lib/i18n.ts';
import { SECURITY_COPY } from '../src/lib/customer/security-copy.js';
import { oauthErrorCopy, oauthStartError } from '../src/lib/customer/oauth.js';
import { magicError } from '../src/lib/customer/magic.js';

const LOCALES = ['en', 'es', 'fr'];

for (const [name, table] of [
	['STRINGS', STRINGS],
	['SECURITY_COPY', SECURITY_COPY]
]) {
	test(`${name}: every language has the same keys`, () => {
		const keys = Object.keys(table.en).sort();
		for (const code of LOCALES) assert.deepEqual(Object.keys(table[code]).sort(), keys, code);
	});
}

test('masthead: Spanish and French are not English', () => {
	for (const code of ['es', 'fr']) {
		for (const key of ['skipToContent', 'ordersLabel', 'newOrderLabel', 'signOutLabel', 'disclaimer']) {
			assert.notEqual(STRINGS[code][key], STRINGS.en[key], `${code}.${key}`);
		}
	}
});

test('oauth: callback and start errors follow the locale, English by default', () => {
	const denied = new URLSearchParams('error=oauth_denied');
	assert.match(oauthErrorCopy(denied), /cancelled/);
	assert.match(oauthErrorCopy(denied, 'es'), /canceló/);
	assert.match(oauthErrorCopy(denied, 'fr'), /annulée/);
	// Unknown and prototype keys fall back, never render raw.
	assert.match(oauthErrorCopy(new URLSearchParams('error=constructor'), 'fr'), /échoué/);
	assert.match(oauthStartError(503, 'Google', 'es'), /Google no está disponible/);
	assert.match(oauthStartError(500, 'Meta', 'fr'), /Meta a échoué/);
});

test('magic: link errors follow the locale, English by default', () => {
	assert.equal(magicError(410, 'consume'), 'Link expired — request a new one.');
	assert.match(magicError(410, 'consume', 'es'), /caducó/);
	assert.match(magicError(400, 'request', 'fr'), /adresse e-mail/);
});
