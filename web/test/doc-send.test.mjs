import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse as parseSvelte } from 'svelte/compiler';
import { parse as parseJs } from 'acorn';
import {
	NOT_CONFIRMED,
	REMOTE_NOTARIZATION_TEXT,
	SLOT,
	bySlot,
	formLink,
	isUS,
	issuerName,
	keepTogether,
	notConfirmed,
	partOf,
	remoteNotarizationRow,
	traceAttrs,
	usable,
	valueLang
} from '../src/lib/public-ui/doc/items.js';
import {
	carrierChips,
	carriersNamed,
	contestLead,
	isPoBox,
	isStreetOnly,
	mapLinks,
	mapQuery,
	noteDay,
	officeAnswer,
	phoneIn,
	postalCity,
	saysUspsOnly,
	telHref
} from '../src/lib/public-ui/doc/postal.js';
import { repeatItems, sendRepeats } from '../src/lib/public-ui/doc/send.js';
import {
	clockText,
	closureApplies,
	closureDates,
	closureOn,
	cutoffRule,
	dayLabel,
	instanceName,
	liveStatus,
	sameOffice,
	splitOffices,
	timesIn,
	upcomingClosures,
	zonedDay,
	zonedNow
} from '../src/lib/public-ui/doc/offices.js';
import { DEGENERATE_LINES, itemDay, panelDay, shortDay, updateEntries } from '../src/lib/public-ui/doc/updates.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const PUBLIC_UI = join(HERE, '..', 'src', 'lib', 'public-ui');
const DOC = join(PUBLIC_UI, 'doc');

const SRC = {
	key: 'https://www.sos.wa.gov/corporations-charities/contact-us',
	kind: 'web',
	title: 'Contact us',
	url: 'https://www.sos.wa.gov/corporations-charities/contact-us',
	host: 'sos.wa.gov',
	capturedAt: '2026-11-03T16:00:00Z',
	acceptedAt: 1793836800,
	channel: null,
	occurredAt: null,
	recorded: null
};

function item(over) {
	return {
		id: 'x',
		slot: null,
		label: 'Detail',
		display: '',
		kind: 'text',
		factIds: [1],
		cites: [{ source: SRC, quote: 'q', summary: null, factId: 1 }],
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

// ---------------------------------------------------------------- items

test('traceAttrs: fact ids on cited values, data-state on the rest', () => {
	assert.deepEqual(traceAttrs(item({ factIds: [3, 7] })), { 'data-fact-ids': '3 7' });
	assert.deepEqual(traceAttrs(notConfirmed('a', 'A')), { 'data-state': 'missing' });
	const computed = item({ id: 'fees-estimate', state: 'computed', factIds: [], inputs: [item({ id: 'fees-per_document' }), item({ id: 'fees-expedite' })] });
	assert.deepEqual(traceAttrs(computed), { 'data-state': 'computed', 'data-inputs': 'fees-per_document fees-expedite' });
	assert.deepEqual(traceAttrs(item({ state: 'absent', factIds: [9] })), { 'data-fact-ids': '9', 'data-state': 'absent' });
	assert.deepEqual(traceAttrs(item({ factIds: [] })), { 'data-state': 'missing' });
});

test('usable: accepted, undisputed and resting on facts', () => {
	assert.equal(usable(item({})), true);
	assert.equal(usable(item({ state: 'stale' })), true);
	for (const state of ['conflict', 'worded', 'missing', 'uncited', 'computed', 'absent']) {
		assert.equal(usable(item({ state })), false, state);
	}
	assert.equal(usable(item({ factIds: [] })), false);
	assert.equal(usable(null), false);
});

test('partOf keeps the value’s facts and cites under its own id', () => {
	const courier = item({ id: 'mailing-courier', factIds: [21], display: '243 Israel Rd SE, Tumwater, WA 98501', copy: true });
	const city = partOf(courier, 'city', 'Tumwater');
	assert.equal(city.id, 'mailing-courier-city');
	assert.deepEqual(city.factIds, [21]);
	assert.equal(city.cites, courier.cites);
	assert.equal(city.copy, false);
	assert.equal(bySlot([null, item({ slot: SLOT.courier })], SLOT.courier)?.slot, 'mailing.courier');
	assert.equal(NOT_CONFIRMED, 'Not confirmed yet — ask the office');
});

test('isUS reads the spellings authority rows use', () => {
	for (const c of ['USA', 'US', 'United States', 'United States of America', 'usa']) assert.equal(isUS(c), true, c);
	for (const c of ['Viet Nam', 'Netherlands', '', null]) assert.equal(isUS(c), false, String(c));
});

test('remoteNotarizationRow: one honest row unless a row already speaks to it', () => {
	const row = remoteNotarizationRow([item({ slot: SLOT.notarization, display: 'Notarial certificate must be complete, signed and stamped by a Washington notary.' })]);
	assert.ok(row);
	assert.equal(row?.state, 'missing');
	assert.equal(row?.display, REMOTE_NOTARIZATION_TEXT);
	assert.deepEqual(traceAttrs(/** @type {any} */ (row)), { 'data-state': 'missing' });
	assert.equal(remoteNotarizationRow([item({ display: 'Remotely notarized documents are accepted.' })]), null);
	assert.equal(remoteNotarizationRow([item({ display: 'Documents notarized online (RON) are accepted.' })]), null);
	assert.equal(remoteNotarizationRow([item({ slot: SLOT.notarization, display: 'RON certificates are accepted.' })]), null);
	assert.ok(remoteNotarizationRow([]));
});

test('remoteNotarizationRow reads only notarization rows, and RON only in capitals', () => {
	// Another Prepare slot that says "remote" is not about notarization.
	assert.ok(remoteNotarizationRow([item({ slot: 'copies.vital_records', display: 'Remote requests for certified copies go to the Department of Health.' })]));
	// An unmapped row must speak of notarization too.
	assert.ok(remoteNotarizationRow([item({ display: 'Remote work days may close the counter early.' })]));
	// "Ron" is a name, not remote online notarization.
	assert.ok(remoteNotarizationRow([item({ slot: SLOT.notarization, display: 'Ask for Ron at the notary desk.' })]));
	assert.ok(remoteNotarizationRow([item({ display: 'The ron notary stamp must be legible.' })]));
});

// --------------------------------------------------------------- postal

test('PO boxes and carriers are read from the text as printed', () => {
	assert.equal(isPoBox('PO Box 40228, Olympia, WA 98504-0228'), true);
	assert.equal(isPoBox('P.O. Box 40234'), true);
	assert.equal(isPoBox('243 Israel Rd SE, Tumwater, WA 98501'), false);
	assert.deepEqual(carriersNamed('Commercial couriers such as UPS and FEDEX must use our overnight address'), ['fedex', 'ups']);
	assert.deepEqual(carriersNamed('pick-ups and groups'), []);
	assert.equal(saysUspsOnly('Requests are accepted by USPS only.'), true);
	assert.equal(saysUspsOnly('Send it only by U.S. Postal Service.'), true);
	assert.equal(saysUspsOnly('Mail it to the PO Box.'), false);
});

test('isStreetOnly: a street address with no PO box (D3: never in the USPS branch)', () => {
	assert.equal(isStreetOnly('243 Israel Rd SE, Tumwater, WA 98501'), true);
	assert.equal(isStreetOnly('801 Capitol Way S, Olympia, WA 98501'), true);
	assert.equal(isStreetOnly('1500 11th Street, Sacramento, CA 95814'), true);
	assert.equal(isStreetOnly('PO Box 40228, Olympia, WA 98504-0228'), false);
	assert.equal(isStreetOnly('Corporations and Charities Division, PO Box 40228, 801 Capitol Way S'), false);
	assert.equal(isStreetOnly('Send all documents with Request form and payment.'), false);
	assert.equal(isStreetOnly('Typically 7-10 business days'), false);
});

test('carrier chips: only from cited statements, never a cross', () => {
	const usps = item({ id: 'mailing-usps', slot: SLOT.usps, factIds: [2], display: 'PO Box 40234, Olympia, WA 98504-0234 (USPS mail only)' });
	const rule = item({ id: 'mailing-courier_rule', slot: SLOT.courierRule, factIds: [5], display: 'Commercial couriers such as UPS and FEDEX must use our overnight address.' });
	const chips = carrierChips({ mail: [usps], courier: [rule] });
	assert.deepEqual(
		chips.mail.map((c) => [c.label, c.stated, c.factIds]),
		[
			['USPS', true, [2]],
			['Other carriers', false, []]
		]
	);
	assert.deepEqual(
		chips.courier.map((c) => [c.label, c.stated]),
		[
			['FedEx', true],
			['UPS', true],
			['Other carriers', false]
		]
	);
	// Nothing stated: no chips. A disputed or uncited statement states nothing.
	assert.deepEqual(carrierChips({ mail: [item({ display: 'PO Box 40228' })], courier: [] }), { mail: [], courier: [] });
	assert.deepEqual(carrierChips({ mail: [{ ...usps, state: 'conflict' }], courier: [{ ...rule, state: 'uncited' }] }), {
		mail: [],
		courier: []
	});
});

test('postalCity reads the city verbatim from US address shapes', () => {
	assert.equal(postalCity('243 Israel Rd SE, Tumwater, WA 98501'), 'Tumwater');
	assert.equal(postalCity('Washington Secretary of State, Corporations & Charities Division\n243 Israel Rd SE\nTumwater, WA 98501'), 'Tumwater');
	assert.equal(postalCity('PO Box 40228, Olympia, WA 98504-0228'), 'Olympia');
	assert.equal(postalCity('960 Washington St, Cheney WA 99004.'), 'Cheney');
	assert.equal(postalCity('PO Box 40234, Olympia, Washington 98504-0234'), 'Olympia');
	// A city run into the box line is not guessed at.
	assert.equal(postalCity('PO Box 40234 Olympia, Washington 98504-0234'), null);
	assert.equal(postalCity('Town Center 3, 243 Israel Road SE, Tumwater, WA 98501.'), 'Tumwater');
	assert.equal(postalCity('No address here'), null);
	assert.equal(postalCity(''), null);
});

test('mapQuery: joined lines, asides dropped, city and country appended once (prototype port)', () => {
	assert.equal(
		mapQuery('243 Israel Rd SE\nTumwater, WA 98501 (public counter; 2nd floor)', 'Tumwater', 'USA'),
		'243 Israel Rd SE, Tumwater, WA 98501, USA'
	);
	assert.equal(mapQuery('960 Washington St', 'Cheney', 'United States of America'), '960 Washington St, Cheney, USA');
	assert.equal(mapQuery('Av. Independencia #994, Ciudad de México', null, 'MX'), 'Av. Independencia #994, Ciudad de México, México');
	assert.equal(mapQuery('Calle 1, Mexicali, México', null, 'México'), 'Calle 1, Mexicali, México');
	assert.equal(mapQuery('', null, 'USA'), '');
	const links = mapLinks('960 Washington St, Cheney, USA');
	assert.equal(links?.google, 'https://www.google.com/maps/search/?api=1&query=960%20Washington%20St%2C%20Cheney%2C%20USA');
	assert.equal(links?.apple, 'https://maps.apple.com/?q=960%20Washington%20St%2C%20Cheney%2C%20USA');
	assert.equal(mapLinks(''), null);
});

test('phones: the first number as printed, and its tel: link', () => {
	assert.equal(phoneIn('360-725-0377 (option 3)'), '360-725-0377');
	assert.equal(phoneIn('Call (360) 725-0377, option 3'), '(360) 725-0377');
	assert.equal(telHref('360-725-0377 (option 3)'), 'tel:3607250377');
	assert.equal(telHref('+84 24 3799 3125'), 'tel:+842437993125');
	assert.equal(telHref('option 3'), null);
});

test('officeAnswer: a decision backed by the office\'s answer unlocks Copy on the value it prefers (plan §3)', () => {
	const a = item({ id: 'mailing-usps-a', factIds: [2], display: 'PO BOX 40228 Olympia WA 98504-0228' });
	const b = item({ id: 'mailing-usps-b', factIds: [7, 9], display: 'PO Box 40234 Olympia WA 98504-0234' });
	const disputed = item({ id: 'mailing-usps', state: 'conflict', factIds: [2, 7, 9], alternatives: [a, b] });
	const email = { id: 31, kind: 'email', date: '2026-11-04', channel: 'apostilles@sos.wa.gov', summary: 'Apostille mail goes to PO Box 40234.', status: 'published' };
	const call = { id: 32, kind: 'call', date: '2026-11-04', channel: 'Apostille desk', summary: 'The desk said PO Box 40234.' };
	const byEmail = { id: 40, kind: 'decision', preferred_fact_id: 7, supports: [31, 32], summary: 'Use 40234.' };
	const byCall = { id: 41, kind: 'decision', preferred_fact_id: 7, supports: [32], summary: 'Use 40234.' };
	const answer = officeAnswer(disputed, [email, call, byEmail]);
	assert.equal(answer?.channel, 'email');
	assert.equal(answer?.unlocks, true);
	assert.equal(answer?.preferred?.id, 'mailing-usps-b');
	assert.equal(answer?.note.id, 31);
	// A phone answer backs a decision too: the staff call is the planned
	// way to settle the box (plan §3, §8).
	const callOnly = officeAnswer(disputed, [call, byCall]);
	assert.equal(callOnly?.channel, 'call');
	assert.equal(callOnly?.unlocks, true);
	assert.equal(callOnly?.note?.id, 32);
	assert.equal(callOnly?.preferred?.id, 'mailing-usps-b');
	// An email wins over a call when both back decisions; a visit counts.
	assert.equal(officeAnswer(disputed, [call, byCall, email, byEmail])?.channel, 'email');
	const visit = { id: 33, kind: 'visit', date: '2026-11-04', channel: 'Tumwater counter', summary: 'The counter said PO Box 40234.' };
	assert.equal(officeAnswer(disputed, [visit, { ...byCall, supports: [33] }])?.channel, 'visit');
	// A decision resting on no answer of the office is shown, and unlocks nothing.
	const bare = officeAnswer(disputed, [{ ...byCall, supports: [] }]);
	assert.deepEqual([bare?.preferred?.id, bare?.note, bare?.unlocks], ['mailing-usps-b', null, false]);
	// A backed decision wins over a bare one, whatever the order.
	assert.equal(officeAnswer(disputed, [{ ...byCall, id: 42, supports: [] }, call, byCall])?.unlocks, true);
	// A withdrawn answer backs nothing: the published decision still shows,
	// unlocking nothing.
	const withdrawn = officeAnswer(disputed, [{ ...email, status: 'withdrawn' }, byEmail]);
	assert.deepEqual([withdrawn?.decision.id, withdrawn?.note, withdrawn?.unlocks], [40, null, false]);
	// A decision naming no shown value, a withdrawn decision, or no notes: nothing.
	assert.equal(officeAnswer(disputed, [email, { ...byEmail, preferred_fact_id: 99 }]), null);
	assert.equal(officeAnswer(disputed, [email, { ...byEmail, status: 'withdrawn' }]), null);
	assert.equal(officeAnswer(disputed, null), null);
	assert.equal(noteDay('2026-11-04'), 'November 4, 2026');
	assert.equal(noteDay('soon'), '');
});

test('contestLead: the visible line of a contradicting staff note', () => {
	assert.equal(contestLead({ kind: 'call', date: '2026-11-04' }), 'Staff note: the office told us otherwise by phone on November 4, 2026');
	assert.equal(contestLead({ kind: 'email', date: null }), 'Staff note: the office told us otherwise by email');
	assert.equal(contestLead({ kind: 'visit', date: '2026-11-04' }), 'Staff note: the office told us otherwise at the office on November 4, 2026');
});

// ------------------------------------------------- form link, language, names

const PDF = 'https://www.sos.wa.gov/sites/default/files/2025-06/request-form.pdf';
const PAGE = 'https://www.sos.wa.gov/corporations-charities/apostilles-services-information-resource-page';
const cited = (url, kind = 'web') => [{ source: { ...SRC, key: url, url, kind }, quote: 'q', summary: null, factId: 1 }];

test('formLink (D6): a PDF first, else the page; a worded form takes its alternatives\' links', () => {
	assert.deepEqual(formLink(item({ cites: cited(PDF, 'pdf') })), { url: PDF, pdf: true });
	assert.deepEqual(formLink(item({ cites: cited(`${PAGE}#:~:text=form`) })), { url: PAGE, pdf: false });
	assert.deepEqual(formLink(item({ cites: [...cited(PAGE), ...cited(PDF, 'upload')] })), { url: PDF, pdf: true });
	// Production: two form facts worded differently share the slot.
	const worded = item({
		state: 'worded',
		cites: [],
		alternatives: [
			item({ id: 'v1', factIds: [9], display: 'Apostille Authentication Request Form (PDF Fillable Form)', cites: cited(PAGE) }),
			item({ id: 'v2', factIds: [12], display: 'Apostille Authentication Request (Fillable Form)', cites: cited(PDF, 'pdf') })
		]
	});
	assert.deepEqual(formLink(worded), { url: PDF, pdf: true });
	const pagesOnly = item({ ...worded, alternatives: worded.alternatives.map((a) => ({ ...a, cites: cited(PAGE) })) });
	assert.deepEqual(formLink(pagesOnly), { url: PAGE, pdf: false });
	// No https page, a staff note, or nothing to act on: no link.
	assert.equal(formLink(item({ cites: cited('http://www.sos.wa.gov/form.pdf') })), null);
	assert.equal(formLink(item({ cites: [{ source: { ...SRC, kind: 'call', url: null }, quote: null, summary: 'x', factId: 1 }] })), null);
	assert.equal(formLink(item({ state: 'uncited', cites: [] })), null);
	assert.equal(formLink(null), null);
});

test('valueLang (WCAG 3.1.2): Vietnamese and other scripts, never English', () => {
	assert.equal(valueLang('Trang chủ - Cổng thông tin Bộ Ngoại Giao'), 'vi');
	assert.equal(valueLang('Trang chủ'), 'vi');
	assert.equal(valueLang('Cổng thông tin'), 'vi');
	assert.equal(valueLang('Apostille Authentication Request Form'), null);
	assert.equal(valueLang('Ministry of Foreign Affairs of Viet Nam, Consular Department (Bộ Ngoại giao)'), null);
	assert.equal(valueLang('Café de la Paix'), null);
	assert.equal(valueLang('Міністерство юстиції України'), 'uk');
	assert.equal(valueLang('Министерство иностранных дел'), 'ru');
	// Cyrillic that no letter ties to one language stays unmarked.
	assert.equal(valueLang('Министерство юстиции'), null);
	assert.equal(valueLang('Υπουργείο Εξωτερικών'), 'el');
	assert.equal(valueLang('外務省'), 'zh');
	assert.equal(valueLang('外務省のアポスティーユ'), 'ja');
	assert.equal(valueLang(''), null);
});

test('keepTogether: ZIP+4 codes are runs of their own; the text is unchanged', () => {
	const text = 'PO BOX 40220\nOlympia, WA 98504-0220';
	const runs = keepTogether(text);
	assert.deepEqual(runs, [
		{ text: 'PO BOX 40220\nOlympia, WA ', keep: false },
		{ text: '98504-0220', keep: true }
	]);
	assert.equal(runs.map((r) => r.text).join(''), text);
	assert.deepEqual(keepTogether('Olympia, WA 98501'), [{ text: 'Olympia, WA 98501', keep: false }]);
	assert.deepEqual(keepTogether(''), [{ text: '', keep: false }]);
});

test('issuerName: "the" before an office, never before a country\'s name', () => {
	assert.equal(issuerName('Washington Secretary of State — Apostilles & Authentications', 'USA'), 'the Washington Secretary of State — Apostilles & Authentications');
	assert.equal(issuerName('The Secretary of the Commonwealth', 'USA'), 'The Secretary of the Commonwealth');
	assert.equal(issuerName('Viet Nam — receiving authorities', 'Viet Nam'), 'Viet Nam — receiving authorities');
	assert.equal(issuerName('United States Department of State', 'United States'), 'the United States Department of State');
	assert.equal(issuerName('Ministry of Justice', null), 'the Ministry of Justice');
});

// -------------------------------------------------------------- repeats

test('sendRepeats: the send step\'s copies of other steps\' values, under ids of their own', () => {
	const phone = item({ id: 'contacts-phone', slot: SLOT.phone, kind: 'phone', display: '360-725-0377', factIds: [241] });
	const option = item({ id: 'contacts-phone_option', slot: SLOT.phoneOption, display: 'option 3', factIds: [242] });
	const enclose = item({ id: 'mailing-enclose', slot: SLOT.enclose, display: 'Send all documents', factIds: [209] });
	const back = item({ id: 'mailing-return', slot: SLOT.return, display: 'First-class mail', factIds: [239] });
	const courier = item({ id: 'mailing-courier', slot: SLOT.courier, display: '243 Israel Rd SE, Tumwater, WA 98501', factIds: [219] });
	const a = item({ id: 'mailing-usps-v1', slot: SLOT.usps, display: 'PO Box 40228', factIds: [2] });
	const b = item({ id: 'mailing-usps-v2', slot: SLOT.usps, display: 'PO Box 40234', factIds: [3] });
	const usps = item({ id: 'mailing-usps', slot: SLOT.usps, state: 'conflict', factIds: [2, 3], cites: [], alternatives: [a, b] });
	const sections = [
		{ id: 'form', step: 3, heading: 'Fill in the request form', items: [enclose], also: [] },
		{ id: 'send', step: 5, heading: 'Send or bring your documents', items: [usps, courier], also: [] },
		{ id: 'after', step: 6, heading: 'Get it back', items: [back], also: [] },
		{ id: 'contact', step: null, heading: 'Contact the office', items: [phone, option], also: [] }
	];
	const r = sendRepeats(sections);
	assert.deepEqual(
		repeatItems(r).map((i) => [i.id, i.display, i.factIds, i.copy]),
		[
			['mailing-enclose-send', 'Send all documents', [209], false],
			['mailing-return-send', 'First-class mail', [239], false],
			['contacts-phone-conflict', '360-725-0377', [241], false],
			['contacts-phone_option-conflict', 'option 3', [242], false],
			['contacts-phone-courier', '360-725-0377', [241], false]
		]
	);
	// A worded box has no "What you can do"; a PO box by courier has no
	// recipient line; a row of the send step itself is never a repeat.
	const worded = sendRepeats(sections.map((s) => (s.id === 'send' ? { ...s, items: [{ ...usps, state: 'worded' }, { ...courier, display: 'PO Box 40234' }, back] } : s)));
	assert.deepEqual(repeatItems(worded).map((i) => i.id), ['mailing-enclose-send']);
	assert.equal(sendRepeats([{ id: 'fees', step: 4, heading: 'Pay the fees', items: [], also: [] }]), null);
	assert.deepEqual(repeatItems(null), []);
});

// -------------------------------------------------------------- offices

const NOW_MON_10 = new Date('2026-11-09T18:00:00Z'); // Mon Nov 9, 10:00 PT

test('zonedNow reads the wall clock in the office zone', () => {
	assert.deepEqual(zonedNow(NOW_MON_10, 'America/Los_Angeles'), { date: '2026-11-09', day: 'mon', minutes: 600 });
	assert.equal(zonedNow(NOW_MON_10, 'Not/AZone'), null);
});

test('closureDates: instance first, then dates in the text', () => {
	assert.deepEqual(closureDates(item({ instance: '2026_11_11', display: 'Veterans Day' })), ['2026-11-11']);
	assert.deepEqual(closureDates(item({ display: 'Closed Thursday, November 26 and Friday, November 27, 2026' })), ['2026-11-26', '2026-11-27']);
	assert.deepEqual(closureDates(item({ display: 'Closed November 26-27, 2026' })), ['2026-11-26', '2026-11-27']);
	assert.deepEqual(closureDates(item({ display: 'Closed November 11, 2026 for Veterans Day' })), ['2026-11-11']);
	assert.deepEqual(closureDates(item({ display: 'Closed 12/25/2026' })), ['2026-12-25']);
	// No year: the capture's year, rolled forward past the capture day.
	assert.deepEqual(closureDates(item({ display: 'Closed Friday, January 1' })), ['2027-01-01']);
	assert.deepEqual(closureDates(item({ display: 'Closed Wednesday, November 11' })), ['2026-11-11']);
	assert.deepEqual(closureDates(item({ display: 'Closed on state holidays' })), []);
});

test('upcomingClosures: future days only, soonest first, three then the rest', () => {
	const c = (id, display) => item({ id, slot: SLOT.closure, display });
	const list = [
		c('c5', 'Closed December 25, 2026'),
		c('c1', 'Closed November 2, 2026'),
		c('c2', 'Closed November 11, 2026'),
		c('c3', 'Closes at 2:00 p.m. on Friday, November 13, 2026'),
		c('c4', 'Closed November 26 and 27, 2026'),
		c('c6', 'Closed on state holidays')
	];
	const up = upcomingClosures(list, NOW_MON_10);
	assert.equal(up.today, '2026-11-09');
	assert.deepEqual(
		up.next.map((x) => [x.item.id, x.date]),
		[
			['c2', '2026-11-11'],
			['c3', '2026-11-13'],
			['c4', '2026-11-26']
		]
	);
	assert.deepEqual(
		up.later.map((x) => x.item.id),
		['c5']
	);
	assert.deepEqual(
		up.undated.map((x) => x.id),
		['c6']
	);
	assert.equal(dayLabel('2026-11-11', up.today), 'Wed, Nov 11');
	assert.equal(dayLabel('2027-01-01', up.today), 'Fri, Jan 1, 2027');
});

test('cut-off rules and times as printed', () => {
	assert.deepEqual(cutoffRule('Same day service requests must be received at least 30 minutes prior to closing.'), { before: 30 });
	assert.deepEqual(cutoffRule('documents must arrive 30 minutes before closing'), { before: 30 });
	assert.deepEqual(cutoffRule('Arrive one hour before the office closes.'), { before: 60 });
	assert.deepEqual(cutoffRule('Walk-in requests must arrive by 3:30 p.m.'), { at: 930 });
	assert.equal(cutoffRule('Same-day service only'), null);
	assert.deepEqual(timesIn('Mon–Thu 8:00 a.m.–4:00 p.m., Fri 8 am to noon'), [480, 960, 480, 720]);
	assert.equal(clockText(930), '3:30 p.m.');
	assert.equal(clockText(720), 'noon');
	assert.equal(clockText(480), '8:00 a.m.');
});

const TUMWATER = {
	id: 31,
	name: 'Tumwater office',
	hours: [
		{ days: ['mon', 'tue', 'wed', 'thu'], open: '08:00', close: '16:00' },
		{ days: ['fri'], open: '08:00', close: '15:30' }
	],
	timezone: 'America/Los_Angeles',
	timezone_source: 'staff_confirmed',
	address: { city: 'Tumwater' },
	fact_ids: [4]
};
const CHENEY = {
	...TUMWATER,
	id: 32,
	name: 'Cheney office',
	address: { city: 'Cheney' },
	hours: [{ days: ['mon', 'tue', 'wed', 'fri'], open: '09:00', close: '15:00' }]
};
const CUTOFF = item({ id: 'walkin-cutoff', slot: SLOT.cutoff, factIds: [6], display: 'Same day service requests must be received at least 30 minutes prior to closing.' });
const CLOSURES = [
	item({ id: 'c-nov11', slot: SLOT.closure, factIds: [8], display: 'Closed Wednesday, November 11, 2026 (Veterans Day)' }),
	item({ id: 'c-nov13', slot: SLOT.closure, factIds: [10], display: 'The office closes at 2:00 p.m. on Friday, November 13, 2026.' })
];

test('liveStatus (D4): Monday 10:00 PT, Tumwater cut-off 3:30 p.m.', () => {
	const s = liveStatus({ office: TUMWATER, closures: CLOSURES, cutoff: CUTOFF, now: NOW_MON_10 });
	assert.equal(s?.open, true);
	assert.equal(s?.text, 'Open now · closes 4:00 p.m.');
	assert.equal(s?.cutoff, '3:30 p.m.');
	assert.deepEqual(
		s?.inputs.map((i) => i.id),
		['walkin-cutoff']
	);
});

test('liveStatus (D4): Thursday at Cheney is closed, opening Friday', () => {
	const thu = new Date('2026-11-12T18:00:00Z');
	const s = liveStatus({ office: CHENEY, closures: CLOSURES, cutoff: CUTOFF, now: thu });
	assert.equal(s?.open, false);
	assert.equal(s?.text, 'Closed now · opens tomorrow at 9:00 a.m.');
});

test('liveStatus (D4): Nov 11 closed; Nov 13 closes early with cut-off 1:30 p.m.', () => {
	const nov11 = liveStatus({ office: TUMWATER, closures: CLOSURES, cutoff: CUTOFF, now: new Date('2026-11-11T18:00:00Z') });
	assert.equal(nov11?.text, 'Closed today');
	assert.equal(nov11?.closure?.id, 'c-nov11');
	const nov13 = liveStatus({ office: TUMWATER, closures: CLOSURES, cutoff: CUTOFF, now: new Date('2026-11-13T18:00:00Z') });
	assert.equal(nov13?.open, true);
	assert.equal(nov13?.text, 'Open now · closes 2:00 p.m.');
	assert.equal(nov13?.cutoff, '1:30 p.m.');
	assert.deepEqual(
		nov13?.inputs.map((i) => i.id),
		['walkin-cutoff', 'c-nov13']
	);
	// Tuesday evening Nov 10: the next opening skips the Nov 11 closure.
	const tueEve = liveStatus({ office: TUMWATER, closures: CLOSURES, cutoff: CUTOFF, now: new Date('2026-11-11T02:00:00Z') });
	assert.equal(tueEve?.text, 'Closed now · opens Thu, Nov 12 at 8:00 a.m.');
});

test('liveStatus needs a confirmed timezone', () => {
	assert.equal(liveStatus({ office: { ...TUMWATER, timezone_source: 'model' }, now: NOW_MON_10 }), null);
	assert.equal(liveStatus({ office: { ...TUMWATER, timezone: null }, now: NOW_MON_10 }), null);
	assert.equal(liveStatus({ office: { ...TUMWATER, hours: [] }, now: NOW_MON_10 }), null);
});

test('office names', () => {
	assert.equal(instanceName('tumwater'), 'Tumwater');
	assert.equal(instanceName('new_york_city'), 'New York City');
	assert.equal(sameOffice(TUMWATER, 'tumwater'), true);
	assert.equal(sameOffice(TUMWATER, 'cheney'), false);
});

test('closureDates: a range names every day in it; a reopening closes nothing', () => {
	assert.deepEqual(closureDates(item({ display: 'Closed November 26 – December 1, 2026' })), [
		'2026-11-26',
		'2026-11-27',
		'2026-11-28',
		'2026-11-29',
		'2026-11-30',
		'2026-12-01'
	]);
	// Without a capture, the range's printed year serves both ends.
	assert.equal(closureDates(item({ display: 'Closed November 26 – December 1, 2026', cites: [] })).length, 6);
	assert.deepEqual(closureDates(item({ display: 'Closed November 23-27, 2026' })), [
		'2026-11-23',
		'2026-11-24',
		'2026-11-25',
		'2026-11-26',
		'2026-11-27'
	]);
	assert.deepEqual(closureDates(item({ display: 'Closed December 30, 2026 through January 1, 2027' })), [
		'2026-12-30',
		'2026-12-31',
		'2027-01-01'
	]);
	assert.deepEqual(closureDates(item({ display: 'Closed 11/26/2026 - 11/28/2026' })), ['2026-11-26', '2026-11-27', '2026-11-28']);
	assert.deepEqual(closureDates(item({ display: 'Closed November 11, 2026. Reopens at 8:00 a.m. on November 12, 2026.' })), ['2026-11-11']);
	// "until 2 p.m." is a time, not a day.
	assert.deepEqual(closureDates(item({ display: 'Closed November 26 until 2 p.m.' })), ['2026-11-26']);
	// A capture at 10 p.m. Pacific on Dec 31 dates a yearless closure in that year.
	const late = item({ display: 'Closed Thursday, December 31', cites: [{ source: { ...SRC, capturedAt: '2027-01-01T06:00:00Z' }, quote: 'q', summary: null, factId: 1 }] });
	assert.deepEqual(closureDates(late), ['2026-12-31']);
	assert.equal(zonedDay('2027-01-01T06:00:00Z'), '2026-12-31');
	assert.equal(zonedDay('2026-11-02'), '2026-11-02');
	assert.equal(zonedDay('soon'), null);
});

test('closureOn ties a closing time to the date it sits next to, or says it is unclear', () => {
	const thanksgiving = item({ display: 'Closed November 26 and 27, 2026; closes at 3:00 p.m. on November 25, 2026.' });
	assert.deepEqual(closureOn(thanksgiving, '2026-11-26'), { allDay: true });
	assert.deepEqual(closureOn(thanksgiving, '2026-11-25'), { closesAt: 900 });
	assert.equal(closureOn(thanksgiving, '2026-11-24'), null);
	// One clause, two dates and a time: which day closes early?
	assert.deepEqual(closureOn(item({ display: 'Closed November 26, closes at noon November 25.' }), '2026-11-25'), { unclear: true });
	// Two times: a closed window, not a closing time.
	assert.deepEqual(
		closureOn(item({ display: 'The office will be closed on November 25, 2026 from 8:00 a.m. to 10:00 a.m. for training.' }), '2026-11-25'),
		{ unclear: true }
	);
	// A dated instance whose text names no date: its time is that day's.
	assert.deepEqual(closureOn(item({ instance: '2026_11_13', display: 'The office closes at 2:00 p.m.' }), '2026-11-13'), { closesAt: 840 });
});

test('liveStatus (D4): a closure that names the other office leaves this one open', () => {
	const fri10 = new Date('2026-11-13T18:00:00Z');
	const cheneyOnly = item({ id: 'c-cheney', slot: SLOT.closure, factIds: [12], display: 'The Cheney office is closed Friday, November 13, 2026.' });
	const tumwater = liveStatus({ office: TUMWATER, closures: [cheneyOnly], cutoff: CUTOFF, now: fri10, others: [CHENEY] });
	assert.equal(tumwater?.text, 'Open now · closes 3:30 p.m.');
	assert.deepEqual(
		tumwater?.inputs.map((i) => i.id),
		['walkin-cutoff']
	);
	assert.equal(liveStatus({ office: CHENEY, closures: [cheneyOnly], cutoff: CUTOFF, now: fri10, others: [TUMWATER] })?.text, 'Closed today');
	// The other office known only by its name on the page.
	assert.equal(liveStatus({ office: TUMWATER, closures: [cheneyOnly], cutoff: CUTOFF, now: fri10, others: ['Cheney'] })?.open, true);
	// Every office, even when it also names one.
	const everyOffice = item({ id: 'c-all', slot: SLOT.closure, factIds: [14], display: 'All offices are closed Friday, November 13, 2026, including Cheney.' });
	assert.equal(liveStatus({ office: TUMWATER, closures: [everyOffice], cutoff: CUTOFF, now: fri10, others: [CHENEY] })?.text, 'Closed today');
	assert.equal(closureApplies(cheneyOnly, ['Tumwater'], ['Cheney']), false);
	assert.equal(closureApplies(cheneyOnly, ['Tumwater'], []), true);
});

test('liveStatus (D4): an early close on one date of a multi-date closure', () => {
	const thanksgiving = item({ id: 'c-thanks', slot: SLOT.closure, factIds: [16], display: 'Closed November 26 and 27, 2026; closes at 3:00 p.m. on November 25, 2026.' });
	const at = (iso) => liveStatus({ office: TUMWATER, closures: [thanksgiving], cutoff: CUTOFF, now: new Date(iso) });
	assert.equal(at('2026-11-26T18:00:00Z')?.text, 'Closed today');
	const wed = at('2026-11-25T18:00:00Z');
	assert.equal(wed?.text, 'Open now · closes 3:00 p.m.');
	assert.equal(wed?.cutoff, '2:30 p.m.');
	// Wednesday evening: Thursday and Friday are closed, then the weekend.
	assert.equal(at('2026-11-26T02:00:00Z')?.text, 'Closed now · opens Mon, Nov 30 at 8:00 a.m.');
	// A range closes every day in it.
	const week = item({ id: 'c-week', slot: SLOT.closure, factIds: [18], display: 'Closed November 26 – December 1, 2026' });
	assert.equal(liveStatus({ office: TUMWATER, closures: [week], cutoff: CUTOFF, now: new Date('2026-11-30T18:00:00Z') })?.text, 'Closed today');
	// Unclear today: nothing is calculated.
	const unclear = item({ id: 'c-unclear', slot: SLOT.closure, factIds: [20], display: 'Closed November 26, closes at noon November 25.' });
	assert.equal(liveStatus({ office: TUMWATER, closures: [unclear], cutoff: CUTOFF, now: new Date('2026-11-25T18:00:00Z') }), null);
});

test('splitOffices sorts office groups and office records by shape', () => {
	const group = { instance: 'tumwater', name: null, items: [item({ slot: SLOT.office })] };
	const { groups, records } = splitOffices([group, TUMWATER, { id: 'x' }, null, { instance: 'cheney' }]);
	assert.deepEqual(groups.map((g) => g.instance), ['tumwater']);
	assert.deepEqual(records.map((r) => r.id), [31]);
	assert.deepEqual(splitOffices(null), { groups: [], records: [] });
});

// -------------------------------------------------------------- updates

test('updates: newest first; a diff past the line limit collapses', () => {
	const notice = item({ id: 'service_notice-notice-1102', slot: 'service_notice.notice', cites: [{ source: { ...SRC, capturedAt: '2026-11-02T20:00:00Z', acceptedAt: null }, quote: 'q', summary: null, factId: 1 }] });
	const small = { sourceUrl: 'https://www.sos.wa.gov/a', date: '2026-11-03', since: '2026-10-31', added: ['New line'], removed: [], more: 0 };
	const big = { sourceUrl: 'https://www.sos.wa.gov/b', date: '2026-10-30', since: '2026-10-01', added: Array(12).fill('x'), removed: Array(12).fill('y'), more: DEGENERATE_LINES - 20 };
	const entries = updateEntries({ items: [notice], panels: [big, small] });
	assert.deepEqual(
		entries.map((e) => [e.kind, e.date]),
		[
			['change', '2026-11-03'],
			['notice', '2026-11-02'],
			['change', '2026-10-30']
		]
	);
	const last = entries[2];
	assert.equal(last.kind === 'change' && last.degenerate, true);
	assert.equal(entries[0].kind === 'change' && entries[0].degenerate, false);
	assert.equal(itemDay(notice), '2026-11-02');
	assert.equal(shortDay('2026-11-03'), 'Nov 3, 2026');
	assert.deepEqual(updateEntries({}), []);
});

test('updates: Pacific days, and the notice’s own date first', () => {
	const cite = (over) => ({ source: { ...SRC, capturedAt: null, acceptedAt: null, ...over }, quote: 'q', summary: null, factId: 1 });
	// Approved Nov 4 at 4:53 p.m. Pacific (Nov 5 in UTC).
	assert.equal(itemDay(item({ display: 'Online services are back.', cites: [cite({ acceptedAt: 1793840000 })] })), '2026-11-04');
	// Read at 10:30 p.m. Pacific on Nov 4.
	assert.equal(itemDay(item({ display: 'Online services are back.', cites: [cite({ capturedAt: '2026-11-05T06:30:00Z', acceptedAt: 1793840000 })] })), '2026-11-04');
	// The day the notice states, before the day we read or approved it.
	const restored = item({ display: 'Service was restored on November 3, 2026 at 12:01 p.m.', cites: [cite({ capturedAt: '2026-11-04T18:00:00Z', acceptedAt: 1793840000 })] });
	assert.equal(itemDay(restored), '2026-11-03');
	// A date ahead of the reading is a plan, not the notice's day.
	const planned = item({ display: 'We expect to restore service on November 6.', cites: [cite({ capturedAt: '2026-11-02T20:00:00Z' })] });
	assert.equal(itemDay(planned), '2026-11-02');
	// The first reading, not the newest.
	const twice = item({ display: 'Online services are down.', cites: [cite({ capturedAt: '2026-11-03T18:00:00Z' }), cite({ capturedAt: '2026-11-02T18:00:00Z' })] });
	assert.equal(itemDay(twice), '2026-11-02');
	assert.equal(itemDay(item({ instance: '2026_11_03', display: 'Restored.' })), '2026-11-03');
	// A change panel read at 9 p.m. Pacific is that day's.
	assert.equal(panelDay({ sourceUrl: 'https://www.sos.wa.gov/a', date: '2026-11-04', capturedAt: '2026-11-04T05:00:00Z', since: '', added: ['x'], removed: [], more: 0 }), '2026-11-03');
	assert.equal(panelDay({ sourceUrl: 'https://www.sos.wa.gov/a', date: '2026-11-04', since: '', added: ['x'], removed: [], more: 0 }), '2026-11-04');
});

// ------------------------------------------------------------ copy rules

// Vendor model names are spelled in halves: the repo's model gate
// (`cargo xtask model-gate`) refuses any tracked line that names one.
const VENDOR = ['gem' + 'ini', 'clau' + 'de-'].join('|');
const BANNED = new RegExp(String.raw`\b(?:dossier|slot|topic|fact|placeholder|not wired|skeleton|lorem|todo|coming soon|${VENDOR})\b`, 'i');

// Attributes whose values are code (ids, classes, links, data hooks),
// and attributes a person reads or hears.
const CODE_ATTRS = new Set([
	'class', 'id', 'for', 'href', 'src', 'name', 'type', 'value', 'role', 'rel', 'target', 'datetime', 'style',
	'pattern', 'inputmode', 'autocomplete', 'aria-controls', 'aria-labelledby', 'aria-describedby', 'aria-live',
	'aria-hidden', 'aria-pressed', 'aria-disabled', 'aria-current', 'aria-expanded', 'tabindex', 'key'
]);
const HUMAN_ATTRS = new Set(['aria-label', 'aria-description', 'aria-roledescription', 'aria-valuetext', 'title', 'alt', 'placeholder']);

// A Tailwind class list: lowercase utility-shaped tokens, at least one
// with a utility's marks (a digit, a variant colon, brackets, a slash,
// a known prefix or bare utility).
const UTILITY = /^!?(?:[a-z0-9-]+(?:\[[^\]]*\])?:)*-?[a-z][a-z0-9.-]*(?:\/\d+)?(?:-\[[^\]]*\]|-\([^)]*\))?$|^\[[^\]]+\]$/;
const UTILITY_MARK =
	/[\d:[\]/]|^(?:flex|grid|block|inline|hidden|contents|relative|absolute|fixed|sticky|static|truncate|underline|uppercase|italic|border|rounded|shadow|transition|sr-only|not-italic|no-underline|line-through|group|peer)$|^(?:text|bg|border|rounded|font|gap|p[xytrbl]?|m[xytrbl]?|w|h|min|max|size|inline|items|justify|leading|tracking|flex|grid|col|row|list|break|whitespace|overflow|shadow|ring|outline|opacity|cursor|z|top|left|right|bottom|inset|translate|transition|duration|ease|animate|motion|aria|has|group|peer|self|place|content|order|basis|decoration|scroll|snap|align|fill|stroke|sr)-/;
const classList = (t) => {
	const tokens = t.trim().split(/\s+/);
	return tokens.every((w) => UTILITY.test(w)) && tokens.some((w) => UTILITY_MARK.test(w));
};
// Prose: two or more plain words, and not a class list.
const PLAIN_WORD = /^[("“‘]?[A-Za-z][A-Za-z'’-]*[)"”’.,;:!?]*$/;
const prose = (t) => t.trim().split(/\s+/).filter((w) => PLAIN_WORD.test(w)).length >= 2 && !classList(t);

/**
 * The static text of a string literal or template literal (its
 * computed parts left out), or null for anything else.
 * @param {any} node
 * @returns {string|null}
 */
function literalText(node) {
	if (node?.type === 'Literal' && typeof node.value === 'string' && !node.regex) return node.value;
	if (node?.type === 'TemplateLiteral') return node.quasis.map((q) => q.value.cooked ?? q.value.raw).join(' ');
	return null;
}

/**
 * Every string a component or helper can put in front of a reader:
 * template text, the values of read-aloud attributes, and string
 * literals in markup expressions and scripts that read as prose.
 * Computed parts of template literals are left out; import paths,
 * object keys, regular expressions and class lists are code.
 * @param {any} root an AST (svelte/compiler modern, or ESTree)
 * @returns {{ text: string, where: string }[]}
 */
function copyStrings(root) {
	/** @type {{ text: string, where: string }[]} */
	const out = [];
	const SKIP = new Set(['start', 'end', 'loc', 'range', 'metadata', 'css', 'comments', 'options', 'leadingComments', 'trailingComments']);
	/**
	 * Modes: 'text' inside markup (template text is always copy; string
	 * literals in its expressions when they read as prose), 'all' inside
	 * a read-aloud attribute (everything), 'prose' elsewhere.
	 * @param {any} node
	 * @param {'text'|'all'|'prose'} mode
	 */
	const visit = (node, mode) => {
		if (!node || typeof node !== 'object') return;
		if (Array.isArray(node)) {
			for (const n of node) visit(n, mode);
			return;
		}
		switch (node.type) {
			case 'ImportDeclaration':
			case 'ExportAllDeclaration':
			case 'ClassDirective':
			case 'StyleDirective':
			case 'BindDirective':
			case 'OnDirective':
				return;
			case 'Text':
				if (mode !== 'prose' || prose(node.data)) out.push({ text: node.data, where: 'text' });
				return;
			case 'Attribute': {
				const name = String(node.name);
				if (CODE_ATTRS.has(name) || name.startsWith('data-') || name.startsWith('on')) return;
				visit(node.value, HUMAN_ATTRS.has(name) ? 'all' : 'prose');
				return;
			}
			case 'Property':
				if (node.computed) visit(node.key, mode);
				visit(node.value, mode);
				return;
			default: {
				const text = literalText(node);
				if (text !== null) {
					if (mode === 'all' || prose(text)) out.push({ text, where: node.type });
					if (node.type === 'Literal') return;
				}
			}
		}
		for (const [key, value] of Object.entries(node)) {
			if (SKIP.has(key) || (node.type === 'TemplateLiteral' && key === 'quasis')) continue;
			const markup = key === 'fragment' || (node.type === 'Fragment' && key === 'nodes');
			visit(value, markup ? 'text' : mode === 'text' ? 'prose' : mode);
		}
	};
	visit(root, 'prose');
	return out;
}

/** Every Svelte component and helper module of the money and send blocks. */
function sources() {
	const files = readdirSync(DOC)
		.filter((n) => n.endsWith('.svelte') || n.endsWith('.js'))
		.map((n) => join(DOC, n));
	return [...files, join(PUBLIC_UI, 'fees.js')];
}

test('template copy in the money and send blocks: no digits, $, dates; no banned words', () => {
	let checked = 0;
	for (const file of sources()) {
		const src = readFileSync(file, 'utf8');
		const ast = file.endsWith('.svelte')
			? parseSvelte(src, { modern: true })
			: parseJs(src, { ecmaVersion: 'latest', sourceType: 'module' });
		for (const { text, where } of copyStrings(file.endsWith('.svelte') ? { instance: ast.instance, module: ast.module, fragment: ast.fragment } : ast)) {
			checked += 1;
			const name = `${file.slice(PUBLIC_UI.length + 1)} (${where}): ${JSON.stringify(text.trim().slice(0, 80))}`;
			assert.doesNotMatch(text, /[\d$]/, `${name}: digits or $ in template copy`);
			assert.doesNotMatch(text, BANNED, `${name}: banned word in template copy`);
		}
	}
	assert.ok(checked > 100, `only ${checked} strings linted`);
});

test('the copy lint reaches expressions, read-aloud attributes and scripts', () => {
	const ast = parseSvelte(
		`<script lang="ts">const a: string = 'Pay within 30 days'; const cls = 'm-0 mt-1 text-ink'; const id = 'fees-estimate';</script>` +
			`<p aria-label={\`Open \${x} on 2 maps\`} class="mt-2" data-x="a b 3">Hi {x ? 'Yes it costs $5' : \`\${n} more changed lines\`}</p>`,
		{ modern: true }
	);
	const texts = copyStrings({ instance: ast.instance, module: ast.module, fragment: ast.fragment }).map((c) => c.text.trim());
	assert.ok(texts.includes('Pay within 30 days'));
	assert.ok(texts.includes('Open   on 2 maps'));
	assert.ok(texts.includes('Yes it costs $5'));
	assert.ok(texts.includes('more changed lines'));
	assert.ok(!texts.includes('m-0 mt-1 text-ink'));
	assert.ok(!texts.some((t) => t.includes('fees-estimate') || t.includes('a b 3')));
});
