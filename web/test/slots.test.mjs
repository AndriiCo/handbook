import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseStatement } from '../src/lib/public-ui/guide.js';
import {
	PROFILES,
	SECTIONS,
	SLOTS,
	TOPICS,
	TOPIC_SECTION,
	canonicalKeys,
	pairNotices,
	profileFor,
	resolveSlot,
	slotById,
	splitKey
} from '../src/lib/public-ui/slots.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..', '..');

// The frozen vocabulary, read from the Rust enum's wire forms: a new
// topic fails here until the registry places it.
const FROZEN = [
	...readFileSync(join(ROOT, 'crates', 'contracts', 'src', 'topics.rs'), 'utf8').matchAll(
		/Topic::\w+ => "([a-z_]+)"/g
	)
].map((m) => m[1]);

// The 14 production keys of usa-wa-sos (GET /authorities/usa-wa-sos/facts,
// 2026-11-03) and the slot each must fill.
const WA_LIVE = [
	['address', 'apostille_main_branch_street', 'address.office', 'tumwater'],
	['mailing', 'apostille_mailing_address', 'mailing.usps', null],
	['contacts', 'apostille_phone', 'contacts.phone', null],
	['address', 'apostille_eastern_branch_street', 'address.office', 'cheney'],
	['authentication', 'domestic_validity', 'authentication.domestic_validity', null],
	['mailing', 'submission_requirement', 'mailing.enclose', null],
	['forms', 'request_form_name', 'forms.form', null],
	['fees', 'same_day_fee', 'fees.same_day', null],
	['forms', 'request_form', 'forms.form', null],
	['payment', 'payee', 'payment.payee', null],
	['fees', 'standard_fee', 'fees.per_document', null],
	['processing_time', 'standard_time', 'processing_time.standard', null],
	['fees', 'expedited_fee', 'fees.expedite', null],
	['fees', 'same_day_inperson_fee', 'fees.same_day', null]
];

// Viet Nam's 10 production keys: general e-Apostille text and site
// chrome, no office procedure. None fills a slot; the page lists them
// under "Also on the office's pages".
const VN_LIVE = [
	['authentication', 'authorized_issuer'],
	['authentication', 'e_apostille_definition'],
	['authentication', 'e_apostille_creation'],
	['rejections', 'acceptance_rule'],
	['authentication', 'validity'],
	['authentication', 'e_app_program'],
	['website', 'site_title'],
	['jurisdiction', 'authority_name'],
	['website', 'homepage_label'],
	['website', 'portal_label']
];

// Judge replay fields (evals/replay/drafts-spark.jsonl) no slot fits.
const JUDGE_UNPLACED = new Set([
	'appointment/policy',
	'appointment/preference',
	'contacts/written_queries',
	'copies/copies_required',
	'copies/copy_requirement',
	'copies/requirement',
	'holiday_hours/holiday_schedule_posting',
	'holiday_hours/schedule',
	'hours/afternoon_policy',
	'hours/availability',
	'hours/midday_closure',
	'id_requirements/pickup_id',
	'id_requirements/third_party_pickup',
	'legalization_chain/consular_legalization_requirement',
	'legalization_chain/convention_accession',
	'legalization_chain/convention_membership',
	'legalization_chain/effective_date',
	'legalization_chain/entry_into_force_date',
	'legalization_chain/legalization_route',
	'legalization_chain/post_issuance_requirement',
	'legalization_chain/route',
	'mailing/transit_note',
	'mailing/transit_time',
	'payment/payment_timing',
	'po_box/note',
	'po_box/superseded_warning',
	'processing_time/email_reply_time',
	'processing_time/email_response_time',
	'processing_time/incomplete_application',
	'processing_time/incomplete_applications',
	'rejections/rejection_condition',
	'rejections/rejection_reason',
	'rejections/return_condition',
	'translation/handling',
	'translation/requirement',
	'translation/translation_binding',
	'translation/translation_requirement',
	'walkin/afternoon_policy',
	'walkin/policy',
	'walkin/walkin_accepted'
]);

// Keyed facts of the Washington e2e fixtures that deliberately fill no
// slot. `methods` says how to pay but names no payee, so it is no
// "Payable to" value (a payee-bearing `accepted_methods` is one); the
// page lists it under "Also on the office's pages".
const WA_FIXTURE_UNMAPPED = new Set(['payment/methods']);

const KINDS = [
	'money',
	'money_tier',
	'postal',
	'payee',
	'phone',
	'email',
	'hours',
	'time_rule',
	'duration',
	'closure',
	'text',
	'link_from_citation',
	'notice',
	'notice_update'
];

/** Every `{topic}/{key}` a slot reads, with the slot that claims it. */
function claims() {
	const out = [];
	for (const s of SLOTS) {
		for (const key of s.keys) out.push([`${s.topic}/${key}`, s.id]);
		for (const a of s.also) for (const key of a.keys) out.push([`${a.topic}/${key}`, s.id]);
	}
	return out;
}

test('every frozen topic maps to exactly one section', () => {
	assert.equal(new Set(FROZEN).size, 26);
	assert.deepEqual([...TOPICS].sort(), [...FROZEN].sort());
	for (const topic of FROZEN) {
		const homes = SECTIONS.filter((s) => s.topics.includes(topic)).map((s) => s.id);
		assert.equal(homes.length, 1, `${topic} sits in ${homes.join(', ') || 'no section'}`);
		assert.equal(TOPIC_SECTION[topic], homes[0]);
	}
	assert.equal(TOPIC_SECTION.phone_script, 'hidden');
	assert.equal(TOPIC_SECTION.service_notice, 'status');
});

test('sections run in step order and pseudo-sections never head the page', () => {
	assert.deepEqual(
		SECTIONS.map((s) => [s.id, s.step]),
		[
			['before', 1],
			['prepare', 2],
			['form', 3],
			['fees', 4],
			['send', 5],
			['after', 6],
			['contact', null],
			['status', null],
			['hidden', null]
		]
	);
	for (const s of SECTIONS) assert.equal(s.pseudo, s.heading === null, s.id);
	assert.deepEqual(
		SECTIONS.filter((s) => s.pseudo).map((s) => s.id),
		['status', 'hidden']
	);
	// J1: "Pay the fees" is the only region whose name says "fees".
	assert.deepEqual(
		SECTIONS.filter((s) => /fees/i.test(s.heading ?? '')).map((s) => s.heading),
		['Pay the fees']
	);
	for (const [id, topics] of [
		['before', ['jurisdiction', 'authentication']],
		['prepare', ['notarization', 'copies']],
		['form', ['forms']],
		['fees', ['fees', 'payment']],
		['send', ['mailing', 'po_box', 'address', 'walkin', 'hours', 'holiday_hours']],
		['after', ['processing_time']],
		['contact', ['contacts']]
	]) {
		for (const topic of topics) assert.equal(TOPIC_SECTION[topic], id, topic);
	}
	assert.equal(slotById('website.verify')?.section, 'after');
	assert.equal(slotById('mailing.return')?.section, 'after');
	assert.equal(slotById('jurisdiction.refusals')?.section, 'before');
	// Step 3 shows "Send all documents with Request form and payment".
	assert.equal(slotById('mailing.enclose')?.section, 'form');
	// SLOTS is in page order: a presenter can bucket rows by section as is.
	const order = SECTIONS.map((s) => s.id);
	const at = SLOTS.map((s) => order.indexOf(s.section));
	assert.deepEqual(at, [...at].sort((a, b) => a - b));
});

test('slot keys are unique per topic, across slots and their also topics', () => {
	const owner = new Map();
	for (const [at, id] of claims()) {
		assert.ok(!owner.has(at), `${at} is claimed by ${owner.get(at)} and ${id}`);
		owner.set(at, id);
	}
	assert.equal(new Set(SLOTS.map((s) => s.id)).size, SLOTS.length);
	for (const s of SLOTS) {
		// `{topic}.{name}`: the canonical key, except the USPS address,
		// whose key stays production's `apostille_mailing_address`.
		const usps = s.keys[0] === 'apostille_mailing_address';
		assert.equal(s.id, `${s.topic}.${usps ? 'usps' : s.keys[0]}`);
		assert.ok(FROZEN.includes(s.topic), `${s.id}: topic`);
		for (const a of s.also) assert.ok(FROZEN.includes(a.topic), `${s.id}: also ${a.topic}`);
		for (const key of [...s.keys, ...s.also.flatMap((a) => a.keys)]) {
			// The statement grammar, and no `__`: that splits off an instance.
			assert.match(key, /^[a-z][a-z0-9_]*$/, `${s.id}: ${key}`);
			assert.ok(!key.includes('__'), `${s.id}: ${key}`);
		}
		for (const [key, instance] of Object.entries(s.instances)) {
			assert.ok(s.keys.includes(key), `${s.id}: instance key ${key}`);
			assert.match(instance, /^[a-z0-9]+(?:_[a-z0-9]+)*$/);
		}
		assert.ok(KINDS.includes(s.kind), `${s.id}: kind ${s.kind}`);
		assert.ok([null, 'cost', 'time', 'pay', 'send', 'form'].includes(s.glance), `${s.id}: glance`);
		assert.ok([null, 'mail', 'courier', 'counter'].includes(s.route), `${s.id}: route`);
		assert.equal(s.copy, s.kind === 'payee' || s.kind === 'postal', `${s.id}: copy`);
		assert.ok(
			SECTIONS.some((x) => x.id === s.section),
			`${s.id}: section ${s.section}`
		);
	}
	assert.deepEqual([...new Set(SLOTS.map((s) => s.kind))].sort(), [...KINDS].sort());
	assert.equal(SLOTS.filter((s) => s.topic === 'phone_script').length, 0);
});

test('labels and headings pass the copy rule', () => {
	const copy = [
		...SLOTS.map((s) => [s.id, s.label]),
		...SECTIONS.filter((s) => s.heading !== null).map((s) => [s.id, s.heading])
	];
	for (const [id, text] of copy) {
		assert.doesNotMatch(text, /[0-9$]/, `${id}: digits or $ in "${text}"`);
		assert.doesNotMatch(text, /\b(dossier|slot|topic|fact)s?\b/i, `${id}: jargon in "${text}"`);
		assert.doesNotMatch(text, /_/, `${id}: raw key in "${text}"`);
		assert.doesNotMatch(
			text,
			/\b(january|february|march|april|may|june|july|august|september|october|november|december)\b/i,
			`${id}: date in "${text}"`
		);
		assert.match(text, /^[A-Z]/, `${id}: sentence case`);
		assert.ok(text.split(/\s+/).length <= 6, `${id}: "${text}" is over six words`);
	}
	assert.equal(new Set(SLOTS.map((s) => s.label)).size, SLOTS.length, 'labels are distinct');
});

test('every live Washington key resolves to its slot', () => {
	for (const [topic, key, id, instance] of WA_LIVE) {
		const hit = resolveSlot(topic, key);
		assert.ok(hit, `${topic}/${key} resolves`);
		assert.equal(hit.slot.id, id, `${topic}/${key}`);
		assert.equal(hit.instance, instance, `${topic}/${key} instance`);
	}
});

test('every keyed fact in the Washington e2e fixtures fills a slot, or is listed unmapped', () => {
	/** @param {string} name */
	const read = (name) => JSON.parse(readFileSync(join(HERE, '..', 'e2e', 'fixtures', name), 'utf8'));
	const facts = [
		...read('facts-usa-wa-sos.json').facts,
		...read('plan-usa-wa-sos.json').facts,
		...read('orders-plan.json').facts
	];
	const seen = new Set();
	for (const fact of facts) {
		const { fieldKey } = parseStatement(fact.statement);
		if (!fieldKey) continue;
		const at = `${fact.topic}/${splitKey(fieldKey).base}`;
		seen.add(at);
		if (WA_FIXTURE_UNMAPPED.has(at)) {
			assert.equal(resolveSlot(fact.topic, fieldKey), null, `fact ${fact.id}: ${at} is listed unmapped`);
		} else {
			assert.ok(resolveSlot(fact.topic, fieldKey), `fact ${fact.id}: ${at} fills no slot and is not listed unmapped`);
		}
	}
	assert.ok(seen.size >= 7, 'the fixture keeps its keyed facts');
	for (const at of WA_FIXTURE_UNMAPPED) assert.ok(seen.has(at), `${at} is no longer in the fixtures`);
	// The fixture's `forms:` statement is the request form.
	assert.equal(resolveSlot('forms', 'forms')?.slot.id, 'forms.form');
	// Copy buttons stay on payee and postal values only.
	for (const s of SLOTS.filter((x) => x.copy)) assert.ok(['payee', 'postal'].includes(s.kind), s.id);
});

test("Viet Nam's live keys stay deliberately unmapped", () => {
	for (const [topic, key] of VN_LIVE) assert.equal(resolveSlot(topic, key), null, `${topic}/${key}`);
});

test('judge replay fields are placed, or listed as deliberately unplaced', () => {
	const corpus = readFileSync(join(ROOT, 'evals', 'replay', 'drafts-spark.jsonl'), 'utf8');
	const fields = new Set();
	for (const line of corpus.split('\n').filter(Boolean)) {
		const draft = JSON.parse(line);
		if (draft.slug !== 'usa-wa-sos' && draft.slug !== 'vietnam') continue;
		for (const r of draft.records ?? []) fields.add(`${r.topic}/${r.field}`);
	}
	assert.ok(fields.size > 0);
	for (const at of fields) {
		const [topic, key] = at.split('/');
		if (JUDGE_UNPLACED.has(at)) assert.equal(resolveSlot(topic, key), null, `${at} is listed unplaced`);
		else assert.ok(resolveSlot(topic, key), `${at} has no slot and is not listed unplaced`);
	}
	for (const at of JUDGE_UNPLACED) assert.ok(fields.has(at), `${at} is no longer in the corpus`);
	// The fields the design review named.
	for (const [topic, key, id] of [
		['fees', 'standard_authentication_fee', 'fees.per_document'],
		['contacts', 'phone', 'contacts.phone'],
		['address', 'location', 'address.office'],
		['hours', 'counter_hours', 'hours.weekday'],
		['forms', 'required_form', 'forms.form']
	]) {
		assert.equal(resolveSlot(topic, key)?.slot.id, id, `${topic}/${key}`);
	}
});

test('canonicalKeys offers canonical keys only, in registry order', () => {
	assert.equal(canonicalKeys('fees')[0].key, 'per_document');
	assert.deepEqual(canonicalKeys('fees'), [
		{ key: 'per_document', label: 'Fee per document' },
		{ key: 'expedite', label: 'Expedited service fee' },
		{ key: 'same_day', label: 'Same-day service fee' },
		{ key: 'refund', label: 'Refunds' }
	]);
	assert.deepEqual(
		canonicalKeys('contacts').map((c) => c.key),
		['phone', 'phone_option', 'phone_hours', 'email', 'chat', 'tty', 'no_texts']
	);
	assert.deepEqual(
		canonicalKeys('address').map((c) => c.key),
		['office']
	);
	assert.deepEqual(
		canonicalKeys('service_notice').map((c) => c.key),
		['notice', 'restored']
	);
	// `also` topics and slot-less topics offer nothing.
	assert.deepEqual(canonicalKeys('po_box'), []);
	assert.deepEqual(canonicalKeys('phone_script'), []);
	assert.deepEqual(canonicalKeys('not_a_topic'), []);
	for (const topic of FROZEN) {
		const aliases = SLOTS.filter((s) => s.topic === topic).flatMap((s) => s.keys.slice(1));
		for (const { key } of canonicalKeys(topic)) assert.ok(!aliases.includes(key), `${topic}: ${key}`);
	}
});

test('resolveSlot reads instances, aliases and also topics', () => {
	assert.equal(resolveSlot('address', 'apostille_main_branch_street')?.instance, 'tumwater');
	assert.equal(resolveSlot('address', 'apostille_eastern_branch_street')?.instance, 'cheney');
	assert.deepEqual(
		[resolveSlot('hours', 'weekday__cheney')?.slot.id, resolveSlot('hours', 'weekday__cheney')?.instance],
		['hours.weekday', 'cheney']
	);
	assert.equal(resolveSlot('address', 'office__tumwater')?.instance, 'tumwater');
	assert.equal(resolveSlot('address', 'office')?.instance, null);
	// An explicit suffix wins over the alias's own instance.
	assert.equal(resolveSlot('address', 'apostille_main_branch_street__cheney')?.instance, 'cheney');
	assert.equal(resolveSlot('holiday_hours', 'closure__2026_11_11')?.instance, '2026_11_11');
	assert.equal(resolveSlot('walkin', 'same_day_only__cheney')?.slot.id, 'walkin.same_day_only');
	assert.equal(resolveSlot('po_box', 'po_box')?.slot.id, 'mailing.usps');
	assert.equal(resolveSlot('hours', 'phone_hours')?.slot.id, 'contacts.phone_hours');
	assert.equal(resolveSlot('rejections', 'not_accepted')?.slot.id, 'jurisdiction.refusals');
	assert.equal(resolveSlot('jurisdiction', 'refusals')?.slot.many, true);
	// Keys only resolve in their own topics.
	assert.equal(resolveSlot('fees', 'weekday'), null);
	assert.equal(resolveSlot('hours', 'not_a_key'), null);
	assert.equal(resolveSlot('hours', null), null);
	assert.equal(resolveSlot('hours', ''), null);
	assert.equal(resolveSlot('hours', 'weekday__'), null);
	assert.equal(resolveSlot('hours', 'weekday__Cheney'), null);
	assert.equal(resolveSlot('hours', 'weekday__cheney_'), null);
	assert.equal(resolveSlot('hours', 'unknown__cheney'), null);
});

test('splitKey splits at the first double underscore', () => {
	assert.deepEqual(splitKey('weekday__cheney'), { base: 'weekday', instance: 'cheney' });
	assert.deepEqual(splitKey('same_day_only__cheney'), { base: 'same_day_only', instance: 'cheney' });
	assert.deepEqual(splitKey('notice__2026_11_02'), { base: 'notice', instance: '2026_11_02' });
	assert.deepEqual(splitKey('per_document'), { base: 'per_document', instance: null });
	assert.deepEqual(splitKey('__cheney'), { base: '__cheney', instance: null });
	assert.deepEqual(splitKey('weekday__'), { base: 'weekday__', instance: null });
});

test('profileFor matches US state rows by kind, or by slug when kind is missing', () => {
	const usState = PROFILES.find((p) => p.id === 'us_state');
	assert.ok(usState);
	// Production: country "USA" or "United States of America", kind "state".
	assert.equal(profileFor({ country: 'USA', kind: 'state', slug: 'usa-wa-sos' }), usState);
	assert.equal(profileFor({ country: 'United States of America', kind: 'state', slug: 'usa-ny-dos' }), usState);
	// The e2e fixture row carries no kind at all; a null kind reads the same.
	const fixture = JSON.parse(readFileSync(join(HERE, '..', 'e2e', 'fixtures', 'authorities.json'), 'utf8'));
	const wa = fixture.authorities.find((a) => a.slug === 'usa-wa-sos');
	assert.equal(wa.kind ?? null, null);
	assert.equal(profileFor(wa), usState);
	assert.equal(profileFor({ country: 'United States', kind: null, slug: 'usa-wa-sos' }), usState);
	assert.equal(profileFor({ slug: 'usa-tx-sos' }), usState);
	// Not a US state page.
	assert.equal(profileFor({ country: 'USA', kind: 'office', slug: 'usa-dos' }), null);
	assert.equal(profileFor({ country: 'Vietnam', kind: null, slug: 'vietnam' }), null);
	assert.equal(profileFor({ country: 'Viet Nam', kind: 'country', slug: 'vietnam' }), null);
	assert.equal(profileFor({ country: 'USA', kind: null, slug: 'usa-ny-dos' }), null);
	assert.equal(profileFor({ country: 'Vietnam', kind: null, slug: 'usa-wa-sos' }), null);
	assert.equal(profileFor(null), null);
	assert.equal(profileFor(undefined), null);
	// Critical slots exist, and each is an "At a glance" row.
	assert.deepEqual(usState.critical, [
		'fees.per_document',
		'payment.payee',
		'mailing.usps',
		'processing_time.standard',
		'forms.form'
	]);
	for (const id of usState.critical) assert.ok(slotById(id)?.glance, id);
});

/** A service-notice fact as /facts returns it. */
function notice(id, statement) {
	return { id, topic: 'service_notice', status: 'accepted', statement };
}

test('pairNotices pairs each restore with the notice it names', () => {
	const outage = notice(31, 'notice__2026_11_02: Apostille services are unavailable until further notice.');
	const back = notice(32, 'restored__2026_11_02: Apostille services have been restored.');
	assert.deepEqual(pairNotices([back, outage]), [{ id: '2026_11_02', notice: outage, restored: back }]);
	assert.deepEqual(pairNotices([outage]), [{ id: '2026_11_02', notice: outage, restored: null }]);
});

test('pairNotices keeps unnamed and unmatched lines apart', () => {
	const bare = notice(40, 'notice: The Cheney office is closed today.');
	const unkeyed = notice(41, 'Counter service is paused this afternoon.');
	const other = notice(42, 'outage__2026_11_02: Systems are down.');
	const orphan = notice(43, 'restored__2026_10_01: The online request form is back.');
	const loose = notice(44, 'restored: Service is back.');
	const fee = { id: 45, topic: 'fees', statement: 'notice__2026_11_02: not a notice' };
	assert.deepEqual(pairNotices([bare, unkeyed, other, orphan, loose, fee]), [
		{ id: null, notice: bare, restored: null },
		{ id: null, notice: unkeyed, restored: null },
		{ id: null, notice: other, restored: null },
		{ id: '2026_10_01', notice: null, restored: orphan },
		{ id: null, notice: null, restored: loose }
	]);
	assert.deepEqual(pairNotices([]), []);
});

test('pairNotices lets the newest restore of an id win and pairs every notice of it', () => {
	const first = notice(50, 'notice__2026_11_02: Apostille services are unavailable.');
	const reworded = notice(51, 'notice__2026_11_02: Apostille services remain unavailable.');
	const early = notice(52, 'restored__2026_11_02: Partly restored.');
	const late = notice(53, 'restored__2026_11_02: Fully restored.');
	assert.deepEqual(pairNotices([first, reworded, early, late]), [
		{ id: '2026_11_02', notice: first, restored: late },
		{ id: '2026_11_02', notice: reworded, restored: late }
	]);
	// In any input order: the larger fact id is the newer restore.
	assert.deepEqual(pairNotices([late, early, first]), [{ id: '2026_11_02', notice: first, restored: late }]);
	assert.equal(resolveSlot('service_notice', 'notice__2026_11_02')?.slot.kind, 'notice');
	assert.equal(resolveSlot('service_notice', 'restored__2026_11_02')?.slot.kind, 'notice_update');
});

test('the registry is frozen', () => {
	assert.ok(Object.isFrozen(SLOTS) && Object.isFrozen(SLOTS[0]) && Object.isFrozen(SLOTS[0].keys));
	assert.ok(Object.isFrozen(SECTIONS) && Object.isFrozen(TOPIC_SECTION));
	assert.throws(() => {
		/** @type {any} */ (SLOTS[0]).label = 'changed';
	});
});
