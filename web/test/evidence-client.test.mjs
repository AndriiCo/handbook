import test from 'node:test';
import assert from 'node:assert/strict';
import {
	EvidenceError,
	NOTE_KINDS,
	PUBLIC_NOTE_LINE,
	anchorChoices,
	channelFor,
	charCount,
	createNote,
	defaultAnchor,
	fetchAcceptedFacts,
	fetchNotes,
	fromLocalInput,
	inPublicConflict,
	kindLabel,
	noteBody,
	noteProblem,
	normalizeFact,
	normalizeNote,
	notesForFact,
	plainText,
	preferredChoices,
	retireBody,
	retireFact,
	shorten,
	slotChoices,
	slotFor,
	slotLabel,
	supportChoices,
	toLocalInput,
	withdrawBody,
	withdrawNote
} from '../src/lib/admin/evidence-client.js';

const SLUG = 'usa-wa-sos';
const BOX_A = 'apostille_mailing_address: PO Box 40220, Olympia, WA 98504-0220';
const BOX_B = 'apostille_mailing_address: PO Box 40234, Olympia, WA 98504-0234';

/** An accepted `/facts` row (normalized shape). */
function fact(id, topic, statement, conflicts = [], slug = SLUG) {
	return {
		id,
		authority_slug: slug,
		topic,
		statement,
		status: 'accepted',
		created_at: 1,
		accepted_at: 2,
		citations: [],
		conflict: conflicts.length > 0,
		conflicts
	};
}

const FACTS = [
	fact(201, 'mailing', BOX_A, [{ id: 202, topic: 'mailing', statement: BOX_B, status: 'accepted' }]),
	fact(202, 'mailing', BOX_B, [
		{ id: 201, topic: 'mailing', statement: BOX_A, status: 'accepted' },
		{ id: 299, topic: 'mailing', statement: 'apostille_mailing_address: PO Box 1', status: 'proposed' }
	]),
	fact(203, 'contacts', 'phone: (360) 555-0142'),
	fact(204, 'contacts', 'email: Write to apostille@sos.example.'),
	fact(205, 'fees', 'per_document: $15 per document'),
	// Disagrees only with a proposal: not in conflict on the public read.
	fact(206, 'fees', 'same_day: $50', [{ id: 300, topic: 'fees', statement: 'same_day: $60', status: 'proposed' }]),
	fact(207, 'mailing', BOX_A, [], 'other-sos')
];

function note(id, fields) {
	return {
		id,
		authority_slug: SLUG,
		kind: 'call',
		fact_id: null,
		preferred_fact_id: null,
		slot: '',
		stance: 'context',
		occurred_at: 1793639400,
		date: '2026-11-02',
		channel: '',
		summary: 'synthetic summary',
		supports: [],
		status: 'published',
		created_by: 'staff-1',
		created_at: 1793639460,
		...fields
	};
}

const subject = (over = {}) => ({
	id: 202,
	authority_slug: SLUG,
	topic: 'mailing',
	statement: BOX_B,
	status: 'accepted',
	related: [201],
	...over
});

test('kinds, stances and the public line are the plan copy', () => {
	assert.deepEqual(
		NOTE_KINDS.map((k) => k.label),
		['Phone call', 'Email', 'Visit report', 'Staff decision', "Not found on the office's pages"]
	);
	assert.equal(kindLabel('absence'), "Not found on the office's pages");
	assert.equal(
		PUBLIC_NOTE_LINE,
		'Notes are public on the guide. Write what the office said; never names or personal numbers; calls are not recorded.'
	);
});

test('plainText and charCount count what the server stores', () => {
	assert.equal(plainText('  The desk \n said\tso. '), 'The desk said so.');
	assert.equal(charCount('  a  b\n c '), 5);
	// Code points, not UTF-16 units (an emoji is one character).
	assert.equal(charCount('ok 👍'), 4);
	assert.equal(shorten('one two three four', 9), 'one two…');
	assert.equal(shorten('short', 9), 'short');
});

test('slotFor reads the registry slot, its instance, or falls back to the key or topic', () => {
	assert.equal(slotFor('mailing', BOX_B), 'mailing.usps');
	assert.equal(slotFor('po_box', 'po_box: PO Box 40220'), 'mailing.usps');
	assert.equal(slotFor('address', 'apostille_main_branch_street: 801 Capitol Way S'), 'address.office__tumwater');
	assert.equal(slotFor('hours', 'weekday__cheney: Monday to Friday'), 'hours.weekday__cheney');
	assert.equal(slotFor('fees', 'surcharge: 5 USD'), 'fees.surcharge');
	assert.equal(slotFor('fees', 'Legalization costs 300,000 VND per document.'), 'fees');
	assert.equal(slotFor('Fees', 'per_document: $15'), '');
	assert.equal(slotFor('fees.x', 'per_document: $15'), '');
	// Never longer than the server's 64 characters.
	const long = `${'k'.repeat(70)}: value`;
	assert.equal(slotFor('fees', long), 'fees');
});

test('slotLabel and slotChoices name fields plainly', () => {
	assert.equal(slotLabel('mailing.usps'), 'Mailing address');
	assert.equal(slotLabel('address.office__tumwater'), 'Office address (tumwater)');
	assert.equal(slotLabel('fees.surcharge'), 'Surcharge');
	assert.equal(slotLabel(''), 'No field');
	const choices = slotChoices('hours.weekday__cheney');
	assert.equal(choices[0].value, 'hours.weekday__cheney');
	assert.ok(choices.some((c) => c.value === 'fees.same_day'));
	assert.equal(slotChoices('fees.same_day')[0].value, 'jurisdiction.scope');
});

test('notesForFact keeps published notes about the fact, its siblings or its field', () => {
	const notes = [
		note(1, { fact_id: 202, slot: 'mailing.usps' }),
		note(2, { fact_id: 201 }),
		note(3, { kind: 'decision', preferred_fact_id: 201, slot: 'mailing.usps', supports: [1] }),
		note(4, { kind: 'absence', slot: 'mailing.usps' }),
		note(5, { kind: 'absence', slot: 'fees.same_day' }),
		note(6, { fact_id: 202, status: 'withdrawn' }),
		note(7, { fact_id: 202, authority_slug: 'other-sos' })
	];
	assert.deepEqual(
		notesForFact(notes, subject()).map((n) => n.id),
		[1, 2, 3, 4]
	);
	// An unkeyed proposal's topic covers every slot of that topic.
	const unkeyed = subject({ id: 12, statement: 'Mailed requests go to PO Box 4000.', status: 'proposed', related: [] });
	assert.deepEqual(
		notesForFact(notes, unkeyed).map((n) => n.id),
		[1, 3, 4]
	);
	assert.deepEqual(notesForFact([], subject()), []);
});

test("channelFor starts calls and emails on the office's own accepted contact", () => {
	assert.equal(channelFor('call', FACTS), '(360) 555-0142');
	assert.equal(channelFor('email', FACTS), 'apostille@sos.example');
	assert.equal(channelFor('visit', FACTS), '');
	assert.equal(channelFor('call', []), '');
	assert.equal(channelFor('call', [fact(1, 'contacts', `phone: ${'1 '.repeat(100)}`)]), '');
});

test('anchors: an accepted fact anchors to itself; a proposal to the live fact it changes', () => {
	assert.equal(defaultAnchor(subject(), FACTS), 202);
	assert.deepEqual(
		anchorChoices(subject(), FACTS).map((f) => f.id),
		[202]
	);
	const proposal = subject({ id: 12, statement: 'apostille_mailing_address: PO Box 4000', status: 'proposed', related: [203] });
	const ids = anchorChoices(proposal, FACTS).map((f) => f.id);
	// Same field first, then related, then same topic, then the rest; never another office.
	assert.deepEqual(ids, [201, 202, 203, 204, 205, 206]);
	assert.equal(defaultAnchor(proposal, FACTS), 203);
	assert.equal(defaultAnchor({ ...proposal, related: [] }, FACTS), 201);
	assert.equal(defaultAnchor({ ...proposal, topic: 'website', statement: 'verify: x', related: [] }, FACTS), null);
});

test('a decision chooses among accepted facts that disagree on the public read', () => {
	assert.equal(inPublicConflict(FACTS[0]), true);
	assert.equal(inPublicConflict(FACTS[5]), false);
	assert.deepEqual(
		preferredChoices(subject(), FACTS).map((f) => f.id),
		[201, 202]
	);
	assert.deepEqual(preferredChoices(subject({ id: 205, topic: 'fees', statement: 'per_document: $15', related: [] }), FACTS), []);
	const notes = [note(1, {}), note(2, { kind: 'decision' }), note(3, { kind: 'absence', status: 'withdrawn' }), note(4, { authority_slug: 'x' })];
	assert.deepEqual(
		supportChoices(notes, SLUG).map((n) => n.id),
		[1]
	);
});

const base = {
	slug: SLUG,
	kind: 'call',
	stance: 'confirms',
	factId: 202,
	slot: 'mailing.usps',
	preferredFactId: null,
	supports: [],
	when: 1793639400,
	channel: ' (360) 555-0142  option 3 ',
	summary: 'The desk said PO Box 40234 takes apostille mail.'
};

test('noteBody: a call names its fact and slot, never a preferred fact or supports', () => {
	assert.deepEqual(noteBody(base, 1793640000), {
		body: {
			authority_slug: SLUG,
			kind: 'call',
			stance: 'confirms',
			occurred_at: 1793639400,
			channel: '(360) 555-0142 option 3',
			summary: 'The desk said PO Box 40234 takes apostille mail.',
			fact_id: 202,
			slot: 'mailing.usps'
		}
	});
	const bare = noteBody({ ...base, slot: '', preferredFactId: 201, supports: [1] }, null);
	assert.ok('body' in bare);
	assert.equal('slot' in bare.body, false);
	assert.equal('preferred_fact_id' in bare.body, false);
	assert.equal('supports' in bare.body, false);
	assert.equal('recorded' in bare.body, false);
});

test('noteBody: an absence names a slot; a decision its slot, preferred fact and supports', () => {
	const absence = noteBody({ ...base, kind: 'absence', stance: 'context', factId: null, slot: 'fees.same_day', channel: 'FAQ' }, null);
	assert.deepEqual(absence, {
		body: {
			authority_slug: SLUG,
			kind: 'absence',
			stance: 'context',
			occurred_at: 1793639400,
			channel: 'FAQ',
			summary: 'The desk said PO Box 40234 takes apostille mail.',
			slot: 'fees.same_day'
		}
	});
	const decision = noteBody({ ...base, kind: 'decision', factId: 202, preferredFactId: 202, supports: [31, 31, 32] }, null);
	assert.ok('body' in decision);
	assert.deepEqual(decision.body, {
		authority_slug: SLUG,
		kind: 'decision',
		stance: 'confirms',
		occurred_at: 1793639400,
		channel: '',
		summary: 'The desk said PO Box 40234 takes apostille mail.',
		slot: 'mailing.usps',
		preferred_fact_id: 202,
		supports: [31, 32]
	});
});

test('noteProblem says what a note still needs, in order', () => {
	assert.equal(noteProblem({ ...base, slug: '' }, null), 'This note has no office.');
	assert.equal(noteProblem({ ...base, kind: 'chat' }, null), 'Pick a kind.');
	assert.equal(noteProblem({ ...base, stance: '' }, null), 'Pick a stance.');
	assert.equal(noteProblem({ ...base, factId: null }, null), 'Pick the accepted fact this note is about.');
	assert.equal(noteProblem({ ...base, kind: 'decision', preferredFactId: null, supports: [1] }, null), 'Pick the preferred fact.');
	assert.equal(
		noteProblem({ ...base, kind: 'decision', preferredFactId: 202, supports: [] }, null),
		'Pick at least one note the decision rests on.'
	);
	assert.equal(
		noteProblem({ ...base, kind: 'decision', preferredFactId: 202, supports: Array.from({ length: 21 }, (_, i) => i + 1) }, null),
		'A decision rests on at most 20 notes.'
	);
	assert.equal(noteProblem({ ...base, kind: 'absence', slot: '' }, null), 'Pick the field.');
	assert.equal(noteProblem({ ...base, kind: 'absence', slot: 'Fees.same day' }, null), 'Pick the field.');
	assert.equal(noteProblem({ ...base, when: null }, null), 'Pick the date and time it happened.');
	assert.equal(noteProblem({ ...base, when: 1793640000 + 600 }, 1793640000), 'The date and time are in the future.');
	assert.equal(noteProblem({ ...base, when: 1793640000 + 600 }, null), null);
	assert.equal(noteProblem({ ...base, channel: '   ' }, null), 'Name the channel: the phone line, email address or place.');
	assert.equal(noteProblem({ ...base, kind: 'absence', slot: 'fees.same_day', channel: '' }, null), null);
	assert.equal(noteProblem({ ...base, channel: 'x'.repeat(161) }, null), 'The channel is at most 160 characters.');
	assert.equal(noteProblem({ ...base, summary: 'Too few' }, null), 'The summary needs 8-280 characters.');
	assert.equal(noteProblem({ ...base, summary: 'x'.repeat(281) }, null), 'The summary needs 8-280 characters.');
	assert.equal(noteProblem({ ...base, summary: 'x'.repeat(280) }, null), null);
	assert.deepEqual(noteBody({ ...base, stance: '' }, null), { error: 'Pick a stance.' });
});

test('retire and withdraw reasons', () => {
	assert.equal(retireBody('short'), null);
	assert.equal(retireBody('   seven  '), null);
	assert.deepEqual(retireBody('  wrong page, wrong box  '), { reason: 'wrong page, wrong box' });
	assert.equal(retireBody('x'.repeat(501)), null);
	assert.deepEqual(retireBody('x'.repeat(500)), { reason: 'x'.repeat(500) });
	assert.equal(withdrawBody('   '), null);
	assert.deepEqual(withdrawBody(' called back '), { reason: 'called back' });
	assert.equal(withdrawBody('é'.repeat(513)), null, '1026 bytes');
	assert.deepEqual(withdrawBody('é'.repeat(512)), { reason: 'é'.repeat(512) });
});

test('datetime-local values round-trip on the browser clock', () => {
	const at = fromLocalInput('2026-11-02T09:10');
	assert.equal(typeof at, 'number');
	assert.equal(toLocalInput(/** @type {number} */ (at)), '2026-11-02T09:10');
	assert.equal(fromLocalInput('2026-11-02T09:10:30'), /** @type {number} */ (at) + 30);
	assert.equal(fromLocalInput('2026-02-30T09:10'), null);
	assert.equal(fromLocalInput('yesterday'), null);
	assert.equal(fromLocalInput(''), null);
	assert.equal(toLocalInput(Number.NaN), '');
});

test('normalizers keep known shapes and drop the rest', () => {
	assert.equal(normalizeNote(null), null);
	assert.equal(normalizeNote({ id: 1, authority_slug: SLUG, kind: 'chat', stance: 'confirms', summary: 's', occurred_at: 1 }), null);
	const n = normalizeNote({
		id: 7,
		authority_slug: SLUG,
		kind: 'decision',
		stance: 'confirms',
		summary: 'Use the 40234 box.',
		occurred_at: 5,
		preferred_fact_id: 202,
		supports: [1, 'x', 2],
		status: 'withdrawn'
	});
	assert.deepEqual(n && { ...n }, {
		id: 7,
		authority_slug: SLUG,
		kind: 'decision',
		fact_id: null,
		preferred_fact_id: 202,
		slot: '',
		stance: 'confirms',
		occurred_at: 5,
		date: '',
		channel: '',
		summary: 'Use the 40234 box.',
		supports: [1, 2],
		status: 'withdrawn',
		created_by: '',
		created_at: 0
	});
	assert.equal(normalizeFact({ id: 1 }), null);
	const f = normalizeFact({
		id: 3,
		authority_slug: SLUG,
		topic: 'fees',
		statement: 'per_document: $15',
		citations: [{ quote: 'The fee is $15.', source_url: 'https://sos.example/fees', captured_at: '2026-11-02T17:00:00Z', kind: 'pdf' }, null],
		conflicts: [{ id: 4, statement: 'per_document: $20', status: 'accepted' }, { id: 'x' }]
	});
	assert.equal(f?.conflict, true);
	assert.deepEqual(f?.conflicts, [{ id: 4, topic: 'fees', statement: 'per_document: $20', status: 'accepted' }]);
	assert.equal(f?.citations[0].kind, 'pdf');
	assert.equal(f?.status, 'accepted');
	assert.equal(f?.accepted_at, null);
});

/** A fetch double that records each call and answers `responses` in turn. */
function fakeFetch(responses) {
	const calls = [];
	const fn = async (url, init = {}) => {
		calls.push({ url, init });
		const next = responses.shift();
		if (next instanceof Error) throw next;
		const { status = 200, body, text } = next;
		return new Response(text ?? JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
	};
	return { fn, calls };
}

test('fetch helpers read the staff routes and post with the CSRF header', async () => {
	const { fn, calls } = fakeFetch([
		{ body: { notes: [note(1, {}), { id: 'bad' }], count: 2 } },
		{ body: { facts: [FACTS[0]], count: 1 } },
		{ status: 201, body: note(9, { kind: 'absence', slot: 'fees.same_day' }) },
		{ body: note(9, { status: 'withdrawn' }) },
		{ body: { id: 202, authority_slug: SLUG, status: 'superseded' } }
	]);
	const notes = await fetchNotes('/api', SLUG, {}, fn);
	assert.deepEqual(
		notes.map((n) => n.id),
		[1]
	);
	assert.equal(calls[0].url, '/api/admin/evidence?slug=usa-wa-sos&status=published');
	const facts = await fetchAcceptedFacts('/api', SLUG, fn);
	assert.equal(facts[0].id, 201);
	assert.equal(calls[1].url, '/api/facts?status=accepted&limit=200&authority=usa-wa-sos');
	const created = await createNote('/api', { kind: 'absence' }, fn);
	assert.equal(created.id, 9);
	assert.equal(calls[2].url, '/api/admin/evidence');
	assert.equal(calls[2].init.method, 'POST');
	assert.equal(calls[2].init.body, JSON.stringify({ kind: 'absence' }));
	assert.ok(calls[2].init.headers['x-csrf']);
	assert.equal(calls[2].init.credentials, 'same-origin');
	const withdrawn = await withdrawNote('/api', 9, { reason: 'called back' }, fn);
	assert.equal(withdrawn.status, 'withdrawn');
	assert.equal(calls[3].url, '/api/admin/evidence/9/withdraw');
	assert.deepEqual(await retireFact('/api', 202, { reason: 'wrong page, wrong box' }, fn), {
		id: 202,
		authority_slug: SLUG,
		status: 'superseded'
	});
	assert.equal(calls[4].url, '/api/admin/facts/202/retire');
	assert.equal(calls[4].init.body, JSON.stringify({ reason: 'wrong page, wrong box' }));
});

test("refusals carry the server's own reason; other failures say what happened", async () => {
	const reason = "summary has the number 2065550199, which is not in this office's accepted facts";
	const { fn } = fakeFetch([
		{ status: 422, body: { error: reason } },
		{ status: 409, body: { error: 'decision 12 rests on this note; withdraw the decision first' } },
		{ status: 409, body: { error: 'fact is not accepted' } },
		{ status: 401, body: {} },
		{ status: 200, text: '<!doctype html><html></html>' },
		new TypeError('network down'),
		{ status: 503, body: { error: 'store unavailable' } }
	]);
	await assert.rejects(createNote('/api', {}, fn), (err) => {
		assert.ok(err instanceof EvidenceError);
		assert.equal(err.status, 422);
		assert.equal(err.reason, reason);
		assert.equal(err.message, reason);
		return true;
	});
	await assert.rejects(withdrawNote('/api', 3, { reason: 'x' }, fn), (err) => err.reason === 'decision 12 rests on this note; withdraw the decision first');
	await assert.rejects(retireFact('/api', 3, { reason: 'wrong page' }, fn), (err) => err.status === 409 && err.reason === 'fact is not accepted');
	await assert.rejects(fetchNotes('/api', SLUG, {}, fn), (err) => err.status === 401 && err.reason === null && /session/i.test(err.message));
	await assert.rejects(fetchNotes('/api', SLUG, {}, fn), (err) => err.status === 0 && err.message === 'Staff notes is unavailable on this host.');
	await assert.rejects(fetchAcceptedFacts('/api', SLUG, fn), (err) => err.status === 0 && /could not reach/.test(err.message));
	await assert.rejects(fetchAcceptedFacts('/api', SLUG, fn), (err) => err.status === 503);
});
