import test, { mock } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
	DEFAULT_ATTEMPTS,
	MAX_PAYLOAD_BYTES,
	WorkError,
	auditLabel,
	canStagePacks,
	clockOffset,
	enqueueBody,
	enqueueWork,
	examplePayload,
	fetchResultText,
	fetchWorkItem,
	fetchWorkList,
	fmtBytes,
	isJsonType,
	leaseText,
	listPath,
	normalizeDetail,
	normalizeItem,
	normalizeList,
	normalizeStage,
	parseMaxAttempts,
	parsePayload,
	poll,
	resultHref,
	resultView,
	retryWork,
	cancelWork,
	approveEnrolment,
	cleanCode,
	denyEnrolment,
	fetchEnrolments,
	finalWorkerName,
	normalizeEnrolments,
	revokeWorker,
	validWorkerName,
	workerNameProblem,
	stagePacks,
	stagedText,
	statusChanges,
	statusLabel,
	statusTone
} from '../src/lib/admin/work-client.js';

// The contract seed both sides test against (work-queue.md §3).
const HERE = dirname(fileURLToPath(import.meta.url));
const CONTRACT = JSON.parse(readFileSync(join(HERE, '..', '..', 'docs', 'contracts', 'admin-work-items.json'), 'utf8'));

test('normalizeList reads the contract list sample', () => {
	const list = normalizeList(CONTRACT.list);
	assert.ok(list);
	assert.equal(list.items.length, 2);
	const [leased, done] = list.items;
	assert.equal(leased.status, 'leased');
	assert.equal(leased.lease_owner, 'plant');
	assert.equal(leased.lease_expires_at, 1793750700);
	assert.equal(leased.result, null);
	assert.deepEqual(leased.payload, { countries: ['EG'] });
	assert.deepEqual(done.result, { sha256: '0'.repeat(64), bytes: 1234, content_type: 'application/json' });
	assert.deepEqual(done.summary, { pages: 1, failed: 0 });
	// Every seed item carries `staged`; the sample has none staged yet.
	for (const raw of CONTRACT.list.items) assert.equal(typeof raw.staged, 'boolean');
	assert.deepEqual(
		list.items.map((i) => i.staged),
		CONTRACT.list.items.map((i) => i.staged)
	);
	assert.equal(list.workers[0].name, 'chandra@plant');
	assert.equal(list.workers[0].job_id, 7);
	assert.equal(list.workers[0].conn, 1);
	assert.equal(list.workers[0].credential_id, '0123456789abcdef0123456789abcdef');
	assert.equal('identity' in list.workers[0], false);
	assert.equal(list.workers[0].version_matches, true);
	assert.deepEqual(list.workers[0].capabilities, ['research.nonparty', 'scrape.urls', 'eval.gold', 'precheck.glimmer']);
	assert.deepEqual(list.kinds, [
		{ kind: 'research.nonparty', about: 'Non-party legalization packs', example: { countries: ['EG'] }, max_runtime_secs: 28800 }
	]);
	assert.deepEqual(list.counts, { pending: 0, leased: 1, done: 1, failed: 0 });
	assert.equal(list.offset, 0);
});

test('normalizeList drops junk rows and defaults missing sections', () => {
	const list = normalizeList({ items: [null, { id: 'x' }, { id: 3, kind: 'scrape.urls', status: 'pending' }] });
	assert.ok(list);
	assert.equal(list.items.length, 1);
	assert.equal(list.items[0].max_attempts, DEFAULT_ATTEMPTS);
	assert.equal(list.items[0].lease_owner, null);
	assert.deepEqual(list.workers, []);
	assert.deepEqual(list.kinds, []);
	assert.deepEqual(list.counts, { pending: 0, leased: 0, done: 0, failed: 0 });
	assert.equal(normalizeList({}), null);
	assert.equal(normalizeList(null), null);
	// A kind without an object example gets {} (the payload box needs one).
	assert.deepEqual(normalizeList({ items: [], kinds: [{ kind: 'k', example: [1] }, { kind: '' }] })?.kinds, [
		{ kind: 'k', about: '', example: {}, max_runtime_secs: null }
	]);
	// A result without sha/bytes is no result.
	assert.equal(normalizeItem({ id: 1, kind: 'k', status: 'done', result: { bytes: 3 } })?.result, null);
	// `staged` defaults to false; only a real true counts.
	assert.equal(normalizeItem({ id: 1, kind: 'k', status: 'done' })?.staged, false);
	assert.equal(normalizeItem({ id: 1, kind: 'k', status: 'done', staged: 'yes' })?.staged, false);
	assert.equal(normalizeItem({ id: 1, kind: 'k', status: 'done', staged: true })?.staged, true);
	// `retry_after` is a number or null.
	assert.equal(normalizeItem({ id: 1, kind: 'k', status: 'pending', retry_after: 161 })?.retry_after, 161);
	assert.equal(normalizeItem({ id: 1, kind: 'k', status: 'pending', retry_after: 'soon' })?.retry_after, null);
});

test('normalizeDetail reads the contract detail sample', () => {
	const detail = normalizeDetail(CONTRACT.detail);
	assert.ok(detail);
	assert.equal(detail.item.id, 6);
	assert.deepEqual(
		detail.audit.map((a) => a.action),
		['work_enqueue', 'work_checkout', 'work_complete']
	);
	assert.equal(detail.audit[2].reason, '1234 bytes');
	assert.equal(detail.item.staged, CONTRACT.detail.item.staged);
	assert.equal(normalizeDetail({ audit: [] }), null);
	assert.deepEqual(normalizeDetail({ item: CONTRACT.detail.item, audit: [{ at: 1 }, null] })?.audit, []);
});

test('examplePayload pretty-prints the kind example', () => {
	const kinds = normalizeList(CONTRACT.list)?.kinds ?? [];
	assert.equal(examplePayload(kinds, 'research.nonparty'), '{\n  "countries": [\n    "EG"\n  ]\n}');
	assert.equal(examplePayload(kinds, 'unknown.kind'), '{}');
});

test('parsePayload accepts objects only, within 64 KiB', () => {
	assert.deepEqual(parsePayload('{"urls": ["https://example.com/synthetic/1"]}'), {
		ok: true,
		payload: { urls: ['https://example.com/synthetic/1'] }
	});
	assert.deepEqual(parsePayload(' {} '), { ok: true, payload: {} });
	for (const [text, why] of [
		['', 'Enter the payload'],
		['   ', 'Enter the payload'],
		['{', 'not valid JSON'],
		['[1, 2]', 'not an array'],
		['null', 'not null'],
		['"x"', 'not a string'],
		['42', 'not a number'],
		['true', 'not true']
	]) {
		const out = parsePayload(text);
		assert.equal(out.ok, false, text);
		assert.ok(!out.ok && out.error.includes(why), `${text}: ${!out.ok && out.error}`);
	}
	// The cap is on the compact form; whitespace in the box is free.
	const fits = JSON.stringify({ a: 'x'.repeat(MAX_PAYLOAD_BYTES - 8) });
	assert.equal(fits.length, MAX_PAYLOAD_BYTES);
	assert.equal(parsePayload(fits).ok, true);
	const over = parsePayload(JSON.stringify({ a: 'x'.repeat(MAX_PAYLOAD_BYTES - 7) }));
	assert.ok(!over.ok && over.error.includes('64 KiB'));
	// Multi-byte text counts in UTF-8 bytes, as the server reads it.
	assert.equal(parsePayload(JSON.stringify({ a: 'é'.repeat(MAX_PAYLOAD_BYTES / 2) })).ok, false);
});

test('parseMaxAttempts: blank is 3, whole numbers 1 to 10 only', () => {
	assert.equal(parseMaxAttempts(null), 3);
	assert.equal(parseMaxAttempts(undefined), 3);
	assert.equal(parseMaxAttempts(''), 3);
	assert.equal(parseMaxAttempts(1), 1);
	assert.equal(parseMaxAttempts('10'), 10);
	for (const bad of [0, 11, -1, 2.5, '2.5', 'abc', '1e1', Number.NaN, {}]) {
		assert.equal(parseMaxAttempts(bad), null, String(bad));
	}
});

test('enqueueBody names the failing field', () => {
	assert.deepEqual(enqueueBody('scrape.urls', JSON.stringify(CONTRACT.enqueue_request.payload), 3), {
		ok: true,
		body: CONTRACT.enqueue_request
	});
	assert.equal(enqueueBody('', '{}', 3).ok === false && enqueueBody('', '{}', 3).field, 'kind');
	const payload = enqueueBody('k', '[]', 3);
	assert.ok(!payload.ok && payload.field === 'payload');
	const attempts = enqueueBody('k', '{}', 11);
	assert.ok(!attempts.ok && attempts.field === 'max_attempts');
	// Blank max attempts sends the default, never nothing.
	assert.deepEqual(enqueueBody('k', '{}', null), { ok: true, body: { kind: 'k', payload: {}, max_attempts: 3 } });
});

test('statusTone, statusLabel and auditLabel', () => {
	assert.equal(statusTone('done'), 'done');
	assert.equal(statusTone('leased'), 'active');
	assert.equal(statusTone('failed'), 'error');
	assert.equal(statusTone('pending'), 'muted');
	assert.equal(statusTone('something_new'), 'muted');
	assert.equal(statusLabel('leased'), 'Leased');
	assert.equal(statusLabel('something_new'), 'something new');
	assert.equal(auditLabel('work_checkout'), 'Checked out');
	assert.equal(auditLabel('work_expire'), 'Lease expired');
	assert.equal(auditLabel('work_other'), 'work other');
});

test('leaseText counts down and up against the server clock', () => {
	assert.equal(leaseText(null, 100), null);
	assert.equal(leaseText(1045, 1000), 'expires in 45 s');
	assert.equal(leaseText(1300, 1000), 'expires in 5 min');
	assert.equal(leaseText(1000 + 2 * 3600, 1000), 'expires in 2 h');
	assert.equal(leaseText(1000, 1000), 'expires now');
	assert.equal(leaseText(1000, 1012), 'expired 12 s ago');
	assert.equal(leaseText(1000, 1000 + 3 * 86400), 'expired 3 d ago');
});

test('clockOffset reads the Date header, ignores noise', () => {
	const at = Date.parse('2026-11-03T12:00:00Z');
	assert.equal(clockOffset('Tue, 03 Nov 2026 12:02:00 GMT', at), 120);
	assert.equal(clockOffset('Tue, 03 Nov 2026 11:59:00 GMT', at), -60);
	assert.equal(clockOffset('Tue, 03 Nov 2026 12:00:01 GMT', at), 0);
	assert.equal(clockOffset(null, at), 0);
	assert.equal(clockOffset('not a date', at), 0);
});

test('isJsonType and fmtBytes', () => {
	assert.equal(isJsonType('application/json'), true);
	assert.equal(isJsonType('application/json; charset=utf-8'), true);
	assert.equal(isJsonType('application/problem+json'), true);
	assert.equal(isJsonType('text/html'), false);
	assert.equal(isJsonType(null), false);
	assert.equal(fmtBytes(12), '12 B');
	assert.equal(fmtBytes(1234), '1.2 KiB');
	assert.equal(fmtBytes(64 * 1024), '64 KiB');
	assert.equal(fmtBytes(16 * 1024 * 1024), '16 MiB');
	assert.equal(fmtBytes(-1), '—');
});

test('resultView pretty-prints, caps and says so', () => {
	assert.deepEqual(resultView('{"a":[1]}'), { text: '{\n  "a": [\n    1\n  ]\n}', parsed: true, truncated: false, chars: 22 });
	const plain = resultView('not json');
	assert.equal(plain.parsed, false);
	assert.equal(plain.text, 'not json');
	const big = resultView(JSON.stringify({ a: 'x'.repeat(100) }), 50);
	assert.equal(big.truncated, true);
	assert.equal(big.text.length, 50);
	assert.ok(big.chars > 50);
	// A cut landing inside a surrogate pair drops the lone half.
	const emoji = resultView('ab\u{1F600}cd', 3);
	assert.equal(emoji.text, 'ab');
	assert.equal(emoji.truncated, true);
});

test('statusChanges announces only items that changed', () => {
	const items = normalizeList(CONTRACT.list)?.items ?? [];
	const before = new Map([
		[7, 'pending'],
		[6, 'done']
	]);
	assert.deepEqual(statusChanges(before, items), ['Job 7 (research.nonparty) is leased.']);
	assert.deepEqual(statusChanges(new Map(), items), []);
});

test('canStagePacks: finished research.nonparty, or discovery with packs', () => {
	const done = { kind: 'research.nonparty', status: 'done', result: { sha256: 'a', bytes: 1, content_type: 'application/json' } };
	assert.equal(canStagePacks(done), true);
	assert.equal(canStagePacks({ ...done, staged: true }), true); // re-staging is allowed
	assert.equal(canStagePacks({ ...done, kind: 'scrape.urls' }), false);
	assert.equal(canStagePacks({ ...done, status: 'leased' }), false);
	assert.equal(canStagePacks({ ...done, result: null }), false);
	// discovery.bootstrap stages when its summary counts world packs (D-3).
	const discovery = { ...done, kind: 'discovery.bootstrap', summary: { packs: 2 } };
	assert.equal(canStagePacks(discovery), true);
	assert.equal(canStagePacks({ ...discovery, summary: { packs: 0 } }), false);
	assert.equal(canStagePacks({ ...discovery, summary: null }), false);
	assert.equal(canStagePacks({ ...discovery, status: 'failed' }), false);
	// The seed's done item is scrape.urls, so it does not qualify.
	for (const item of normalizeList(CONTRACT.list)?.items ?? []) assert.equal(canStagePacks(item), false);
});

test('normalizeStage and stagedText read the contract stage sample', () => {
	const out = normalizeStage(CONTRACT.stage_response, 9);
	assert.deepEqual(out, { id: 9, dir: 'packs-inbox/9', packs: ['EG'], count: 1 });
	assert.equal(stagedText(out), 'Staged 1 pack in packs-inbox/9 — load it from the Packs page.');
	assert.equal(
		stagedText({ id: 4, dir: 'packs-inbox/4', packs: ['EG', 'IQ'], count: 2 }),
		'Staged 2 packs in packs-inbox/4 — load them from the Packs page.'
	);
	assert.equal(stagedText({ id: 4, dir: 'packs-inbox/4', packs: [], count: 0 }), 'The bundle held no packs; packs-inbox/4 is empty.');
	// Missing fields fall back to the id and the inbox convention.
	assert.deepEqual(normalizeStage({ packs: ['EG', 3, 'IQ'] }, 5), { id: 5, dir: 'packs-inbox/5', packs: ['EG', 'IQ'], count: 2 });
	assert.deepEqual(normalizeStage(null, 5), { id: 5, dir: 'packs-inbox/5', packs: [], count: 0 });
});

test('listPath and resultHref', () => {
	assert.equal(listPath('all'), '/admin/work-items?limit=50');
	assert.equal(listPath('failed'), '/admin/work-items?limit=50&status=failed');
	assert.equal(listPath('bogus', 10), '/admin/work-items?limit=10');
	assert.equal(resultHref('/apostille/api/v1', 6), '/apostille/api/v1/admin/work-items/6/result');
});

test('poll: one run at a time, paused while hidden, at once on return', async () => {
	mock.timers.enable({ apis: ['setTimeout'] });
	const flush = () => new Promise((resolve) => setImmediate(resolve));
	/** @type {Map<string, () => void>} */
	const listeners = new Map();
	const doc = {
		hidden: false,
		addEventListener: (/** @type {string} */ type, /** @type {() => void} */ fn) => listeners.set(type, fn),
		removeEventListener: (/** @type {string} */ type) => listeners.delete(type)
	};
	let runs = 0;
	/** @type {(() => void) | null} */
	let release = null;
	const run = () =>
		new Promise((resolve) => {
			runs += 1;
			release = () => resolve(undefined);
		});
	try {
		const poller = poll(run, { intervalMs: 5000, doc });
		mock.timers.tick(4999);
		assert.equal(runs, 0);
		mock.timers.tick(1);
		assert.equal(runs, 1);
		// While the run is in flight no second one starts.
		mock.timers.tick(20_000);
		assert.equal(runs, 1);
		// A manual refresh during the run queues exactly one more.
		poller.now();
		poller.now();
		release?.();
		await flush();
		assert.equal(runs, 2);
		release?.();
		await flush();
		mock.timers.tick(5000);
		assert.equal(runs, 3);
		release?.();
		await flush();
		// Hidden: the timer is dropped and nothing runs.
		doc.hidden = true;
		listeners.get('visibilitychange')?.();
		mock.timers.tick(60_000);
		assert.equal(runs, 3);
		// Visible again: one run at once, then the interval resumes.
		doc.hidden = false;
		listeners.get('visibilitychange')?.();
		assert.equal(runs, 4);
		release?.();
		await flush();
		mock.timers.tick(5000);
		assert.equal(runs, 5);
		release?.();
		await flush();
		poller.stop();
		assert.equal(listeners.size, 0);
		mock.timers.tick(60_000);
		assert.equal(runs, 5);
	} finally {
		mock.timers.reset();
	}
});

test('work items round-trip against a stub admin API', async () => {
	const seen = [];
	const server = createServer((req, res) => {
		let body = '';
		req.on('data', (chunk) => (body += chunk));
		req.on('end', () => {
			seen.push(`${req.method} ${req.url}`);
			const json = (status, data) => {
				res.writeHead(status, { 'content-type': 'application/json' });
				res.end(JSON.stringify(data));
			};
			if (req.method === 'GET' && req.url === '/api/v1/admin/work-items?limit=50&status=done') {
				json(200, CONTRACT.list);
			} else if (req.method === 'POST' && req.url === '/api/v1/admin/work-items') {
				assert.ok(req.headers['x-csrf'], 'enqueue sends x-csrf');
				assert.equal(req.headers['content-type'], 'application/json');
				const parsed = JSON.parse(body);
				if (parsed.kind === 'nope') json(422, { error: 'unknown kind: nope' });
				else {
					assert.deepEqual(parsed, CONTRACT.enqueue_request);
					json(201, CONTRACT.enqueue_response);
				}
			} else if (req.method === 'POST' && req.url === '/api/v1/admin/work-items/5/retry') {
				assert.ok(req.headers['x-csrf'], 'retry sends x-csrf');
				json(200, CONTRACT.retry_response);
			} else if (req.method === 'POST' && req.url === '/api/v1/admin/work-items/7/cancel') {
				assert.ok(req.headers['x-csrf'], 'cancel sends x-csrf');
				json(200, CONTRACT.cancel_response);
			} else if (req.method === 'POST' && req.url === '/api/v1/admin/work-items/6/cancel') {
				json(409, { error: 'only pending or leased items cancel' });
			} else if (req.method === 'POST' && req.url === '/api/v1/admin/work-items/9/stage-packs') {
				assert.ok(req.headers['x-csrf'], 'stage sends x-csrf');
				assert.deepEqual(JSON.parse(body), {});
				json(200, CONTRACT.stage_response);
			} else if (req.method === 'POST' && req.url === '/api/v1/admin/work-items/6/stage-packs') {
				json(409, { error: 'job 6 is scrape.urls, not research.nonparty' });
			} else if (req.method === 'POST' && req.url === '/api/v1/admin/work-items/7/stage-packs') {
				res.writeHead(422);
				res.end();
			} else if (req.method === 'POST' && req.url === '/api/v1/admin/work-items/99/stage-packs') {
				res.writeHead(404);
				res.end();
			} else if (req.method === 'POST' && req.url === '/api/v1/admin/work-items/9/retry') {
				json(403, { code: 'csrf_failed', error: 'request token rejected' });
			} else if (req.url === '/api/v1/admin/work-items/6') {
				json(200, CONTRACT.detail);
			} else if (req.url === '/api/v1/admin/work-items/6/result') {
				res.writeHead(200, { 'content-type': 'application/json', 'content-disposition': 'attachment' });
				res.end('{"pages":[]}');
			} else if (req.url === '/api/v1/admin/work-items/7/result') {
				json(404, { error: 'no result yet' });
			} else {
				res.writeHead(401);
				res.end();
			}
		});
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const base = `http://127.0.0.1:${server.address().port}/api/v1`;
	try {
		const list = await fetchWorkList(base, 'done');
		assert.equal(list.items.length, 2);
		assert.equal(typeof list.offset, 'number');
		const created = await enqueueWork(base, CONTRACT.enqueue_request);
		assert.deepEqual(created, { id: 8, status: 'pending' });
		await assert.rejects(enqueueWork(base, { kind: 'nope', payload: {}, max_attempts: 3 }), (err) => {
			assert.ok(err instanceof WorkError);
			assert.equal(err.status, 422);
			assert.match(err.message, /unknown kind: nope/);
			return true;
		});
		assert.deepEqual(await retryWork(base, 5), { id: 5, status: 'pending' });
		await assert.rejects(retryWork(base, 9), /request token rejected/);
		assert.deepEqual(await cancelWork(base, 7), { id: 7, status: 'failed' });
		await assert.rejects(cancelWork(base, 6), (err) => err instanceof WorkError && err.status === 409);
		assert.deepEqual(await stagePacks(base, 9), { id: 9, dir: 'packs-inbox/9', packs: ['EG'], count: 1 });
		await assert.rejects(stagePacks(base, 6), (err) => {
			assert.ok(err instanceof WorkError);
			assert.equal(err.status, 409);
			assert.match(err.message, /scrape\.urls, not research\.nonparty/);
			return true;
		});
		await assert.rejects(stagePacks(base, 7), (err) => err instanceof WorkError && err.status === 422 && /packs\.v1/.test(err.message));
		await assert.rejects(stagePacks(base, 99), (err) => err instanceof WorkError && err.status === 404);
		const detail = await fetchWorkItem(base, 6);
		assert.equal(detail.audit.length, 3);
		assert.equal(await fetchResultText(base, 6), '{"pages":[]}');
		await assert.rejects(fetchResultText(base, 7), /no result yet/);
		await assert.rejects(fetchWorkItem(base, 99), /log in on the admin host/);
		assert.deepEqual(seen, [
			'GET /api/v1/admin/work-items?limit=50&status=done',
			'POST /api/v1/admin/work-items',
			'POST /api/v1/admin/work-items',
			'POST /api/v1/admin/work-items/5/retry',
			'POST /api/v1/admin/work-items/9/retry',
			'POST /api/v1/admin/work-items/7/cancel',
			'POST /api/v1/admin/work-items/6/cancel',
			'POST /api/v1/admin/work-items/9/stage-packs',
			'POST /api/v1/admin/work-items/6/stage-packs',
			'POST /api/v1/admin/work-items/7/stage-packs',
			'POST /api/v1/admin/work-items/99/stage-packs',
			'GET /api/v1/admin/work-items/6',
			'GET /api/v1/admin/work-items/6/result',
			'GET /api/v1/admin/work-items/7/result',
			'GET /api/v1/admin/work-items/99'
		]);
	} finally {
		server.close();
	}
});

test('a host that cannot be reached says so', async () => {
	await assert.rejects(
		fetchWorkList('http://127.0.0.1:1/api/v1', 'all'),
		(err) => err instanceof WorkError && err.status === 0 && /could not be reached/.test(err.message)
	);
	// The result link is a plain GET: cookie auth, no CSRF needed.
	assert.equal(resultHref('', 1), '/admin/work-items/1/result');
});

test('contract: enrolment keys are present and the token routes are gone', () => {
	for (const k of ['enrolments', 'approve_request', 'approve_response', 'deny_response', 'revoke_response']) {
		assert.ok(CONTRACT[k], k);
	}
	assert.equal(CONTRACT.token_request, undefined);
	assert.equal(CONTRACT.token_response, undefined);
	for (const w of CONTRACT.list.workers) assert.equal('identity' in w, false);
});

test('normalizeEnrolments reads the contract sample', () => {
	const out = normalizeEnrolments(CONTRACT.enrolments);
	assert.ok(out);
	assert.equal(out.today, '2026-11-04');
	assert.equal(out.server_version, '0.1.0+1bda381');
	assert.equal(out.enrolment_on, true);
	assert.equal(out.offset, 0);
	assert.deepEqual(out.pending, CONTRACT.enrolments.pending);
	assert.deepEqual(out.credentials, CONTRACT.enrolments.credentials);
});

test('normalizeEnrolments drops junk and defaults', () => {
	assert.equal(normalizeEnrolments(null), null);
	assert.equal(normalizeEnrolments({ today: '2026-11-04' }), null);
	const out = normalizeEnrolments({
		today: 'yesterday',
		enrolment_on: false,
		pending: [null, { id: '' }, { id: 'ab', username: 'x', hostname: 'y', capabilities: ['a', 3] }],
		credentials: [{ id: 'c1' }, { id: 'c2', name: 'x@y', connected: 'yes' }]
	});
	assert.ok(out);
	assert.equal(out.today, null);
	assert.equal(out.enrolment_on, false);
	assert.equal(out.pending.length, 1);
	assert.equal(out.pending[0].proposed_name, 'x@y');
	assert.deepEqual(out.pending[0].capabilities, ['a']);
	assert.equal(out.pending[0].version_matches, null);
	assert.equal(out.credentials.length, 1);
	assert.equal(out.credentials[0].connected, false);
	assert.equal(out.credentials[0].revoked_at, null);
	// enrolment_on defaults to on when the server leaves it out.
	assert.equal(normalizeEnrolments({ pending: [] })?.enrolment_on, true);
});

test('worker names follow the rule', () => {
	for (const ok of ['chandra@plant', 'chandra@plant 2026-11-04', 'a@b', 'a.b_c-d@h.lan', `${'u'.repeat(32)}@${'h'.repeat(63)}`]) {
		assert.equal(validWorkerName(ok), true, ok);
		assert.equal(workerNameProblem(ok), null, ok);
	}
	for (const bad of [
		'',
		'plant',
		'Chandra@plant',
		'chandra@plant:1',
		'chandra@plant 2026-11-04 2026-11-04',
		'chandra@plant  2026-11-04',
		'chandra@plant 2026-1-4',
		'chandra@@plant',
		`${'u'.repeat(33)}@h`,
		`u@${'h'.repeat(64)}`,
		' chandra@plant',
		'<b>@plant',
		null
	]) {
		assert.equal(validWorkerName(bad), false, String(bad));
	}
	assert.match(workerNameProblem(''), /Enter a name/);
	assert.match(workerNameProblem('Chandra@plant'), /lower case/);
	assert.match(workerNameProblem('a@b 2026-11-04 2026-11-04'), /already ends in a date/);
	assert.match(workerNameProblem('a:b@c'), /user@host/);
});

test('finalWorkerName appends the server day only with Add date', () => {
	assert.equal(finalWorkerName('chandra@plant', false, '2026-11-04'), 'chandra@plant');
	assert.equal(finalWorkerName('chandra@plant', true, '2026-11-04'), 'chandra@plant 2026-11-04');
	assert.equal(finalWorkerName('chandra@plant', true, CONTRACT.enrolments.today), CONTRACT.approve_response.name);
	assert.equal(validWorkerName(finalWorkerName('chandra@plant', true, '2026-11-04')), true);
});

test('cleanCode upper-cases, drops spaces and adds the dash', () => {
	assert.equal(cleanCode('k7qf-2m'), 'K7QF-2M');
	assert.equal(cleanCode(' k7qf 2m '), 'K7QF-2M');
	assert.equal(cleanCode('K7QF2M'), 'K7QF-2M');
	assert.equal(cleanCode(''), '');
	assert.equal(cleanCode(undefined), '');
	assert.equal(cleanCode('abc'), 'ABC');
});

test('enrolment calls round-trip against a stub admin API', async () => {
	const seen = [];
	let tries = 3;
	const server = createServer((req, res) => {
		let body = '';
		req.on('data', (chunk) => (body += chunk));
		req.on('end', () => {
			seen.push(`${req.method} ${req.url}`);
			const json = (status, data) => {
				res.writeHead(status, { 'content-type': 'application/json' });
				res.end(JSON.stringify(data));
			};
			const P = '/api/v1/admin/workers';
			const pid = CONTRACT.enrolments.pending[0].id;
			const cid = CONTRACT.enrolments.credentials[0].id;
			if (req.method === 'GET' && req.url === `${P}/enrolments`) {
				json(200, CONTRACT.enrolments);
			} else if (req.method === 'GET' && req.url === `${P}/enrolments?revoked=1`) {
				const revoked = { ...CONTRACT.enrolments.credentials[0], id: 'f'.repeat(32), revoked_at: 1793750500, revoked_by: 'staff-2', connected: false };
				json(200, { ...CONTRACT.enrolments, credentials: [...CONTRACT.enrolments.credentials, revoked] });
			} else if (req.method === 'POST' && req.url === `${P}/enrolments/${pid}/approve`) {
				assert.ok(req.headers['x-csrf'], 'approve sends x-csrf');
				assert.equal(req.headers['content-type'], 'application/json');
				const parsed = JSON.parse(body);
				assert.deepEqual(Object.keys(parsed).sort(), ['add_date', 'code', 'name']);
				if (parsed.code === 'STEP-UP') json(403, { error: 'step-up required: confirm with your passkey', step_up: 'required' });
				else if (parsed.name === 'Bad@') json(422, { error: 'bad name' });
				else if (parsed.name === 'taken@plant') json(409, { error: 'name in use' });
				else if (parsed.code !== CONTRACT.approve_request.code) {
					tries -= 1;
					if (tries <= 0) json(409, { error: 'denied after 3 wrong codes' });
					else json(422, { error: 'wrong pairing code', tries_left: tries });
				} else {
					assert.deepEqual(parsed, CONTRACT.approve_request);
					json(200, CONTRACT.approve_response);
				}
			} else if (req.method === 'POST' && req.url === `${P}/enrolments/gone/approve`) {
				json(409, { error: 'not pending' });
			} else if (req.method === 'POST' && req.url === `${P}/enrolments/${pid}/deny`) {
				assert.ok(req.headers['x-csrf'], 'deny sends x-csrf');
				json(200, CONTRACT.deny_response);
			} else if (req.method === 'POST' && req.url === `${P}/enrolments/gone/deny`) {
				json(409, { error: 'not pending' });
			} else if (req.method === 'POST' && req.url === `${P}/credentials/${cid}/revoke`) {
				assert.ok(req.headers['x-csrf'], 'revoke sends x-csrf');
				json(200, CONTRACT.revoke_response);
			} else if (req.method === 'POST' && req.url === `${P}/credentials/stale/revoke`) {
				json(403, { error: 'step-up required', step_up: 'required' });
			} else if (req.method === 'POST' && req.url === `${P}/credentials/gone/revoke`) {
				res.writeHead(404);
				res.end();
			} else if (req.method === 'POST' && req.url === `${P}/token`) {
				res.writeHead(404);
				res.end();
			} else {
				res.writeHead(401);
				res.end();
			}
		});
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const base = `http://127.0.0.1:${server.address().port}/api/v1`;
	const pid = CONTRACT.enrolments.pending[0].id;
	const cid = CONTRACT.enrolments.credentials[0].id;
	const { code, name } = CONTRACT.approve_request;
	try {
		const list = await fetchEnrolments(base);
		assert.equal(list.pending.length, 1);
		assert.equal(list.credentials.length, 1);
		assert.equal(typeof list.offset, 'number');
		const withRevoked = await fetchEnrolments(base, { revoked: true });
		assert.equal(withRevoked.credentials.length, 2);
		assert.equal(withRevoked.credentials[1].revoked_by, 'staff-2');

		// Wrong codes: 422 with tries_left, then the 3rd denies (409).
		await assert.rejects(approveEnrolment(base, pid, { code: 'AAAA-AA', name, addDate: true }), (err) => {
			assert.ok(err instanceof WorkError);
			assert.equal(err.status, 422);
			assert.equal(err.triesLeft, 2);
			assert.equal(err.serverError, 'wrong pairing code');
			assert.equal(err.stepUp, false);
			return true;
		});
		await assert.rejects(approveEnrolment(base, pid, { code: 'AAAA-AA', name, addDate: true }), (err) => err.triesLeft === 1);
		await assert.rejects(approveEnrolment(base, pid, { code: 'AAAA-AA', name, addDate: true }), (err) => {
			assert.equal(err.status, 409);
			assert.equal(err.serverError, 'denied after 3 wrong codes');
			assert.equal(err.triesLeft, null);
			assert.match(err.message, /denied after 3 wrong codes/);
			return true;
		});
		assert.deepEqual(await approveEnrolment(base, pid, { code, name, addDate: true }), CONTRACT.approve_response);
		await assert.rejects(approveEnrolment(base, pid, { code, name: 'Bad@', addDate: false }), (err) => err.status === 422 && err.serverError === 'bad name');
		await assert.rejects(approveEnrolment(base, pid, { code, name: 'taken@plant', addDate: false }), (err) => err.status === 409 && err.serverError === 'name in use');
		await assert.rejects(approveEnrolment(base, 'gone', { code, name, addDate: false }), (err) => err.status === 409 && err.serverError === 'not pending');
		// A stale step-up is flagged so the page can confirm in place.
		await assert.rejects(approveEnrolment(base, pid, { code: 'STEP-UP', name, addDate: false }), (err) => {
			assert.equal(err.status, 403);
			assert.equal(err.stepUp, true);
			assert.match(err.message, /step-up required/);
			return true;
		});

		assert.deepEqual(await denyEnrolment(base, pid), CONTRACT.deny_response);
		await assert.rejects(denyEnrolment(base, 'gone'), (err) => err.status === 409 && /not pending/.test(err.message));

		assert.deepEqual(await revokeWorker(base, cid), CONTRACT.revoke_response);
		await assert.rejects(revokeWorker(base, 'stale'), (err) => err.status === 403 && err.stepUp === true);
		await assert.rejects(revokeWorker(base, 'gone'), (err) => err.status === 404 && /already revoked/.test(err.message));
		await assert.rejects(fetchEnrolments(`${base}/nope`), /log in on the admin host/);

		assert.deepEqual(seen, [
			'GET /api/v1/admin/workers/enrolments',
			'GET /api/v1/admin/workers/enrolments?revoked=1',
			`POST /api/v1/admin/workers/enrolments/${pid}/approve`,
			`POST /api/v1/admin/workers/enrolments/${pid}/approve`,
			`POST /api/v1/admin/workers/enrolments/${pid}/approve`,
			`POST /api/v1/admin/workers/enrolments/${pid}/approve`,
			`POST /api/v1/admin/workers/enrolments/${pid}/approve`,
			`POST /api/v1/admin/workers/enrolments/${pid}/approve`,
			'POST /api/v1/admin/workers/enrolments/gone/approve',
			`POST /api/v1/admin/workers/enrolments/${pid}/approve`,
			`POST /api/v1/admin/workers/enrolments/${pid}/deny`,
			'POST /api/v1/admin/workers/enrolments/gone/deny',
			`POST /api/v1/admin/workers/credentials/${cid}/revoke`,
			'POST /api/v1/admin/workers/credentials/stale/revoke',
			'POST /api/v1/admin/workers/credentials/gone/revoke',
			'GET /api/v1/nope/admin/workers/enrolments'
		]);
	} finally {
		server.close();
	}
});
