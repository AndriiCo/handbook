import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import {
	fetchSetupStatus,
	fetchSetupSteps,
	saveSetupKey,
	setupKeyHeaders
} from '../src/lib/admin/setup-client.js';

const KEY = `setup_${'a'.repeat(64)}`;

test('setupKeyHeaders gates on shape, never logs the key', () => {
	assert.deepEqual(setupKeyHeaders(KEY), { 'x-setup-key': KEY });
	assert.equal(setupKeyHeaders(`setup_${'a'.repeat(63)}`), null);
	assert.equal(setupKeyHeaders('a'.repeat(64)), null);
	assert.equal(setupKeyHeaders(''), null);
	assert.equal(setupKeyHeaders(null), null);
	assert.equal(setupKeyHeaders(undefined), null);
	assert.equal(setupKeyHeaders(42), null);
});

test('setup round-trips against a stub admin API', async () => {
	const seen = [];
	const server = createServer((req, res) => {
		let body = '';
		req.on('data', (chunk) => (body += chunk));
		req.on('end', () => {
			seen.push(`${req.method} ${req.url}`);
			res.writeHead(200, { 'content-type': 'application/json' });
			if (req.url === '/api/v1/admin/setup/status') {
				assert.ok(!('x-setup-key' in req.headers), 'status needs no key');
				res.end(JSON.stringify({ setup_required: true }));
			} else if (req.url === '/api/v1/admin/setup/steps') {
				assert.equal(req.headers['x-setup-key'], KEY);
				res.end(
					JSON.stringify({
						keys: [
							{ key: 'net.public_host', type: 'text', value: 'example.test' },
							{ key: 'acme.enabled', type: 'bool', value: true },
							{ key: 'telnyx.api_key', type: 'secret', set: true },
							{ key: 'setup.key_hash', type: 'secret', set: true },
							{ key: 'junk' }
						]
					})
				);
			} else if (
				req.method === 'POST' &&
				req.url === '/api/v1/admin/setup/keys/net.public_host'
			) {
				assert.equal(req.headers['x-setup-key'], KEY);
				const parsed = JSON.parse(body);
				assert.equal(parsed.type, 'text');
				assert.equal(parsed.value, 'example.test');
				res.end(JSON.stringify({ key: 'net.public_host', type: 'text', set: true }));
			} else {
				res.end(JSON.stringify({ ok: true }));
			}
		});
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const base = `http://127.0.0.1:${server.address().port}/api/v1`;
	try {
		const status = await fetchSetupStatus(base);
		assert.equal(status.setup_required, true);
		const rows = await fetchSetupSteps(base, KEY);
		assert.equal(rows.length, 4);
		assert.equal(rows[0].key, 'net.public_host');
		assert.equal(rows[0].value, 'example.test');
		assert.equal(rows[2].set, true);
		assert.ok(!('value' in rows[2]), 'secrets carry no value');
		const saved = await saveSetupKey(base, KEY, rows[0]);
		assert.equal(saved, true);
		assert.deepEqual(seen, [
			'GET /api/v1/admin/setup/status',
			'GET /api/v1/admin/setup/steps',
			'POST /api/v1/admin/setup/keys/net.public_host'
		]);
	} finally {
		server.close();
	}
});

test('malformed keys never leave the client; shapes are checked', async () => {
	let hits = 0;
	const server = createServer((req, res) => {
		hits += 1;
		res.writeHead(200, { 'content-type': 'application/json' });
		res.end(JSON.stringify({ setup_required: 'yes' }));
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const base = `http://127.0.0.1:${server.address().port}/api/v1`;
	try {
		await assert.rejects(fetchSetupSteps(base, 'nope'), /malformed/);
		await assert.rejects(saveSetupKey(base, 'nope', { key: 'x' }), /malformed/);
		assert.equal(hits, 0);
		await assert.rejects(fetchSetupStatus(base), /unexpected shape/);
		assert.equal(hits, 1);
	} finally {
		server.close();
	}
});

test('setup maps 401/403/429/422 from the gate', async () => {
	const codes = { status: 200, error: 'x' };
	const server = createServer((req, res) => {
		res.writeHead(codes.status, { 'content-type': 'application/json' });
		res.end(JSON.stringify({ error: codes.error }));
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const base = `http://127.0.0.1:${server.address().port}/api/v1`;
	try {
		codes.status = 401;
		await assert.rejects(fetchSetupSteps(base, KEY), /Setup key required/);
		codes.status = 403;
		codes.error = 'setup key invalid or window closed';
		await assert.rejects(fetchSetupSteps(base, KEY), /window already closed/);
		codes.error = 'admin host required';
		await assert.rejects(fetchSetupSteps(base, KEY), /admin host required/);
		codes.status = 429;
		await assert.rejects(fetchSetupSteps(base, KEY), /Too many attempts/);
		codes.status = 422;
		codes.error = 'unknown wizard key';
		await assert.rejects(
			saveSetupKey(base, KEY, { key: 'made.up', type: 'text', value: 'x' }),
			/Setup save: unknown wizard key\./
		);
	} finally {
		server.close();
	}
});
