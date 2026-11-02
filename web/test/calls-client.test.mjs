import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import {
	defaultScript,
	dialBody,
	dialCall,
	e164Valid,
	fetchCall,
	fetchCalls,
	hangupCall,
	isLive,
	markQuestionAsked,
	normalizeCallDetail,
	normalizeCallSummary,
	quoteBody,
	quoteToDraft,
	recordCall
} from '../src/lib/admin/calls-client.js';

test('e164Valid mirrors the server shape', () => {
	assert.equal(e164Valid('+13605550100'), true);
	for (const bad of ['', '3605550100', '+1', '+1-360-555-0100', '+1360555010a', null, 42]) {
		assert.equal(e164Valid(bad), false, String(bad));
	}
});

test('dialBody builds valid bodies and rejects the rest', () => {
	const ok = dialBody('+13605550100', 'usa-wa-sos', ['Fee?', 'Hours?'], true);
	assert.deepEqual(ok, {
		to: '+13605550100',
		questions: ['Fee?', 'Hours?'],
		consent_confirmed: true,
		authority: 'usa-wa-sos'
	});
	assert.equal(dialBody('3605550100', null, [], true), null);
	assert.equal(dialBody('+13605550100', null, [], false), null);
	assert.equal(dialBody('+13605550100', null, [''], true), null);
	assert.equal(dialBody('+13605550100', null, [], true), null);
	assert.equal(dialBody('+13605550100', null, ['x'.repeat(281)], true), null);
	assert.equal(dialBody('+13605550100', null, new Array(21).fill('q'), true), null);
	// Blank authority is dropped, not sent.
	assert.ok(!('authority' in (dialBody('+13605550100', '  ', ['q'], true) ?? {})));
});

test('defaultScript fits the server limits and copies', () => {
	const script = defaultScript();
	assert.ok(script.length >= 1 && script.length <= 20);
	for (const line of script) {
		assert.ok(line.length >= 1 && line.length <= 280, line);
	}
	script.push('mutated');
	assert.equal(defaultScript().includes('mutated'), false);
});

test('normalizeCallSummary types rows, nulls junk', () => {
	const row = normalizeCallSummary({
		call_id: 'c-1',
		status: 'ringing',
		authority: 'usa-wa-sos',
		started_at: 100,
		ended_at: null,
		recording_status: 'recording',
		questions_asked: 1,
		questions_total: 3
	});
	assert.equal(row?.questions_asked, 1);
	assert.equal(normalizeCallSummary({ call_id: 'c-1' }), null);
	assert.equal(normalizeCallSummary(null), null);
});

test('normalizeCallDetail types questions and transcript', () => {
	const detail = normalizeCallDetail({
		call_id: 'c-1',
		status: 'in_progress',
		status_detail: null,
		authority: null,
		questions: [{ text: 'Fee?', asked: true }],
		transcript: [{ seq: 0, speaker: null, text: 'hello', at: 5 }],
		transcript_status: 'untranscribed-terms',
		recording_status: 'recording',
		ended_at: null
	});
	assert.equal(detail?.questions[0].asked, true);
	assert.equal(detail?.transcript[0].text, 'hello');
	assert.equal(detail?.transcript_status, 'untranscribed-terms');
	assert.equal(normalizeCallDetail({ call_id: 'c-1', status: 'x' }), null);
	assert.equal(
		normalizeCallDetail({
			call_id: 'c-1',
			status: 'x',
			questions: [{ text: 'q' }],
			transcript: []
		}),
		null
	);
});

test('quoteBody builds valid bodies and rejects the rest', () => {
	assert.deepEqual(quoteBody([0, 1], 'fees', 'Counter fee is $8.'), {
		segment_ids: [0, 1],
		topic: 'fees',
		statement: 'Counter fee is $8.'
	});
	// Trims topic and statement.
	assert.deepEqual(quoteBody([2], '  fees  ', '  Fee.  '), {
		segment_ids: [2],
		topic: 'fees',
		statement: 'Fee.'
	});
	assert.equal(quoteBody([], 'fees', 'Fee.'), null);
	assert.equal(quoteBody([0, -1], 'fees', 'Fee.'), null);
	assert.equal(quoteBody([0.5], 'fees', 'Fee.'), null);
	assert.equal(quoteBody(['0'], 'fees', 'Fee.'), null);
	assert.equal(quoteBody([0], '   ', 'Fee.'), null);
	assert.equal(quoteBody([0], 'fees', '   '), null);
	assert.equal(quoteBody([0], 'fees', 'x'.repeat(1025)), null);
});

test('isLive gates polling', () => {
	assert.equal(isLive('queued'), true);
	assert.equal(isLive('ringing'), true);
	assert.equal(isLive('in_progress'), true);
	assert.equal(isLive('completed'), false);
	assert.equal(isLive('failed'), false);
});

test('calls round-trip against a stub admin API', async () => {
	const seen = [];
	const server = createServer((req, res) => {
		let body = '';
		req.on('data', (chunk) => (body += chunk));
		req.on('end', () => {
			seen.push(`${req.method} ${req.url}`);
			res.writeHead(200, { 'content-type': 'application/json' });
			if (req.method === 'GET' && req.url === '/api/v1/calls') {
				res.end(JSON.stringify({ calls: [{ call_id: 'c-1', status: 'queued', started_at: 1 }] }));
			} else if (req.method === 'POST' && req.url === '/api/v1/calls/dial') {
				const parsed = JSON.parse(body);
				assert.equal(parsed.consent_confirmed, true);
				assert.ok(req.headers['x-csrf'], 'mutation sends x-csrf');
				res.end(JSON.stringify({ call_id: 'c-2', status: 'queued' }));
			} else if (req.method === 'POST' && req.url === '/api/v1/calls/c-1/quote') {
			const parsed = JSON.parse(body);
			assert.deepEqual(parsed.segment_ids, [0]);
			assert.equal(parsed.topic, 'fees');
			assert.ok(req.headers['x-csrf'], 'mutation sends x-csrf');
			res.end(
				JSON.stringify({ fact_id: 7, call_id: 'c-1', topic: 'fees', status: 'proposed' })
			);
		} else if (req.url === '/api/v1/calls/c-1') {
				res.end(
					JSON.stringify({
						call_id: 'c-1',
						status: 'completed',
						questions: [],
						transcript: [],
						recording_status: 'ready'
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
		const calls = await fetchCalls(base);
		assert.equal(calls[0].call_id, 'c-1');
		const detail = await fetchCall(base, 'c-1');
		assert.equal(detail?.status, 'completed');
		const dialed = await dialCall(base, dialBody('+13605550100', null, ['q'], true));
		assert.equal(dialed.call_id, 'c-2');
		await hangupCall(base, 'c-1');
		await recordCall(base, 'c-1', true);
		await recordCall(base, 'c-1', false);
		await markQuestionAsked(base, 'c-1', 0);
		const quoted = await quoteToDraft(base, 'c-1', quoteBody([0], 'fees', 'Fee.'));
		assert.equal(quoted.fact_id, 7);
		assert.equal(quoted.topic, 'fees');
		assert.deepEqual(seen, [
			'GET /api/v1/calls',
			'GET /api/v1/calls/c-1',
			'POST /api/v1/calls/dial',
			'POST /api/v1/calls/c-1/hangup',
			'POST /api/v1/calls/c-1/record/start',
			'POST /api/v1/calls/c-1/record/stop',
			'POST /api/v1/calls/c-1/questions/0/asked',
			'POST /api/v1/calls/c-1/quote'
		]);
	} finally {
		server.close();
	}
});
