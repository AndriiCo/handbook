import test from 'node:test';
import assert from 'node:assert/strict';
import {
	factCount,
	filterAuthorities,
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
