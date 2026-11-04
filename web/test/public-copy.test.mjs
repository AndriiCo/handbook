import { test } from 'node:test';
import assert from 'node:assert/strict';
import { HOME_COPY, MAP_COPY, shortDate } from '../src/lib/public-ui/home-copy.js';
import { PLACE_COPY } from '../src/lib/public-ui/place-copy.js';
import { NOTIFY_COPY } from '../src/lib/public-ui/notify-copy.js';
import { DOC_CLASSES } from '../src/lib/public-ui/location.js';
import { factCount, publicError, relativeDay } from '../src/lib/public-ui/handbook.js';

const LOCALES = ['en', 'es', 'fr'];

// One argument that serves every copy function: a count (7, plural) and
// a name or document label ('7'); never equal to 1 or 'province'.
const ANY = {
	valueOf: () => 7,
	toString: () => '7',
	toLocaleString: () => '7',
	toLowerCase: () => 'document'
};

/**
 * Every leaf of a copy table as `path → string` (functions called with
 * ANY for each parameter).
 * @param {unknown} value
 * @param {string} [path]
 * @returns {Map<string, string>}
 */
function leaves(value, path = '', out = new Map()) {
	if (typeof value === 'string') out.set(path, value);
	else if (typeof value === 'function')
		out.set(path, value(...Array.from({ length: value.length }, () => ANY)));
	else if (value && typeof value === 'object')
		for (const [k, v] of Object.entries(value)) leaves(v, path ? `${path}.${k}` : k, out);
	return out;
}

// Leaves that are the same word in a language (brand names, cognates).
const SAME = new Set(['es:steps.0.model', 'fr:steps.0.model', 'es:federal', 'fr:international']);

for (const [name, table] of [
	['HOME_COPY', HOME_COPY],
	['MAP_COPY', MAP_COPY],
	['PLACE_COPY', PLACE_COPY],
	['NOTIFY_COPY', NOTIFY_COPY]
]) {
	test(`${name}: every language has the same keys`, () => {
		const keys = [...leaves(table.en).keys()].sort();
		for (const code of LOCALES) assert.deepEqual([...leaves(table[code]).keys()].sort(), keys, code);
	});

	test(`${name}: es and fr differ from English`, () => {
		const en = leaves(table.en);
		for (const code of ['es', 'fr']) {
			for (const [key, value] of leaves(table[code])) {
				assert.ok(value.trim(), `${code}.${key} is empty`);
				if (SAME.has(`${code}:${key}`)) continue;
				assert.notEqual(value, en.get(key), `${code}.${key}`);
			}
		}
	});
}

test('PLACE_COPY: the English chips are location.js DOC_CLASSES, word for word', () => {
	for (const doc of DOC_CLASSES) {
		for (const code of LOCALES) assert.ok(PLACE_COPY[code].docClasses[doc.id], `${code}.${doc.id}`);
		assert.equal(PLACE_COPY.en.docClasses[doc.id], doc.label);
	}
});

test('copy keeps the original English wording', () => {
	assert.equal(HOME_COPY.en.matchCount(1), '1 match');
	assert.equal(HOME_COPY.en.indexSummary(12, 3), '12 authorities in 3 countries. Open a country to see its authorities.');
	assert.equal(PLACE_COPY.en.placeCount(2, 'state'), '2 states');
	assert.equal(
		PLACE_COPY.en.byPlaceAnswer('Company document', 'state'),
		'A company document is apostilled in the state that issued it. Pick the state below.'
	);
	assert.equal(MAP_COPY.en.parties(1), '1 party named in staff-approved HCCH facts');
});

test('plurals: Spanish counts 0 as plural, French counts 0 and 1 as singular', () => {
	assert.equal(HOME_COPY.es.authorityCount(0), '0 autoridades');
	assert.equal(HOME_COPY.es.authorityCount(1), '1 autoridad');
	assert.equal(HOME_COPY.fr.authorityCount(0), '0 autorité');
	assert.equal(HOME_COPY.fr.authorityCount(2), '2 autorités');
	assert.equal(PLACE_COPY.fr.placeCount(1, 'province'), '1 province');
	assert.equal(PLACE_COPY.es.placeCount(3, 'province'), '3 provincias');
	assert.equal(factCount(0, 'es'), '0 datos');
	assert.equal(factCount(1, 'es'), '1 dato');
	assert.equal(factCount(0, 'fr'), '0 fait');
	assert.equal(factCount(12, 'fr'), '12 faits');
	assert.equal(factCount(12), '12 facts');
});

test('relativeDay speaks the site language', () => {
	const nowMs = new Date(2026, 10, 1, 9, 0).getTime();
	const today = new Date(2026, 10, 1, 0, 30).toISOString();
	const yesterday = new Date(2026, 9, 31, 23, 0).toISOString();
	const days3 = new Date(2026, 9, 29, 12, 0).toISOString();
	assert.deepEqual(
		[today, yesterday, days3].map((iso) => relativeDay(iso, nowMs, 'es')),
		['hoy', 'ayer', 'hace 3 días']
	);
	assert.deepEqual(
		[today, yesterday, days3].map((iso) => relativeDay(iso, nowMs, 'fr')),
		['aujourd’hui', 'hier', 'il y a 3 jours']
	);
	assert.equal(relativeDay(days3, nowMs), '3 days ago');
});

test('publicError: English by default, translated per locale, never server text', () => {
	for (const kind of ['index', 'dossier']) {
		for (const status of [404, 429, 500]) {
			const en = publicError(status, kind);
			assert.equal(publicError(status, kind, 'en'), en);
			for (const code of ['es', 'fr']) {
				const copy = publicError(status, kind, code);
				assert.ok(copy && copy !== en, `${code} ${kind} ${status}`);
			}
		}
	}
	assert.equal(publicError(429, 'index', 'es'), 'Demasiados intentos: espere un minuto e inténtelo de nuevo.');
});

test('shortDate: English keeps the ISO day, es/fr read it in their language', () => {
	assert.equal(shortDate('2026-11-03T22:15:00Z', 'en'), '2026-11-03');
	assert.match(shortDate('2026-11-03T22:15:00Z', 'es'), /3 nov.* 2026/);
	assert.match(shortDate('2026-11-03', 'fr'), /3 nov.* 2026/);
	assert.equal(shortDate('', 'fr'), '');
});
