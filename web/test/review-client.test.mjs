import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import {
	acceptBody,
	acceptFact,
	createRule,
	fetchQueue,
	fetchQueueItem,
	fetchRules,
	fetchSuppressions,
	normalizeQueueItem,
	normalizeRule,
	normalizeSuppression,
	rejectBody,
	rejectFact,
	revokeRule,
	ruleBody
} from '../src/lib/admin/review-client.js';

const ITEM = {
	id: 7,
	authority_slug: 't-sos',
	topic: 'fees',
	statement: 'Fee is $30.',
	status: 'proposed',
	created_by: 'agent',
	created_at: 100,
	diff: { base_statement: 'Fee is $20.', base_fact_id: 3, base_status: 'accepted' },
	citations: [{ capture_id: 9, start_byte: 0, end_byte: 4, quote: 'Fee ' }],
	reasoning_untrusted: 'matched the fees page',
	blast_radius: {
		conflict: true,
		conflicts: [{ id: 3, statement: 'Fee is $20.', status: 'accepted' }],
		open_tasks: 1
	},
	provenance: {
		capture_id: 9,
		capture_kind: 'web',
		trust: 'official',
		authenticity: 'verified',
		sidecar: 'text/9.txt'
	}
};

test('normalizeQueueItem types enriched items, nulls junk', () => {
	const item = normalizeQueueItem(ITEM);
	assert.equal(item?.id, 7);
	assert.equal(item?.diff?.base_fact_id, 3);
	assert.equal(item?.citations[0].quote, 'Fee ');
	assert.equal(item?.reasoning_untrusted, 'matched the fees page');
	assert.equal(item?.blast_radius.conflict, true);
	assert.equal(item?.provenance.trust, 'official');
	// Diff and reasoning are optional.
	const bare = normalizeQueueItem({ ...ITEM, diff: null, reasoning_untrusted: null });
	assert.equal(bare?.diff, null);
	assert.equal(bare?.reasoning_untrusted, null);
	assert.equal(normalizeQueueItem({ id: 7 }), null);
	assert.equal(normalizeQueueItem({ ...ITEM, citations: [{ capture_id: 9 }] }), null);
	assert.equal(normalizeQueueItem({ ...ITEM, blast_radius: null }), null);
	assert.equal(normalizeQueueItem({ ...ITEM, provenance: { capture_id: 9 } }), null);
	assert.equal(normalizeQueueItem(null), null);
});

test('normalizeRule and normalizeSuppression type rows, null junk', () => {
	const rule = normalizeRule({
		id: 2,
		authority_id: 1,
		authority_slug: 't-sos',
		topic: 'fees',
		statement: 'Fee is $99.',
		reason: 'stale',
		created_by: 'staff',
		created_at: 50,
		revoked_at: null
	});
	assert.equal(rule?.authority_slug, 't-sos');
	assert.equal(rule?.revoked_at, null);
	const global = normalizeRule({
		id: 3,
		authority_id: null,
		authority_slug: null,
		topic: null,
		statement: 'x',
		reason: '',
		created_by: 'staff',
		created_at: 51,
		revoked_at: 60
	});
	assert.equal(global?.authority_slug, null);
	assert.equal(global?.revoked_at, 60);
	assert.equal(normalizeRule({ id: 2 }), null);

	const skip = normalizeSuppression({
		id: 4,
		capture_id: 9,
		authority_slug: 't-sos',
		topic: 'fees',
		statement: 'Fee is $99.',
		path: 'B',
		rule_id: 2,
		created_at: 70
	});
	assert.equal(skip?.path, 'B');
	assert.equal(skip?.rule_id, 2);
	const noskip = normalizeSuppression({
		id: 5,
		capture_id: 9,
		authority_slug: 't-sos',
		topic: 'fees',
		statement: 'Fee is $99.',
		path: 'A',
		rule_id: null,
		created_at: 71
	});
	assert.equal(noskip?.rule_id, null);
	assert.equal(normalizeSuppression({ id: 4 }), null);
});

test('acceptBody builds valid bodies and rejects the rest', () => {
	assert.deepEqual(acceptBody(undefined, undefined), {});
	assert.deepEqual(acceptBody(' looks right ', undefined), { reason: 'looks right' });
	assert.deepEqual(acceptBody(null, 'Fee is $30.'), { statement: 'Fee is $30.' });
	// Blank reason is dropped, not sent.
	assert.deepEqual(acceptBody('   ', undefined), {});
	assert.equal(acceptBody(undefined, '   '), null);
	assert.equal(acceptBody(undefined, 'x'.repeat(2001)), null);
	assert.equal(acceptBody(undefined, 42), null);
});

test('rejectBody requires a reason and gates suppress scope', () => {
	assert.deepEqual(rejectBody(' stale page ', undefined), { reason: 'stale page' });
	assert.deepEqual(rejectBody('bad', { scope: 'authority' }), {
		reason: 'bad',
		suppress: { scope: 'authority' }
	});
	assert.deepEqual(rejectBody('bad', { scope: 'global', lock_topic: false }), {
		reason: 'bad',
		suppress: { scope: 'global', lock_topic: false }
	});
	assert.equal(rejectBody('', undefined), null);
	assert.equal(rejectBody('   ', undefined), null);
	assert.equal(rejectBody('x'.repeat(1025), undefined), null);
	assert.equal(rejectBody('bad', { scope: 'region' }), null);
	assert.equal(rejectBody('bad', { scope: 'authority', lock_topic: 'yes' }), null);
	assert.equal(rejectBody('bad', 'authority'), null);
});

test('ruleBody builds valid bodies and rejects the rest', () => {
	assert.deepEqual(ruleBody('Fee is $99.', 't-sos', 'fees', 'stale'), {
		statement: 'Fee is $99.',
		authority: 't-sos',
		topic: 'fees',
		reason: 'stale'
	});
	// Blank scope trims out to a global rule.
	assert.deepEqual(ruleBody('Fee is $99.', '  ', '', ''), { statement: 'Fee is $99.' });
	assert.equal(ruleBody('   ', null, null, null), null);
	assert.equal(ruleBody('x'.repeat(2001), null, null, null), null);
	assert.equal(ruleBody('x', 42, null, null), null);
	assert.equal(ruleBody('x', null, null, 'y'.repeat(1025)), null);
});

test('review round-trips against a stub admin API', async () => {
	const seen = [];
	const server = createServer((req, res) => {
		let body = '';
		req.on('data', (chunk) => (body += chunk));
		req.on('end', () => {
			seen.push(`${req.method} ${req.url}`);
			res.writeHead(200, { 'content-type': 'application/json' });
			if (req.method === 'GET' && req.url === '/api/v1/queue?limit=50') {
				res.end(JSON.stringify({ items: [ITEM], count: 1 }));
			} else if (req.url === '/api/v1/queue/7') {
				res.end(JSON.stringify(ITEM));
			} else if (req.method === 'POST' && req.url === '/api/v1/facts/7/accept') {
				const parsed = JSON.parse(body);
				assert.equal(parsed.reason, 'looks right');
				assert.ok(req.headers['x-csrf'], 'mutation sends x-csrf');
				res.end(JSON.stringify({ status: 'accepted' }));
			} else if (req.method === 'POST' && req.url === '/api/v1/facts/7/reject') {
				const parsed = JSON.parse(body);
				assert.equal(parsed.suppress.scope, 'authority');
				res.end(JSON.stringify({ status: 'rejected' }));
			} else if (req.method === 'GET' && req.url === '/api/v1/suppress-rules') {
				res.end(JSON.stringify({ rules: [], count: 0 }));
			} else if (req.method === 'POST' && req.url === '/api/v1/suppress-rules') {
				const parsed = JSON.parse(body);
				assert.equal(parsed.statement, 'Fee is $99.');
				assert.ok(req.headers['x-csrf'], 'mutation sends x-csrf');
				res.end(JSON.stringify({ id: 2 }));
			} else if (
				req.method === 'POST' &&
				req.url === '/api/v1/suppress-rules/2/revoke'
			) {
				res.end(JSON.stringify({ status: 'revoked' }));
			} else if (req.method === 'GET' && req.url === '/api/v1/suppressions?limit=50') {
				res.end(
					JSON.stringify({
						suppressions: [
							{
								id: 4,
								capture_id: 9,
								authority_slug: 't-sos',
								topic: 'fees',
								statement: 'Fee is $99.',
								path: 'B',
								rule_id: 2,
								created_at: 70
							}
						],
						count: 1
					})
				);
			} else {
				res.end(JSON.stringify({ ok: true }));
			}
		});
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const base = `http://127.0.0.1:${server.address().port}/api/v1`;
	try {
		const queue = await fetchQueue(base, { limit: 50 });
		assert.equal(queue.count, 1);
		assert.equal(queue.items[0]?.id, 7);
		const item = await fetchQueueItem(base, 7);
		assert.equal(item.provenance.sidecar, 'text/9.txt');
		const accepted = await acceptFact(base, 7, acceptBody('looks right', undefined));
		assert.equal(accepted.status, 'accepted');
		const rejected = await rejectFact(base, 7, rejectBody('stale', { scope: 'authority' }));
		assert.equal(rejected.status, 'rejected');
		const rules = await fetchRules(base);
		assert.equal(rules.count, 0);
		const created = await createRule(base, ruleBody('Fee is $99.', 't-sos', 'fees', 'stale'));
		assert.equal(created.id, 2);
		await revokeRule(base, 2);
		const log = await fetchSuppressions(base, { limit: 50 });
		assert.equal(log.suppressions[0]?.path, 'B');
		assert.deepEqual(seen, [
			'GET /api/v1/queue?limit=50',
			'GET /api/v1/queue/7',
			'POST /api/v1/facts/7/accept',
			'POST /api/v1/facts/7/reject',
			'GET /api/v1/suppress-rules',
			'POST /api/v1/suppress-rules',
			'POST /api/v1/suppress-rules/2/revoke',
			'GET /api/v1/suppressions?limit=50'
		]);
	} finally {
		server.close();
	}
});

test('review shows the server denial verbatim, IP included', async () => {
	const server = createServer((req, res) => {
		if (req.url.includes('denied')) {
			res.writeHead(403, { 'content-type': 'application/json' });
			res.end(
				JSON.stringify({
					error: 'admin access is limited to the configured IP range (admin.allow_ips) — your IP is 198.51.100.9',
					code: 'ip_not_allowed',
					ip: '198.51.100.9'
				})
			);
			return;
		}
		res.writeHead(401, { 'content-type': 'application/json' });
		res.end(JSON.stringify({ error: 'unauthorized' }));
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const base = `http://127.0.0.1:${server.address().port}/api/v1`;
	try {
		// 401: no/invalid session — the fix is logging in again.
		await assert.rejects(fetchQueue(base, { limit: 50 }), /log in on the admin host again/);
		// 403: the server's answer carries the refused IP — it must reach the user.
		await assert.rejects(fetchQueueItem(base, 'denied'), /198\.51\.100\.9/);
		await assert.rejects(fetchQueueItem(base, 'denied'), /admin\.allow_ips/);
	} finally {
		server.close();
	}
});

test('review surfaces 409 and 422 server detail', async () => {
	const server = createServer((req, res) => {
		res.writeHead(req.url.endsWith('/accept') ? 409 : 422, {
			'content-type': 'application/json'
		});
		res.end(
			JSON.stringify({
				error: req.url.endsWith('/accept') ? 'fact is not proposed' : 'reason required (1-1024 chars)'
			})
		);
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const base = `http://127.0.0.1:${server.address().port}/api/v1`;
	try {
		await assert.rejects(
			acceptFact(base, 7, acceptBody('x', undefined)),
			/Accept: fact is not proposed\./
		);
		await assert.rejects(
			rejectFact(base, 7, rejectBody('bad', undefined)),
			/Reject: reason required \(1-1024 chars\)\./
		);
	} finally {
		server.close();
	}
});
