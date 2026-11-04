import { test } from 'node:test';
import assert from 'node:assert/strict';
import { LOGIN_COPY } from '../src/lib/customer/login-copy.js';
import { PAGE_COPY, recordsLang, translateHref } from '../src/lib/public-ui/translate.js';

const LOCALES = ['en', 'es', 'fr'];

for (const [name, table] of [
	['LOGIN_COPY', LOGIN_COPY],
	['PAGE_COPY', PAGE_COPY]
]) {
	test(`${name}: every language has the same keys`, () => {
		const keys = Object.keys(table.en).sort();
		for (const code of LOCALES) assert.deepEqual(Object.keys(table[code]).sort(), keys, code);
	});
}

test('LOGIN_COPY: es and fr differ from English', () => {
	for (const code of ['es', 'fr']) {
		for (const [key, value] of Object.entries(LOGIN_COPY[code])) {
			if (typeof value !== 'string') continue;
			assert.notEqual(value, LOGIN_COPY.en[key], `${code}.${key}`);
		}
	}
});

test('PAGE_COPY: only es and fr carry the translation-pending notice', () => {
	assert.equal(PAGE_COPY.en.pending, null);
	assert.ok(PAGE_COPY.es.pending?.heading);
	assert.ok(PAGE_COPY.fr.pending?.heading);
});

test('recordsLang: the site language picks the records view', () => {
	assert.equal(recordsLang('', 'en'), null);
	assert.equal(recordsLang('', 'es'), 'es');
	assert.equal(recordsLang('?l=fr', 'fr'), 'fr');
});

test('recordsLang: an older ?lang link wins only without ?l', () => {
	assert.equal(recordsLang('?lang=es', 'en'), 'es');
	assert.equal(recordsLang('?lang=es&l=fr', 'fr'), 'fr');
	assert.equal(recordsLang('?lang=es&l=en', 'en'), null);
});

test('translateHref: sets the site language parameter', () => {
	assert.equal(translateHref('/apostille/authority/usa/washington', 'es'), '/apostille/authority/usa/washington?l=es');
	assert.equal(translateHref('/apostille/authority/usa/washington', null), '/apostille/authority/usa/washington?l=en');
});
