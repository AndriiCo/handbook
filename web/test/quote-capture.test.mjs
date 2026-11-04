import test from 'node:test';
import assert from 'node:assert/strict';
import {
	MAX_QUOTE_CHARS,
	MIN_QUOTE_CHARS,
	charCount,
	chipKey,
	findAll,
	lineBreakMap,
	nextMatch,
	normalize,
	parseCaptureText,
	parseCaptures,
	parsePrepared,
	proposeOutcome,
	quoteBody,
	quoteProblem,
	quoteTopics,
	selectionQuote,
	sizeLabel,
	splitStatement,
	statementOf,
	tidy,
	trimRange,
	trimWs,
	uriPath,
	utf8Offset,
	valueInQuote
} from '../src/lib/admin/quote-capture.js';

const bytes = (s) => new TextEncoder().encode(s).length;
/** What a textarea's value holds for `s` (HTML line-break normalization). */
const asTextarea = (s) => s.replace(/\r\n/g, '\n').replace(/\r/g, '\n');

// A synthetic sidecar: CRLF and lone-CR breaks, 2-, 3- and 4-byte
// characters ahead of the span a reviewer quotes.
const SIDECAR =
	'Café — Corporations & Charities Division\r\n' +
	'Apostilles 😀 and certificates\r' +
	'Office hours: Monday through Friday,\r\n  8 a.m. to 5 p.m. (except holidays)\r\n' +
	'Fee: $15.00 per document.\n';

test('normalize matches analyze::normalize: Unicode whitespace runs, trimmed, lowercased', () => {
	assert.equal(normalize('  Town\tCenter\r\n 3,  TUMWATER '), 'town center 3, tumwater');
	// NBSP, NEL and the ideographic space are whitespace to Rust; BOM is not.
	assert.equal(normalize('PO\u00a0Box\u0085 40234\u3000Olympia'), 'po box 40234 olympia');
	assert.equal(normalize('a\ufeffb'), 'a\ufeffb');
	assert.equal(normalize(''), '');
	assert.equal(normalize(undefined), '');
	assert.equal(tidy('  Monday\n through\t\tFriday  '), 'Monday through Friday');
	assert.equal(trimWs('\u2028 Mon \u3000'), 'Mon');
});

test('valueInQuote is the server gate: case and spacing aside, never empty', () => {
	const quote = 'Walk-in customers go to Town Center 3, Tumwater.';
	assert.equal(valueInQuote('town center 3, tumwater', quote), true);
	assert.equal(valueInQuote('Town   Center\n3', quote), true);
	assert.equal(valueInQuote('$15 per document', quote), false);
	assert.equal(valueInQuote('   ', quote), false);
	assert.equal(valueInQuote('', quote), false);
});

test('splitStatement mirrors approval.rs edited_value after trimming', () => {
	assert.deepEqual(splitStatement('per_document: $8'), { key: 'per_document', value: '$8' });
	assert.deepEqual(splitStatement('weekday: Mon-Fri 8-5'), { key: 'weekday', value: 'Mon-Fri 8-5' });
	assert.deepEqual(splitStatement('a:  b with spaces'), { key: 'a', value: 'b with spaces' });
	assert.deepEqual(splitStatement('  weekday__tumwater:\tMonday to Friday \n'), {
		key: 'weekday__tumwater',
		value: 'Monday to Friday'
	});
	// Only the first colon splits; the value keeps the rest.
	assert.deepEqual(splitStatement('verify: https://example.test/check'), { key: 'verify', value: 'https://example.test/check' });
	for (const unkeyed of ['plain statement, no key', 'UPPER: not a key', 'no colon at all', 'field:nospace', 'field:   ', ': value', '9lives: x', 'two words: x', '']) {
		assert.equal(splitStatement(unkeyed), null, unkeyed);
	}
});

test('statementOf writes key: value on one line', () => {
	assert.equal(statementOf('weekday__tumwater', ' Monday\n through  Friday,\r\n8 a.m. '), 'weekday__tumwater: Monday through Friday, 8 a.m.');
	assert.equal(statementOf(' per_document ', '$15.00'), 'per_document: $15.00');
	assert.deepEqual(splitStatement(statementOf('per_document', '$15.00')), { key: 'per_document', value: '$15.00' });
});

test('charCount counts characters, not UTF-16 units', () => {
	assert.equal(charCount('Café 😀'), 6);
	assert.equal('Café 😀'.length, 7);
	assert.equal(charCount(''), 0);
});

test('utf8Offset agrees with TextEncoder on every prefix, multibyte and astral included', () => {
	const text = 'a é — € 😀 z\u{10ffff}!';
	for (let i = 0; i <= text.length; i++) {
		// A prefix that ends inside a pair encodes its high half as U+FFFD.
		assert.equal(utf8Offset(text, i), bytes(text.slice(0, i)), `index ${i}`);
	}
	assert.equal(utf8Offset(SIDECAR, SIDECAR.length), bytes(SIDECAR));
	assert.equal(utf8Offset('x\ud800y', 3), bytes('x\ud800y'));
	assert.equal(utf8Offset('abc', 99), 3);
	assert.equal(utf8Offset('abc', -1), 0);
});

test('lineBreakMap shows the text as a textarea holds it and maps indices back', () => {
	for (const text of [SIDECAR, 'no breaks at all', 'a\r\n\r\nb', '\r\r\n\n\r', 'x\ry\r\nz', '']) {
		const { shown, toOriginal } = lineBreakMap(text);
		assert.equal(shown, asTextarea(text), JSON.stringify(text));
		for (let i = 0; i <= shown.length; i++) {
			const j = toOriginal(i);
			// Every shown prefix comes from an original prefix that never splits a CRLF.
			assert.equal(asTextarea(text.slice(0, j)), shown.slice(0, i), `${JSON.stringify(text)} @${i}`);
			assert.ok(!(text[j - 1] === '\r' && text[j] === '\n'), `split pair @${i}`);
		}
	}
	// A text without CR is shown as is (no copy of a large capture).
	const plain = 'Fee: $15.\nHours: 8-5.';
	assert.equal(lineBreakMap(plain).shown, plain);
	assert.equal(lineBreakMap(plain).toOriginal(7), 7);
});

test('selectionQuote quotes the original text with UTF-8 offsets through the CRLF map', () => {
	const { shown, toOriginal } = lineBreakMap(SIDECAR);
	const wanted = 'Monday through Friday,\r\n  8 a.m. to 5 p.m.';
	const visible = asTextarea(wanted);
	const at = shown.indexOf(visible);
	assert.ok(at > 0);
	// The reviewer selects the span plus the spaces around it.
	const sel = selectionQuote(SIDECAR, toOriginal, at - 1, at + visible.length + 1);
	const from = SIDECAR.indexOf(wanted);
	assert.equal(sel.quote, wanted);
	assert.equal(sel.start, bytes(SIDECAR.slice(0, from)));
	assert.equal(sel.end, bytes(SIDECAR.slice(0, from + wanted.length)));
	assert.equal(sel.chars, [...wanted].length);
	// The server's check: the bytes [start, end) of the sidecar are the quote.
	const raw = new TextEncoder().encode(SIDECAR);
	assert.equal(new TextDecoder().decode(raw.slice(sel.start, sel.end)), sel.quote);
	// A reversed or empty selection quotes nothing.
	assert.deepEqual(selectionQuote(SIDECAR, toOriginal, at, at), { quote: '', start: sel.start, end: sel.start, chars: 0 });
	const whitespace = shown.indexOf('\n  8');
	assert.equal(selectionQuote(SIDECAR, toOriginal, whitespace, whitespace + 3).quote, '');
});

test('selectionQuote around astral characters and at the text edges', () => {
	const text = '😀 Apostille 😀';
	const { toOriginal } = lineBreakMap(text);
	// A selection boundary inside a surrogate pair grows to the whole emoji.
	const sel = selectionQuote(text, toOriginal, 1, text.length - 1);
	assert.equal(sel.quote, text);
	assert.deepEqual([sel.start, sel.end], [0, bytes(text)]);
	assert.deepEqual(trimRange(text, 5, 2), { start: 3, end: 5 });
	assert.deepEqual(trimRange('  ab  ', -4, 99), { start: 2, end: 4 });
});

test('findAll is case-insensitive and whitespace-flexible, with regex characters literal', () => {
	const { shown } = lineBreakMap(SIDECAR);
	const hits = findAll(shown, 'monday THROUGH   friday, 8 A.M.');
	assert.equal(hits.length, 1);
	assert.equal(shown.slice(hits[0].start, hits[0].end), 'Monday through Friday,\n  8 a.m.');
	assert.deepEqual(
		findAll(shown, '$15.00 per').map((h) => shown.slice(h.start, h.end)),
		['$15.00 per']
	);
	assert.equal(findAll(shown, '(except holidays)').length, 1);
	assert.equal(findAll(shown, 'a.m').length, 1);
	assert.equal(findAll(shown, 'a.m.|x').length, 0);
	assert.deepEqual(findAll(shown, '   '), []);
	assert.deepEqual(findAll(shown, ''), []);
	assert.equal(findAll('aaaa', 'a').length, 4);
	assert.equal(findAll('aaaa', 'a', 2).length, 2);
});

test('nextMatch steps forward from the selection and wraps both ways', () => {
	const matches = [
		{ start: 2, end: 5 },
		{ start: 10, end: 13 },
		{ start: 20, end: 23 }
	];
	assert.equal(nextMatch(matches, 0, 0), 0);
	assert.equal(nextMatch(matches, 2, 5), 1);
	assert.equal(nextMatch(matches, 10, 13), 2);
	assert.equal(nextMatch(matches, 20, 23), 0);
	assert.equal(nextMatch(matches, 10, 13, true), 0);
	assert.equal(nextMatch(matches, 2, 5, true), 2);
	assert.equal(nextMatch(matches, 15, 15, true), 1);
	assert.equal(nextMatch([], 0, 0), -1);
});

test('quoteProblem gives the first reason the server would refuse', () => {
	const quote = 'Office hours: Monday through Friday, 8 a.m. to 5 p.m.';
	const ok = { quote, topic: 'hours', statement: 'weekday__tumwater: Monday through Friday, 8 a.m. to 5 p.m.' };
	assert.equal(quoteProblem(ok), null);
	assert.match(quoteProblem({ ...ok, quote: '' }) ?? '', /^Select the words/);
	assert.equal(quoteProblem({ ...ok, quote: 'Mon-Fri' }), 'A quote must be 8 to 400 characters (this one is 7).');
	// The limit counts characters: seven emoji are seven.
	assert.match(quoteProblem({ ...ok, quote: '😀'.repeat(7) }) ?? '', /this one is 7\)/);
	assert.match(quoteProblem({ ...ok, quote: 'x'.repeat(MAX_QUOTE_CHARS + 1) }) ?? '', /this one is 401\)/);
	assert.equal(quoteProblem({ ...ok, quote: 'x'.repeat(MIN_QUOTE_CHARS), statement: 'k: xx' }), null);
	assert.equal(quoteProblem({ ...ok, topic: '' }), 'Pick a topic.');
	assert.match(quoteProblem({ ...ok, topic: 'phone_script' }) ?? '', /staff-only/);
	assert.match(quoteProblem({ ...ok, topic: 'membership' }) ?? '', /Unknown handbook topic/);
	assert.equal(quoteProblem({ ...ok, statement: 'Monday through Friday' }), "The statement must be 'key: value'.");
	assert.equal(quoteProblem({ ...ok, statement: statementOf('', 'Monday') }), 'The statement needs a key before the colon.');
	assert.match(quoteProblem({ ...ok, statement: 'Weekday: Monday' }) ?? '', /“Weekday” must be lowercase/);
	assert.equal(quoteProblem({ ...ok, statement: statementOf('weekday', '') }), 'The value needs at least 2 characters.');
	assert.equal(quoteProblem({ ...ok, statement: 'weekday: M' }), 'The value needs at least 2 characters.');
	assert.equal(quoteProblem({ ...ok, statement: 'weekday:Monday' }), 'Put a space between the colon and the value.');
	assert.match(quoteProblem({ ...ok, statement: 'weekday: Saturday' }) ?? '', /not in the quote/);
	assert.match(quoteProblem({ ...ok, statement: `weekday: ${'Monday '.repeat(300)}` }) ?? '', /at most 2000/);
	// Case and spacing aside, as on the server.
	assert.equal(quoteProblem({ ...ok, statement: 'weekday: MONDAY   through friday' }), null);
	// A prepared quote is placed (and measured) by the server.
	assert.equal(quoteProblem({ ...ok, quote: 'Mon-Fri 8-5', statement: 'weekday: Mon-Fri', located: true }), null);
	assert.equal(quoteProblem({ ...ok, quote: '  ', located: true }), 'The quote is empty.');
});

test('quoteBody sends offsets as a pair, or none for a prepared quote', () => {
	const input = {
		quote: 'Monday through Friday, 8 a.m.',
		start: 120,
		end: 149,
		topic: 'hours',
		statement: '  weekday__tumwater: Monday through Friday \n'
	};
	assert.deepEqual(quoteBody(input), {
		body: {
			quote: 'Monday through Friday, 8 a.m.',
			start: 120,
			end: 149,
			topic: 'hours',
			statement: 'weekday__tumwater: Monday through Friday'
		}
	});
	const { start, end, ...prepared } = input;
	assert.deepEqual(quoteBody(prepared), {
		body: { quote: 'Monday through Friday, 8 a.m.', topic: 'hours', statement: 'weekday__tumwater: Monday through Friday' }
	});
	assert.ok(start && end);
	for (const bad of [{ start: 120 }, { end: 149 }, { start: 149, end: 120 }, { start: -1, end: 4 }, { start: 1.5, end: 4 }]) {
		assert.match(/** @type {any} */ (quoteBody({ ...prepared, ...bad })).error, /select it again/, JSON.stringify(bad));
	}
	assert.deepEqual(quoteBody({ ...input, topic: 'phone_script' }), {
		error: 'phone_script stays staff-only: quote into another topic.'
	});
});

test('proposeOutcome reads each answer of the quote route', () => {
	assert.deepEqual(proposeOutcome(201, { fact_id: 512, status: 'proposed' }), { kind: 'proposed', text: 'Proposed #512.', factId: 512 });
	assert.deepEqual(
		proposeOutcome(409, { error: 'this statement is already on file', fact_id: 88, status: 'accepted' }),
		{ kind: 'duplicate', text: 'Already on file as #88 (accepted).', factId: 88 }
	);
	assert.deepEqual(
		proposeOutcome(409, { error: 'quote appears 3 times in this capture; select the one you mean in the text', count: 3 }),
		{ kind: 'ambiguous', text: 'Appears 3 times in this capture: select the one you mean in the text.', factId: null }
	);
	assert.equal(proposeOutcome(422, { error: "quote not found in this capture's text" }).kind, 'not_found');
	assert.deepEqual(proposeOutcome(409, { error: 'the selected text no longer matches this capture; reload it' }), {
		kind: 'error',
		text: 'the selected text no longer matches this capture; reload it',
		factId: null
	});
	assert.equal(proposeOutcome(422, { error: 'edited value not in citation quote' }).text, 'edited value not in citation quote');
	assert.equal(proposeOutcome(0, null, 'Propose could not reach the admin host.').text, 'Propose could not reach the admin host.');
});

test('parsePrepared reads JSON Lines with line numbers, bad lines and refused rows', () => {
	const good = { capture_id: 130, quote: 'Monday through Friday, 8 a.m. to 5 p.m.', topic: 'hours', statement: 'weekday__tumwater: Monday through Friday' };
	const text = [
		JSON.stringify(good),
		'',
		'   ',
		'{"capture_id": 130, "quote": "oops"',
		'[1, 2]',
		JSON.stringify({ ...good, capture_id: '130' }),
		JSON.stringify({ ...good, statement: '' }),
		JSON.stringify({ ...good, topic: 'phone_script' }),
		JSON.stringify({ ...good, statement: 'weekday: Saturday' })
	].join('\r\n');
	const rows = parsePrepared(text);
	assert.deepEqual(
		rows.map((r) => r.line),
		[1, 4, 5, 6, 7, 8, 9]
	);
	assert.deepEqual(rows[0], { line: 1, raw: JSON.stringify(good), row: good, error: null });
	assert.match(rows[1].error ?? '', /^Line 4: not JSON/);
	assert.equal(rows[1].row, null);
	assert.equal(rows[2].error, 'Line 5: expected one {…} object.');
	assert.equal(rows[3].error, 'Line 6: capture_id must be a capture number.');
	assert.equal(rows[4].error, 'Line 7: statement must be a non-empty string.');
	// Rows the server would refuse keep their fields and say why.
	assert.deepEqual(rows[5].row, { ...good, topic: 'phone_script' });
	assert.match(rows[5].error ?? '', /staff-only/);
	assert.match(rows[6].error ?? '', /not in the quote/);
	assert.deepEqual(parsePrepared(''), []);
});

test('parsePrepared keeps byte offsets, both or neither, and quoteBody sends them', () => {
	const good = { capture_id: 130, quote: 'Monday through Friday, 8 a.m. to 5 p.m.', topic: 'hours', statement: 'weekday__tumwater: Monday through Friday' };
	const text = [
		JSON.stringify({ ...good, start: 2152, end: 2191 }),
		JSON.stringify({ ...good, start: 2152 }),
		JSON.stringify({ ...good, start: 10, end: 10 }),
		JSON.stringify({ ...good, start: -1, end: 4 }),
		JSON.stringify({ ...good, start: '1', end: 4 }),
		JSON.stringify({ ...good, start: 1, end: 4, quote: 'short' })
	].join('\n');
	const rows = parsePrepared(text);
	assert.deepEqual(rows[0].row, { ...good, start: 2152, end: 2191 });
	assert.equal(rows[0].error, null);
	for (const r of rows.slice(1, 5)) assert.match(r.error ?? '', /byte offsets/);
	// With offsets the quote is the exact slice, so its length is checked here.
	assert.match(rows[5].error ?? '', /8 to 400 characters/);
	const sent = quoteBody(/** @type {any} */ (rows[0].row));
	assert.deepEqual(sent, { body: { quote: good.quote, start: 2152, end: 2191, topic: 'hours', statement: good.statement } });
	const located = quoteBody(/** @type {any} */ (parsePrepared(JSON.stringify(good))[0].row));
	assert.deepEqual(located, { body: { quote: good.quote, topic: 'hours', statement: good.statement } });
});

test('quoteTopics offers every handbook topic but phone_script', () => {
	const topics = quoteTopics().map((c) => c.topic);
	assert.equal(topics.length, 25);
	assert.ok(!topics.includes('phone_script'));
	assert.ok(topics.includes('hours') && topics.includes('service_notice'));
});

test('chipKey keeps the instance only for slots that hold one value per instance', () => {
	assert.equal(chipKey('weekday__tumwater', 'weekday', 'hours'), 'weekday__tumwater');
	assert.equal(chipKey('counter_hours__cheney', 'weekday', 'hours'), 'weekday__cheney');
	// The alias implies the office.
	assert.equal(chipKey('apostille_eastern_branch_street', 'office', 'address'), 'office__cheney');
	// A fee is one value: the instance drops.
	assert.equal(chipKey('expedite__tumwater', 'per_document', 'fees'), 'per_document');
	assert.equal(chipKey('', 'per_document', 'fees'), 'per_document');
});

test('capture answers parse defensively', () => {
	const list = parseCaptures({
		slug: 'usa-wa-sos',
		captures: [
			{ id: 130, source_id: 12, uri: 'https://www.sos.wa.gov/apostille', kind: 'warc', trust: 'official', captured_at: 1793700000, bytes: 27298 },
			{ id: 'x', uri: 'https://www.sos.wa.gov/' },
			null,
			{ id: 131, uri: 'https://www.sos.wa.gov/fees' }
		]
	});
	assert.deepEqual(list, [
		{ id: 130, source_id: 12, uri: 'https://www.sos.wa.gov/apostille', kind: 'warc', trust: 'official', captured_at: 1793700000, bytes: 27298 },
		{ id: 131, source_id: null, uri: 'https://www.sos.wa.gov/fees', kind: '', trust: '', captured_at: 0, bytes: 0 }
	]);
	assert.deepEqual(parseCaptures(null), []);
	const doc = { id: 130, slug: 'usa-wa-sos', uri: 'https://www.sos.wa.gov/apostille', kind: 'warc', trust: 'official', captured_at: 1793700000, text: SIDECAR };
	assert.deepEqual(parseCaptureText(doc), doc);
	assert.equal(parseCaptureText({ id: 130 }), null);
	assert.equal(parseCaptureText({ error: 'capture not found' }), null);
});

test('uriPath and sizeLabel name a capture row', () => {
	assert.equal(uriPath('https://www.sos.wa.gov/corps/apostille.aspx?tab=fees'), '/corps/apostille.aspx?tab=fees');
	assert.equal(uriPath('https://www.sos.wa.gov/'), 'www.sos.wa.gov');
	assert.equal(uriPath('upload:staff/scan.pdf'), 'staff/scan.pdf');
	assert.equal(uriPath('not a url'), 'not a url');
	assert.equal(sizeLabel(940), '940 B');
	assert.equal(sizeLabel(27298), '27 KB');
	assert.equal(sizeLabel(5000), '4.9 KB');
	assert.equal(sizeLabel(1572864), '1.5 MB');
	assert.equal(sizeLabel(Number.NaN), '—');
});
