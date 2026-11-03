import test from 'node:test';
import assert from 'node:assert/strict';
import {
	CONFIRM_WORD,
	isLockoutKey,
	requestRestart,
	restartNeeded,
	savedSinceStart,
	waitForRestart
} from '../src/lib/admin/restart-client.js';
import { fetchConfigWithStart, normalizeRow } from '../src/lib/admin/config-client.js';

/** A fetch stub answering from a queue of [status, body] pairs (or thrown errors). */
function stubFetch(answers) {
	const calls = [];
	const fn = async (url, init = {}) => {
		calls.push({ url, init });
		const next = answers.length > 1 ? answers.shift() : answers[0];
		if (next instanceof Error) throw next;
		const [status, body] = next;
		return new Response(JSON.stringify(body), {
			status,
			headers: { 'content-type': 'application/json' }
		});
	};
	return { fn, calls };
}

test('lockout keys are the net/tls/admin/auth/acme families', () => {
	const lockout = ['net.bind', 'tls.cert_file', 'admin.allow_ips', 'auth.origin', 'acme.enabled'];
	for (const key of lockout) {
		assert.ok(isLockoutKey(key), key);
	}
	for (const key of ['labels.fedex_account', 'labels.live', 'network.x', 'stripe.secret_key']) {
		assert.ok(!isLockoutKey(key), key);
	}
	assert.equal(CONFIRM_WORD, 'restart');
});

test('restart needed only for boot keys saved after the start', () => {
	assert.equal(restartNeeded('boot', 200, 100), true);
	assert.equal(restartNeeded('boot', 100, 100), false);
	assert.equal(restartNeeded('boot', 50, 100), false);
	assert.equal(restartNeeded('live', 200, 100), false);
	assert.equal(restartNeeded('boot', undefined, 100), false);
	assert.equal(restartNeeded('boot', 200, null), false);
	assert.equal(savedSinceStart(200, 100), true);
});

test('config list carries updated_at per row and started_at', async () => {
	const { fn } = stubFetch([
		[
			200,
			{
				keys: [
					{ key: 'net.bind', type: 'text', value: '10.0.0.2', updated_at: 1_900 },
					{ key: 'labels.fedex_client_id', type: 'secret', set: true, updated_at: 1_950 },
					{ key: 'old.row', type: 'bool', value: true }
				],
				started_at: 1_800
			}
		]
	]);
	const { rows, startedAt } = await fetchConfigWithStart('/api', fn);
	assert.equal(startedAt, 1_800);
	assert.deepEqual(
		rows.map((r) => [r.key, r.updated_at]),
		[
			['net.bind', 1_900],
			['labels.fedex_client_id', 1_950],
			['old.row', undefined]
		]
	);
	// Secrets stay value-free with the stamp added.
	assert.ok(!('value' in rows[1]));
	const badStamp = normalizeRow({ key: 'a', type: 'text', value: 'v', updated_at: 'x' });
	assert.equal(badStamp?.updated_at, undefined);
});

test('requestRestart maps every server answer', async () => {
	const accepted = stubFetch([
		[202, { started_at: 1_800, pending_keys: ['net.bind'], warnings: [] }]
	]);
	const csrf = () => ({ 'x-csrf': 't' });
	assert.deepEqual(await requestRestart('/api', ' restart ', accepted.fn, csrf), {
		kind: 'accepted',
		startedAt: 1_800,
		pendingKeys: ['net.bind'],
		warnings: []
	});
	const sent = accepted.calls[0];
	assert.equal(sent.url, '/api/admin/restart');
	assert.equal(sent.init.method, 'POST');
	assert.equal(sent.init.headers['x-csrf'], 't');
	assert.deepEqual(JSON.parse(sent.init.body), { confirm: 'restart' });

	const empty = stubFetch([[202, { started_at: 1 }]]);
	await requestRestart('/api', '  ', empty.fn);
	assert.deepEqual(JSON.parse(empty.calls[0].init.body), {});

	const reason = 'set both tls.cert_file and tls.key_file, or neither';
	const refused = stubFetch([[422, { error: 'restart refused: …', reason }]]);
	assert.deepEqual(await requestRestart('/api', '', refused.fn), {
		kind: 'refused',
		message: reason
	});

	const confirm = stubFetch([
		[
			428,
			{
				error: 'type it',
				confirm_required: true,
				confirm_word: 'restart',
				risky_keys: ['admin.allow_ips']
			}
		]
	]);
	assert.deepEqual(await requestRestart('/api', '', confirm.fn), {
		kind: 'confirm',
		message: 'type it',
		word: 'restart',
		riskyKeys: ['admin.allow_ips']
	});

	const wait = stubFetch([[429, { error: 'try again in 90 s', retry_after: 90 }]]);
	assert.deepEqual(await requestRestart('/api', '', wait.fn), {
		kind: 'wait',
		message: 'try again in 90 s',
		seconds: 90
	});

	await assert.rejects(requestRestart('/api', '', stubFetch([[401, {}]]).fn), /log in/);
	const broken = stubFetch([[500, { error: 'boom' }]]);
	await assert.rejects(requestRestart('/api', '', broken.fn), /boom/);
});

test('waitForRestart polls through the outage until started_at moves', async () => {
	let clock = 0;
	const sleeps = [];
	const ticks = [];
	const { fn, calls } = stubFetch([
		[200, { started_at: 100 }],
		new TypeError('fetch failed'),
		[502, { error: 'bad gateway' }],
		[200, { started_at: 160 }]
	]);
	const next = await waitForRestart('/api', 100, {
		fetchFn: fn,
		sleep: async (ms) => {
			sleeps.push(ms);
			clock += ms;
		},
		now: () => clock,
		onTick: (ms) => ticks.push(ms)
	});
	assert.equal(next, 160);
	assert.equal(calls.length, 4);
	assert.ok(calls.every((c) => c.url === '/api/admin/version'));
	assert.deepEqual(sleeps, [1000, 1000, 1000, 1000], 'once a second');
	assert.deepEqual(ticks, [1000, 2000, 3000, 4000]);
});

test('waitForRestart gives up after the timeout', async () => {
	let clock = 0;
	const { fn } = stubFetch([[200, { started_at: 100 }]]);
	await assert.rejects(
		waitForRestart('/api', 100, {
			fetchFn: fn,
			sleep: async (ms) => {
				clock += ms;
			},
			now: () => clock,
			timeoutMs: 5_000
		}),
		/not come back/
	);
	assert.equal(clock, 5_000);
});
