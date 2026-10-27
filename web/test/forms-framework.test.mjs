import test from 'node:test';
import assert from 'node:assert/strict';
import { errorsForKey, isValid, validateSubmission } from '../src/lib/forms/validation.js';
import { formatPhone, normalizePhoneDigits } from '../src/lib/forms/phone.js';
import {
	COUNTRIES,
	countryNameForCode,
	isCountryName,
	searchCountries
} from '../src/lib/forms/countries.js';
import {
	ADDRESS_PARTS,
	US_STATES,
	formatAddressOneLine,
	isUsStateCode,
	missingAddressParts,
	stateNameForCode
} from '../src/lib/forms/address.js';

test('validation helpers filter and summarize without changing codes', () => {
	const fields = [
		{ key: 'email', label: 'Email', type: 'email', required: true },
		{ key: 'name', label: 'Name', type: 'text', required: true }
	];
	const errors = validateSubmission(fields, { email: 'not-an-email' });
	assert.equal(isValid(errors), false);
	assert.deepEqual(errorsForKey(errors, 'email'), [
		{ key: 'email', code: 'email', message: 'Email must be a valid email address' }
	]);
	assert.deepEqual(errorsForKey(errors, 'name'), [
		{ key: 'name', code: 'required', message: 'Name is required' }
	]);
	assert.deepEqual(errorsForKey(errors, 'other'), []);
	assert.equal(isValid(validateSubmission(fields, { email: 'a@b.co', name: 'Ann' })), true);
});

test('formatPhone canonicalizes NANP and passes the rest through', () => {
	assert.equal(formatPhone('5551234567'), '(555) 123-4567');
	assert.equal(formatPhone('(555) 123-4567'), '(555) 123-4567');
	assert.equal(formatPhone('1-555-123-4567'), '+1 (555) 123-4567');
	assert.equal(formatPhone('+1 (555) 123-4567'), '+1 (555) 123-4567');
	// Non-NANP input is trimmed, never mangled or rejected.
	assert.equal(formatPhone('  +44 20 7946 0958 '), '+44 20 7946 0958');
	assert.equal(formatPhone('555-12'), '555-12');
	assert.equal(formatPhone(''), '');
});

test('normalizePhoneDigits strips separators only', () => {
	assert.equal(normalizePhoneDigits('+1 (555) 123-4567'), '15551234567');
	assert.equal(normalizePhoneDigits('555.123.4567'), '5551234567');
});

test('country list is sorted, unique, and ISO-shaped', () => {
	assert.ok(COUNTRIES.length > 190, 'covers the ISO list');
	const names = COUNTRIES.map((c) => c.name);
	assert.deepEqual([...names].sort((a, b) => a.localeCompare(b, 'en')), names);
	assert.equal(new Set(COUNTRIES.map((c) => c.code)).size, COUNTRIES.length);
	for (const c of COUNTRIES) {
		assert.match(c.code, /^[A-Z]{2}$/, `code shape: ${c.code}`);
	}
});

test('searchCountries ranks prefix matches first', () => {
	const hits = searchCountries('united');
	assert.deepEqual(
		hits.map((c) => c.name),
		['United Arab Emirates', 'United Kingdom', 'United States']
	);
	assert.deepEqual(searchCountries('  '), COUNTRIES.slice(0, 10));
	assert.deepEqual(searchCountries('no-such-country-xyz'), []);
	assert.ok(searchCountries('congo').length >= 2);
	assert.ok(searchCountries('VIET', 3).some((c) => c.name === 'Viet Nam'));
});

test('country name/code lookups are case-insensitive', () => {
	assert.equal(isCountryName('Egypt'), true);
	assert.equal(isCountryName('  egypt '), true);
	assert.equal(isCountryName('Narnia'), false);
	assert.equal(countryNameForCode('us'), 'United States');
	assert.equal(countryNameForCode(' VN '), 'Viet Nam');
	assert.equal(countryNameForCode('XX'), null);
});

test('US states cover 50 + DC + territories', () => {
	assert.equal(US_STATES.length, 56);
	assert.equal(isUsStateCode('wa'), true);
	assert.equal(isUsStateCode('WA'), true);
	assert.equal(isUsStateCode('XX'), false);
	assert.equal(stateNameForCode('wa'), 'Washington');
	assert.equal(stateNameForCode('PR'), 'Puerto Rico');
	assert.equal(stateNameForCode('XX'), null);
});

test('missingAddressParts follows the address_block check order', () => {
	assert.deepEqual(ADDRESS_PARTS, ['name', 'street', 'city', 'state', 'zip']);
	assert.deepEqual(missingAddressParts({}), ADDRESS_PARTS);
	assert.deepEqual(missingAddressParts({ name: 'Ann', street: '1 Main', city: 'X' }), [
		'state',
		'zip'
	]);
	assert.deepEqual(
		missingAddressParts({ name: 'Ann', street: '1 Main', city: 'X', state: 'WA', zip: '9' }),
		[]
	);
	assert.deepEqual(missingAddressParts({ name: '  ' }), [
		'name',
		'street',
		'city',
		'state',
		'zip'
	]);
	// Non-objects pass the rule (mirrors the documented quirk).
	assert.deepEqual(missingAddressParts('whatever'), []);
	assert.deepEqual(missingAddressParts(null), []);
});

test('formatAddressOneLine skips blanks and joins segments', () => {
	assert.equal(
		formatAddressOneLine({
			name: 'Ann Lee',
			street: '1 Main St',
			line2: 'Apt 2',
			city: 'Olympia',
			state: 'WA',
			zip: '98501'
		}),
		'Ann Lee, 1 Main St Apt 2, Olympia, WA 98501'
	);
	assert.equal(formatAddressOneLine({ city: 'Olympia', state: 'WA' }), 'Olympia, WA');
	assert.equal(formatAddressOneLine({}), '');
	assert.equal(formatAddressOneLine(null), '');
	assert.equal(formatAddressOneLine(undefined), '');
});
