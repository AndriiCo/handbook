import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
	isBlank,
	parseMoneyCents,
	validateField,
	validateSubmission
} from '../src/lib/customer/forms-validate.js';

const here = dirname(fileURLToPath(import.meta.url));
const vectors = JSON.parse(
	readFileSync(join(here, 'forms-validate-vectors.json'), 'utf8')
);

test('parity vectors: TS port matches the pinned codes and messages', () => {
	assert.equal(vectors.version, 1);
	assert.ok(vectors.cases.length > 30, 'vectors cover every rule');
	for (const c of vectors.cases) {
		const values = c.omit ? {} : { [c.field.key]: c.value };
		const got = validateSubmission([c.field], values).map((e) => ({
			code: e.code,
			message: e.message
		}));
		assert.deepEqual(got, c.expect, `case: ${c.name}`);
	}
});

test('isBlank matches Rust semantics', () => {
	assert.equal(isBlank(undefined), true);
	assert.equal(isBlank(null), true);
	assert.equal(isBlank(''), true);
	assert.equal(isBlank('  '), true);
	assert.equal(isBlank([]), true);
	assert.equal(isBlank({}), true);
	assert.equal(isBlank({ a: '', b: null }), true);
	assert.equal(isBlank({ a: 'x' }), false);
	assert.equal(isBlank(0), false);
	assert.equal(isBlank(false), false);
	assert.equal(isBlank('x'), false);
});

test('parseMoneyCents matches Rust edges', () => {
	assert.equal(parseMoneyCents('25'), 2500);
	assert.equal(parseMoneyCents('25.5'), 2550);
	assert.equal(parseMoneyCents('25.00'), 2500);
	assert.equal(parseMoneyCents('0.99'), 99);
	for (const bad of ['', '-1', '+1', '1.234', '1.', '.5', 'x', '1,000']) {
		assert.equal(parseMoneyCents(bad), null, `input ${bad}`);
	}
});

test('validateField works standalone for inline use', () => {
	const field = { key: 'email', label: 'Email', type: 'email', required: true };
	assert.deepEqual(validateField(field, 'a@b.co'), []);
	assert.deepEqual(validateField(field, 'nope'), [
		{ key: 'email', code: 'email', message: 'Email must be a valid email address' }
	]);
});

test('unknown keys are ignored', () => {
	const fields = [{ key: 'a', label: 'A', type: 'text', required: true }];
	assert.deepEqual(validateSubmission(fields, { a: 'x', extra: 'draft' }), []);
});
