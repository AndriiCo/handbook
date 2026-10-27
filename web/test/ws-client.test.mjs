import test from 'node:test';
import assert from 'node:assert/strict';
import { backoffMs, isValidTopic, parseEnvelope } from '../src/lib/ws-client.js';

test('isValidTopic accepts the five frozen topics, rejects the rest', () => {
	assert.equal(isValidTopic('authority:wa-tumwater'), true);
	assert.equal(isValidTopic('call:1'), true);
	assert.equal(isValidTopic('submission:abc-123'), true);
	assert.equal(isValidTopic('shipment:xyz'), true);
	assert.equal(isValidTopic('billing:9'), true);
	assert.equal(isValidTopic('order:1'), false); // P2 nouns win over `order`
	assert.equal(isValidTopic('authority:'), false);
	assert.equal(isValidTopic('CALL:1'), false);
	assert.equal(isValidTopic(''), false);
	assert.equal(isValidTopic(null), false);
});

test('backoffMs grows exponentially and caps at 30s', () => {
	const first = backoffMs(0);
	assert.ok(first >= 400 && first <= 600, `attempt 0 in [400,600], got ${first}`);
	const third = backoffMs(3);
	assert.ok(third >= 3200 && third <= 4800, `attempt 3 in [3200,4800], got ${third}`);
	for (let attempt = 10; attempt < 20; attempt += 1) {
		const wait = backoffMs(attempt);
		assert.ok(wait <= 36000, `attempt ${attempt} capped, got ${wait}`);
	}
});

test('parseEnvelope keeps valid frames, drops malformed ones', () => {
	const good = parseEnvelope(
		JSON.stringify({ topic: 'shipment:1', type: 'label_ready', seq: 2, payload: {} })
	);
	assert.equal(good?.type, 'label_ready');
	assert.equal(parseEnvelope('not json'), null);
	assert.equal(parseEnvelope(JSON.stringify({ topic: 'order:1', type: 'x' })), null);
	assert.equal(parseEnvelope(JSON.stringify({ topic: 'call:1' })), null);
	assert.equal(parseEnvelope(JSON.stringify(['call:1'])), null);
});
