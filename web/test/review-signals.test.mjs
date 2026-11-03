import test from 'node:test';
import assert from 'node:assert/strict';
import {
	blockedTag,
	cited,
	groupByAuthority,
	isVerified,
	markChanged,
	plural,
	quoteOk,
	riskCls,
	riskiestFirst,
	riskOf,
	splitBlocked
} from '../src/lib/admin/review-signals.js';

const QUOTE = 'The office charges 30 USD per document (synthetic).';

function row(id, over = {}) {
	return {
		id,
		authority_slug: 't-sos',
		topic: 'fees',
		statement: `Synthetic statement ${id}.`,
		created_by: 'muse-spark-1.3 via analysis-agent',
		created_at: 100 + id,
		diff: null,
		citations: [{ capture_id: 9, start_byte: 0, end_byte: QUOTE.length, quote: QUOTE }],
		blast_radius: { conflict: false, conflicts: [] },
		provenance: { capture_id: 9, trust: 'official' },
		...over
	};
}

test('quoteOk mirrors the 8-400 char accept gate (code points, not UTF-16 units)', () => {
	assert.equal(quoteOk('1234567'), false);
	assert.equal(quoteOk('12345678'), true);
	assert.equal(quoteOk('x'.repeat(400)), true);
	assert.equal(quoteOk('x'.repeat(401)), false);
	// 7 astral chars are 14 UTF-16 units but 7 scalar values: too short.
	assert.equal(quoteOk('😀'.repeat(7)), false);
	assert.equal(quoteOk(null), false);
	assert.equal(cited(row(1, { citations: [] })), false);
	assert.equal(cited(row(1, { citations: [{ capture_id: 9, start_byte: 0, end_byte: 1, quote: '' }] })), false);
	assert.equal(cited(row(1)), true);
});

test('riskOf ranks changed, conflict, uncited, low-trust; unknown trust earns no badge', () => {
	assert.deepEqual(riskOf(row(1, { diff: { base_statement: 'a', base_fact_id: 2, base_status: 'accepted' } })), {
		rank: 0,
		badge: 'Changed'
	});
	assert.equal(riskOf(row(1, { blast_radius: { conflict: true, conflicts: [] } })).badge, 'Conflict');
	assert.equal(riskOf(row(1, { citations: [] })).badge, 'Uncited');
	assert.equal(riskOf(row(1, { provenance: { capture_id: 9, trust: 'secondary' } })).badge, 'Low-trust');
	assert.equal(riskOf(row(1, { provenance: { capture_id: 0, trust: '' } })).badge, null);
	assert.equal(riskOf(row(1)).badge, null);
	assert.equal(riskCls('Changed'), 'border-changed text-changed');
	assert.equal(riskCls('Uncited'), 'border-error text-error');
	assert.equal(riskCls('Low-trust'), 'border-line text-ink-muted');
});

test('isVerified needs a gate-sized quote, no diff, no conflict, no Wonder contradiction', () => {
	assert.equal(isVerified(row(1), undefined), true);
	assert.equal(isVerified(row(1), { fact_id: 1, verdict: 'supported' }), true);
	assert.equal(isVerified(row(1), { fact_id: 1, verdict: 'not_found' }), true);
	assert.equal(isVerified(row(1), { fact_id: 1, verdict: 'contradicted' }), false);
	assert.equal(isVerified(row(1, { citations: [] }), undefined), false);
	assert.equal(isVerified(row(1, { blast_radius: { conflict: true, conflicts: [] } }), undefined), false);
	assert.equal(
		isVerified(row(1, { diff: { base_statement: 'a', base_fact_id: 2, base_status: 'accepted' } }), undefined),
		false
	);
});

test('riskiestFirst is a stable risk sort', () => {
	const items = [row(1), row(2, { citations: [] }), row(3), row(4, { blast_radius: { conflict: true, conflicts: [] } })];
	assert.deepEqual(
		riskiestFirst(items).map((i) => i.id),
		[4, 2, 1, 3]
	);
});

test('groupByAuthority counts open and verified rows and keeps group order stable', () => {
	const items = [
		row(1, { authority_slug: 'b-one' }),
		row(2, { authority_slug: 'a-two' }),
		row(3, { authority_slug: 'a-two', citations: [] }),
		row(4, { authority_slug: 'a-two' }),
		row(5, { authority_slug: 'c-three' })
	];
	const checks = { 4: { fact_id: 4, verdict: 'contradicted' } };
	const groups = groupByAuthority(items, { 2: 'accepted' }, checks, { 'a-two': 'Authority Two' });
	assert.deepEqual(
		groups.map((g) => [g.slug, g.name, g.items.length, g.open, g.verified]),
		[
			['a-two', 'Authority Two', 3, 2, 0],
			['b-one', 'b-one', 1, 1, 1],
			['c-three', 'c-three', 1, 1, 1]
		]
	);
	// Riskiest first inside the group: the uncited row leads.
	assert.deepEqual(
		groups[0].items.map((i) => i.id),
		[3, 2, 4]
	);
	// Oldest open row (the decided #2 does not count).
	assert.equal(groups[0].oldest, 103);
	// Deciding every row does not move the group.
	const after = groupByAuthority(items, { 2: 'accepted', 3: 'rejected', 4: 'rejected' }, checks);
	assert.equal(after[0].slug, 'a-two');
	assert.equal(after[0].open, 0);
	assert.equal(after[0].oldest, 0);
});

test('blockedTag and splitBlocked read the bulk refusal vocabulary', () => {
	assert.equal(blockedTag(409, 'fact is not proposed'), 'Decided elsewhere');
	assert.equal(blockedTag(404, 'fact not found'), 'No longer exists');
	assert.equal(blockedTag(422, 'accept requires a verified citation'), 'Needs citation');
	assert.equal(blockedTag(422, 'changes a published fact; review it on its own'), 'Changes a published fact');
	assert.equal(blockedTag(422, 'the cross-check found the source contradicts it'), 'Wonder contradicted');
	assert.equal(blockedTag(422, 'fact not in authority'), 'Other authority');
	assert.equal(blockedTag(422, ''), 'Blocked');
	const split = splitBlocked([
		{ id: 7, ok: true },
		{ id: 8, ok: false, code: 409, error: 'fact is not proposed' },
		{ id: 9, ok: false, code: 422, error: 'accept requires a verified citation' },
		null,
		{ ok: true }
	]);
	assert.deepEqual(split.passing, [7]);
	assert.deepEqual(split.refused, [
		{ id: 8, code: 409, tag: 'Decided elsewhere' },
		{ id: 9, code: 422, tag: 'Needs citation' }
	]);
	assert.deepEqual(splitBlocked(undefined), { refused: [], passing: [] });
});

test('markChanged marks the changed words only; plural counts', () => {
	const { base, next } = markChanged('Takes 5 working days.', 'Takes 3 working days.');
	assert.deepEqual(
		base.filter((s) => s.changed).map((s) => s.text),
		['5']
	);
	assert.deepEqual(
		next.filter((s) => s.changed).map((s) => s.text),
		['3']
	);
	assert.equal(plural(1, 'fact'), '1 fact');
	assert.equal(plural(3, 'fact'), '3 facts');
	assert.equal(plural(2, 'authority', 'authorities'), '2 authorities');
});
