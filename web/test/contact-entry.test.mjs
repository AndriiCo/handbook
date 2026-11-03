import test from 'node:test';
import assert from 'node:assert/strict';
import {
	MAX_SWEEP_URLS,
	classifyEntry,
	countLine,
	countryName,
	defaultCountryFor,
	entryKey,
	extractEntries,
	formatSaved,
	planSubmit
} from '../src/lib/admin/contact-entry.js';

// Synthetic vectors only (AGENTS.md §7): 555-01xx numbers and
// example.gov / example.test addresses, never a real office contact.

test('URLs with a scheme, bare hosts and their warnings', () => {
	const a = classifyEntry('https://www.example.gov/apostilles');
	assert.equal(a.kind, 'url');
	assert.equal(a.value, 'https://www.example.gov/apostilles');
	assert.equal(a.sendable, true);
	assert.equal(a.warning, null);

	const b = classifyEntry('example.gov/apostilles');
	assert.equal(b.kind, 'url');
	assert.equal(b.value, 'https://example.gov/apostilles');

	const c = classifyEntry('http://example.com');
	assert.equal(c.kind, 'url');
	assert.equal(c.sendable, true);
	assert.match(c.warning ?? '', /https/);

	assert.equal(classifyEntry('https://example.gov/x).').value, 'https://example.gov/x');
	assert.equal(classifyEntry('<https://example.gov/a>').value, 'https://example.gov/a');
	assert.equal(classifyEntry('https://example.gov/a_(b)').value, 'https://example.gov/a_(b)');
});

test('non-http schemes, IPs and long URLs are invalid', () => {
	for (const raw of ['ftp://x.y', 'file:///etc/hosts', '192.0.2.10/admin', 'https://localhost/x']) {
		const e = classifyEntry(raw);
		assert.equal(e.kind, 'invalid', raw);
		assert.equal(e.sendable, false, raw);
		assert.ok(e.error, raw);
	}
	const long = `https://example.gov/${'a'.repeat(2100)}`;
	assert.equal(classifyEntry(long).kind, 'invalid');
});

test('emails are lowercased and checked like the server', () => {
	const a = classifyEntry('Apostilles@Example.GOV');
	assert.equal(a.kind, 'email');
	assert.equal(a.value, 'apostilles@example.gov');
	assert.equal(classifyEntry('mailto:a@b.gov').value, 'a@b.gov');
	assert.equal(classifyEntry('"Desk" <desk@example.test>').kind, 'invalid');
	assert.equal(classifyEntry('<desk@example.test>').value, 'desk@example.test');
	for (const bad of ['a@b', 'a b@example.gov', 'a@@example.gov', 'a@-x.gov']) {
		assert.equal(classifyEntry(bad).kind, 'invalid', bad);
	}
});

test('phones become E.164 and display internationally', () => {
	const a = classifyEntry('(360) 555-0100', 'US');
	assert.equal(a.kind, 'phone');
	assert.equal(a.value, '+13605550100');
	assert.equal(a.display, '+1 360 555 0100');
	assert.equal(a.country, 'US');
	assert.equal(a.details, '');

	const b = classifyEntry('360-555-0100 ext. 2', 'US');
	assert.equal(b.kind, 'phone');
	assert.equal(b.value, '+13605550100');
	assert.equal(b.details, 'ext. 2');
	assert.match(b.display, /^\+1 360 555 0100 ext\. 2$/);

	const c = classifyEntry('0024 3555 0100', 'US');
	assert.equal(c.kind, 'phone');
	assert.ok(c.value.startsWith('+24'), c.value);

	const d = classifyEntry('024 3555 0100', 'VN');
	assert.equal(d.value, '+842435550100');
	assert.equal(d.display, '+84 24 3555 0100');
	assert.equal(d.country, 'VN');

	const e = classifyEntry('tel:+1 360 555 0123');
	assert.equal(e.value, '+13605550123');
});

test('phone failures carry a reason', () => {
	assert.equal(classifyEntry('12345', 'US').kind, 'invalid');
	assert.equal(classifyEntry('hello', 'US').kind, 'invalid');
	const noCountry = classifyEntry('360 555 0100', null);
	assert.equal(noCountry.kind, 'invalid');
	assert.match(noCountry.error ?? '', /country code/);
	const short = classifyEntry('+1 360 555 01');
	assert.equal(short.kind, 'invalid');
	assert.match(short.error ?? '', /short/i);
	assert.match(classifyEntry('024 3555 0100', 'US').error ?? '', /United States/);
});

test('a pasted contact block splits into typed entries', () => {
	const { entries, ignored } = extractEntries(
		'Phone: (360) 555-0100\nEmail: a@example.gov, https://example.gov/x',
		'US'
	);
	assert.deepEqual(
		entries.map((e) => [e.kind, e.value]),
		[
			['phone', '+13605550100'],
			['email', 'a@example.gov'],
			['url', 'https://example.gov/x']
		]
	);
	assert.equal(ignored, 2);
});

test('digits inside URLs and emails never read as phones', () => {
	const { entries } = extractEntries(
		'https://example.gov/forms/3605550100.pdf 3605550111@example.test',
		'US'
	);
	assert.deepEqual(
		entries.map((e) => e.kind),
		['url', 'email']
	);
});

test('a single pasted token is classified as typed', () => {
	const { entries } = extractEntries('12345', 'US');
	assert.equal(entries.length, 1);
	assert.equal(entries[0].kind, 'invalid');
	const list = extractEntries('example.gov/a; example.gov/b\n+1 360 555 0142', 'US');
	assert.deepEqual(
		list.entries.map((e) => e.value),
		['https://example.gov/a', 'https://example.gov/b', '+13605550142']
	);
});

test('list items are read as typed; label words are dropped', () => {
	const { entries, ignored } = extractEntries('example.gov/a, 555-01, Email, desk@example.gov', 'US');
	assert.deepEqual(
		entries.map((e) => e.kind),
		['url', 'invalid', 'email']
	);
	assert.equal(ignored, 1);
});

test('a foreign number without + surfaces as an invalid chip', () => {
	const { entries } = extractEntries('Hotline 024 3555 0100', 'US');
	assert.equal(entries.length, 1);
	assert.equal(entries[0].kind, 'invalid');
});

test('default country from the authority row', () => {
	assert.equal(defaultCountryFor({ country: 'United States', slug: 'usa-wa-sos' }), 'US');
	assert.equal(defaultCountryFor({ country: 'USA', slug: 'usa-wa-sos' }), 'US');
	assert.equal(defaultCountryFor({ country: 'United States of America' }), 'US');
	assert.equal(defaultCountryFor({ country: 'Viet Nam', slug: 'vietnam' }), 'VN');
	assert.equal(defaultCountryFor({ country: 'Vietnam', slug: 'vietnam' }), 'VN');
	assert.equal(defaultCountryFor({ country: 'Netherlands', slug: 'hcch' }), 'NL');
	assert.equal(defaultCountryFor({ country: '', slug: 'us-wa-sos' }), 'US');
	assert.equal(defaultCountryFor({ country: '', slug: 'usdos-authentications' }), 'US');
	assert.equal(defaultCountryFor({ country: '', slug: 'hcch' }), null);
	assert.equal(defaultCountryFor(null), null);
	assert.equal(countryName('VN'), 'Vietnam');
});

test('entry keys de-duplicate by kind and normalized value', () => {
	const a = classifyEntry('(360) 555-0100', 'US');
	const b = classifyEntry('+1 360-555-0100', 'US');
	assert.equal(entryKey(a), entryKey(b));
	assert.notEqual(entryKey(a), entryKey(classifyEntry('a@example.gov')));
});

test('saved contacts format for the on-file row', () => {
	const phone = formatSaved({ kind: 'phone', value: '+13605550100', details: 'ext. 2' });
	assert.equal(phone.kind, 'phone');
	assert.equal(phone.display, '+1 360 555 0100 ext. 2');
	assert.equal(phone.key, entryKey(classifyEntry('(360) 555-0100', 'US')));
	assert.match(phone.title, /United States/);
	const email = formatSaved({ kind: 'email', value: 'desk@example.gov', details: '' });
	assert.equal(email.key, 'email:desk@example.gov');
	assert.equal(formatSaved({ kind: 'webform', value: 'https://example.gov/contact' }).kind, 'url');
});

test('submit plan caps URLs and skips contacts on file', () => {
	const urls = Array.from({ length: 12 }, (_, i) => classifyEntry(`https://example.gov/p${i}`));
	const saved = new Set([formatSaved({ kind: 'email', value: 'desk@example.gov' }).key]);
	const plan = planSubmit(
		[
			...urls,
			classifyEntry('desk@example.gov'),
			classifyEntry('new@example.gov'),
			classifyEntry('(360) 555-0142', 'US'),
			classifyEntry('555-01', 'US'),
			classifyEntry('http://example.test/fees')
		],
		saved
	);
	assert.equal(plan.urls.length, 10);
	assert.equal(plan.overLimit.length, 3);
	assert.deepEqual(
		plan.contacts.map((e) => e.value),
		['new@example.gov', '+13605550142']
	);
	assert.equal(plan.onFile.length, 1);
	assert.equal(plan.invalid.length, 1);
	assert.equal(plan.states.get('email:desk@example.gov'), 'saved');
	assert.equal(plan.states.get('url:http://example.test/fees'), 'over');
	assert.equal(planSubmit([classifyEntry('http://example.test/fees')], new Set()).states.get('url:http://example.test/fees'), 'warning');
});

test('count line names what a submit will do', () => {
	assert.equal(MAX_SWEEP_URLS, 10);
	assert.equal(
		countLine({ urls: 3, emails: 1, phones: 2, onFile: 0, invalid: 1, overLimit: 0 }),
		'3 URLs to fetch · 1 email · 2 phones to save · 1 needs fixing'
	);
	assert.equal(countLine({ urls: 0, emails: 0, phones: 0, onFile: 0, invalid: 0, overLimit: 0 }), 'No entries yet');
});
