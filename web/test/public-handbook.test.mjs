import test from 'node:test';
import assert from 'node:assert/strict';
import {
	approvedAgo,
	clampName,
	factCount,
	filterAuthorities,
	groupByCountry,
	groupFactsByTopic,
	listAuthorities,
	publicError,
	readDossier,
	relativeDay,
	seedLedgerEvents,
	topicLabel
} from '../src/lib/public-ui/handbook.js';
import { magicError } from '../src/lib/customer/magic.js';

function fakeIo(responses) {
	const seen = [];
	return {
		seen,
		async get(path) {
			seen.push(path);
			return responses[seen.length - 1] ?? { ok: true, status: 200, body: {} };
		}
	};
}

const INDEX = [
	{ slug: 'vietnam', name: 'Viet Nam Consular', country: 'Viet Nam', fact_count: 12 },
	{ slug: 'austin-tx', name: 'Texas Secretary of State', country: 'United States', fact_count: 0 }
];

test('listAuthorities gets /authorities', async () => {
	const io = fakeIo([{ ok: true, status: 200, body: { authorities: INDEX } }]);
	const res = await listAuthorities(io);
	assert.equal(res.ok, true);
	assert.deepEqual(io.seen, ['/authorities']);
});

test('readDossier gets /authorities/{slug}/facts', async () => {
	const io = fakeIo([{ ok: true, status: 200, body: { authority: 'vietnam', facts: [] } }]);
	const res = await readDossier('vietnam', io);
	assert.equal(res.ok, true);
	assert.deepEqual(io.seen, ['/authorities/vietnam/facts']);
});

test('readDossier rejects bad slugs without requesting', async () => {
	for (const bad of ['', 'a/b', 42, null, undefined]) {
		const io = fakeIo([]);
		const res = await readDossier(bad, io);
		assert.equal(res.ok, false);
		assert.equal(res.status, 400);
		assert.deepEqual(io.seen, []);
	}
});

test('readDossier propagates non-ok untouched', async () => {
	const io = fakeIo([{ ok: false, status: 404, body: { error: 'unknown authority' } }]);
	const res = await readDossier('nope', io);
	assert.equal(res.ok, false);
	assert.equal(res.status, 404);
});

test('filterAuthorities matches name/slug/country, case-insensitive', () => {
	assert.deepEqual(filterAuthorities(INDEX, 'texas').map((a) => a.slug), ['austin-tx']);
	assert.deepEqual(filterAuthorities(INDEX, 'VIET').map((a) => a.slug), ['vietnam']);
	assert.deepEqual(filterAuthorities(INDEX, 'united states').map((a) => a.slug), ['austin-tx']);
	assert.deepEqual(filterAuthorities(INDEX, 'zzz'), []);
});

test('filterAuthorities empty query returns the full list', () => {
	assert.equal(filterAuthorities(INDEX, '').length, 2);
	assert.equal(filterAuthorities(INDEX, '   ').length, 2);
});

test('groupByCountry sorts countries A→Z, International last, facts first', () => {
	const rows = [
		{ slug: 'fr-ca-2', name: 'Cour d’appel', country: 'France', fact_count: 0 },
		{ slug: 'hcch', name: 'HCCH', country: '', fact_count: 0 },
		{ slug: 'fr-ca-1', name: 'Notaires', country: 'France', fact_count: 3 },
		{ slug: 'al-ca-1', name: 'Ministry', country: 'Albania', fact_count: 0 }
	];
	const groups = groupByCountry(rows);
	assert.deepEqual(
		groups.map((g) => [g.country, g.rows.map((a) => a.slug), g.facts]),
		[
			['Albania', ['al-ca-1'], 0],
			['France', ['fr-ca-1', 'fr-ca-2'], 3],
			['International', ['hcch'], 0]
		]
	);
});

test('groupByCountry holds a world-scale index in one row per country', () => {
	const rows = [];
	for (let c = 0; c < 200; c++) {
		for (let i = 0; i < 4; i++) {
			rows.push({ slug: `c${c}-${i}`, name: `Authority ${i}`, country: `Country ${c}` });
		}
	}
	const groups = groupByCountry(rows);
	assert.equal(groups.length, 200);
	assert.equal(groups.reduce((n, g) => n + g.rows.length, 0), 800);
});

test('clampName keeps short names and cuts long ones at a word', () => {
	assert.equal(clampName('Ministry of Justice'), 'Ministry of Justice');
	const long = 'Pour les actes dont les signataires sont établis, '.repeat(20);
	const clamped = clampName(long);
	assert.ok(clamped.length <= 121, clamped);
	assert.ok(clamped.endsWith('…'));
	assert.ok(!/[\s,]…$/.test(clamped));
	assert.equal(clampName(null), '');
});

test('groupFactsByTopic groups in first-seen order', () => {
	const groups = groupFactsByTopic([
		{ id: '1', topic: 'fees', statement: 's1' },
		{ id: '2', topic: 'hours', statement: 's2' },
		{ id: '3', topic: 'fees', statement: 's3' }
	]);
	assert.deepEqual(
		groups.map((g) => [g.topic, g.facts.map((f) => f.id)]),
		[
			['fees', ['1', '3']],
			['hours', ['2']]
		]
	);
	assert.deepEqual(groupFactsByTopic(null), []);
});

test('topicLabel and factCount share copy literals', () => {
	assert.equal(topicLabel('processing_time'), 'Processing time');
	assert.equal(factCount(0), '0 facts');
	assert.equal(factCount(1), '1 fact');
	assert.equal(factCount(12), '12 facts');
});

test('publicError maps index/dossier statuses', () => {
	assert.equal(publicError(500, 'index'), 'Could not load the authority index — try again.');
	assert.equal(
		publicError(404, 'dossier'),
		'Unknown authority — it may have been renamed or removed.'
	);
	assert.equal(publicError(500, 'dossier'), 'Could not load this dossier — try again.');
});

test('publicError 429 shares the magic.js sentence', () => {
	assert.equal(publicError(429, 'index'), magicError(429, 'request'));
	assert.equal(publicError(429, 'dossier'), magicError(429, 'request'));
});

test('seedLedgerEvents keeps unique ids on conflict cliques', () => {
	// Staging vietnam: facts 1, 5, 8 conflict with each other, so the
	// same sibling id appears under several parents. Bare sibling ids
	// duplicated here and crashed the keyed ledger render (each_key_duplicate).
	const clique = [
		{ id: 1, topic: 't', citations: [], conflicts: [{ id: 5 }, { id: 8 }] },
		{ id: 5, topic: 't', citations: [], conflicts: [{ id: 1 }, { id: 8 }] },
		{ id: 8, topic: 't', citations: [], conflicts: [{ id: 1 }, { id: 5 }] }
	];
	const events = seedLedgerEvents(clique);
	const ids = events.map((e) => e.id);
	assert.equal(ids.length, new Set(ids).size);
	assert.equal(events.filter((e) => e.kind === 'verified').length, 3);
	assert.equal(events.filter((e) => e.kind === 'changed').length, 6);
});

test('approvedAgo formats seconds through days, empty on bad stamps', () => {
	const nowMs = new Date(2026, 10, 1, 12, 0, 0).getTime();
	const nowSec = Math.floor(nowMs / 1000);
	assert.equal(approvedAgo(nowSec - 42, nowMs), 'approved 42s ago');
	assert.equal(approvedAgo(nowSec - 5 * 60, nowMs), 'approved 5m ago');
	assert.equal(approvedAgo(nowSec - 3 * 3600, nowMs), 'approved 3h ago');
	assert.equal(approvedAgo(nowSec - 4 * 86400, nowMs), 'approved 4d ago');
	assert.equal(approvedAgo(undefined, nowMs), '');
	assert.equal(approvedAgo(0, nowMs), '');
	assert.equal(approvedAgo('yesterday', nowMs), '');
	assert.equal(approvedAgo(nowSec + 60, nowMs), '');
});

test('seedLedgerEvents says approved Ns ago on verified facts', () => {
	const nowSec = Math.floor(Date.now() / 1000);
	const events = seedLedgerEvents([
		{ id: 1, topic: 'fees', created_at: nowSec - 90, citations: [], conflicts: [] }
	]);
	assert.equal(events[0].text, 'Fees — verified quote · approved 1m ago');
	// Facts without a stamp keep the old line (older fixtures, no crash).
	const legacy = seedLedgerEvents([{ id: 2, topic: 'fees', citations: [], conflicts: [] }]);
	assert.equal(legacy[0].text, 'Fees — verified quote');
});

test('seedLedgerEvents labels accepted conflicts as conflicting reports', () => {
	const clique = [
		{
			id: 1,
			topic: 'fees',
			citations: [],
			conflicts: [{ id: 2, statement: 'Other fee.', status: 'accepted' }]
		}
	];
	const events = seedLedgerEvents(clique);
	assert.equal(
		events.find((e) => e.kind === 'changed').text,
		'Fees — conflicting accepted report'
	);
});

test('relativeDay counts calendar days in the viewer zone', () => {
	// Local-midnight arithmetic: timezone-independent. Now is 01:00;
	// a 23:00 capture from "2 h ago" is yesterday, not today.
	const nowMs = new Date(2026, 10, 1, 1, 0, 0).getTime();
	assert.equal(relativeDay(new Date(2026, 10, 1, 0, 30).toISOString(), nowMs), 'today');
	assert.equal(relativeDay(new Date(2026, 9, 31, 23, 0).toISOString(), nowMs), 'yesterday');
	assert.equal(relativeDay(new Date(2026, 9, 29, 12, 0).toISOString(), nowMs), '3 days ago');
	assert.equal(relativeDay('not-a-date', nowMs), '');
});

test('publicError takes status only — server text can never leak', () => {
	// The mapper signature has no body parameter by construction; every
	// output is a fixed literal (the 404 dossier sentence is our own
	// copy, not the server's `unknown authority` error echoed).
	for (const kind of ['index', 'dossier']) {
		for (const status of [400, 404, 429, 500]) {
			const copy = publicError(status, kind);
			assert.match(copy, /[a-z]/i);
			assert.doesNotMatch(copy, /<script>|{|}/);
		}
	}
});
