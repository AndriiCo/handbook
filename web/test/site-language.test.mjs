import { test } from 'node:test';
import assert from 'node:assert/strict';
import { LOGIN_COPY } from '../src/lib/customer/login-copy.js';
import { PAGE_COPY, recordsLang, translateHref } from '../src/lib/public-ui/translate.js';
import { GUIDE_COPY } from '../src/lib/public-ui/doc/copy/index.js';
import { present } from '../src/lib/public-ui/present.js';

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

test('PAGE_COPY: no translation-pending notice (the guide is translated)', () => {
	for (const code of LOCALES) assert.equal(/** @type {any} */ (PAGE_COPY[code]).pending, undefined, code);
});

// The guide's own words (doc/copy): es and fr mirror en key for key,
// with the same kinds of values (a string, a function, a pair of
// strings around a value), and every word differs from English except
// names that read the same in each language.
const SAME = new Set([
	'Apostille Handbook',
	'Citations',
	'Citation',
	'Contact',
	'Sources',
	'Standard',
	'Formulario',
	'Formulaire',
	'Documents fédéraux',
	'Copies',
	'Contacts',
	'Traduction',
	'Notarisation',
	'Authentification',
	'Paiement',
	'Formulaires',
	'Adresse',
	'Refus',
	'Horaires',
	'document'
]);

/** @param {unknown} a @param {unknown} b @param {string} at */
function sameShape(a, b, at) {
	if (typeof a === 'function') {
		assert.equal(typeof b, 'function', at);
		return;
	}
	if (Array.isArray(a)) {
		assert.ok(Array.isArray(b) && b.length === a.length, at);
		return;
	}
	if (a && typeof a === 'object') {
		assert.ok(b && typeof b === 'object', at);
		// A map the English table leaves empty (topics) is free-form.
		if (Object.keys(a).length === 0) return;
		assert.deepEqual(Object.keys(/** @type {object} */ (b)).sort(), Object.keys(a).sort(), at);
		for (const [k, v] of Object.entries(a)) sameShape(v, /** @type {any} */ (b)[k], `${at}.${k}`);
		return;
	}
	assert.equal(typeof b, typeof a, at);
}

test('GUIDE_COPY: es and fr have the shape of en', () => {
	for (const code of ['es', 'fr']) sameShape(GUIDE_COPY.en, GUIDE_COPY[code], code);
});

test('GUIDE_COPY: es and fr strings are translated', () => {
	/** @param {any} en @param {any} other @param {string} at */
	const walk = (en, other, at) => {
		if (typeof en === 'string') {
			if (/\p{L}/u.test(en) && !SAME.has(en.trim()) && !/^[a-z-]+(-[A-Z]+)?$/.test(en)) assert.notEqual(other, en, at);
			return;
		}
		if (en && typeof en === 'object') for (const k of Object.keys(en)) walk(en[k], other?.[k], `${at}.${k}`);
	};
	for (const code of ['es', 'fr']) walk(GUIDE_COPY.en, GUIDE_COPY[code], code);
});

test('GUIDE_COPY: every phrase fills its arguments', () => {
	/** @param {any} node @param {string} at */
	const walk = (node, at) => {
		if (typeof node === 'function') {
			// A count first, then words; a list where a phrase joins names.
			const args = Array.from({ length: Math.max(node.length, 1) }, (_, i) => (i === 0 ? 3 : 'X'));
			let text;
			try {
				text = String(node(...args));
			} catch {
				text = String(node(3, ['A', 'B'], 'X'));
			}
			assert.ok(text.trim() && !/undefined|NaN|\[object/.test(text), `${at}: ${text}`);
			return;
		}
		if (node && typeof node === 'object') for (const [k, v] of Object.entries(node)) walk(v, `${at}.${k}`);
	};
	for (const code of LOCALES) walk(GUIDE_COPY[code], code);
});

test('present: the page words follow lang; values stay as written', () => {
	const facts = [
		{ id: 1, topic: 'fees', statement: 'per_document: $10 per document', citations: [] },
		{ id: 2, topic: 'payment', statement: 'payee: Secretary of State', citations: [] },
		{ id: 3, topic: 'mailing', statement: 'usps: PO Box 40228, Olympia, WA 98504-0228', citations: [] }
	];
	const authority = { slug: 'usa-wa-sos', name: 'Washington Secretary of State', region: 'Washington', country: 'USA', kind: 'state' };
	const en = present({ authority, facts, now: Date.UTC(2026, 10, 4) });
	const es = present({ authority, facts, now: Date.UTC(2026, 10, 4), lang: 'es' });
	const fr = present({ authority, facts, now: Date.UTC(2026, 10, 4), lang: 'fr' });
	assert.equal(en.header.h1, 'Get an apostille in Washington');
	assert.equal(es.header.h1, 'Obtener una apostilla en Washington');
	assert.match(fr.header.h1, /^Obtenir une apostille/);
	assert.equal(es.sections.find((s) => s.id === 'fees')?.heading, 'Pague las tarifas');
	assert.equal(fr.sections.find((s) => s.id === 'fees')?.heading, 'Réglez les frais');
	const fee = (/** @type {any} */ p) => p.sections.flatMap((s) => s.items).find((i) => i.slot === 'fees.per_document');
	assert.equal(fee(es).label, 'Tarifa por documento');
	assert.equal(fee(es).display, fee(en).display);
	assert.equal(fee(fr).display, '$10 per document');
	assert.deepEqual(
		es.sections.map((s) => s.id),
		en.sections.map((s) => s.id)
	);
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
