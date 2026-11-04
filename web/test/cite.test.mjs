import test from 'node:test';
import assert from 'node:assert/strict';
import {
	CITE_PARAM,
	CITE_STORAGE_KEY,
	canonicalUrl,
	citeAnnouncement,
	citeHref,
	citePreference,
	datesLine,
	describeSource,
	formatDay,
	formatMoment,
	formatRange,
	letterFor,
	linkHref,
	numberPage,
	numberSources,
	orderedItems,
	pagesPhrase,
	sourceCount,
	sourceKey,
	titleFromUrl,
	trustLine
} from '../src/lib/public-ui/cite.js';

const SITE = 'https://www.sos.wa.gov/corporations-charities/apostilles-program';
const FAQ = `${SITE}/apostille-frequently-asked-questions`;
const FEES = `${SITE}/fees`;
const FORM = 'https://www.sos.wa.gov/sites/default/files/apostille-request-form.pdf';
const LA = { timeZone: 'America/Los_Angeles', now: Date.parse('2026-11-04T18:00:00Z') };
const CAPTURED = '2026-10-31T16:00:00Z';
const ACCEPTED = Date.parse('2026-11-03T20:00:00Z') / 1000;

/** A web citation as the presenter builds it (contract Cite). */
function web(url, quote, over = {}) {
	return {
		source: {
			key: url,
			kind: 'web',
			title: '',
			url,
			host: null,
			capturedAt: CAPTURED,
			acceptedAt: ACCEPTED,
			channel: null,
			occurredAt: null,
			recorded: null,
			...over
		},
		quote,
		summary: null,
		factId: 1
	};
}

/** A staff evidence note citation (call, email, decision, absence). */
function note(id, kind, summary, over = {}) {
	return {
		source: {
			key: `note:${id}`,
			kind,
			title: '',
			url: null,
			host: null,
			capturedAt: null,
			acceptedAt: ACCEPTED,
			channel: null,
			occurredAt: Date.parse('2026-11-04T17:10:00Z') / 1000,
			recorded: false,
			...over
		},
		quote: null,
		summary,
		factId: null
	};
}

/** A contract Item. */
function item(id, label, display, cites, over = {}) {
	return {
		id,
		slot: null,
		label,
		display,
		kind: 'text',
		factIds: [],
		cites,
		state: 'ok',
		stale: null,
		alternatives: null,
		inputs: null,
		copy: false,
		route: null,
		instance: null,
		...over
	};
}

const FEE_QUOTE = 'The fee is $15.00 per document.';

function guide() {
	return [
		item('glance-cost', 'Cost', '$15.00 per document', [web(FEES, FEE_QUOTE)]),
		item('before-scope', 'Who issues it', 'documents issued in Washington', [
			web(FAQ, 'We authenticate documents issued in Washington.')
		]),
		item('fees-per_document', 'Fee per document', '$15.00 per document', [web(FEES, FEE_QUOTE)]),
		item('fees-expedite', 'Expedite fee', '$100.00 for each group of documents', [
			web(FEES, 'Expedited service adds $100.00 for each group of documents.')
		]),
		item('form-form', 'Request form', 'Apostille Authentication Request Form', [
			web(FORM, 'Apostille Authentication Request Form', { kind: 'pdf' })
		])
	];
}

test('letters run a…z, then aa, ab (bijective base 26)', () => {
	assert.deepEqual([0, 1, 25, 26, 27, 51, 52, 701, 702].map(letterFor), [
		'a',
		'b',
		'z',
		'aa',
		'ab',
		'az',
		'ba',
		'zz',
		'aaa'
	]);
});

test('numbers follow first appearance, one per source, and are stable', () => {
	const { sources, refs } = numberSources(guide());
	assert.deepEqual(
		sources.map((s) => [s.n, s.key]),
		[
			[1, FEES],
			[2, FAQ],
			[3, FORM]
		]
	);
	assert.deepEqual(refs['glance-cost'], [{ n: 1, letter: 'a', refId: 'ref-1a', letters: ['a'], key: FEES }]);
	assert.deepEqual(refs['before-scope'], [{ n: 2, letter: 'a', refId: 'ref-2a', letters: ['a'], key: FAQ }]);
	// Same output for the same input, every time.
	assert.deepEqual(numberSources(guide()), numberSources(guide()));
	// A different reading order renumbers by the new first appearance.
	const reordered = numberSources([...guide()].reverse());
	assert.deepEqual(
		reordered.sources.map((s) => [s.n, s.key]),
		[
			[1, FORM],
			[2, FEES],
			[3, FAQ]
		]
	);
});

test('letters a, b, c… per distinct quote within one source', () => {
	const { sources, refs } = numberSources(guide());
	const fees = sources[0];
	assert.deepEqual(
		fees.quotes.map((q) => [q.letter, q.text]),
		[
			['a', FEE_QUOTE],
			['b', 'Expedited service adds $100.00 for each group of documents.']
		]
	);
	assert.deepEqual(refs['fees-expedite'], [{ n: 1, letter: 'b', refId: 'ref-1b', letters: ['b'], key: FEES }]);
	assert.equal(fees.quotes[1].label, 'Expedite fee');
	assert.equal(fees.quotes[1].value, '$100.00 for each group of documents');
});

test('the same quote cited by two items reuses its letter, with unique marker ids', () => {
	const { sources, refs } = numberSources(guide());
	const [a] = sources[0].quotes;
	assert.equal(refs['glance-cost'][0].letter, 'a');
	assert.equal(refs['fees-per_document'][0].letter, 'a');
	assert.equal(refs['fees-per_document'][0].refId, 'ref-1a-2');
	// The backlink goes to the first marker; every use is listed.
	assert.equal(a.refId, 'ref-1a');
	assert.equal(a.itemId, 'glance-cost');
	assert.deepEqual(
		a.uses.map((u) => u.refId),
		['ref-1a', 'ref-1a-2']
	);
	// Whitespace differences are the same words.
	const spaced = numberSources([
		item('x', 'X', 'v', [web(FEES, 'The fee is  $15.00\nper document.')]),
		item('y', 'Y', 'v', [web(FEES, FEE_QUOTE)])
	]);
	assert.equal(spaced.sources[0].quotes.length, 1);
	assert.equal(spaced.refs.y[0].letter, 'a');
});

test('an item with several sources gets one group, sorted by number', () => {
	const items = [
		item('a', 'Scope', 'v', [web(FAQ, 'q1')]),
		item('b', 'Merged row', 'v', [web(FEES, 'q2'), web(FAQ, 'q3')]),
		item('c', 'Same page twice', 'v', [web(FAQ, 'q4'), web(FAQ, 'q1')])
	];
	const { refs, sources } = numberSources(items);
	assert.deepEqual(
		refs.b.map((r) => r.n),
		[1, 2]
	);
	assert.equal(refs.b[0].refId, 'ref-1b');
	// Two quotes from one source on one item: one marker, both letters.
	assert.deepEqual(refs.c, [{ n: 1, letter: 'c', refId: 'ref-1c', letters: ['c', 'a'], key: FAQ }]);
	assert.deepEqual(
		sources[0].quotes.map((q) => q.letter),
		['a', 'b', 'c']
	);
	const ids = Object.values(refs).flat().map((r) => r.refId);
	assert.equal(new Set(ids).size, ids.length);
});

test('a page URL with a fragment is the same source; endnote URLs carry no fragment', () => {
	const items = [
		item('a', 'A', 'v', [web(`${FEES}#:~:text=The%20fee`, FEE_QUOTE)]),
		item('b', 'B', 'v', [web(`${FEES}#top`, 'Another sentence.')])
	];
	const { sources } = numberSources(items);
	assert.equal(sources.length, 1);
	assert.equal(sources[0].key, FEES);
	assert.equal(sources[0].url, FEES);
	assert.equal(canonicalUrl(`${FEES}#:~:text=x`), FEES);
	assert.equal(sourceKey({ key: 'note:9' }), 'note:9');
	assert.equal(sourceKey({ key: '', url: `${FAQ}#x` }), FAQ);
	assert.equal(sourceKey({ key: '', url: 'not a url' }), null);
});

test('only http(s) addresses survive; only https ones are linked', () => {
	assert.equal(canonicalUrl('javascript:alert(1)'), null);
	assert.equal(canonicalUrl('data:text/html,<b>x</b>'), null);
	assert.equal(canonicalUrl('file:///etc/passwd'), null);
	assert.equal(canonicalUrl('http://www.sos.wa.gov/fees#x'), 'http://www.sos.wa.gov/fees');
	assert.equal(linkHref(`${FEES}#x`), FEES);
	assert.equal(linkHref('http://www.sos.wa.gov/fees'), null);
	assert.equal(linkHref('javascript:alert(1)'), null);
	const { sources } = numberSources([
		item('a', 'A', 'v', [web('javascript:alert(1)', 'q')]),
		item('b', 'B', 'v', [web('https://www.sos.wa.gov/x', 'q', { url: 'data:text/html,hi' })]),
		item('c', 'C', 'v', [web('http://www.sos.wa.gov/old', 'q')])
	]);
	assert.equal(sources[0].url, null);
	assert.equal(describeSource(sources[0], LA).url, null);
	assert.equal(describeSource(sources[0], LA).href, null);
	assert.equal(describeSource(sources[0], LA).link, null);
	// A bad url beside an https key falls back to the key.
	assert.equal(sources[1].url, 'https://www.sos.wa.gov/x');
	assert.equal(describeSource(sources[1], LA).href, 'https://www.sos.wa.gov/x');
	// http prints as text: a url, but no link.
	const old = describeSource(sources[2], LA);
	assert.equal(old.url, 'http://www.sos.wa.gov/old');
	assert.equal(old.href, null);
	assert.equal(old.link, null);
});

test('evidence notes number like any other source; summaries are never quotes', () => {
	const call = note(7, 'call', 'The desk confirmed the box on the division page.', {
		channel: 'Apostille desk, the published line (option three)'
	});
	const items = [
		item('a', 'Mailing address', 'v', [web(FAQ, 'q1'), call]),
		item('b', 'Mailing address again', 'v', [call])
	];
	const { sources, refs } = numberSources(items);
	assert.deepEqual(
		sources.map((s) => [s.n, s.key, s.kind]),
		[
			[1, FAQ, 'web'],
			[2, 'note:7', 'call']
		]
	);
	const [q] = sources[1].quotes;
	assert.equal(q.text, null);
	assert.equal(q.summary, 'The desk confirmed the box on the division page.');
	assert.equal(q.letter, 'a');
	assert.equal(refs.b[0].letter, 'a');
	assert.equal(sources[1].recorded, false);
});

test('a staff decision lists the sources it rests on', () => {
	const call = note(7, 'call', 'The desk named the current box.');
	const decision = note(8, 'decision', 'Use the box the desk confirmed.');
	const items = [
		item('a', 'Mailing address', 'v', [web(FAQ, 'q1')]),
		item('b', 'Our recommendation', 'v', [web(FEES, 'q2'), call, decision]),
		item('c', 'Elsewhere', 'v', [
			note(9, 'decision', 'Another call.', { supports: ['note:7', FAQ] })
		])
	];
	const { sources } = numberSources(items);
	const byKey = Object.fromEntries(sources.map((s) => [s.key, s]));
	assert.deepEqual(byKey['note:8'].basedOn, [2, 3]);
	assert.deepEqual(byKey['note:9'].basedOn, [1, 3]);
	assert.deepEqual(byKey['note:7'].basedOn, []);
});

test('a calculated value gets its own source listing each input with its number', () => {
	const [, , perDoc, expedite] = guide();
	const total = item('fees-total', 'Estimated total', '$145.00', [], {
		state: 'computed',
		kind: 'money',
		inputs: [perDoc, expedite]
	});
	const items = [...guide(), total];
	const { sources, refs } = numberSources(items);
	const calc = sources.find((s) => s.kind === 'computed');
	assert.ok(calc);
	assert.equal(calc.key, 'computed:fees-total');
	assert.equal(calc.n, 4);
	assert.equal(calc.url, null);
	assert.deepEqual(refs['fees-total'], [
		{ n: 4, letter: 'a', refId: 'ref-4a', letters: ['a'], key: 'computed:fees-total' }
	]);
	const [q] = calc.quotes;
	assert.equal(q.text, null);
	assert.equal(q.summary, '$145.00');
	assert.deepEqual(q.inputs, [
		{ label: 'Fee per document', display: '$15.00 per document', refs: [{ n: 1, letter: 'a' }] },
		{
			label: 'Expedite fee',
			display: '$100.00 for each group of documents',
			refs: [{ n: 1, letter: 'b' }]
		}
	]);
	// The inputs' endnotes can send the reader back to the total.
	assert.ok(sources[0].quotes[1].uses.some((u) => u.refId === 'ref-4a'));
});

test('calculations are numbered after every real source, whatever their place', () => {
	const fee = item('fee', 'Fee per document', '$15.00 per document', [web(FEES, FEE_QUOTE)]);
	const total = item('total', 'Estimated total', '$45.00', [], { state: 'computed', inputs: [fee] });
	const { sources, refs } = numberSources([total, fee]);
	assert.deepEqual(
		sources.map((s) => [s.n, s.key]),
		[
			[1, FEES],
			[2, 'computed:total']
		]
	);
	assert.deepEqual(sources[1].quotes[0].inputs[0].refs, [{ n: 1, letter: 'a' }]);
	assert.equal(refs.fee[0].n, 1);
	assert.equal(refs.total[0].refId, 'ref-2a');
});

test('a calculator switching to "ask the office" renumbers no real source', () => {
	const contact = `${SITE}/contact-us`;
	const fee = item('fee', 'Fee per document', '$15.00 per document', [web(FEES, FEE_QUOTE)]);
	const reach = item('contact', 'Phone', '360-725-0377', [web(contact, 'Call 360-725-0377.')]);
	const computed = item('total', 'Your total', 'Total to enclose: $45.00 = 3 × $15.00', [web(FEES, FEE_QUOTE)], {
		state: 'computed',
		inputs: [fee]
	});
	const ask = item('total', 'Your total', 'Ask the office', [], { state: 'missing' });
	const a = numberSources([fee, computed, reach]);
	const b = numberSources([fee, ask, reach]);
	assert.deepEqual(
		a.sources.map((s) => [s.n, s.key]),
		[
			[1, FEES],
			[2, contact],
			[3, 'computed:total']
		]
	);
	assert.deepEqual(
		b.sources.map((s) => [s.n, s.key]),
		[
			[1, FEES],
			[2, contact]
		]
	);
	assert.equal(a.refs.contact[0].refId, 'ref-2a');
	assert.equal(b.refs.contact[0].refId, 'ref-2a');
	// The total cites the fee page too: one group, page first.
	assert.deepEqual(
		a.refs.total.map((r) => r.n),
		[1, 3]
	);
	// The announcement counts the office's sources, not calculations.
	assert.equal(sourceCount(a.sources), 2);
	assert.equal(sourceCount(b.sources), 2);
	assert.equal(sourceCount(null), 0);
});

test('a calculation with terms carries each term with its input refs', () => {
	const [, , perDoc, expedite] = guide();
	const total = item('fees-total', 'Your total', 'Total to enclose: $145.00', [], {
		state: 'computed',
		inputs: [perDoc, expedite],
		terms: [
			{ text: '3 × $15.00', itemId: 'fees-per_document' },
			{ text: '+ $100.00 expedite fee (1–10 documents)', itemId: 'fees-expedite' },
			{ text: '= $145.00', itemId: null },
			{ text: '   ' },
			null
		]
	});
	const { sources } = numberSources([...guide(), total]);
	const calc = sources.find((s) => s.kind === 'computed');
	assert.deepEqual(calc.quotes[0].terms, [
		{ text: '3 × $15.00', refs: [{ n: 1, letter: 'a' }] },
		{ text: '+ $100.00 expedite fee (1–10 documents)', refs: [{ n: 1, letter: 'b' }] },
		{ text: '= $145.00', refs: [] }
	]);
	// Without terms the quote has none; the card prints the display.
	const plain = numberSources([...guide(), { ...total, terms: undefined }]);
	assert.equal(plain.sources.find((s) => s.kind === 'computed').quotes[0].terms, null);
});

test('conflict alternatives are numbered after their parent, each with its own markers', () => {
	const contact = `${SITE}/contact-us`;
	const parent = item('mail', 'Mailing address', 'The office lists different boxes', [], {
		state: 'conflict',
		alternatives: [
			item('mail-alt-1', 'Mailing address', 'PO Box 1111', [web(contact, 'PO Box 1111')]),
			item('mail-alt-2', 'Mailing address', 'PO Box 2222', [web(FAQ, 'PO Box 2222')])
		]
	});
	const { sources, refs } = numberSources([parent]);
	assert.deepEqual(refs.mail, []);
	assert.deepEqual(
		sources.map((s) => s.key),
		[contact, FAQ]
	);
	assert.equal(refs['mail-alt-2'][0].n, 2);
});

test('items, cites and sources that cannot be numbered are skipped', () => {
	const { sources, refs } = numberSources([
		null,
		{ id: '' },
		item('ok', 'A', 'v', [null, { source: { key: '', url: '' } }, web(FAQ, 'q')]),
		item('ok', 'Duplicate id', 'v', [web(FEES, 'q2')])
	]);
	assert.deepEqual(
		sources.map((s) => s.key),
		[FAQ]
	);
	assert.deepEqual(Object.keys(refs), ['ok']);
	assert.deepEqual(numberSources(undefined), { sources: [], refs: {} });
});

test('a presented page numbers the status band, its canonical order, then whatever it leaves out', () => {
	const [cost, scope, perDoc, expedite, form] = guide();
	const notice = item('status-notice', 'Service notice', 'Closed today', [
		web(`${SITE}/notices`, 'The office is closed today.')
	]);
	const page = {
		status: { active: [notice], resolved: [], lastRead: null },
		glance: [cost],
		sections: [
			{ id: 'before', step: 1, heading: 'h', items: [scope], also: [] },
			{ id: 'fees', step: 4, heading: 'h', items: [perDoc, expedite], also: [form] }
		],
		offices: [],
		closures: [],
		updates: [],
		order: ['glance-cost', 'before-scope', 'fees-per_document', 'fees-expedite', 'missing-id']
	};
	// The notice renders under the h1, above the glance: it is [1].
	assert.deepEqual(
		orderedItems(page).map((it) => it.id),
		['status-notice', 'glance-cost', 'before-scope', 'fees-per_document', 'fees-expedite', 'form-form']
	);
	const { sources } = numberPage(page);
	assert.deepEqual(
		sources.map((s) => s.key),
		[`${SITE}/notices`, FEES, FAQ, FORM]
	);
	// A status item the order places itself keeps that place.
	const placed = { ...page, order: ['glance-cost', 'status-notice', 'before-scope'] };
	assert.deepEqual(
		orderedItems(placed).map((it) => it.id),
		['glance-cost', 'status-notice', 'before-scope', 'fees-per_document', 'fees-expedite', 'form-form']
	);
	assert.deepEqual(orderedItems(null), []);
	assert.deepEqual(orderedItems({ order: ['x'] }), []);
});

test('every source carries the contract fields', () => {
	const { sources } = numberSources(guide());
	for (const s of sources) {
		for (const k of [
			'n',
			'key',
			'kind',
			'title',
			'url',
			'host',
			'capturedAt',
			'acceptedAt',
			'channel',
			'occurredAt',
			'recorded',
			'quotes'
		]) {
			assert.ok(k in s, `${s.key} lacks ${k}`);
		}
		for (const q of s.quotes) {
			for (const k of ['letter', 'text', 'summary', 'label', 'itemId', 'refId']) {
				assert.ok(k in q, `${s.key} quote lacks ${k}`);
			}
		}
	}
	assert.equal(sources[0].host, 'sos.wa.gov');
	assert.equal(sources[0].title, 'Fees');
	assert.equal(sources[1].title, 'Apostille frequently asked questions');
});

test('a source cited from several captures keeps the date range', () => {
	const later = web(FEES, 'Second quote.', {
		capturedAt: '2026-11-02T16:00:00Z',
		acceptedAt: ACCEPTED + 86400
	});
	const { sources } = numberSources([
		item('a', 'A', 'v', [web(FEES, FEE_QUOTE)]),
		item('b', 'B', 'v', [later])
	]);
	const [s] = sources;
	assert.equal(s.capturedMin, CAPTURED);
	assert.equal(s.capturedAt, '2026-11-02T16:00:00Z');
	assert.equal(s.acceptedMin, ACCEPTED);
	assert.equal(s.acceptedAt, ACCEPTED + 86400);
	assert.equal(s.quotes[1].capturedAt, '2026-11-02T16:00:00Z');
	assert.equal(datesLine(s, LA), 'Captured Oct 31 – Nov 2 · approved Nov 3 – 4');
	assert.equal(datesLine(s.quotes[0], LA), 'Captured Oct 31 · approved Nov 3');
});

test('dates read in the reader zone, the year only outside this year', () => {
	assert.equal(formatDay(CAPTURED, LA), 'Oct 31');
	assert.equal(formatDay(ACCEPTED, LA), 'Nov 3');
	assert.equal(formatDay('2025-10-31T16:00:00Z', LA), 'Oct 31, 2025');
	assert.equal(formatDay(CAPTURED, { ...LA, year: 'always' }), 'Oct 31, 2026');
	assert.equal(formatDay(null, LA), '');
	assert.equal(formatDay('not a date', LA), '');
	assert.equal(formatRange(CAPTURED, '2026-11-04T16:00:00Z', LA), 'Oct 31 – Nov 4');
	assert.equal(
		formatRange('2026-11-04T16:00:00Z', CAPTURED, { ...LA, year: 'always' }),
		'Oct 31 – Nov 4, 2026'
	);
	assert.equal(formatRange(CAPTURED, CAPTURED, LA), 'Oct 31');
	assert.equal(formatRange(null, CAPTURED, LA), 'Oct 31');
	assert.equal(
		formatMoment(Date.parse('2026-11-04T17:10:00Z') / 1000, LA),
		'Nov 4, 9:10 a.m. PT'
	);
	assert.equal(
		formatMoment(Date.parse('2026-11-04T22:05:00Z') / 1000, LA),
		'Nov 4, 2:05 p.m. PT'
	);
	assert.equal(datesLine({ capturedAt: null, acceptedAt: ACCEPTED }, LA), 'Approved Nov 3');
	assert.equal(datesLine({ capturedAt: null, acceptedAt: null }, LA), '');
});

test('the trust line counts pages and calls; empty without data', () => {
	const trust = {
		capturedMin: CAPTURED,
		capturedMax: '2026-11-04T16:00:00Z',
		acceptedMax: ACCEPTED + 86400,
		pages: 8,
		calls: 1,
		emails: 0
	};
	assert.equal(
		trustLine(trust, LA),
		'Sources captured Oct 31 – Nov 4 · approved by staff Nov 4 · 8 official pages, 1 phone call'
	);
	assert.equal(
		trustLine({ ...trust, capturedMin: null, capturedMax: null, pages: 1, calls: 0, emails: 2 }, LA),
		'Approved by staff Nov 4 · 1 official page, 2 emails'
	);
	assert.equal(trustLine(null, LA), '');
	// Pages off the listed official site are never called official.
	assert.equal(
		trustLine({ ...trust, pages: 8, otherPages: 1, calls: 0 }, LA),
		'Sources captured Oct 31 – Nov 4 · approved by staff Nov 4 · 8 official pages, 1 other web page'
	);
	assert.equal(trustLine({ ...trust, pages: 0, otherPages: 2, calls: 0 }, LA), 'Sources captured Oct 31 – Nov 4 · approved by staff Nov 4 · 2 web pages');
	assert.equal(pagesPhrase({ pages: 0, otherPages: 0 }), '');
	assert.equal(pagesPhrase({ pages: 3 }), '3 official pages');
});

test('a note that contradicts its value reads as contesting it in the card and the endnote', () => {
	const call = note(7, 'call', 'The desk said $20.00.', { channel: 'Apostille desk', stance: 'contradicts' });
	const { sources } = numberSources([item('a', 'Fee per document', '$15.00 per document', [web(FAQ, 'Fee is $15.00 per document.'), call])]);
	const src = sources.find((s) => s.kind === 'call');
	assert.equal(src?.stance, 'contradicts');
	const info = describeSource(/** @type {any} */ (src), LA);
	assert.match(info.lead, /^Phone call · Apostille desk · .* · staff notes, not recorded · contradicts the value shown$/);
	assert.match(info.head, /contradicts the value shown\.$/);
	// A confirming note says nothing of the kind.
	const ok = numberSources([item('a', 'Fee', '$15.00', [{ ...call, source: { ...call.source, stance: 'confirms' } }])]).sources[0];
	assert.doesNotMatch(describeSource(ok, LA).lead, /contradicts/);
});

test('card and endnote copy per evidence kind', () => {
	const call = note(7, 'call', 'summary', { channel: 'Apostille desk' });
	const items = [
		item('a', 'A', 'v', [web(FAQ, 'q')]),
		item('b', 'B', 'v', [web(FORM, 'Form title', { kind: 'pdf', title: 'Apostille request form' })]),
		item('c', 'C', 'v', [web(FORM, 'Form', { kind: 'upload', key: `${FORM}?v=2`, url: `${FORM}?v=2` })]),
		item('d', 'D', 'v', [call]),
		item('e', 'E', 'v', [note(8, 'email', 's', { channel: 'apostilles@sos.wa.gov' })]),
		item('f', 'F', 'v', [note(9, 'decision', 's')]),
		item('g', 'G', 'v', [note(10, 'absence', 's', { channel: 'resource page, FAQ' })]),
		item('h', 'H', '$45.00', [], { state: 'computed', inputs: [] })
	];
	const { sources } = numberSources(items);
	const d = sources.map((s) => describeSource(s, LA));

	assert.equal(d[0].lead, 'Official web page · sos.wa.gov');
	assert.equal(d[0].head, 'Apostille frequently asked questions — sos.wa.gov.');
	assert.equal(d[0].title, 'Apostille frequently asked questions');
	assert.equal(d[0].url, FAQ);
	assert.equal(d[0].dates, 'Captured Oct 31 · approved Nov 3');
	assert.equal(d[0].link, 'Open the page');
	assert.equal(d[0].fragment, true);

	assert.equal(d[1].lead, 'Official PDF · Apostille request form · sos.wa.gov');
	assert.equal(d[1].head, 'Apostille request form — official PDF, sos.wa.gov.');
	assert.equal(d[1].title, null, 'the title is in the lead');
	assert.equal(d[1].link, 'Open PDF');
	assert.equal(d[1].href, FORM);
	assert.equal(d[1].fragment, false);

	assert.equal(d[2].lead, 'Official PDF · Apostille request form · uploaded by staff from sos.wa.gov');
	assert.equal(d[2].title, null);
	assert.equal(d[2].link, 'Open PDF');
	assert.equal(d[2].fragment, false);

	assert.equal(
		d[3].lead,
		'Phone call · Apostille desk · Nov 4, 9:10 a.m. PT · staff notes, not recorded'
	);
	assert.equal(d[3].head, 'Phone call — Apostille desk. Nov 4, 9:10 a.m. PT · staff notes, not recorded.');
	assert.equal(d[3].check, 'Check it yourself on the same number.');
	assert.equal(d[3].url, null);

	assert.equal(
		d[4].lead,
		"Email from apostilles@sos.wa.gov · Nov 4 · staff note; the email isn't published"
	);
	assert.equal(d[5].lead, 'Staff note · Nov 4');
	assert.equal(d[6].lead, 'Not found on these office pages: resource page, FAQ · Nov 4 · checked by staff');
	assert.equal(d[7].lead, 'Calculated on this page');
	for (const x of d.slice(3)) {
		assert.equal(x.link, null);
		assert.equal(x.href, null);
	}
	// An absence note without its pages names none.
	const bare = numberSources([item('z', 'Z', 'v', [note(11, 'absence', 's')])]).sources[0];
	assert.equal(describeSource(bare, LA).lead, "Not found on the office's pages · Nov 4 · checked by staff");
});

test('the site root reads as the home page, never the host twice', () => {
	const root = 'https://www.sos.wa.gov/';
	const { sources } = numberSources([
		item('a', 'A', 'v', [web(root, 'q')]),
		item('b', 'B', 'v', [web(FEES, 'q', { title: 'www.SOS.wa.gov' })])
	]);
	assert.equal(sources[0].title, 'Home page');
	const home = describeSource(sources[0], LA);
	assert.equal(home.head, 'Home page — sos.wa.gov.');
	assert.equal(home.title, 'Home page');
	const echo = describeSource(sources[1], LA);
	assert.equal(echo.head, 'sos.wa.gov.');
	assert.equal(echo.title, null);
	assert.equal(echo.lead, 'Official web page · sos.wa.gov');
});

test('template copy carries no digits, $ or dates of its own', () => {
	const bare = {
		key: 'k',
		kind: 'web',
		title: '',
		url: null,
		host: null,
		capturedAt: null,
		acceptedAt: null,
		channel: null,
		occurredAt: null,
		recorded: null,
		n: 1,
		quotes: [],
		capturedMin: null,
		acceptedMin: null,
		basedOn: []
	};
	for (const kind of ['web', 'pdf', 'upload', 'call', 'email', 'visit', 'decision', 'absence', 'computed']) {
		const d = describeSource({ ...bare, kind }, LA);
		const text = [d.lead, d.head, d.title ?? '', d.dates, d.link ?? '', d.check ?? ''].join(' ');
		assert.doesNotMatch(text, /[0-9$]/, `${kind}: ${text}`);
		assert.doesNotMatch(text, /\b(dossier|slot|topic|fact)\b/i, `${kind}: ${text}`);
	}
	assert.doesNotMatch(citeAnnouncement(false, 3), /[0-9]/);
});

test('titles come from the last path segment', () => {
	assert.equal(titleFromUrl(FAQ), 'Apostille frequently asked questions');
	assert.equal(titleFromUrl(FORM), 'Apostille request form');
	assert.equal(titleFromUrl('https://www.sos.wa.gov/'), null);
	assert.equal(titleFromUrl(null), null);
});

test('state precedence: ?cite=1|0, then the stored choice, then off', () => {
	assert.equal(CITE_PARAM, 'cite');
	assert.equal(CITE_STORAGE_KEY, 'hb_cite');
	assert.equal(citePreference('?cite=1', '0'), true);
	assert.equal(citePreference('?cite=0', '1'), false);
	assert.equal(citePreference('?lang=es&cite=1', null), true);
	assert.equal(citePreference('?cite=yes', '1'), true);
	assert.equal(citePreference('', '1'), true);
	assert.equal(citePreference('', '0'), false);
	assert.equal(citePreference('', null), false);
	assert.equal(citePreference(undefined, 'garbage'), false);
});

test('the toggle URL keeps the path, other parameters and the hash', () => {
	const base = 'https://andrii.ai/apostille/authority/usa/washington';
	assert.equal(citeHref(base, true), '/apostille/authority/usa/washington?cite=1');
	assert.equal(
		citeHref(`${base}?lang=es#fees`, true),
		'/apostille/authority/usa/washington?lang=es&cite=1#fees'
	);
	assert.equal(citeHref(`${base}?cite=1`, false), '/apostille/authority/usa/washington?cite=0');
});

test('announcements', () => {
	assert.equal(citeAnnouncement(true, 9), 'Citations shown: 9 sources numbered');
	assert.equal(citeAnnouncement(true, 1), 'Citations shown: 1 source numbered');
	assert.equal(citeAnnouncement(false, 9), 'Citations hidden');
});
