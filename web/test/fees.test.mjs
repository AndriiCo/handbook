import test from 'node:test';
import assert from 'node:assert/strict';
import {
	ASK_ID,
	ASK_OFFICE,
	ESTIMATE_ID,
	MAX_DOCS,
	askSentence,
	askText,
	asksOffice,
	clauses,
	estimate,
	estimateItem,
	estimateText,
	feeModel,
	money,
	readTiers,
	tierTable
} from '../src/lib/public-ui/fees.js';

// Washington-shaped fee items (contract Item). Quotes follow the
// office's published wording; values are what staff accepted.
const SRC = {
	key: 'https://www.sos.wa.gov/corporations-charities/apostilles-program/fees',
	kind: 'web',
	title: 'Apostille fees',
	url: 'https://www.sos.wa.gov/corporations-charities/apostilles-program/fees',
	host: 'sos.wa.gov',
	capturedAt: '2026-11-02T18:00:00Z',
	acceptedAt: 1793750400,
	channel: null,
	occurredAt: null,
	recorded: null
};

function item(over) {
	return {
		id: 'fees-x',
		slot: 'fees.x',
		label: 'Fee',
		display: '',
		kind: 'money',
		factIds: [1],
		cites: [],
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

const cite = (quote, source = SRC) => ({ source, quote, summary: null, factId: 1 });

const PER = item({
	id: 'fees-per_document',
	slot: 'fees.per_document',
	label: 'Fee per document',
	display: '$15.00 per document',
	factIds: [11],
	cites: [cite('Apostille or certificate of authentication: $15.00 per document.')]
});

const EXPEDITE = item({
	id: 'fees-expedite',
	slot: 'fees.expedite',
	label: 'Expedited service fee',
	display: '$100 expedite fee per each 10 documents',
	kind: 'money_tier',
	factIds: [12],
	cites: [
		cite(
			'Expedited service: $100 expedite fee per each 10 documents (1–10 docs = $100; 11–20 = $200), in addition to $15.00/document.'
		)
	]
});

const SAME_DAY_FULL = item({
	id: 'fees-same_day',
	slot: 'fees.same_day',
	label: 'Same-day service fee',
	display: 'an additional $150 same day fee for the first 10 documents',
	kind: 'money_tier',
	route: 'counter',
	factIds: [13],
	cites: [
		cite(
			'Same day service is available for an additional $150 same day fee for the first 10 documents. If there are more than 10 documents an additional $100 expedite fee will apply. (1-10 documents = $150.00, 11-20 documents = $250, etc.)'
		)
	]
});

const SAME_DAY_FIRST_TEN = item({
	...SAME_DAY_FULL,
	cites: [cite('Same day service is available for an additional $150 same day fee for the first 10 documents.')]
});

const WA = feeModel([PER, EXPEDITE, SAME_DAY_FULL]);

test('tierTable reads explicit ranges in each printed style', () => {
	assert.deepEqual(tierTable('1-10 documents = $150.00, 11-20 documents = $250, etc.'), [
		{ from: 1, to: 10, cents: 15000 },
		{ from: 11, to: 20, cents: 25000 }
	]);
	assert.deepEqual(tierTable('11-20 = $250'), [{ from: 11, to: 20, cents: 25000 }]);
	assert.deepEqual(tierTable('1–10 documents: $150'), [{ from: 1, to: 10, cents: 15000 }]);
	assert.deepEqual(tierTable('(1–10 docs = $100; 11–20 = $200)'), [
		{ from: 1, to: 10, cents: 10000 },
		{ from: 11, to: 20, cents: 20000 }
	]);
	assert.deepEqual(tierTable('1 to 10 documents: $1,000.50'), [{ from: 1, to: 10, cents: 100050 }]);
});

test('"for the first 10 documents" covers 1–10 only', () => {
	assert.deepEqual(tierTable('an additional $150 same day fee for the first 10 documents'), [
		{ from: 1, to: 10, cents: 15000 }
	]);
	assert.deepEqual(tierTable('For the first 10 documents the fee is $150.'), [{ from: 1, to: 10, cents: 15000 }]);
});

test('"per 10 documents" covers only the first block unless later blocks are stated', () => {
	assert.deepEqual(tierTable('$100 per 10 documents'), [{ from: 1, to: 10, cents: 10000 }]);
	assert.deepEqual(tierTable('$100 expedite fee per each 10 documents'), [{ from: 1, to: 10, cents: 10000 }]);
	assert.deepEqual(tierTable('$100 for every 10 documents (1–10 docs = $100; 11–20 = $200)'), [
		{ from: 1, to: 10, cents: 10000 },
		{ from: 11, to: 20, cents: 20000 }
	]);
	// "each additional" names no first block.
	assert.deepEqual(tierTable('$100 for each additional 10 documents'), []);
});

test('tierTable never reads tiers out of phones, ZIP codes, days or per-document fees', () => {
	assert.deepEqual(tierTable('Call 360-725-0377 or write to PO Box 40228, Olympia, WA 98504-0228.'), []);
	assert.deepEqual(tierTable('Typically 7-10 business days. $15.00 per document.'), []);
	assert.deepEqual(tierTable('$15.00 per document'), []);
	assert.deepEqual(tierTable(''), []);
});

test('a passage that gives one count two amounts refuses its tiers', () => {
	const read = readTiers('1-10 documents = $150; 1-10 documents = $100');
	assert.equal(read.conflict, true);
	assert.deepEqual(read.tiers, []);
});

test('clauses keep a parenthetical tier list whole', () => {
	assert.deepEqual(clauses('Fee: $100 per 10 documents (1–10 docs = $100; 11–20 = $200). Next sentence.'), [
		'Fee: $100 per 10 documents (1–10 docs = $100; 11–20 = $200).',
		'Next sentence.'
	]);
});

test('feeModel: per-document cents, and each speed with its cited tiers and item ids', () => {
	assert.equal(WA.perDocument?.cents, 1500);
	assert.equal(WA.perDocument?.reason, null);
	assert.deepEqual(WA.speeds.standard?.itemIds, ['fees-per_document']);
	assert.deepEqual(WA.speeds.expedite?.itemIds, ['fees-per_document', 'fees-expedite']);
	assert.deepEqual(WA.speeds.expedite?.tiers, [
		{ from: 1, to: 10, cents: 10000 },
		{ from: 11, to: 20, cents: 20000 }
	]);
	assert.deepEqual(WA.speeds.same_day?.tiers, [
		{ from: 1, to: 10, cents: 15000 },
		{ from: 11, to: 20, cents: 25000 }
	]);
	assert.equal(WA.currency, 'USD');
});

test('feeModel offers only speeds whose fee item exists', () => {
	const model = feeModel([PER]);
	assert.ok(model.speeds.standard);
	assert.equal(model.speeds.expedite, null);
	assert.equal(model.speeds.same_day, null);
	assert.equal(feeModel([]).perDocument, null);
	// An honest "not confirmed" row is not a fee.
	const missing = item({ ...PER, factIds: [], cites: [], state: 'missing', display: 'Not confirmed yet — ask the office' });
	assert.equal(feeModel([missing]).perDocument, null);
});

test('one passage listing two speeds never mixes their tiers', () => {
	const both =
		'Expedited service: $100 per 10 documents (1-10 documents = $100, 11-20 documents = $200). Same day service: $150 for the first 10 documents (1-10 documents = $150.00, 11-20 documents = $250, etc.)';
	const model = feeModel([
		PER,
		item({ ...EXPEDITE, display: '$100 per 10 documents', cites: [cite(both)] }),
		item({ ...SAME_DAY_FULL, display: '$150 for the first 10 documents', cites: [cite(both)] })
	]);
	assert.deepEqual(model.speeds.expedite?.tiers, [
		{ from: 1, to: 10, cents: 10000 },
		{ from: 11, to: 20, cents: 20000 }
	]);
	assert.deepEqual(model.speeds.same_day?.tiers, [
		{ from: 1, to: 10, cents: 15000 },
		{ from: 11, to: 20, cents: 25000 }
	]);
});

test('a "first N" or "per N" phrase binds only to the fee item’s own amount', () => {
	// The phrase sits next to the per-document fee, not the same-day fee.
	const sameDay = feeModel([
		PER,
		item({
			...SAME_DAY_FULL,
			display: 'an additional $150',
			cites: [cite('Same day service is an additional $150, on top of $15.00 per document, for the first 10 documents.')]
		})
	]);
	assert.deepEqual(sameDay.speeds.same_day?.tiers, []);
	assert.equal(sameDay.speeds.same_day?.reason, 'unclear');
	assert.equal(total(sameDay, 3, 'same_day'), 'ask:unclear');
	// The phrase sits next to the same-day amount: the expedite fee states no tier.
	const expedite = feeModel([
		PER,
		item({
			...EXPEDITE,
			display: '$100 expedite fee',
			cites: [cite('Expedite fee $100, same day fee $150, for the first 10 documents.')]
		})
	]);
	assert.deepEqual(expedite.speeds.expedite?.tiers, []);
	assert.equal(total(expedite, 3, 'expedite'), 'ask:unclear');
	// Two fees in one clause, each with its own phrase: each keeps its own tier.
	const both = 'Expedite fee $100 for the first 10 documents, same day fee $150 for the first 10 documents.';
	const model = feeModel([
		PER,
		item({ ...EXPEDITE, display: '$100 expedite fee', cites: [cite(both)] }),
		item({ ...SAME_DAY_FULL, display: '$150 same day fee', cites: [cite(both)] })
	]);
	assert.deepEqual(model.speeds.expedite?.tiers, [{ from: 1, to: 10, cents: 10000 }]);
	assert.deepEqual(model.speeds.same_day?.tiers, [{ from: 1, to: 10, cents: 15000 }]);
	assert.equal(total(model, 3, 'expedite'), '$145.00');
	assert.equal(total(model, 3, 'same_day'), '$195.00');
	// Unanchored reading is unchanged; anchored, the stray phrase is counted.
	assert.deepEqual(readTiers('$15.00 per document, for the first 10 documents').tiers, [{ from: 1, to: 10, cents: 1500 }]);
	assert.deepEqual(readTiers('an additional $150, on top of $15.00 per document, for the first 10 documents', 15000), {
		tiers: [],
		conflict: false,
		stray: 1
	});
});

test('a speed value that restates the per-document fee anchors on its own amount', () => {
	// The live Washington values: each speed restates "$15.00 per document" first.
	const expeditePage = 'Fee is $15.00 per document and an additional $100 expedite fee per 10 documents.';
	const sameDayPage =
		'Same day services in-person (Tumwater or Cheney): Fee is $15.00 per document and an additional $150 same day fee for the first 10 documents.';
	const model = feeModel([
		PER,
		item({ ...EXPEDITE, display: expeditePage, cites: [cite(expeditePage)] }),
		item({ ...SAME_DAY_FULL, display: sameDayPage, cites: [cite(sameDayPage)] })
	]);
	assert.deepEqual(model.speeds.expedite?.tiers, [{ from: 1, to: 10, cents: 10000 }]);
	assert.deepEqual(model.speeds.same_day?.tiers, [{ from: 1, to: 10, cents: 15000 }]);
	assert.equal(total(model, 3, 'expedite'), '$145.00');
	assert.equal(total(model, 3, 'same_day'), '$195.00');
	// A speed fee equal to the per-document fee still anchors on it.
	const same = feeModel([
		PER,
		item({ ...EXPEDITE, display: '$15.00 expedite fee', cites: [cite('Expedite fee $15.00 for the first 10 documents.')] })
	]);
	assert.deepEqual(same.speeds.expedite?.tiers, [{ from: 1, to: 10, cents: 1500 }]);
});

test('a first block at another amount than the fee’s own asks', () => {
	const model = feeModel([
		PER,
		item({
			...EXPEDITE,
			display: '$100 expedite fee',
			cites: [cite('Expedite fee $100 per document, same day fee 1-10 documents = $150.')]
		})
	]);
	assert.equal(model.speeds.expedite?.reason, 'unclear');
	assert.equal(total(model, 3, 'expedite'), 'ask:unclear');
});

test('feeModel offers no speed for a stated absence or a value with no source on file', () => {
	const absent = item({ ...SAME_DAY_FULL, state: 'absent', display: 'Same-day service is not offered by mail.' });
	assert.equal(feeModel([PER, absent]).speeds.same_day, null);
	const uncited = item({ ...EXPEDITE, state: 'uncited', cites: [] });
	assert.equal(feeModel([PER, uncited]).speeds.expedite, null);
	assert.equal(feeModel([item({ ...PER, state: 'uncited', cites: [] })]).perDocument, null);
});

const total = (model, n, speed) => {
	const r = estimate(model, n, speed);
	return r.ask ? `ask:${r.reason}` : money(r.totalCents);
};

test('estimate: n = 1/3/10/11/12/20/21/25 against the cited Washington tiers', () => {
	const rows = [
		[1, 'standard', '$15.00'],
		[1, 'expedite', '$115.00'],
		[1, 'same_day', '$165.00'],
		[3, 'standard', '$45.00'],
		[3, 'expedite', '$145.00'],
		[10, 'expedite', '$250.00'],
		[10, 'same_day', '$300.00'],
		[11, 'expedite', '$365.00'],
		[11, 'same_day', '$415.00'],
		[12, 'same_day', '$430.00'],
		[20, 'same_day', '$550.00'],
		[20, 'standard', '$300.00'],
		[21, 'standard', 'ask:over-max'],
		[21, 'same_day', 'ask:over-max'],
		[25, 'standard', 'ask:over-max'],
		[25, 'expedite', 'ask:over-max'],
		[25, 'same_day', 'ask:over-max']
	];
	for (const [n, speed, want] of rows) assert.equal(total(WA, n, speed), want, `${n} ${speed}`);
	assert.equal(MAX_DOCS, 20);
});

test('3 expedited = $145.00, written from the model only', () => {
	const r = estimate(WA, 3, 'expedite');
	assert.equal(r.ask, false);
	if (r.ask) return;
	assert.equal(r.totalCents, 14500);
	assert.equal(r.route, 'mail');
	assert.deepEqual(
		r.inputs.map((i) => i.id),
		['fees-per_document', 'fees-expedite']
	);
	assert.equal(estimateText(r), 'Total to enclose: $145.00 = 3 × $15.00 + $100.00 expedite fee (1–10 documents)');
});

test('12 same-day = $430.00 when the 11–20 tier is cited, at the counter', () => {
	const r = estimate(WA, 12, 'same_day');
	assert.equal(r.ask, false);
	if (r.ask) return;
	assert.equal(r.totalCents, 43000);
	assert.equal(estimateText(r), 'Total at the counter: $430.00 = 12 × $15.00 + $250.00 same-day fee (11–20 documents)');
});

test('11 same-day with only "first 10" cited = ask; 10 still answers', () => {
	const model = feeModel([PER, EXPEDITE, SAME_DAY_FIRST_TEN]);
	assert.deepEqual(model.speeds.same_day?.tiers, [{ from: 1, to: 10, cents: 15000 }]);
	assert.equal(total(model, 10, 'same_day'), '$300.00');
	assert.equal(total(model, 11, 'same_day'), 'ask:no-tier');
	assert.equal(total(model, 12, 'same_day'), 'ask:no-tier');
});

test('a tier only the accepted value states (not its quote) is uncited: ask', () => {
	const model = feeModel([
		PER,
		item({
			...SAME_DAY_FULL,
			display: '$150 for the first 10 documents (1-10 documents = $150.00, 11-20 documents = $250)',
			cites: [cite('Same day service: an additional $150 same day fee for the first 10 documents.')]
		})
	]);
	assert.equal(total(model, 10, 'same_day'), '$300.00');
	assert.equal(total(model, 12, 'same_day'), 'ask:no-tier');
});

test('uncited tiers and amounts = ask', () => {
	const noQuote = feeModel([PER, item({ ...EXPEDITE, cites: [] })]);
	assert.equal(total(noQuote, 3, 'expedite'), 'ask:uncited');
	// A staff note is never the office's text.
	const noteOnly = feeModel([
		PER,
		item({
			...EXPEDITE,
			cites: [{ source: { ...SRC, kind: 'call', key: 'note:4' }, quote: null, summary: '$100 per 10 documents', factId: null }]
		})
	]);
	assert.equal(total(noteOnly, 3, 'expedite'), 'ask:uncited');
	// The quote must state the value's own amount.
	const otherAmount = feeModel([PER, item({ ...EXPEDITE, cites: [cite('Expedited service: $90 per 10 documents.')] })]);
	assert.equal(total(otherAmount, 3, 'expedite'), 'ask:uncited');
	// An uncited per-document fee answers nothing at all.
	const perUncited = feeModel([item({ ...PER, cites: [] }), EXPEDITE]);
	assert.equal(perUncited.perDocument?.cents, null);
	assert.equal(total(perUncited, 3, 'standard'), 'ask:uncited');
	assert.equal(total(perUncited, 3, 'expedite'), 'ask:uncited');
	// An amount with no count range stated is not a tier.
	const flat = feeModel([PER, item({ ...EXPEDITE, display: '$100 expedite fee', cites: [cite('Expedite fee: $100.')] })]);
	assert.equal(total(flat, 3, 'expedite'), 'ask:no-tier');
});

test('a disputed input = ask, whichever input it is', () => {
	const perDisputed = feeModel([item({ ...PER, state: 'conflict' }), EXPEDITE]);
	assert.equal(total(perDisputed, 3, 'standard'), 'ask:conflict');
	assert.equal(total(perDisputed, 3, 'expedite'), 'ask:conflict');
	const speedDisputed = feeModel([PER, item({ ...EXPEDITE, state: 'conflict' })]);
	assert.equal(total(speedDisputed, 3, 'standard'), '$45.00');
	assert.equal(total(speedDisputed, 3, 'expedite'), 'ask:conflict');
	const worded = feeModel([PER, item({ ...EXPEDITE, state: 'worded' })]);
	assert.equal(total(worded, 3, 'expedite'), 'ask:conflict');
	// The value contradicting its own quote is a dispute too.
	const contradicted = feeModel([
		PER,
		item({ ...EXPEDITE, display: '$100 per 10 documents (1–10 docs = $120)' })
	]);
	assert.equal(total(contradicted, 3, 'expedite'), 'ask:conflict');
});

test('no model, unknown speeds and bad counts = ask', () => {
	assert.equal(total(null, 3, 'standard'), 'ask:no-fee');
	assert.equal(total(feeModel([PER]), 3, 'expedite'), 'ask:no-speed');
	assert.equal(total(WA, 0, 'standard'), 'ask:count');
	assert.equal(total(WA, 2.5, 'standard'), 'ask:count');
	assert.equal(total(feeModel([item({ ...PER, display: 'Fifteen dollars' })]), 3, 'standard'), 'ask:no-amount');
	// A page in another currency gets no `$` model (Viet Nam).
	const vnd = feeModel([item({ ...PER, display: '300,000 VND per document', cites: [cite('300,000 VND per document')] })]);
	assert.equal(total(vnd, 1, 'standard'), 'ask:no-amount');
});

test('estimateItem is a computed Item whose cites are its inputs’ cites', () => {
	const est = estimateItem(WA, 3, 'expedite');
	assert.ok(est);
	if (!est) return;
	assert.equal(est.id, 'fees-estimate');
	assert.equal(est.state, 'computed');
	assert.deepEqual(est.factIds, []);
	assert.deepEqual(
		est.inputs?.map((i) => i.id),
		['fees-per_document', 'fees-expedite']
	);
	assert.deepEqual(est.cites, [...PER.cites, ...EXPEDITE.cites]);
	assert.equal(est.display, 'Total to enclose: $145.00 = 3 × $15.00 + $100.00 expedite fee (1–10 documents)');
	assert.equal(est.stale, null);
	assert.equal(estimateItem(WA, 25, 'expedite'), null);
	assert.equal(ESTIMATE_ID, 'fees-estimate');
	assert.notEqual(ASK_ID, ESTIMATE_ID);
});

test('estimateItem carries the worst staleness of its inputs', () => {
	const muted = { level: 'muted', text: 'Checked four months ago' };
	const warning = { level: 'warning', text: 'May be out of date — check with the office before you send money' };
	const oldPer = item({ ...PER, state: 'stale', stale: muted });
	assert.deepEqual(estimateItem(feeModel([oldPer, EXPEDITE]), 3, 'expedite')?.stale, muted);
	const oldExpedite = item({ ...EXPEDITE, state: 'stale', stale: warning });
	assert.deepEqual(estimateItem(feeModel([oldPer, oldExpedite]), 3, 'expedite')?.stale, warning);
	// The standard total rests on the per-document fee only.
	assert.deepEqual(estimateItem(feeModel([PER, oldExpedite]), 3, 'standard')?.stale, null);
});

test('over-max names the calculator\'s limit, never a gap in the office\'s pages', () => {
	// The per-document fee prints no limit: 21 standard documents is the
	// calculator's own stop.
	assert.equal(askText('over-max'), "This calculator doesn't work out totals for that many documents.");
	assert.doesNotMatch(askText('over-max'), /office/);
	assert.equal(askSentence('over-max'), "This calculator doesn't work out totals for that many documents. Ask the office before you pay.");
	// A count no cited tier covers is the office's gap.
	assert.equal(askText('no-tier'), "The office's pages don't give this fee for that many documents.");
});

test('a fee a staff note contests asks the office, for every speed', () => {
	const contested = item({ ...PER, state: 'contested' });
	const model = feeModel([contested, EXPEDITE]);
	assert.equal(model.perDocument?.cents, null);
	for (const speed of ['standard', 'expedite']) assert.deepEqual(estimate(model, 3, speed), { ask: true, reason: 'contested' });
	assert.equal(estimateItem(model, 3, 'standard'), null);
	// A contested speed fee asks for that speed only.
	const speedOnly = feeModel([PER, item({ ...EXPEDITE, state: 'contested' })]);
	assert.deepEqual(estimate(speedOnly, 3, 'expedite'), { ask: true, reason: 'contested' });
	assert.equal(estimate(speedOnly, 3, 'standard').ask, false);
	assert.equal(askSentence('contested'), 'The office told us something different from its pages. Ask the office before you pay.');
});

test('askText is plain template copy: no digits, no $', () => {
	for (const reason of ['no-fee', 'count', 'no-speed', 'conflict', 'contested', 'uncited', 'no-amount', 'no-tier', 'unclear', 'over-max']) {
		const text = askText(reason);
		assert.ok(text.length > 10, reason);
		assert.doesNotMatch(text, /[\d$]/, reason);
		assert.doesNotMatch(askSentence(reason), /[\d$]/, reason);
	}
});

test('a count still to enter is a hint, not a reason to call the office', () => {
	assert.equal(asksOffice('count'), false);
	assert.equal(askSentence('count'), 'Enter how many documents you are sending.');
	for (const reason of ['conflict', 'contested', 'over-max', 'no-tier', 'unclear', 'uncited', 'no-fee']) {
		assert.equal(asksOffice(reason), true, reason);
		assert.equal(askSentence(reason), `${askText(reason)} ${ASK_OFFICE}`, reason);
	}
});

test('money prints two decimals and thousands', () => {
	assert.equal(money(14500), '$145.00');
	assert.equal(money(100050), '$1,000.50');
	assert.equal(money(0), '$0.00');
});
