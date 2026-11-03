import test from 'node:test';
import assert from 'node:assert/strict';
import {
	MAX_CALL_QUESTIONS,
	MAX_EMAIL_QUESTIONS,
	OutreachError,
	addContact,
	appendQuestions,
	discardEmail,
	draftBody,
	draftEmail,
	editProblem,
	emailContacts,
	emailHref,
	fetchEmails,
	fetchQuestions,
	hasQuestion,
	isDirty,
	looksPersonal,
	modelLabel,
	normalizeEmail,
	normalizeEmailList,
	normalizeQuestionSet,
	parseEmailQuery,
	phoneContacts,
	phoneLabel,
	questionLines,
	questionProblem,
	reasonLabel,
	saveEmail,
	sendEmail,
	sentInLastDay,
	statusTone,
	toE164,
	topicLabel,
	validEmail
} from '../src/lib/admin/outreach-client.ts';

const EMAIL = {
	id: 31,
	slug: 'usa-wa-sos',
	to: 'apostille-desk@sos.example.test',
	subject: 'Question about apostille procedures',
	body: 'Hello team,\n\n1. Fee?\n\nThanks.',
	questions: ['Fee?'],
	status: 'draft',
	model: 'muse-spark-1.3',
	note: '',
	error: '',
	drafted_at: 1793750400,
	sent_at: null,
	reply_capture_id: null
};

/** Fake fetch recording calls; answers in order. */
function fakeFetch(answers) {
	const calls = [];
	const fn = async (url, init = {}) => {
		calls.push({ url, init });
		const next = answers.shift();
		if (next instanceof Error) throw next;
		return new Response(JSON.stringify(next.json ?? {}), {
			status: next.status ?? 200,
			headers: { 'content-type': 'application/json' }
		});
	};
	return { fn, calls };
}

test('normalizeQuestionSet keeps valid rows, drops junk and duplicates', () => {
	const set = normalizeQuestionSet({
		slug: 'usa-wa-sos',
		name: 'USA — Washington Secretary of State',
		questions: [
			{ topic: 'fees', question: ' Fee? ', reason: 'contradicted', why: 'w', priority: 'high', fact_ids: [104, 'x', 105] },
			{ topic: 'fees', question: 'Fee?', reason: 'stale', why: '', priority: 'low', fact_ids: [] },
			{ topic: 'po_box', question: 'PO box?', reason: 'missing', priority: 'urgent' },
			{ topic: 'hours', question: 'x'.repeat(281), priority: 'low' },
			{ question: 'No topic?', priority: 'low' },
			null
		],
		contacts: [
			{ id: 1, kind: 'email', value: 'a@sos.example.test', details: 'Desk', visibility: 'private' },
			{ id: 2, kind: 'phone', value: '+15555550142', details: '', visibility: 'private' },
			{ id: 3, kind: 'fax', value: '1' }
		]
	});
	assert.ok(set);
	assert.equal(set.questions.length, 1);
	assert.deepEqual(set.questions[0], {
		topic: 'fees',
		question: 'Fee?',
		reason: 'contradicted',
		why: 'w',
		priority: 'high',
		fact_ids: [104, 105]
	});
	assert.equal(set.contacts.length, 2);
	assert.equal(emailContacts(set.contacts).length, 1);
	assert.equal(phoneContacts(set.contacts)[0].value, '+15555550142');
	assert.equal(normalizeQuestionSet({ questions: [] }), null);
	assert.equal(normalizeQuestionSet(null), null);
	assert.equal(normalizeQuestionSet({ slug: 'x', questions: [] })?.name, 'x');
});

test('appendQuestions dedupes (case and spacing), respects the cap', () => {
	const out = appendQuestions('Fee?\n\n  Hours? ', ['fee?', 'Hours?', 'PO box?', '', 'Mailing  address?']);
	assert.equal(out.text, 'Fee?\nHours?\nPO box?\nMailing  address?');
	assert.equal(out.added, 2);
	assert.equal(out.skipped, 3);
	assert.equal(out.full, false);

	const many = Array.from({ length: 25 }, (_, i) => `Q${i}?`);
	const capped = appendQuestions('', many);
	assert.equal(questionLines(capped.text).length, MAX_CALL_QUESTIONS);
	assert.equal(capped.added, MAX_CALL_QUESTIONS);
	assert.equal(capped.full, true);

	assert.equal(hasQuestion('Fee?\nHours?', '  hours? '), true);
	assert.equal(hasQuestion('Fee?', 'Hours?'), false);
});

test('emailHref and parseEmailQuery round-trip, capped at 12', () => {
	const qs = Array.from({ length: 15 }, (_, i) => `Question ${i} & more?`);
	const href = emailHref('/email', 'usa-wa-sos', ['Fee?', 'fee?', ...qs, 'x'.repeat(281)]);
	assert.ok(href.startsWith('/email?authority=usa-wa-sos&q='));
	const back = parseEmailQuery(href.slice(href.indexOf('?')));
	assert.equal(back.authority, 'usa-wa-sos');
	assert.equal(back.questions.length, MAX_EMAIL_QUESTIONS);
	assert.equal(back.questions[0], 'Fee?');
	assert.equal(back.questions[1], 'Question 0 & more?');
	assert.equal(emailHref('/email', '', []), '/email');
	assert.deepEqual(parseEmailQuery('?authority=Bad Slug!&q=%20'), { authority: '', questions: [] });
});

test('validEmail and looksPersonal mirror the server', () => {
	assert.equal(validEmail('apostille-desk@sos.example.test'), true);
	for (const bad of ['', 'a@b', '@x.test', 'a@@x.test', 'a b@x.test', 'a@.x.test', 'a@x.test.', 'Name <a@x.test>', 42]) {
		assert.equal(validEmail(bad), false, String(bad));
	}
	assert.equal(looksPersonal('Call 555-555-0142 about it'), true);
	assert.equal(looksPersonal('Email a@x.test'), true);
	assert.equal(looksPersonal('Order ord_k2Lp9vW3sQ'), true);
	assert.equal(looksPersonal('Is the fee 15 dollars per document in 2026?'), false);
	assert.equal(questionProblem(''), 'Write the question first.');
	assert.match(questionProblem('x'.repeat(281)) ?? '', /under 280/);
	assert.match(questionProblem('Ask 1234567') ?? '', /public procedure/);
	assert.equal(questionProblem('Which form?'), null);
});

test('draftBody validates slug, recipient and 1–12 questions', () => {
	const ok = draftBody('usa-wa-sos', ' Desk@SOS.example.test ', [' Fee? ', '', 'Hours?']);
	assert.deepEqual(ok, {
		ok: true,
		body: { slug: 'usa-wa-sos', to: 'desk@sos.example.test', questions: ['Fee?', 'Hours?'] }
	});
	assert.equal(draftBody('', 'a@x.test', ['q']).ok, false);
	assert.equal(draftBody('usa-wa-sos', 'nope', ['q']).ok, false);
	assert.equal(draftBody('usa-wa-sos', 'a@x.test', ['  ']).ok, false);
	assert.equal(draftBody('usa-wa-sos', 'a@x.test', new Array(13).fill('q')).ok, false);
	assert.equal(draftBody('usa-wa-sos', 'a@x.test', ['reach me at a@x.test']).ok, false);
});

test('editProblem and isDirty', () => {
	const edit = { to: EMAIL.to, subject: EMAIL.subject, body: EMAIL.body };
	assert.equal(editProblem(edit), null);
	assert.match(editProblem({ ...edit, subject: 'a\nb' }) ?? '', /one line/);
	assert.match(editProblem({ ...edit, subject: 'x'.repeat(141) }) ?? '', /140/);
	assert.match(editProblem({ ...edit, body: '  ' }) ?? '', /empty/);
	assert.match(editProblem({ ...edit, to: 'x' }) ?? '', /recipient/);
	const email = normalizeEmail(EMAIL);
	assert.equal(isDirty(email, edit), false);
	assert.equal(isDirty(email, { ...edit, body: `${EMAIL.body}\n` }), false);
	assert.equal(isDirty(email, { ...edit, subject: 'Other' }), true);
	assert.equal(isDirty(null, edit), false);
});

test('email list shape, daily count, labels', () => {
	const now = 1793750400;
	const { emails, dailyCap } = normalizeEmailList({
		emails: [
			{ ...EMAIL, id: 1, status: 'sent', sent_at: now - 100 },
			{ ...EMAIL, id: 2, status: 'replied', sent_at: now - 3600, reply_capture_id: 88 },
			{ ...EMAIL, id: 3, status: 'sent', sent_at: now - 90_000 },
			{ ...EMAIL, id: 4 },
			{ id: 'bad' }
		],
		daily_cap: 20
	});
	assert.equal(emails.length, 4);
	assert.equal(dailyCap, 20);
	assert.equal(emails[1].reply_capture_id, 88);
	assert.equal(sentInLastDay(emails, now), 2);
	assert.equal(normalizeEmailList(null).dailyCap, 20);
	assert.equal(statusTone('sent'), 'done');
	assert.equal(statusTone('failed'), 'error');
	assert.equal(statusTone('discarded'), 'muted');
	assert.equal(modelLabel('muse-spark-1.3'), 'Muse Spark');
	assert.equal(modelLabel('template'), 'Template');
	assert.equal(topicLabel('processing_time'), 'Processing time');
	assert.equal(topicLabel('fees'), 'Fees');
	assert.equal(reasonLabel('missing_core'), 'Missing core topic');
});

test('phone helpers format E.164 and normalize what staff type', () => {
	assert.equal(phoneLabel('+15555550142'), '+1 555 555 0142');
	assert.equal(phoneLabel('not a number'), 'not a number');
	assert.equal(toE164('(360) 555-0100'), '+13605550100');
	assert.equal(toE164('+1 555 555 0142'), '+15555550142');
	assert.equal(toE164('12'), null);
	assert.equal(toE164(''), null);
});

test('fetch helpers hit the staff paths with CSRF on writes', async () => {
	const { fn, calls } = fakeFetch([
		{ json: { slug: 'usa-wa-sos', name: 'WA', questions: [], contacts: [] } },
		{ status: 201, json: { id: 9, contacts: [{ id: 9, kind: 'email', value: 'a@x.test', details: '', visibility: 'private' }] } },
		{ json: { emails: [EMAIL], daily_cap: 20 } },
		{ status: 201, json: { email: EMAIL } },
		{ json: { email: { ...EMAIL, subject: 'Edited' } } },
		{ json: { email: { ...EMAIL, status: 'sent', sent_at: 1 } } },
		{ json: { email: { ...EMAIL, status: 'discarded' } } }
	]);
	assert.equal((await fetchQuestions('usa-wa-sos', fn)).name, 'WA');
	assert.equal((await addContact('usa-wa-sos', { kind: 'email', value: ' a@x.test ' }, fn)).id, 9);
	assert.equal((await fetchEmails('usa-wa-sos', fn)).emails[0].id, 31);
	assert.equal((await draftEmail({ slug: 'usa-wa-sos', to: 'a@x.test', questions: ['Fee?'] }, fn)).model, 'muse-spark-1.3');
	assert.equal((await saveEmail(31, { to: 'A@x.test ', subject: ' Edited ', body: 'Body ' }, fn)).subject, 'Edited');
	assert.equal((await sendEmail(31, fn)).status, 'sent');
	assert.equal((await discardEmail(31, fn)).status, 'discarded');

	assert.equal(calls[0].url, '/apostille/api/v1/admin/first-source/usa-wa-sos/questions');
	assert.equal(calls[0].init.method, 'GET');
	assert.equal(calls[1].init.method, 'POST');
	assert.deepEqual(JSON.parse(calls[1].init.body), { kind: 'email', value: 'a@x.test', details: '' });
	assert.equal(calls[2].url, '/apostille/api/v1/admin/first-source/emails?slug=usa-wa-sos');
	assert.equal(calls[4].init.method, 'PUT');
	assert.equal(calls[4].url, '/apostille/api/v1/admin/first-source/emails/31');
	assert.deepEqual(JSON.parse(calls[4].init.body), { to: 'a@x.test', subject: 'Edited', body: 'Body' });
	assert.equal(calls[5].url, '/apostille/api/v1/admin/first-source/emails/31/send');
	for (const c of calls.slice(1)) {
		if (c.init.method === 'GET') continue;
		assert.equal(typeof c.init.headers['x-csrf'], 'string', c.url);
	}
});

test('errors carry status, server text and a failed send row', async () => {
	const { fn } = fakeFetch([
		{ status: 502, json: { error: 'smtp refused', email: { ...EMAIL, status: 'failed', error: 'smtp refused' } } },
		{ status: 503, json: { error: 'no mailer configured (mailer.provider in /config)' } },
		{ status: 401, json: {} },
		new TypeError('network down'),
		{ status: 404, json: { error: 'unknown authority slug nope' } }
	]);
	await assert.rejects(sendEmail(31, fn), (err) => {
		assert.ok(err instanceof OutreachError);
		assert.equal(err.status, 502);
		assert.equal(err.message, 'Smtp refused.');
		assert.equal(err.email?.status, 'failed');
		return true;
	});
	await assert.rejects(sendEmail(31, fn), (err) => err.status === 503 && /mailer\.provider in \/config\)\.$/.test(err.message));
	await assert.rejects(fetchEmails(null, fn), (err) => err.status === 401 && /log in/.test(err.message));
	await assert.rejects(fetchEmails(null, fn), (err) => err.status === 0 && /could not reach/.test(err.message));
	await assert.rejects(fetchQuestions('nope', fn), (err) => err.status === 404 && /unknown authority/i.test(err.message));
});
