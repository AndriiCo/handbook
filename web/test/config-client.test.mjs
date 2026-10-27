import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { fetchConfig, normalizeRow, putBody, saveConfigKey } from '../src/lib/admin/config-client.js';

test('normalizeRow types rows and keeps secrets value-free', () => {
	assert.deepEqual(normalizeRow({ key: 'a.b', type: 'bool', value: true })?.value, true);
	assert.deepEqual(normalizeRow({ key: 'a.t', type: 'text', value: 'hi' })?.value, 'hi');
	const secret = normalizeRow({ key: 'a.s', type: 'secret', set: true });
	assert.equal(secret?.set, true);
	assert.ok(!('value' in (secret ?? {})), 'secret row must carry no value');
	assert.equal(normalizeRow({ key: 'a.s', type: 'secret', set: true, value: 'leak' })?.value, undefined);
	assert.equal(normalizeRow({ key: 'x', type: 'bool', value: 'yes' }), null);
	assert.equal(normalizeRow({ key: 'x', type: 'nope' }), null);
	assert.equal(normalizeRow(null), null);
});

test('putBody shapes each type', () => {
	assert.deepEqual(putBody({ type: 'bool', value: true }), { type: 'bool', value: true });
	assert.deepEqual(putBody({ type: 'text', value: 'v' }), { type: 'text', value: 'v' });
	assert.deepEqual(putBody({ type: 'secret', draft: 'new' }), { type: 'secret', value: 'new' });
});

test('fetch + save round-trips against a stub admin API', async () => {
	const store = new Map([
		['mailer.enabled', { key: 'mailer.enabled', type: 'bool', value: false }],
		['site.name', { key: 'site.name', type: 'text', value: 'old' }]
	]);
	const seen = [];
	const server = createServer((req, res) => {
		if (req.method === 'GET' && req.url === '/api/v1/admin/config') {
			res.writeHead(200, { 'content-type': 'application/json' });
			res.end(JSON.stringify([...store.values(), { key: 'meta.key', type: 'secret', set: false }]));
			return;
		}
		const put = req.method === 'PUT' && req.url?.startsWith('/api/v1/admin/config/');
		if (put) {
			let body = '';
			req.on('data', (chunk) => (body += chunk));
			req.on('end', () => {
				seen.push([decodeURIComponent(req.url.split('/').pop()), JSON.parse(body)]);
				res.writeHead(200, { 'content-type': 'application/json' });
				res.end(JSON.stringify({ set: true }));
			});
			return;
		}
		res.writeHead(404);
		res.end();
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	const port = server.address().port;
	try {
		const base = `http://127.0.0.1:${port}/api/v1`;
		const rows = await fetchConfig(base);
		assert.equal(rows.length, 3);
		assert.equal(rows.find((r) => r.key === 'meta.key')?.type, 'secret');

		await saveConfigKey(base, { key: 'mailer.enabled', type: 'bool', value: true });
		await saveConfigKey(base, { key: 'meta.key', type: 'secret', draft: 's3cr3t' });
		assert.deepEqual(seen, [
			['mailer.enabled', { type: 'bool', value: true }],
			['meta.key', { type: 'secret', value: 's3cr3t' }]
		]);
	} finally {
		server.close();
	}
});

test('fetchConfig maps missing endpoints to honest errors', async () => {
	const server = createServer((req, res) => {
		res.writeHead(404);
		res.end();
	});
	await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
	try {
		await assert.rejects(fetchConfig(`http://127.0.0.1:${server.address().port}/api/v1`), /404/);
	} finally {
		server.close();
	}
});
