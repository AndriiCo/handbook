import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import {
	bulkBody,
	bulkDecide,
	factViewToItem,
	fetchProposedFacts
} from '../src/lib/admin/review-client.js';

test('bulkBody builds valid bulk bodies and rejects the rest', () => {
	assert.deepEqual(bulkBody('accept', 't-sos', [7, 8], { requireVerified: true }), {
		decision: 'accept',
		authority: 't-sos',
		ids: [7, 8],
		require_verified: true
	});
	assert.deepEqual(bulkBody('accept', 't-sos', [7], { reason: '  batch ok  ' }), {
		decision: 'accept',
		authority: 't-sos',
		ids: [7],
		reason: 'batch ok'
	});
	assert.deepEqual(
		bulkBody('reject', 't-sos', [7, 9], { reason: 'stale page', suppress: { scope: 'global', lock_topic: false } }),
		{
			decision: 'reject',
			authority: 't-sos',
			ids: [7, 9],
			reason: 'stale page',
			suppress: { scope: 'global', lock_topic: false }
		}
	);
	// Decision, authority, ids.
	assert.equal(bulkBody('approve', 't-sos', [7]), null);
	assert.equal(bulkBody('accept', '', [7]), null);
	assert.equal(bulkBody('accept', 'T SOS', [7]), null);
	assert.equal(bulkBody('accept', 't-sos', []), null);
	assert.equal(bulkBody('accept', 't-sos', [7, 7]), null);
	assert.equal(bulkBody('accept', 't-sos', [0]), null);
	assert.equal(bulkBody('accept', 't-sos', [1.5]), null);
	assert.equal(bulkBody('accept', 't-sos', 'abc'), null);
	assert.equal(
		bulkBody('accept', 't-sos', Array.from({ length: 101 }, (_, i) => i + 1)),
		null
	);
	assert.ok(bulkBody('accept', 't-sos', Array.from({ length: 100 }, (_, i) => i + 1)));
	// Reject needs a reason; reasons cap at 1024.
	assert.equal(bulkBody('reject', 't-sos', [7]), null);
	assert.equal(bulkBody('reject', 't-sos', [7], { reason: '   ' }), null);
	assert.equal(bulkBody('accept', 't-sos', [7], { reason: 'y'.repeat(1025) }), null);
	assert.equal(bulkBody('accept', 't-sos', [7], { reason: 42 }), null);
	// suppress is reject-only; require_verified is accept-only.
	assert.equal(bulkBody('accept', 't-sos', [7], { suppress: { scope: 'authority' } }), null);
	assert.equal(bulkBody('reject', 't-sos', [7], { reason: 'x', suppress: { scope: 'everywhere' } }), null);
	assert.equal(bulkBody('reject', 't-sos', [7], { reason: 'x', requireVerified: true }), null);
	assert.equal(bulkBody('accept', 't-sos', [7], { requireVerified: 'yes' }), null);
	assert.deepEqual(bulkBody('accept', 't-sos', [7], { requireVerified: false }), {
		decision: 'accept',
		authority: 't-sos',
		ids: [7]
	});
});

test('factViewToItem adapts /facts rows to review rows', () => {
	const item = factViewToItem({
		id: 21,
		authority_slug: 't-sos',
		topic: 'fees',
		statement: 'Fee is $30.',
		status: 'proposed',
		created_by: 'muse-spark-1.3 via analysis-agent',
		drafted_by: 'muse-spark-1.3 via analysis-agent',
		created_at: 100,
		citations: [
			{ quote: 'The fee is $30 per document.', source_url: 'https://example.test/fees', captured_at: '2026-10-20T09:00:00Z' }
		],
		conflict: true,
		conflicts: [
			{ id: 3, statement: 'Fee is $20.', status: 'accepted' },
			{ id: 22, statement: 'Fee is $35.', status: 'proposed' }
		]
	});
	assert.equal(item?.id, 21);
	// Diff base mirrors the queue: newest accepted sibling first.
	assert.deepEqual(item?.diff, { base_statement: 'Fee is $20.', base_fact_id: 3, base_status: 'accepted' });
	assert.equal(item?.blast_radius.conflict, true);
	assert.equal(item?.citations[0]?.quote, 'The fee is $30 per document.');
	assert.equal(item?.citations[0]?.source_url, 'https://example.test/fees');
	assert.equal(item?.citations[0]?.capture_id, 0);
	assert.equal(item?.provenance.trust, '');
	const plain = factViewToItem({ id: 5, authority_slug: 'x', topic: 'hours', statement: 's', created_at: 1, citations: [] });
	assert.equal(plain?.diff, null);
	assert.equal(plain?.blast_radius.conflict, false);
	assert.equal(factViewToItem({ id: '5' }), null);
	assert.equal(factViewToItem(null), null);
});

test('bulkDecide posts with CSRF, resolves blocked batches, throws the rest', async () => {
	const seen = [];
	const server = createServer((req, res) => {
		let body = '';
		req.on('data', (chunk) => (body += chunk));
		req.on('end', () => {
			const parsed = body ? JSON.parse(body) : {};
			seen.push({ url: req.url, csrf: req.headers['x-csrf'] ?? null, body: parsed });
			res.setHeader('content-type', 'application/json');
			if (req.url.startsWith('/api/v1/facts?')) {
				res.writeHead(200);
				res.end(
					JSON.stringify({
						facts: [
							{
								id: 21,
								authority_slug: 't-sos',
								topic: 'fees',
								statement: 's',
								created_at: 1,
								citations: [],
								conflict: false,
								conflicts: []
							},
							{ junk: true }
						],
						count: 2
					})
				);
				return;
			}
			const ids = parsed.ids ?? [];
			if (ids.includes(403)) {
				res.writeHead(403);
				res.end(
					JSON.stringify({
						error: 'admin access is limited to the configured IP range (admin.allow_ips) — your IP is 198.51.100.9'
					})
				);
			} else if (ids.includes(409)) {
				res.writeHead(409);
				res.end(
					JSON.stringify({
						error: 'batch blocked; nothing changed',
						results: [
							{ id: 7, ok: true },
							{ id: 409, ok: false, code: 409, error: 'fact is not proposed' }
						]
					})
				);
			} else if (ids.includes(422)) {
				res.writeHead(422);
				res.end(JSON.stringify({ error: 'ids must be distinct positive fact ids' }));
			} else {
				res.writeHead(200);
				res.end(
					JSON.stringify({
						authority: parsed.authority,
						decision: parsed.decision,
						count: ids.length,
						bulk_audit_id: 1234,
						results: ids.map((id) => ({ id, ok: true, status: 'accepted' }))
					})
				);
			}
		});
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const base = `http://127.0.0.1:${server.address().port}/api/v1`;
	try {
		const done = await bulkDecide(base, bulkBody('accept', 't-sos', [7, 8], { requireVerified: true }));
		assert.equal(done.ok, true);
		assert.equal(done.ok && done.count, 2);
		assert.equal(done.ok && done.bulk_audit_id, 1234);
		assert.deepEqual(
			done.results.map((r) => r.status),
			['accepted', 'accepted']
		);
		assert.ok(seen[0].csrf, 'bulk POST sends x-csrf');
		assert.deepEqual(seen[0].body, { decision: 'accept', authority: 't-sos', ids: [7, 8], require_verified: true });
		assert.equal(seen[0].url, '/api/v1/facts/bulk');

		const blocked = await bulkDecide(base, bulkBody('accept', 't-sos', [7, 409]));
		assert.equal(blocked.ok, false);
		assert.equal(!blocked.ok && blocked.status, 409);
		assert.deepEqual(blocked.results, [
			{ id: 7, ok: true },
			{ id: 409, ok: false, code: 409, error: 'fact is not proposed' }
		]);

		// A validation 422 carries no `results`: it throws with the detail.
		await assert.rejects(
			bulkDecide(base, bulkBody('accept', 't-sos', [422])),
			/Bulk decision: ids must be distinct positive fact ids\./
		);
		// Denials stay verbatim, IP included.
		await assert.rejects(bulkDecide(base, bulkBody('accept', 't-sos', [403])), /198\.51\.100\.9/);

		const facts = await fetchProposedFacts(base, { authority: 't-sos' });
		assert.equal(facts.items.length, 1);
		assert.equal(seen.at(-1).url, '/api/v1/facts?status=proposed&authority=t-sos&limit=200');
	} finally {
		server.close();
	}
});
