import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
	NON_CONFLICTING_TOPICS,
	USPS_TOPICS,
	factsDisagree,
	sameField,
	splitStatement,
	statementsDisagree,
	valueTokens
} from '../src/lib/public-ui/compare.js';

const here = dirname(fileURLToPath(import.meta.url));
// The golden cases the Rust twin also runs (crates/web/src/conflicts.rs).
const cases = JSON.parse(readFileSync(join(here, '../../docs/contracts/conflict-cases.json'), 'utf8'));

/** @param {string} v */
const tokens = (v) => [...valueTokens(v)];

test('golden cases: the JS twin matches the shared conflict rule', () => {
	assert.ok(cases.length >= 30, 'the cases cover every rule');
	assert.equal(new Set(cases.map((c) => c.name)).size, cases.length, 'case names are unique');
	for (const c of cases) {
		const topicB = c.topic_b ?? c.topic;
		assert.equal(factsDisagree(c.topic, c.a, topicB, c.b), c.disagree, `case: ${c.name}`);
		assert.equal(factsDisagree(topicB, c.b, c.topic, c.a), c.disagree, `case (reversed): ${c.name}`);
		// One plain topic: the field rule alone decides.
		if (topicB === c.topic && !USPS_TOPICS.includes(c.topic) && !NON_CONFLICTING_TOPICS.includes(c.topic)) {
			assert.equal(statementsDisagree(c.a, c.b), c.disagree, `case (statements): ${c.name}`);
		}
		if (c.tokens_a) assert.deepEqual(tokens(splitStatement(c.a).value), c.tokens_a, `tokens_a: ${c.name}`);
		if (c.tokens_b) assert.deepEqual(tokens(splitStatement(c.b).value), c.tokens_b, `tokens_b: ${c.name}`);
	}
});

test('splitStatement keeps the guide grammar', () => {
	assert.deepEqual(splitStatement('per_document: $15'), { key: 'per_document', value: '$15' });
	assert.deepEqual(splitStatement('weekday__tumwater: Mon-Thu'), { key: 'weekday__tumwater', value: 'Mon-Thu' });
	assert.deepEqual(splitStatement('PO Box 40220'), { key: null, value: 'PO Box 40220' });
	assert.deepEqual(splitStatement('UPPER: not a key'), { key: null, value: 'UPPER: not a key' });
	assert.deepEqual(splitStatement('field:nospace'), { key: null, value: 'field:nospace' });
	assert.deepEqual(splitStatement('field:\u00a0no-break space'), { key: 'field', value: 'no-break space' });
	// JavaScript's `\s` has no NEL, so this one stays unkeyed (as in Rust).
	assert.deepEqual(splitStatement('field:\u0085x'), { key: null, value: 'field:\u0085x' });
	assert.equal(sameField('fee: $1', 'fee: $2'), true);
	assert.equal(sameField('fee: $1', 'expedite: $2'), false);
	assert.equal(sameField('Fee is $1', 'expedite: $2'), true);
});

test('valueTokens reads each kind in its canonical form', () => {
	assert.deepEqual(tokens('PO Box 40228, P.O. Box 7, Post Office Box #12'), ['box:12', 'box:40228', 'box:7']);
	assert.deepEqual(tokens('$15, $15.00, $1,000.5 and $0.50'), ['amt:100050', 'amt:1500', 'amt:50']);
	for (const phone of ['(360) 555-0100', '360.555.0100', '+1 360 555 0100', '1-360-555-0100', '3605550100']) {
		assert.deepEqual(tokens(phone), ['tel:3605550100'], phone);
	}
	// A ZIP+4 also yields its bare ZIP (the identity subset rule then
	// lets `98504` agree with `98504-0228`).
	assert.deepEqual(tokens('Olympia, WA 98504-0228; Tumwater 98501'), ['zip:98501', 'zip:98504', 'zip:98504-0228']);
	assert.deepEqual(tokens('4:00 p.m., 4 PM, 16:00'), ['time:16:00']);
	assert.deepEqual(tokens('12 noon, 12 a.m., 9:30am'), ['time:00:00', 'time:09:30', 'time:12:00']);
	// A bare 12-hour clock time reads either way; a leading zero or an
	// hour past 12 is a 24-hour time.
	assert.deepEqual(tokens('8:00-4:30, 12:15, 08:15, 16:30'), [
		'time:00:15|12:15',
		'time:04:30|16:30',
		'time:08:00|20:00',
		'time:08:15',
		'time:16:30'
	]);
	assert.deepEqual(tokens('7 to 10 business days, 2-3 working days, 1 business day'), [
		'days:1',
		'days:2-3',
		'days:7-10'
	]);
	for (const date of ['November 11, 2026', 'Nov. 11th 2026', '11 November 2026', '11/11/2026', '2026-11-11']) {
		assert.deepEqual(tokens(date), ['date:2026-11-11'], date);
	}
	assert.deepEqual(tokens('Mon.-Thu.'), ['dow:mon', 'dow:thu', 'dow:tue', 'dow:wed']);
	assert.deepEqual(tokens('Fridays'), ['dow:fri']);
	assert.deepEqual(tokens('Fri-Mon'), ['dow:fri', 'dow:mon', 'dow:sat', 'dow:sun']);
	// A box number is never re-read as a ZIP; prose yields nothing.
	assert.deepEqual(tokens('PO Box 40228'), ['box:40228']);
	assert.deepEqual(tokens('Secretary of State'), []);
});
