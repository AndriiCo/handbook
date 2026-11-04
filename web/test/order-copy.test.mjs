import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ORDER_COPY, fmtDay } from '../src/lib/customer/order-copy.js';
import { PRECHECK_COPY } from '../src/lib/customer/precheck-copy.js';
import { UPLOADS_COPY } from '../src/lib/customer/uploads-copy.js';
import { SHIPPING_COPY } from '../src/lib/customer/shipping-copy.js';
import { formatBytes, validateFiles } from '../src/lib/customer/uploads.js';

const LOCALES = ['en', 'es', 'fr'];
const TABLES = [
	['ORDER_COPY', ORDER_COPY],
	['PRECHECK_COPY', PRECHECK_COPY],
	['UPLOADS_COPY', UPLOADS_COPY],
	['SHIPPING_COPY', SHIPPING_COPY]
];

// Words that are the same in the other language, on purpose.
const SAME = new Set([
	'ORDER_COPY.es.total',
	'ORDER_COPY.fr.total',
	'ORDER_COPY.es.colon',
	'ORDER_COPY.fr.steps.destination',
	'ORDER_COPY.fr.steps.document',
	'ORDER_COPY.fr.source'
]);

/** Every leaf as [dotted.path, value]; nested sets (steps, roles, kinds) included. */
function leaves(value, path = []) {
	if (value && typeof value === 'object' && !Array.isArray(value)) {
		return Object.entries(value).flatMap(([k, v]) => leaves(v, [...path, k]));
	}
	return [[path.join('.'), value]];
}

for (const [name, table] of TABLES) {
	test(`${name}: every language has the same keys`, () => {
		const keys = leaves(table.en).map(([k]) => k).sort();
		for (const code of LOCALES) {
			assert.deepEqual(leaves(table[code]).map(([k]) => k).sort(), keys, code);
		}
	});

	test(`${name}: values have the same kind in every language`, () => {
		const kinds = Object.fromEntries(leaves(table.en).map(([k, v]) => [k, typeof v]));
		for (const code of LOCALES) {
			for (const [k, v] of leaves(table[code])) assert.equal(typeof v, kinds[k], `${code}.${k}`);
		}
	});

	test(`${name}: es and fr differ from English`, () => {
		const en = Object.fromEntries(leaves(table.en));
		for (const code of ['es', 'fr']) {
			for (const [k, v] of leaves(table[code])) {
				if (typeof v !== 'string' || SAME.has(`${name}.${code}.${k}`)) continue;
				assert.notEqual(v, en[k], `${code}.${k}`);
			}
		}
	});
}

test('ORDER_COPY: English keeps the wording the e2e suites read', () => {
	const c = ORDER_COPY.en;
	assert.equal(c.planCites(1), 'Plan cites 1 accepted fact');
	assert.equal(c.planCites(3), 'Plan cites 3 accepted facts');
	assert.equal(c.fillDailyCap(30), 'Your account has reached the limit of 30 filled forms in 24 hours. Try again later.');
	assert.equal(c.openGuide('X'), 'Open the guide for X');
	assert.equal(c.payAmount('$49.00'), 'Pay $49.00');
	assert.equal(c.showFact(203), 'Show the cited fact 203');
	assert.equal(c.aboutDays(1), ', about 1 day');
	assert.equal(c.fieldsFilled(2, 5), '2 of 5 fields filled');
	assert.equal(c.fieldProblem('required', 'Your full name', 'Your full name is required'), 'Your full name is required');
});

test('ORDER_COPY: plurals follow the count', () => {
	assert.equal(ORDER_COPY.es.planCites(1), 'El plan cita 1 hecho aceptado');
	assert.equal(ORDER_COPY.es.planCites(2), 'El plan cita 2 hechos aceptados');
	assert.equal(ORDER_COPY.fr.aboutDays(1), ', environ 1 jour');
	assert.equal(ORDER_COPY.fr.aboutDays(4), ', environ 4 jours');
	assert.equal(SHIPPING_COPY.en.summary(4, 2), '4 shipments across 2 orders.');
	assert.equal(SHIPPING_COPY.en.summary(1, 1), '1 shipment across 1 order.');
	assert.equal(SHIPPING_COPY.es.summary(1, 2), '1 envío en 2 pedidos.');
	assert.equal(SHIPPING_COPY.fr.summary(3, 1), '3 envois sur 1 commande.');
	assert.equal(UPLOADS_COPY.en.uploaded(1), '1 file uploaded.');
	assert.equal(UPLOADS_COPY.en.uploaded(2), '2 files uploaded.');
	assert.equal(UPLOADS_COPY.fr.uploaded(2), '2 fichiers envoyés.');
});

test('ORDER_COPY: validator problems are re-worded by code outside English', () => {
	assert.equal(ORDER_COPY.es.fieldProblem('required', 'Nombre', 'Nombre is required'), 'Nombre: campo obligatorio');
	assert.equal(ORDER_COPY.fr.fieldProblem('max_len', 'Nom', 'Nom must be at most 200 characters', 200), 'Nom : 200 caractères au maximum');
	// An unknown code falls back to the validator's own message.
	assert.equal(ORDER_COPY.fr.fieldProblem('new_code', 'Nom', 'Nom is odd'), 'Nom is odd');
});

test('fmtDay: dates follow the site language', () => {
	assert.equal(fmtDay('2026-11-04T15:00:00Z', 'en'), 'Nov 4, 2026');
	assert.match(fmtDay('2026-11-04T15:00:00Z', 'fr'), /^4 nov\.? 2026$/);
	assert.match(fmtDay(1792012800, 'es'), /2026/);
	assert.equal(fmtDay('not a date', 'en'), 'not a date');
	assert.equal(fmtDay('', 'en'), '');
});

test('uploads.js: English unchanged, other languages localized', () => {
	assert.equal(formatBytes(1536), '1.5 KB');
	assert.equal(formatBytes(1536, 'es'), '1,5 KB');
	assert.equal(formatBytes(1536, 'fr'), '1,5 Ko');
	assert.deepEqual(validateFiles([]).errors, ['Choose at least one file.']);
	assert.deepEqual(validateFiles([], 'es').errors, ['Elija al menos un archivo.']);
	const big = [{ name: 'a.pdf', size: 30 * 1024 * 1024 }];
	assert.deepEqual(validateFiles(big).errors, ['“a.pdf” is over 25.0 MB.']);
	assert.deepEqual(validateFiles(big, 'fr').errors, ['« a.pdf » dépasse 25,0 Mo.']);
});
