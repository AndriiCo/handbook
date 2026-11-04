// The customer account pages' copy tables (orders, billing, forms,
// report): every site language has the same keys, and the Spanish and
// French words are actually translated (not English left behind).
import test from 'node:test';
import assert from 'node:assert/strict';
import { ENUM_LABELS, INTL_LOCALE, ORDERS_COPY } from '../src/lib/customer/orders-copy.js';
import { BILLING_COPY } from '../src/lib/customer/billing-copy.js';
import { FORMS_COPY } from '../src/lib/customer/forms-copy.js';

const TABLES = { ENUM_LABELS, ORDERS_COPY, BILLING_COPY, FORMS_COPY };

// Words that are the same in English and the other language on purpose.
const SAME = new Set(['FORMS_COPY.es.noValue']);

/** Sample arguments so interpolated strings can be compared. */
const SAMPLE = [2, 3];

/** Flatten a copy table into `path → string` (functions called with samples). */
function flatten(value, path = '', out = new Map()) {
	if (typeof value === 'function') {
		out.set(path, value(...SAMPLE));
	} else if (Array.isArray(value)) {
		value.forEach((v, i) => flatten(v, `${path}.${i}`, out));
	} else if (value && typeof value === 'object') {
		for (const [k, v] of Object.entries(value)) flatten(v, path ? `${path}.${k}` : k, out);
	} else {
		out.set(path, value);
	}
	return out;
}

/** Same as flatten, but keys only and without calling functions. */
function shape(value, path = '', out = []) {
	if (value && typeof value === 'object' && !Array.isArray(value)) {
		for (const [k, v] of Object.entries(value)) shape(v, path ? `${path}.${k}` : k, out);
	} else {
		out.push(`${path}:${typeof value}`);
	}
	return out.sort();
}

for (const [name, table] of Object.entries(TABLES)) {
	test(`${name}: en, es and fr have identical keys`, () => {
		assert.deepEqual(Object.keys(table).sort(), ['en', 'es', 'fr']);
		const en = shape(table.en);
		assert.deepEqual(shape(table.es), en, 'es keys');
		assert.deepEqual(shape(table.fr), en, 'fr keys');
	});

	test(`${name}: es and fr strings differ from English`, () => {
		// fieldError is checked on its own below (it needs an error object).
		const skip = (p) => p === 'fieldError';
		const en = flatten({ ...table.en, fieldError: undefined });
		for (const lang of ['es', 'fr']) {
			const other = flatten({ ...table[lang], fieldError: undefined });
			for (const [path, text] of other) {
				if (skip(path) || text === undefined) continue;
				assert.equal(typeof text, 'string', `${name}.${lang}.${path}`);
				assert.ok(text.trim() !== '', `${name}.${lang}.${path} is empty`);
				if (SAME.has(`${name}.${lang}.${path}`)) continue;
				assert.notEqual(text, en.get(path), `${name}.${lang}.${path} is still English`);
			}
		}
	});
}

test('French copy uses ’ and a space before : ? !', () => {
	for (const [name, table] of Object.entries(TABLES)) {
		for (const [path, text] of flatten({ ...table.fr, fieldError: undefined })) {
			if (typeof text !== 'string') continue;
			assert.ok(!text.includes("'"), `${name}.fr.${path} uses a straight apostrophe`);
			// `v2` style versions and urls aside, a colon/question/exclamation
			// mark follows a space (or is the time / ratio inside digits).
			assert.ok(!/[^\s\d(]([?!]|:(?!\/\/))/.test(text), `${name}.fr.${path}: ${text}`);
		}
	}
});

test('plurals are functions, not appended "s"', () => {
	assert.equal(ORDERS_COPY.en.count(1), '1 order, newest first.');
	assert.equal(ORDERS_COPY.en.count(2), '2 orders, newest first.');
	assert.match(ORDERS_COPY.es.count(1), /^1 pedido,/);
	assert.match(ORDERS_COPY.fr.count(2), /^2 commandes,/);
	assert.equal(BILLING_COPY.en.count(1), '1 receipt');
	assert.equal(BILLING_COPY.fr.count(2), '2 reçus');
});

test('the English enumeration labels are the ones orders.ts always showed', () => {
	assert.equal(ENUM_LABELS.en.kind.label_fedex, 'FedEx labels');
	assert.equal(ENUM_LABELS.en.status.needs_staff, 'With our team');
	assert.equal(ENUM_LABELS.en.role.return, 'Back to you');
	assert.deepEqual(INTL_LOCALE, { en: 'en-US', es: 'es', fr: 'fr' });
});

test('fieldError keeps the pinned English message and re-words it by code', () => {
	const field = { label: 'Full name', type: 'text', max_len: 40 };
	const required = { code: 'required', message: 'Full name is required' };
	assert.equal(FORMS_COPY.en.fieldError(required, field), 'Full name is required');
	assert.equal(FORMS_COPY.es.fieldError(required, field), '«Full name» es obligatorio');
	assert.equal(FORMS_COPY.fr.fieldError(required, field), '« Full name » est obligatoire');
	const long = { code: 'max_len', message: 'Full name must be at most 40 characters' };
	assert.match(FORMS_COPY.fr.fieldError(long, field), /au plus 40 caractères/);
	const box = { label: 'Agree', type: 'checkbox' };
	const type = { code: 'type', message: 'Agree must be checked or unchecked' };
	assert.match(FORMS_COPY.es.fieldError(type, box), /marcado o desmarcado/);
	const addr = { code: 'address', message: 'Ship to is missing zip' };
	assert.equal(FORMS_COPY.fr.fieldError(addr, { label: 'Ship to' }), '« Ship to » : il manque code postal');
	assert.equal(FORMS_COPY.es.fieldError(addr, { label: 'Ship to' }), 'A «Ship to» le falta: código postal');
	// Unknown codes (or a server error without one) fall back to the message.
	const odd = { message: 'Server says no' };
	assert.equal(FORMS_COPY.es.fieldError(odd, field), 'Server says no');
	assert.equal(FORMS_COPY.fr.fieldError({ code: 'new_rule', message: 'x' }, field), 'x');
});
