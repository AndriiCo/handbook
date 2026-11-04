import test from 'node:test';
import assert from 'node:assert/strict';
import {
	codeOfPlace,
	countryOptions,
	destinationRow,
	hasCountry,
	filterCountries,
	guessHomeCountry,
	methodFor,
	parseRouteQuery,
	placeOfCode,
	routeHref
} from '../src/lib/public-ui/route.js';

test('method: apostille only when both are named parties', () => {
	const members = ['us', 'mx', 'ua'];
	assert.equal(methodFor('us', 'mx', members), 'apostille');
	assert.equal(methodFor('mx', 'us', members), 'apostille');
	assert.equal(methodFor('us', 'eg', members), 'legalization');
	assert.equal(methodFor('eg', 'us', members), 'legalization');
	assert.equal(methodFor('us', 'us', members), 'same');
	assert.equal(methodFor('us', 'mx', []), 'unknown');
	assert.equal(methodFor('us', 'mx', null), 'unknown');
	assert.equal(methodFor('', 'mx', members), 'unknown');
});

test('places and codes', () => {
	assert.equal(codeOfPlace('usa/washington'), 'us');
	assert.equal(codeOfPlace('usa'), 'us');
	assert.equal(codeOfPlace('mx'), 'mx');
	assert.equal(codeOfPlace('nowhere'), '');
	assert.equal(placeOfCode('us'), 'usa');
	assert.equal(placeOfCode('MX'), 'mx');
	assert.equal(placeOfCode('xyz'), '');
});

test('query: valid values pass, anything else drops', () => {
	const q = (s) => parseRouteQuery(new URLSearchParams(s));
	assert.deepEqual(q('from=usa/washington&to=mx&doc=vital_record'), {
		from: 'usa/washington',
		to: 'mx',
		doc: 'vital_record'
	});
	assert.deepEqual(q('from=USA&to=usa'), { from: 'usa', to: 'us', doc: '' });
	assert.deepEqual(q('from=<script>&to=x&doc=passport'), { from: '', to: '', doc: '' });
	assert.deepEqual(q(''), { from: '', to: '', doc: '' });
});

test('href round-trips through the query', () => {
	const href = routeHref('/apostille/route', { from: 'usa', to: 'eg', doc: 'fbi_check' });
	assert.equal(href, '/apostille/route?from=usa&to=eg&doc=fbi_check');
	assert.deepEqual(parseRouteQuery(new URL(href, 'http://x').searchParams), {
		from: 'usa',
		to: 'eg',
		doc: 'fbi_check'
	});
	assert.equal(routeHref('/r', {}), '/r');
});

test('home guess: US time zones only', () => {
	assert.equal(guessHomeCountry({ timeZone: 'America/Los_Angeles' }), 'us');
	assert.equal(guessHomeCountry({ timeZone: 'America/Indiana/Indianapolis' }), 'us');
	assert.equal(guessHomeCountry({ timeZone: 'Pacific/Honolulu' }), 'us');
	assert.equal(guessHomeCountry({ timeZone: 'America/Mexico_City' }), '');
	assert.equal(guessHomeCountry({ timeZone: 'Europe/Kyiv' }), '');
	assert.equal(guessHomeCountry(null), '');
});

test('options: world codes plus index codes, named and sorted', () => {
	const names = new Intl.DisplayNames(['en'], { type: 'region' });
	const opts = countryOptions(['ua', 'mx'], [{ country: 'Viet Nam' }, { iso2: 'EG' }], (c) => names.of(c));
	assert.deepEqual(
		opts.map((o) => o.code),
		['eg', 'mx', 'ua', 'vn']
	);
	assert.deepEqual(
		filterCountries(opts, 'mex').map((o) => o.code),
		['mx']
	);
	assert.deepEqual(
		filterCountries(opts, 'ua').map((o) => o.code),
		['ua']
	);
});

test('destination row: the single country-level row, else none', () => {
	const rows = [
		{ slug: 'egypt-embassy-dc', kind: 'office', country: 'Egypt', region: 'DC' },
		{ slug: 'egypt', kind: 'country', country: 'Egypt', region: '' },
		{ slug: 'vietnam', kind: 'country', country: 'Viet Nam', fact_count: 8 },
		{ slug: 'vn-ca-1', kind: 'country', country: 'Viet Nam', iso2: 'VN', fact_count: 0 },
		{ slug: 'ua-ca-1', kind: 'country', country: 'Ukraine', iso2: 'UA' },
		{ slug: 'ua-ca-2', kind: 'country', country: 'Ukraine', iso2: 'UA' },
		{ slug: 'us-dos-vital', kind: 'office', country: 'United States of America', iso2: 'US' }
	];
	assert.equal(destinationRow(rows, 'eg')?.slug, 'egypt');
	assert.equal(destinationRow(rows, 'vn')?.slug, 'vietnam');
	assert.equal(destinationRow(rows, 'ua'), null);
	assert.equal(destinationRow(rows, 'us'), null);
	assert.equal(hasCountry(rows, 'us'), true);
	assert.equal(hasCountry(rows, 'fr'), false);
});
