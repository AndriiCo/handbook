// WA value checklist (docs/test-plans/wa-checklist.json) and the
// read-only metrics script that measures the guide against it
// (web/scripts/dossier-metrics.mjs). Fixtures are synthetic; postal
// values use the canonical fixture boxes 40220/40234.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
	aliasResolver,
	analyse,
	classifyConflicts,
	datesIn,
	report,
	resolveFact,
	splitKey,
	uspsDisagree
} from '../scripts/dossier-metrics.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const DOCS = join(HERE, '..', '..', 'docs');
const CHECKLIST = JSON.parse(readFileSync(join(DOCS, 'test-plans', 'wa-checklist.json'), 'utf8'));
const TOPICS = [...readFileSync(join(DOCS, 'specs', 'TOPICS.md'), 'utf8').matchAll(/^\| `([a-z_]+)` \|/gm)].map((m) => m[1]);

// Plan section 9.3's vocabulary, as slots.json ids (plus R1's
// jurisdiction.refusals): every checklist slot not marked
// `vocabulary: false` is one of these.
const VOCABULARY = new Set([
	'fees.per_document', 'fees.expedite', 'fees.same_day', 'fees.refund',
	'payment.payee', 'payment.methods_counter',
	'mailing.usps', 'mailing.courier', 'mailing.courier_rule', 'mailing.enclose', 'mailing.return',
	'address.office', 'hours.weekday',
	'walkin.cutoff', 'walkin.same_day_only', 'walkin.no_mail', 'walkin.public_record',
	'holiday_hours.closure',
	'processing_time.standard', 'processing_time.expedited', 'processing_time.same_day',
	'contacts.phone', 'contacts.phone_option', 'contacts.phone_hours', 'contacts.email', 'contacts.chat', 'contacts.tty', 'contacts.no_texts',
	'forms.form', 'forms.revision',
	'authentication.product', 'authentication.domestic_validity',
	'jurisdiction.scope', 'jurisdiction.federal_route', 'jurisdiction.refusals',
	'notarization.wa_notary', 'copies.vital_records', 'website.verify',
	'service_notice.notice', 'service_notice.restored'
]);
const SECTIONS = new Set(['status', 'before', 'prepare', 'form', 'fees', 'send-mail', 'send-courier', 'visit', 'after', 'contact']);
const NOW = Date.parse('2026-11-04T06:00:00Z');

let nextId = 1;
/** One public fact; `quote` and `url` make its citation. */
function fact(topic, statement, over = {}) {
	const { url = CHECKLIST.sources.HUB, quote = statement.replace(/^[a-z_]+: /, ''), ...rest } = over;
	return {
		id: nextId++,
		topic,
		statement,
		status: 'accepted',
		conflict: false,
		conflicts: [],
		citations: [{ quote, source_url: url, captured_at: '2026-11-03T10:00:00Z' }],
		...rest
	};
}

/** Flag `a` and `b` against each other, as the server does. */
function clash(...facts) {
	for (const f of facts) {
		f.conflict = true;
		f.conflicts = facts.filter((g) => g !== f).map((g) => ({ id: g.id, statement: g.statement, status: 'accepted' }));
	}
	return facts;
}

const run = (facts, plan = null) => analyse({ facts, plan, checklist: CHECKLIST, now: NOW });
const value = (r, id) => r.values.find((v) => v.id === id);

test('checklist: 74 values, unique ids, known sections, topics and sources', () => {
	assert.equal(CHECKLIST.values.length, 74);
	assert.equal(new Set(CHECKLIST.values.map((v) => v.id)).size, 74);
	assert.equal(TOPICS.length, 26, 'TOPICS.md lists the 26 frozen topics');
	for (const v of CHECKLIST.values) {
		assert.match(v.id, /^[a-z0-9-]+$/, v.id);
		assert.ok(SECTIONS.has(v.section), `${v.id}: section ${v.section}`);
		assert.equal(typeof v.label, 'string', v.id);
		assert.equal(typeof v.critical, 'boolean', `${v.id}: critical`);
		for (const alias of [v.source, ...(v.also ?? []), ...(v.match ? [v.match.source] : [])]) {
			assert.ok(CHECKLIST.sources[alias], `${v.id}: source alias ${alias}`);
		}
		// The authority row is pack data: it may be shown, never counted.
		if (v.plan !== undefined) assert.ok(!/^authority(\.|$)/.test(v.plan), `${v.id}: plan ${v.plan} is the authority row; use shown`);
		if (v.shown !== undefined) assert.match(v.shown, /^authority\.[a-z_]+$/, `${v.id}: shown`);
		if (v.slot === null) {
			assert.ok(v.plan || v.computed_from, `${v.id}: a value without a slot needs plan or computed_from`);
			continue;
		}
		assert.match(v.slot, /^[a-z_]+\.[a-z_]+$/, v.id);
		assert.ok(TOPICS.includes(v.slot.split('.')[0]), `${v.id}: topic of ${v.slot}`);
		assert.equal(VOCABULARY.has(v.slot), v.vocabulary !== false, `${v.id}: vocabulary flag for ${v.slot}`);
	}
	for (const s of Object.values(CHECKLIST.sources)) assert.match(s, /^https:\/\//);
});

test('checklist: one critical value per critical slot of the us_state profile', () => {
	const critical = CHECKLIST.values.filter((v) => v.critical);
	assert.deepEqual(critical.map((v) => v.slot).sort(), [...CHECKLIST.critical_slots].sort());
	assert.deepEqual(
		[...CHECKLIST.critical_slots].sort(),
		['fees.per_document', 'forms.form', 'mailing.usps', 'payment.payee', 'processing_time.standard']
	);
});

test('checklist: each key lands in its own slot, computed values name real inputs', () => {
	const sample = (key) => key.replace('<id>', '2026_11_02').replace('<yyyy_mm_dd>', '2026_11_11');
	for (const v of CHECKLIST.values) {
		if (v.computed_from) {
			for (const id of v.computed_from) {
				const input = CHECKLIST.values.find((x) => x.id === id);
				assert.ok(input && !input.computed_from, `${v.id}: input ${id}`);
			}
		}
		if (v.slot === null) continue;
		const hit = resolveFact(v.slot.split('.')[0], sample(v.key));
		assert.equal(hit.slot, v.slot, `${v.id}: key ${v.key}`);
		if (v.instance && !v.unscoped_ok) assert.equal(hit.instance, v.instance, `${v.id}: instance`);
	}
	assert.equal(CHECKLIST.closures.length, 6);
	for (const c of CHECKLIST.closures) assert.equal(c.instance, c.date.replaceAll('-', '_'));
});

test('slots: production v1 keys map through the aliases, canonical keys by convention', () => {
	assert.deepEqual(aliasResolver('address', 'apostille_main_branch_street'), { slot: 'address.office', instance: 'tumwater', via: 'registry' });
	assert.deepEqual(aliasResolver('address', 'apostille_main_branch_street__cheney'), { slot: 'address.office', instance: 'cheney', via: 'registry' });
	assert.equal(aliasResolver('mailing', 'apostille_mailing_address').slot, 'mailing.usps');
	assert.equal(aliasResolver('po_box', 'po_box').slot, 'mailing.usps');
	assert.equal(aliasResolver('fees', 'same_day_inperson_fee').slot, 'fees.same_day');
	assert.equal(aliasResolver('fees', 'standard_fee').slot, 'fees.per_document');
	assert.equal(aliasResolver('forms', 'request_form_name').slot, 'forms.form');
	assert.equal(aliasResolver('contacts', 'apostille_phone').slot, 'contacts.phone');
	assert.equal(aliasResolver('processing_time', 'standard_time').slot, 'processing_time.standard');
	assert.equal(aliasResolver('fees', 'constructor'), null, 'no prototype keys');
	assert.deepEqual(resolveFact('hours', 'weekday__cheney'), { slot: 'hours.weekday', instance: 'cheney', via: 'convention' });
	assert.deepEqual(resolveFact('holiday_hours', 'closure__2026_11_11'), { slot: 'holiday_hours.closure', instance: '2026_11_11', via: 'convention' });
	assert.deepEqual(splitKey('weekday__Cheney'), { base: 'weekday__Cheney', instance: null });
});

test('conflicts: flags between different keys are false; same key, unkeyed and postal boxes are real', () => {
	const tumwater = fact('address', 'apostille_main_branch_street: Synthetic Rd, Tumwater');
	const cheney = fact('address', 'apostille_eastern_branch_street: Synthetic St, Cheney');
	const boxA = fact('mailing', 'apostille_mailing_address: PO Box 40220 Olympia, WA 98504-0220');
	const boxB = fact('mailing', 'apostille_mailing_address: PO Box 40234 Olympia, WA 98504-0234');
	const crossKey = fact('mailing', 'mail_destination: PO BOX 40220 Olympia, WA 98504-0234');
	const unkeyed = fact('fees', 'Eight dollars per document.');
	const keyed = fact('fees', 'per_document: Fifteen dollars per document.');
	const lonely = fact('hours', 'weekday: Nine to five', { conflict: true, conflicts: [] });
	clash(tumwater, cheney);
	clash(boxA, boxB, crossKey);
	clash(unkeyed, keyed);
	const c = classifyConflicts([tumwater, cheney, boxA, boxB, crossKey, unkeyed, keyed, lonely]);
	assert.deepEqual(c.falseFacts, [tumwater.id, cheney.id]);
	assert.deepEqual(c.realFacts, [boxA.id, boxB.id, crossKey.id, unkeyed.id, keyed.id, lonely.id]);
	assert.equal(c.pairs.length, 5);
	assert.deepEqual(
		c.pairs.find((p) => p.a === tumwater.id),
		{ a: tumwater.id, b: cheney.id, keyA: 'apostille_main_branch_street', keyB: 'apostille_eastern_branch_street', topic: 'address', real: false }
	);
	assert.ok(uspsDisagree('PO Box 40220 Olympia, WA 98504-0220', 'P.O. Box 40220, Olympia WA 98504-0234'), 'ZIP+4 differs');
	assert.ok(!uspsDisagree('PO Box 40220 Olympia, WA 98504-0220', 'PO BOX 40220 Olympia'), 'same box, one ZIP+4');
	assert.ok(!uspsDisagree('PO Box 40220', 'Send all documents together'), 'no box on one side');
});

test('coverage: production-shaped facts, false conflicts do not block critical slots', () => {
	const facts = [
		fact('address', 'apostille_main_branch_street: Synthetic Rd SE, Tumwater, WA 98501'),
		fact('mailing', 'apostille_mailing_address: PO Box 40234 Olympia, WA 98504-0234'),
		fact('contacts', 'apostille_phone: 360-555-0100'),
		fact('address', 'apostille_eastern_branch_street: Synthetic St, Cheney, WA 99004'),
		fact('authentication', 'domestic_validity: Not valid inside the United States.'),
		fact('mailing', 'submission_requirement: Send all documents with Request form and payment:'),
		fact('forms', 'request_form_name: Synthetic Request Form (PDF Fillable Form)'),
		fact('fees', 'same_day_fee: We charge a $150 same-day service fee'),
		fact('forms', 'request_form: Synthetic Request (Fillable Form)'),
		fact('payment', 'payee: Make check or money order payable to: Secretary of State'),
		fact('fees', 'standard_fee: Standard processing: Fee is $15.00 per document.'),
		fact('processing_time', 'standard_time: Typically processed 7 to 10 business days'),
		fact('fees', 'expedited_fee: An additional $100 expedite fee per 10 documents.'),
		fact('fees', 'same_day_inperson_fee: Same day services in-person: Fee is $15.00 per document'),
		fact('phone_script', 'ask: Which PO box takes apostille mail?'),
		fact('fees', 'refund: Fees are non-refundable.', { status: 'proposed' })
	];
	const [tumwater, usps, , cheney, , enclose, formA, sameDay, formB, , standard, , expedite, inPerson] = facts;
	clash(tumwater, cheney);
	clash(usps, enclose);
	clash(formA, formB);
	clash(sameDay, standard, expedite, inPerson);
	const r = run(facts);
	assert.equal(r.facts.public, 14, 'call scripts and proposed facts are not on the page');
	assert.equal(r.coverage.covered, 15);
	assert.equal(r.coverage.byFacts, 13);
	assert.equal(r.coverage.byComputed, 2, 'map links from the two office addresses');
	assert.equal(r.critical.covered, 5);
	assert.equal(r.conflicts.false, 10);
	assert.equal(r.conflicts.real, 0);
	assert.equal(value(r, 'visit-cheney-same-day-only').state, 'missing', 'a fee fact is not the Cheney rule');
	assert.equal(value(r, 'contact-phone').state, 'covered');
	assert.equal(value(r, 'send-courier-phone').state, 'covered');
	assert.equal(value(r, 'fees-refund').state, 'missing');
	assert.match(report({ ...r, slug: 'usa-wa-sos', base: 'https://example.test', at: '2026-11-04T06:00:00Z', registry: 'embedded aliases' }), /^Coverage: 15\/74 values/m);
});

test('coverage: a disputed USPS box is not covered until a staff decision picks one', () => {
	const boxes = clash(
		fact('mailing', 'apostille_mailing_address: PO Box 40220 Olympia, WA 98504-0220', { url: CHECKLIST.sources.OSC }),
		fact('mailing', 'apostille_mailing_address: PO Box 40234 Olympia, WA 98504-0234', { url: CHECKLIST.sources.CCC }),
		fact('mailing', 'apostille_mailing_address: PO Box 40220 Olympia, WA 98504-0234', { url: CHECKLIST.sources.FORM })
	);
	const open = run(boxes);
	assert.equal(value(open, 'send-mail-usps').state, 'conflict');
	assert.deepEqual(open.critical.slots.find((s) => s.slot === 'mailing.usps').against.sort(), boxes.map((b) => b.id).sort());
	assert.equal(value(open, 'send-mail-form-address').state, 'covered', 'not critical: every variant is listed');
	assert.equal(open.conflicts.real, 3);

	const decided = run(boxes, { evidence: [{ id: 5, kind: 'decision', preferred_fact_id: boxes[1].id, slot: 'mailing.usps' }] });
	assert.equal(value(decided, 'send-mail-usps').state, 'covered');
	assert.equal(value(decided, 'send-mail-usps').by, 'decision');
	assert.equal(decided.coverage.byDecision, 1);
});

test('coverage: cited page, one rule for both offices, /plan rows', () => {
	const formOnly = run([fact('forms', 'revision: Revised 6.2025', { url: CHECKLIST.sources.HUB })]);
	assert.equal(value(formOnly, 'form-revision').state, 'missing', 'the revision must come from the form itself');
	const fromForm = run([fact('forms', 'revision: Revised 6.2025', { url: CHECKLIST.sources.FORM.replaceAll('%20', ' ') })]);
	assert.equal(value(fromForm, 'form-revision').state, 'covered');

	const both = run([fact('walkin', 'cutoff: Documents must be received 30 minutes prior to closing.')]);
	assert.equal(value(both, 'visit-tumwater-cutoff').state, 'covered');
	assert.equal(value(both, 'visit-cheney-cutoff').state, 'covered');
	const cheneyOnly = run([fact('walkin', 'cutoff__cheney: Documents must be received 30 minutes prior to closing.')]);
	assert.equal(value(cheneyOnly, 'visit-tumwater-cutoff').state, 'missing');
	assert.equal(value(cheneyOnly, 'visit-cheney-cutoff').state, 'covered');
	const hoursNoOffice = run([fact('hours', 'weekday: Monday-Thursday 8 a.m. to 4 p.m.')]);
	assert.equal(value(hoursNoOffice, 'visit-tumwater-hours').state, 'missing', 'office hours need their office');

	// An accepted form map counts; the authority row (pack data, no
	// citation) is only "shown, not cited".
	const plan = run([], { authority: { name: 'Synthetic Office', official_url: 'https://www.sos.wa.gov/' }, forms: [{ id: 1 }], evidence: [] });
	assert.equal(value(plan, 'form-fields').by, 'plan');
	assert.equal(plan.coverage.covered, 1);
	assert.equal(plan.coverage.byPlan, 1);
	assert.equal(value(plan, 'before-official-site').state, 'shown');
	assert.equal(value(plan, 'before-issuer').state, 'shown');
	assert.equal(plan.coverage.shown, 2);
	assert.deepEqual(plan.coverage.shownIds, ['before-issuer', 'before-official-site']);
	const text = report({ ...plan, slug: 'usa-wa-sos', base: 'https://example.test', at: '2026-11-04T06:00:00Z', registry: 'embedded aliases' });
	assert.match(text, /^Coverage: 1\/74 values \(0 from facts, 0 by staff decision, 0 computed, 1 from \/plan\)/m);
	assert.match(text, /missing: before-issuer \(shown, not cited\), before-official-site \(shown, not cited\),/);
	assert.match(text, /^ {2}Shown, not cited \(pack data in \/plan, not counted\): 2 \(before-issuer, before-official-site\)$/m);
	const authorityOnly = run([], { authority: { name: 'Synthetic Office', official_url: 'https://www.sos.wa.gov/' } });
	assert.equal(authorityOnly.coverage.covered, 0, 'the authority row alone covers nothing');
	assert.match(report({ ...authorityOnly, slug: 's', base: 'b', at: 'a', registry: 'r' }), /^Coverage: 0\/74 values/m);
	const noUrl = run([], { authority: { name: 'Synthetic Office', official_url: null } });
	assert.equal(value(noUrl, 'before-official-site').state, 'missing');
	assert.equal(run([], { forms: [] }).coverage.covered, 0);
	assert.equal(run([], null).coverage.covered, 0, 'a 404 /plan is no plan');
	assert.equal(run([], null).coverage.shown, 0);
});

test('coverage: the issuing office needs its own fact, not the scope', () => {
	const scope = run([fact('jurisdiction', 'scope: Documents issued or notarized in Washington State')]);
	assert.equal(value(scope, 'before-issuer').state, 'missing', 'which documents the office handles is not who issues');
	assert.equal(scope.coverage.covered, 0, 'jurisdiction.scope is not on the checklist');
	const issuer = run([fact('jurisdiction', 'issuer: Corporations and Charities Division, Apostille Program')]);
	assert.equal(value(issuer, 'before-issuer').state, 'covered');
	assert.equal(value(issuer, 'before-issuer').by, 'facts');
	const both = run([fact('jurisdiction', 'issuer: Corporations and Charities Division')], { authority: { name: 'Synthetic Office' } });
	assert.equal(value(both, 'before-issuer').by, 'facts', 'a cited fact wins over the authority row');
	assert.equal(both.coverage.shown, 0);
});

test('closures: by instance or by a date the statement names', () => {
	const r = run([
		fact('holiday_hours', 'closure__2026_11_11: Veterans Day: offices closed'),
		fact('holiday_hours', 'closure: Thanksgiving, November 26-27, 2026: closed'),
		fact('holiday_hours', 'Christmas Day 12/25/2026 closed'),
		fact('service_notice', 'notice__2026_11_13: Counters close at 2:00 p.m. on Nov. 13')
	]);
	assert.deepEqual(
		r.closures.dates.filter((d) => d.known).map((d) => d.date),
		['2026-11-11', '2026-11-26', '2026-11-27', '2026-12-25']
	);
	assert.equal(r.closures.facts, 3, 'a notice is not a closure until it is re-topicked');
	assert.equal(value(r, 'visit-closures').state, 'covered');
	assert.equal(value(r, 'status-notices').state, 'covered');
});

test('closures: a date only in the quote is not known (the page shows the statement)', () => {
	// The office's holiday list as the quote, one date in the statement.
	const calendar =
		'State Holidays: Veterans Day, November 11, 2026: closed. November 13, 2026: counters close at 2:00 p.m. ' +
		'Thanksgiving, November 26-27, 2026: closed. 12/11/2026 reduced hours. Christmas Day, 2026-12-25: closed.';
	const keyed = run([fact('holiday_hours', 'closure__2026_11_11: Veterans Day: offices closed', { quote: calendar, url: CHECKLIST.sources.CCC })]);
	assert.deepEqual(keyed.closures.dates.filter((d) => d.known).map((d) => d.date), ['2026-11-11']);
	assert.equal(keyed.closures.known, 1);
	assert.match(
		report({ ...keyed, slug: 'usa-wa-sos', base: 'https://example.test', at: '2026-11-04T06:00:00Z', registry: 'embedded aliases' }),
		/^Closures known: 1\/6 \(1 closure fact\(s\)\)  target 6: NOT MET  missing 2026-11-13, 2026-11-26, 2026-11-27, 2026-12-11, 2026-12-25$/m
	);

	const named = run([fact('holiday_hours', 'closure: Thanksgiving, November 26-27, 2026: closed', { quote: calendar })]);
	assert.deepEqual(named.closures.dates.filter((d) => d.known).map((d) => d.date), ['2026-11-26', '2026-11-27']);

	const vague = run([fact('holiday_hours', 'closure: Offices are closed on state holidays.', { quote: calendar })]);
	assert.equal(vague.closures.known, 0, 'no date in the statement, none from the quote');
	assert.equal(vague.closures.facts, 1);
});

test('datesIn: named, numeric and ISO dates, ranges and pairs', () => {
	const days = (s) => datesIn(s).map(({ y, m, d }) => `${y ?? '-'}/${m}/${d}`);
	assert.deepEqual(days('Wednesday, November 11, 2026'), ['2026/11/11']);
	assert.deepEqual(days('Nov. 26 and 27'), ['-/11/26', '-/11/27']);
	assert.deepEqual(days('December 24th–26th'), ['-/12/24', '-/12/25', '-/12/26']);
	assert.deepEqual(days('[UPDATED: 12:01p.m. 11/03/2026]'), ['2026/11/3']);
	assert.deepEqual(days('2026-12-11 reduced hours'), ['2026/12/11']);
	assert.deepEqual(days('VitalChek 24/7, Marriage 12 copies, decision 5'), []);
});

test('freshness: newest and oldest capture, approvals when the read has them', () => {
	const old = fact('fees', 'per_document: $15.00 per document', { accepted_at: Date.parse('2026-11-03T18:00:00Z') / 1000 });
	old.citations[0].captured_at = '2026-10-31T17:10:02Z';
	const fresh = fact('fees', 'expedite: $100 per 10 documents');
	const r = run([old, fresh]);
	assert.deepEqual(r.captures, {
		newest: '2026-11-03T10:00:00Z',
		oldest: '2026-10-31T17:10:02Z',
		newestHours: 20,
		oldestHours: 84.8
	});
	assert.equal(r.approvals.newest, '2026-11-03T18:00:00Z');
	assert.equal(run([fresh]).approvals, null);
});
