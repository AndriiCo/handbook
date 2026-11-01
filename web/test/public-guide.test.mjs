import test from 'node:test';
import assert from 'node:assert/strict';
import {
	GUIDE_STEPS,
	buildGuide,
	domain,
	fieldLabel,
	markQuote,
	parseStatement,
	staleMark
} from '../src/lib/public-ui/guide.js';

const DAY = 86_400_000;
const NOW = Date.parse('2026-11-01T00:00:00Z');

function fact(over) {
	return {
		id: 1,
		topic: 'fees',
		status: 'accepted',
		statement: 'per_document: $8',
		drafted_by: 'muse-spark-1.3 via analysis-agent/aa-prompt-v1',
		citations: [
			{
				quote: 'q',
				source_url: 'https://example.gov/fees',
				captured_at: '2026-10-20T09:00:00Z'
			}
		],
		...over
	};
}

test('parseStatement splits field keys, passes the rest through', () => {
	assert.deepEqual(parseStatement('per_document: $8 per apostille'), {
		fieldKey: 'per_document',
		value: '$8 per apostille'
	});
	assert.deepEqual(parseStatement('plain statement, no key'), {
		fieldKey: null,
		value: 'plain statement, no key'
	});
	assert.deepEqual(parseStatement('UPPER: not a key'), { fieldKey: null, value: 'UPPER: not a key' });
	assert.deepEqual(parseStatement('a:  b with spaces'), { fieldKey: 'a', value: 'b with spaces' });
});

test('fieldLabel maps known keys, humanises the rest', () => {
	assert.equal(fieldLabel('fees', 'per_document'), 'Fee per document');
	assert.equal(fieldLabel('address', 'apostille_main_branch_street'), 'Office address');
	assert.equal(fieldLabel('processing_time', 'standard'), 'Standard processing');
	assert.equal(fieldLabel('hours', 'weekday'), 'Weekday hours');
	assert.equal(fieldLabel('po_box', 'po_box'), 'PO Box');
	assert.equal(fieldLabel('payment', 'methods'), 'Accepted payment');
	assert.equal(fieldLabel('fees', 'some_new_key'), 'Some new key');
	assert.equal(fieldLabel('fees', null), 'Fees');
});

test('steps follow the fixed plan, skip empty ones', () => {
	const steps = buildGuide(
		[fact({ id: 1, topic: 'fees' }), fact({ id: 2, topic: 'processing_time', statement: 'standard: 5 days' })],
		NOW
	);
	assert.deepEqual(
		steps.map((s) => s.id),
		['cost', 'timing']
	);
	assert.equal(steps[0].items[0].label, 'Fee per document');
	assert.equal(steps[1].items[0].value, '5 days');
});

test('phone_script never renders; non-accepted facts excluded', () => {
	const steps = buildGuide(
		[
			fact({ id: 1, topic: 'phone_script', statement: 'script: say hi' }),
			fact({ id: 2, topic: 'fees', status: 'proposed' })
		],
		NOW
	);
	assert.deepEqual(steps, []);
});

test('same-field conflicts keep both values, flagged', () => {
	const steps = buildGuide(
		[
			fact({ id: 1, statement: 'per_document: $8' }),
			fact({ id: 2, statement: 'per_document: $15' }),
			fact({ id: 3, topic: 'payment', statement: 'methods: cash' })
		],
		NOW
	);
	const cost = steps.find((s) => s.id === 'cost');
	assert.equal(cost.items.length, 3);
	assert.deepEqual(
		cost.items.map((i) => [i.value, i.conflict]),
		[
			['$8', true],
			['$15', true],
			['cash', false]
		]
	);
	assert.deepEqual(cost.items[0].factIds, [1]);
});

test('unkeyed statements never conflict', () => {
	const steps = buildGuide(
		[
			fact({ id: 1, statement: 'Walk-ins welcome.' }),
			fact({ id: 2, statement: 'Walk-ins welcome.' })
		],
		NOW
	);
	assert.equal(steps.length, 1);
	assert.ok(steps[0].items.every((i) => i.conflict === false));
});

test('stale marks at 90 and 365 days', () => {
	const fresh = fact({ citations: [{ captured_at: '2026-10-20T09:00:00Z' }] });
	assert.equal(staleMark(fresh, NOW), null);
	const old = fact({
		citations: [{ captured_at: new Date(NOW - 100 * DAY).toISOString() }]
	});
	assert.deepEqual(staleMark(old, NOW), { months: 3, warning: false });
	const ancient = fact({
		citations: [{ captured_at: new Date(NOW - 400 * DAY).toISOString() }]
	});
	const mark = staleMark(ancient, NOW);
	assert.equal(mark.warning, true);
	assert.ok(mark.months >= 12);
	assert.equal(staleMark(fact({ citations: [] }), NOW), null);
});

test('markQuote highlights the shared span, escapes the rest', () => {
	const html = markQuote('Fee is eight dollars per document.', 'Eight dollars per document');
	assert.ok(html.includes('<mark>eight dollars per document</mark>'));
	const escaped = markQuote('<b>Fee</b> is eight dollars per document.', 'nothing shared here xyz');
	assert.ok(!escaped.includes('<mark>'));
	assert.ok(escaped.includes('&lt;b&gt;'));
});

test('domain shortens hosts, degrades gracefully', () => {
	assert.equal(domain('https://www.sos.wa.gov/x'), 'sos.wa.gov');
	assert.equal(domain('not a url'), 'official source');
});

test('items carry citations and the drafting model', () => {
	const steps = buildGuide([fact({ id: 7 })], NOW);
	const item = steps[0].items[0];
	assert.equal(item.citations.length, 1);
	assert.equal(item.draftedBy, 'muse-spark-1.3');
});

test('GUIDE_STEPS covers the frozen vocabulary except phone_script', () => {
	const covered = new Set(GUIDE_STEPS.flatMap((s) => s.topics));
	const frozen = [
		'hours',
		'fees',
		'forms',
		'address',
		'parking',
		'route_photos',
		'rejections',
		'contacts',
		'po_box',
		'walkin',
		'mailing',
		'appointment',
		'payment',
		'processing_time',
		'jurisdiction',
		'legalization_chain',
		'website',
		'holiday_hours',
		'id_requirements',
		'copies',
		'translation',
		'notarization',
		'authentication',
		'dropbox'
	];
	for (const t of frozen) assert.ok(covered.has(t), `step covers ${t}`);
	assert.ok(!covered.has('phone_script'));
});
