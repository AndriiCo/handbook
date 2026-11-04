import test from 'node:test';
import assert from 'node:assert/strict';
import { countryCode, flagFor, kindLabel, laneOf, LANES } from '../src/lib/public-ui/lanes.js';
import { filterAuthorities, groupByCountry } from '../src/lib/public-ui/handbook.js';

test('a US state is a US state; a Mexican state or a Canadian province is not', () => {
	const rows = [
		[{ kind: 'state', country: 'USA' }, 'states', 'US state'],
		[{ kind: 'state', country: 'United States of America' }, 'states', 'US state'],
		[{ kind: 'state', country: 'Mexico', iso2: 'MX' }, 'regions', 'State'],
		[{ kind: 'state', country: 'Mexico' }, 'regions', 'State'],
		[{ kind: 'province', country: 'Canada', iso2: 'CA' }, 'regions', 'Province'],
		[{ kind: 'office', country: 'USA' }, 'other', 'Office'],
		[{ kind: 'country', country: 'Germany' }, 'other', 'Country'],
		[{ kind: '', country: 'USA' }, 'other', ''],
		[null, 'other', '']
	];
	for (const [row, lane, label] of rows) {
		assert.equal(laneOf(row), lane, JSON.stringify(row));
		assert.equal(kindLabel(row), label, JSON.stringify(row));
	}
	// iso2 wins over a country spelling the browser does not know.
	assert.equal(laneOf({ kind: 'state', country: 'Estados Unidos', iso2: 'US' }), 'states');
	assert.equal(laneOf({ kind: 'state', country: 'USA', iso2: 'MX' }), 'regions');
	assert.ok(LANES.includes('regions'));
});

test('flags come from iso2, else the country name, and only for bundled assets', () => {
	assert.equal(flagFor('United States of America'), 'us');
	assert.equal(flagFor('USA'), 'us');
	assert.equal(flagFor('Mexico'), 'mx');
	assert.equal(flagFor('Netherlands (Kingdom of the)'), 'nl');
	assert.equal(flagFor('Republic of Korea'), 'kr');
	assert.equal(flagFor('Viet Nam'), 'vn');
	assert.equal(flagFor('anything', 'DE'), 'de');
	assert.equal(flagFor('Germany', ''), 'de');
	// A real code with no bundled file renders no flag (no 404).
	assert.equal(flagFor('Atlantis', 'AQ'), null);
	assert.equal(flagFor(''), null);
	assert.equal(flagFor(null), null);
	assert.equal(countryCode('Czech Republic'), 'cz');
});

test('search reads region, English and other names; groups carry iso2', () => {
	const rows = [
		{ slug: 'mx-jalisco-sg', name: 'Gobierno del Estado de Jalisco', country: 'Mexico', region: 'Jalisco', iso2: 'MX' },
		{ slug: 'de-by', name: 'Bayerisches Staatsministerium', name_en: 'Bavarian State Ministry', alt_names: ['Bayern', 'Deutschland'], country: 'Germany', iso2: 'DE' },
		{ slug: 'cl-ca-1', name: 'Subsecretario de Justicia', name_en: 'Under-Secretary of Justice', country: 'Chile' }
	];
	assert.deepEqual(filterAuthorities(rows, 'jalisco').map((r) => r.slug), ['mx-jalisco-sg']);
	assert.deepEqual(filterAuthorities(rows, 'deutsch').map((r) => r.slug), ['de-by']);
	assert.deepEqual(filterAuthorities(rows, 'bavarian').map((r) => r.slug), ['de-by']);
	assert.deepEqual(filterAuthorities(rows, 'under-secretary').map((r) => r.slug), ['cl-ca-1']);
	const groups = groupByCountry(rows);
	assert.deepEqual(
		groups.map((g) => [g.country, g.iso2]),
		[
			['Chile', ''],
			['Germany', 'DE'],
			['Mexico', 'MX']
		]
	);
});

test('place pages read a guarded path and drop malformed rows', async () => {
	const { parseLocation, placeWord, readLocation } = await import('../src/lib/public-ui/location.js');
	const seen = [];
	const io = { get: async (/** @type {string} */ p) => (seen.push(p), { ok: true, status: 200, body: {} }) };
	await readLocation('usa/washington', io);
	assert.deepEqual(seen, ['/locations/usa/washington']);
	assert.equal((await readLocation('../etc', io)).status, 404);
	assert.equal((await readLocation('', io)).status, 404);
	const view = parseLocation({
		location: { path: 'ca', label: 'Canada', iso2: 'CA', subdivision: null },
		crumbs: [{ path: 'ca', label: 'Canada' }],
		groups: [
			{ level: 'federal', label: 'Federal', offices: [{ slug: 'ca-gac-fed', name: 'Global Affairs Canada', path: 'ca-gac-fed', kind: 'office', fact_count: 2 }] },
			{ level: 'state', label: 'State', offices: [{ slug: 'ca-on-gov', name: 'Ontario', kind: 'province' }, { name: 'no slug' }] },
			{ level: 'local', label: 'Local', offices: [] }
		],
		places: [{ path: 'ca/on', label: 'Ontario' }, { path: '', label: 'x' }]
	});
	assert.ok(view);
	assert.deepEqual(view.groups.map((g) => [g.level, g.offices.length]), [['federal', 1], ['state', 1]]);
	assert.equal(view.groups[1].offices[0].path, 'ca-on-gov');
	assert.deepEqual(view.places, [{ path: 'ca/on', label: 'Ontario' }]);
	assert.equal(placeWord(view), 'province');
	assert.equal(parseLocation({ error: 'x' }), null);
});

test('place answers: a route, a pick-your-state prompt, or nothing', async () => {
	const { answerFor, parseLocation, placePath, sourceHost } = await import('../src/lib/public-ui/location.js');
	const view = parseLocation({
		location: { path: 'usa/washington', label: 'Washington', iso2: 'US', subdivision: 'US-WA' },
		groups: [],
		routes: [
			{ doc_class: 'vital_record', office: { slug: 'usa-wa-sos', name: 'Washington Secretary of State', path: 'usa/washington' }, prereq: '', source_uri: 'https://www.hcch.net/x', fact_ids: [], verified: false },
			{ doc_class: 'fbi_check', office: { slug: 'usdos-authentications', name: 'U.S. Department of State' }, source_uri: 'javascript:alert(1)', fact_ids: [7, 'x'], verified: true, inherited: true },
			{ doc_class: 'company' }
		],
		by_place: ['state_court', 3]
	});
	assert.ok(view);
	assert.equal(view.routes.length, 2);
	const vital = answerFor(view, 'vital_record');
	assert.equal(vital.kind, 'route');
	assert.equal(vital.kind === 'route' && vital.route.verified, false);
	const fbi = answerFor(view, 'fbi_check');
	assert.ok(fbi.kind === 'route' && fbi.route.inherited && fbi.route.verified);
	assert.equal(fbi.kind === 'route' && fbi.route.source_uri, '', 'only http(s) citations link');
	assert.deepEqual(fbi.kind === 'route' && fbi.route.fact_ids, [7]);
	assert.equal(answerFor(view, 'state_court').kind, 'by_place');
	assert.equal(answerFor(view, 'education').kind, 'none');
	assert.equal(placePath('US'), 'usa');
	assert.equal(placePath('', 'United States of America'), 'usa');
	assert.equal(placePath('DE', 'Germany'), 'de');
	assert.equal(placePath('', 'Germany'), null);
	assert.equal(sourceHost('https://www.hcch.net/en/x'), 'hcch.net');
	assert.equal(sourceHost('nope'), '');
});
