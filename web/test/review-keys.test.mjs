import test from 'node:test';
import assert from 'node:assert/strict';
import {
	LOCKED_TOPIC,
	canMoveTopic,
	chipStatement,
	editAcceptBody,
	keyNote,
	rekey,
	statementKey,
	topicChoices
} from '../src/lib/admin/review-keys.js';

test('rekey replaces or adds only the key prefix', () => {
	assert.equal(
		rekey('standard_fee: Standard processing: Fee is $15.00 per document.', 'per_document'),
		'per_document: Standard processing: Fee is $15.00 per document.'
	);
	assert.equal(
		rekey('PO Box 40220 Olympia, WA 98504-0220', 'apostille_mailing_address'),
		'apostille_mailing_address: PO Box 40220 Olympia, WA 98504-0220'
	);
	// The value keeps its case, punctuation and line breaks.
	assert.equal(
		rekey('office:\nTown Center 3\nTumwater', 'office__tumwater'),
		'office__tumwater: Town Center 3\nTumwater'
	);
	// Not keys under the grammar: capitalised, no space after the colon, a URL.
	assert.equal(rekey('Note: call first', 'notice'), 'notice: Note: call first');
	assert.equal(rekey('fee:$15', 'per_document'), 'per_document: fee:$15');
	assert.equal(rekey('https://www.sos.wa.gov/verify', 'verify'), 'verify: https://www.sos.wa.gov/verify');
	// A key with no value yet only swaps the key.
	assert.equal(rekey('fee:', 'per_document'), 'per_document: ');
	assert.equal(rekey('', 'per_document'), 'per_document: ');
});

test('statementKey reads the key with its instance', () => {
	assert.equal(statementKey('weekday__cheney: Monday to Friday'), 'weekday__cheney');
	assert.equal(statementKey('per_document: $8'), 'per_document');
	assert.equal(statementKey('Fee is $8.'), null);
	assert.equal(statementKey('fee:$8'), null);
});

test('chipStatement carries the instance only into slots that hold one per instance', () => {
	// The alias implies the instance.
	assert.equal(
		chipStatement('apostille_eastern_branch_street: 960 Washington St Cheney, WA 99004', 'office', 'address', 'address'),
		'office__cheney: 960 Washington St Cheney, WA 99004'
	);
	// A restore names the notice it resolves.
	assert.equal(
		chipStatement('notice__2026_11_02: Apostille services have been restored.', 'restored', 'service_notice', 'service_notice'),
		'restored__2026_11_02: Apostille services have been restored.'
	);
	// A closure filed as a notice moves to holiday_hours with its date id.
	assert.equal(
		chipStatement('closure__2026_11_11: Closed for Veterans Day.', 'closure', 'holiday_hours', 'service_notice'),
		'closure__2026_11_11: Closed for Veterans Day.'
	);
	// Phone hours are one value: the office instance drops.
	assert.equal(
		chipStatement('weekday__cheney: Monday to Friday, eight to four.', 'phone_hours', 'contacts', 'hours'),
		'phone_hours: Monday to Friday, eight to four.'
	);
	// Unkeyed statements gain the key.
	assert.equal(
		chipStatement('PO Box 40234 Olympia, WA 98504-0234', 'apostille_mailing_address', 'mailing', 'mailing'),
		'apostille_mailing_address: PO Box 40234 Olympia, WA 98504-0234'
	);
});

test('keyNote says how the page will label the key', () => {
	assert.equal(keyNote('standard_fee: Fee is $8.', 'fees'), 'The page labels it “Fee per document”.');
	assert.equal(
		keyNote('apostille_main_branch_street: Town Center 3', 'address'),
		'The page labels it “Office address” (tumwater).'
	);
	assert.match(keyNote('mystery_key: Fee is $8.', 'fees'), /Also on the office's pages/);
	assert.match(keyNote('Fee is $8.', 'fees'), /^No key yet/);
	assert.match(keyNote('script: Ask about fees.', LOCKED_TOPIC), /never show/);
});

test('canMoveTopic keeps call scripts where they are', () => {
	assert.equal(canMoveTopic('service_notice', 'holiday_hours'), true);
	assert.equal(canMoveTopic('fees', 'fees'), true);
	assert.equal(canMoveTopic('fees', LOCKED_TOPIC), false);
	assert.equal(canMoveTopic(LOCKED_TOPIC, 'fees'), false);
	assert.equal(canMoveTopic(LOCKED_TOPIC, LOCKED_TOPIC), true);
	assert.equal(canMoveTopic('fees', 'not_a_topic'), false);
	// A draft whose topic left the vocabulary may still move into it.
	assert.equal(canMoveTopic('membership', 'jurisdiction'), true);
});

test('editAcceptBody sends the topic only when the reviewer moved the draft', () => {
	const draft = { topic: 'service_notice', statement: 'closure__2026_11_11: Closed for Veterans Day.' };
	assert.deepEqual(editAcceptBody(draft, draft.statement, 'service_notice'), {});
	assert.deepEqual(editAcceptBody(draft, draft.statement, ''), {});
	assert.deepEqual(editAcceptBody(draft, 'closure__2026_11_11: Closed.', 'service_notice'), {
		statement: 'closure__2026_11_11: Closed.'
	});
	assert.deepEqual(editAcceptBody(draft, draft.statement, 'holiday_hours'), { topic: 'holiday_hours' });
	assert.deepEqual(editAcceptBody(draft, 'closure__2026_11_11: Closed.', 'holiday_hours'), {
		statement: 'closure__2026_11_11: Closed.',
		topic: 'holiday_hours'
	});
	assert.equal(editAcceptBody(draft, draft.statement, LOCKED_TOPIC), null);
	assert.equal(editAcceptBody(draft, draft.statement, 'not_a_topic'), null);
	assert.equal(editAcceptBody(draft, '   ', 'holiday_hours'), null);
});

test('topicChoices lists every frozen topic once, alphabetically, with where it shows', () => {
	const choices = topicChoices();
	assert.equal(choices.length, 26);
	assert.deepEqual(
		choices.map((c) => c.topic),
		[...choices.map((c) => c.topic)].sort()
	);
	const where = Object.fromEntries(choices.map((c) => [c.topic, c.where]));
	assert.equal(where.holiday_hours, 'Send or bring your documents');
	assert.equal(where.fees, 'Pay the fees');
	assert.equal(where.service_notice, 'Service status banner');
	assert.equal(where[LOCKED_TOPIC], 'Never shown publicly');
	for (const c of choices) assert.ok(c.where, c.topic);
});
