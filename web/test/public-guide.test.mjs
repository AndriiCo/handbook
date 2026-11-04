import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
	GUIDE_STEPS,
	NOTICE_TOPIC,
	buildGuide,
	changePanels,
	domain,
	draftedBy,
	fieldLabel,
	markQuote,
	parseStatement,
	serviceNotices,
	sourceUrl,
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

test('markQuote keeps source line breaks as breaks', () => {
	const html = markQuote('Town Center 3\n243 Israel Rd SE', '243 Israel Rd SE');
	assert.ok(html.includes('<br>'));
	assert.ok(!html.includes('\n'));
});

test('markQuote marks the exact value span for all 30 gold cases', () => {
	const dir = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'evals', 'authorities');
	const files = readdirSync(dir)
		.filter((f) => f.endsWith('.json'))
		.sort();
	let total = 0;
	let marked = 0;
	const missed = [];
	for (const f of files) {
		const gold = JSON.parse(readFileSync(join(dir, f), 'utf8'));
		for (const c of gold.cases) {
			total++;
			const html = markQuote(c.evidence, `${c.field}: ${c.value}`);
			const span = html.match(/<mark>(.*?)<\/mark>/s)?.[1] ?? '';
			// The marked span, cleaned like the matcher, is the value.
			const clean = (s) =>
				String(s)
					.toLowerCase()
					.split(/\s+/)
					.map((w) => w.replace(/^[^a-z0-9$:/-]+|[^a-z0-9$:/-]+$/g, ''))
					.filter(Boolean)
					.join(' ');
			if (html.includes('<mark>') && clean(span) === clean(c.value)) marked++;
			else missed.push(`${f} ${c.id} [${c.value}]`);
		}
	}
	assert.equal(total, 30);
	assert.deepEqual(missed, []);
	assert.equal(marked, 30);
});

test('markQuote keeps digits and $/:/- in the span', () => {
	assert.ok(
		markQuote('The fee table shows $15 per document.', 'per_document: $15 per document').includes(
			'<mark>$15 per document</mark>'
		)
	);
	assert.ok(
		markQuote('Authentication questions go to (360) 725-0344.', 'line: (360) 725-0344').includes(
			'<mark>(360) 725-0344</mark>'
		)
	);
	assert.ok(
		markQuote('Counter hours are Mon-Fri 8-11:30 for filing.', 'weekday: Mon-Fri 8-11:30').includes(
			'<mark>Mon-Fri 8-11:30</mark>'
		)
	);
});

test('sourceUrl opens the quote with a text fragment', () => {
	const url = sourceUrl(
		'https://example.gov/office',
		'Walk-in customers go to Town Center 3, Tumwater, appointments preferred.'
	);
	assert.ok(url.startsWith('https://example.gov/office#:~:text='), url);
	assert.ok(url.includes(','), url);
	// `-` is escaped so the fragment parser does not split on it.
	const frag = url.split('#:~:text=')[1];
	assert.ok(frag.includes('%2D'), url);
	assert.ok(!frag.replace(/%2D/g, '').includes('-'), url);
	// `&` and `,` inside the quote are percent-encoded, not delimiters.
	const tricky = sourceUrl('https://example.gov/x', 'one & two, three four five six seven');
	assert.ok(tricky.includes('%26'), tricky);
	assert.ok(tricky.includes('%2C'), tricky);
});

test('sourceUrl degrades for pdf, anchors and non-https', () => {
	assert.equal(sourceUrl('https://example.gov/fees.pdf', 'some quote words here now'), 'https://example.gov/fees.pdf');
	assert.equal(
		sourceUrl('https://example.gov/fees.PDF?dl=1', 'some quote words here now'),
		'https://example.gov/fees.PDF?dl=1'
	);
	assert.equal(sourceUrl('https://example.gov/x#page', 'some quote words here now'), 'https://example.gov/x#page');
	assert.equal(sourceUrl('http://example.gov/x', 'some quote words here now'), null);
	assert.equal(sourceUrl('https://example.gov/x', '   '), null);
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

test('a fact staff quoted reads as drafted by staff, never by a user id', () => {
	assert.equal(draftedBy(fact({ drafted_by: 'staff' })), 'staff');
	assert.equal(draftedBy(fact({ drafted_by: undefined, created_by: 'staff:u-1' })), 'staff');
	assert.equal(buildGuide([fact({ drafted_by: 'staff:u-1' })], NOW)[0].items[0].draftedBy, 'staff');
	assert.equal(draftedBy(fact({ drafted_by: 'staffing-model-2' })), 'staffing-model-2');
	assert.equal(draftedBy({}), 'unknown');
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
	// Service notices are the dossier banner, never a guide step.
	assert.ok(!covered.has(NOTICE_TOPIC));
});

const NOTICE_QUOTE =
	'Notice posted 11/02/2026: Apostille services are unavailable until further notice while we upgrade our systems.';

function notice(over) {
	return fact({
		topic: 'service_notice',
		statement: `notice: ${NOTICE_QUOTE}`,
		citations: [
			{
				quote: NOTICE_QUOTE,
				source_url: 'https://www.sos.wa.gov/corporations-charities/apostilles',
				captured_at: '2026-11-02T00:00:00Z'
			}
		],
		...over
	});
}

test('service notices skip the guide and become banner rows, newest first', () => {
	const facts = [
		fact({ id: 1 }),
		notice({ id: 2 }),
		notice({
			id: 3,
			statement: 'notice: Counter closed 10/30',
			citations: [
				{
					quote: 'Counter closed 10/30 for a power outage.',
					source_url: 'https://www.sos.wa.gov/',
					captured_at: '2026-10-30T08:00:00Z'
				}
			]
		}),
		notice({ id: 4, status: 'proposed' }),
		notice({ id: 5, citations: [] })
	];
	const steps = buildGuide(facts, NOW);
	assert.deepEqual(
		steps.flatMap((s) => s.items.flatMap((i) => i.factIds)),
		[1],
		'notices never enter a step'
	);
	const rows = serviceNotices(facts);
	assert.deepEqual(
		rows.map((r) => r.id),
		[2, 3],
		'accepted + cited only, newest capture first'
	);
	assert.equal(rows[0].text, NOTICE_QUOTE);
	assert.equal(rows[0].quote, NOTICE_QUOTE);
	assert.equal(rows[0].capturedAt, '2026-11-02T00:00:00Z');
	assert.match(rows[0].sourceUrl, /^https:\/\/www\.sos\.wa\.gov\//);
	assert.deepEqual(serviceNotices(null), []);
});

test('changePanels keeps real changes with both dates and drops junk', () => {
	const panels = changePanels({
		changes: [
			{
				source_url: 'https://www.sos.wa.gov/',
				captured_at: '2026-10-20T00:00:00Z',
				previous_captured_at: '2026-10-19T00:00:00Z',
				added: [],
				removed: ['Old line'],
				more_added: 0,
				more_removed: 0
			},
			{
				source_url: 'https://www.sos.wa.gov/corporations-charities/apostilles',
				captured_at: '2026-11-02T00:00:00Z',
				previous_captured_at: '2026-11-01T00:00:00Z',
				added: [NOTICE_QUOTE],
				removed: [],
				more_added: 2,
				more_removed: 0
			},
			{ source_url: 'http://insecure.example/', captured_at: '2026-11-02T00:00:00Z', added: ['x'] },
			{ source_url: 'https://www.sos.wa.gov/x', captured_at: '2026-11-02T00:00:00Z' },
			null
		]
	});
	assert.equal(panels.length, 2);
	assert.equal(panels[0].date, '2026-11-02');
	assert.equal(panels[0].since, '2026-11-01');
	assert.deepEqual(panels[0].added, [NOTICE_QUOTE]);
	assert.equal(panels[0].more, 2);
	assert.deepEqual(panels[1].removed, ['Old line']);
	assert.deepEqual(changePanels({}), []);
	assert.deepEqual(changePanels(null), []);
});
