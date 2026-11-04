import test from 'node:test';
import assert from 'node:assert/strict';
import {
	bytesToBase64,
	composeMail,
	composeProblem,
	fakeRibbon,
	fakeSlugs,
	fetchOutbox,
	normalizeOutbox,
	replyDraft
} from '../src/lib/admin/fake-client.js';

test('the FAKE ribbon names every fake integration', () => {
	const fakes = [
		{ slug: 'mail_out', label: 'Outgoing email' },
		{ slug: 'labels', label: 'FedEx/USPS labels' }
	];
	const text = fakeRibbon(fakes);
	assert.ok(text?.startsWith('FAKE — Outgoing email, FedEx/USPS labels.'), text ?? '');
	assert.match(text ?? '', /nothing here sends mail, ships a parcel or charges a card/);
	assert.deepEqual(fakeSlugs(fakes), ['mail_out', 'labels']);
});

test('no fakes, no ribbon', () => {
	for (const fakes of [undefined, null, [], {}, 'mail_out', [{ slug: 'x' }], [{ label: ' ' }]]) {
		assert.equal(fakeRibbon(fakes), null, JSON.stringify(fakes));
	}
	assert.deepEqual(fakeSlugs([{ label: 'x' }, null, { slug: 'labels' }]), ['labels']);
});

const OUTREACH = {
	id: 3,
	at: 1_793_750_000,
	kind: 'outreach',
	to: 'apostille@example.gov',
	subject: 'Two questions',
	headers: { 'Message-ID': '<outreach-7-ab@andrii.ai>', 'X-Bad': 5 },
	body: 'Hello'
};

test('the outbox normalizes and drops malformed rows', () => {
	const box = normalizeOutbox({ outbox: true, inbox: 1, messages: [OUTREACH, { id: 'x' }, null] });
	assert.equal(box.outbox, true);
	assert.equal(box.inbox, false);
	assert.equal(box.messages.length, 1);
	assert.deepEqual(box.messages[0].headers, { 'Message-ID': '<outreach-7-ab@andrii.ai>' });
	assert.deepEqual(normalizeOutbox(null), { outbox: false, inbox: false, messages: [] });
});

test('a reply answers as the recipient and threads on our Message-ID', () => {
	assert.deepEqual(replyDraft(normalizeOutbox({ messages: [OUTREACH] }).messages[0]), {
		from: 'apostille@example.gov',
		subject: 'Re: Two questions',
		body: '',
		in_reply_to: '<outreach-7-ab@andrii.ai>'
	});
	const again = replyDraft({ ...OUTREACH, subject: 'RE: Two questions', headers: {} });
	assert.equal(again.subject, 'RE: Two questions');
	assert.equal(again.in_reply_to, '');
});

test('compose checks mirror the server', () => {
	const ok = { from: 'ops@example.gov', subject: 'Closed', body: 'Closed Monday.', in_reply_to: '', dmarc: 'pass' };
	assert.equal(composeProblem(ok), null);
	assert.match(composeProblem({ ...ok, from: 'Ops <ops@example.gov>' }) ?? '', /plain address/);
	assert.match(composeProblem({ ...ok, subject: 'a\nb' }) ?? '', /one line/);
	assert.match(composeProblem({ ...ok, body: ' ' }) ?? '', /body/);
	assert.match(composeProblem({ ...ok, in_reply_to: 'no-brackets@x' }) ?? '', /message-id/);
	assert.match(composeProblem({ ...ok, dmarc: 'maybe' }) ?? '', /DMARC/);
	assert.equal(composeProblem({ ...ok, in_reply_to: '<outreach-7-ab@andrii.ai>', dmarc: 'fail' }), null);
});

test('attachments encode as standard base64', () => {
	assert.equal(bytesToBase64(new TextEncoder().encode('Open 8-4 Mon-Thu.')), 'T3BlbiA4LTQgTW9uLVRodS4=');
	assert.equal(bytesToBase64(new Uint8Array([0xff, 0x00, 0xfe])), '/wD+');
	const big = new Uint8Array(100_000).fill(65);
	assert.equal(atob(bytesToBase64(big)).length, 100_000);
});

test('compose posts the draft with a CSRF header; errors carry the server text', async () => {
	const calls = [];
	const ok = async (url, init) => {
		calls.push({ url, init });
		return new Response(JSON.stringify({ captured: true, capture_id: 9, trust: 'official' }), { status: 201 });
	};
	const out = await composeMail(
		{ from: ' ops@example.gov ', subject: 'Closed', body: 'Closed Monday.', in_reply_to: ' ', dmarc: 'pass' },
		ok
	);
	assert.equal(out.capture_id, 9);
	assert.equal(calls[0].url, '/apostille/api/v1/admin/fake/inbox');
	assert.equal(calls[0].init.method, 'POST');
	assert.ok(calls[0].init.headers['x-csrf']);
	assert.deepEqual(JSON.parse(calls[0].init.body), {
		from: 'ops@example.gov',
		subject: 'Closed',
		body: 'Closed Monday.',
		dmarc: 'pass',
		attachments: []
	});
	const refused = async () =>
		new Response(JSON.stringify({ error: 'incoming email is not in fake mode (mail.mode)' }), { status: 409 });
	await assert.rejects(
		composeMail({ from: 'a@b.test', subject: 's', body: 'b', in_reply_to: '', dmarc: 'pass' }, refused),
		/Delivering failed: incoming email is not in fake mode \(mail\.mode\)\./
	);
	const listed = await fetchOutbox(async () => new Response(JSON.stringify({ outbox: true, messages: [OUTREACH] })));
	assert.equal(listed.messages[0].kind, 'outreach');
});
