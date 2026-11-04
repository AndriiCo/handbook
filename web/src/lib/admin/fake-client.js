// W-3 fake modes in the staff console. Admin-only: never imported from
// (public) or (customer) routes.
//
// GET  {apiBase}/admin/version     → { …, fakes: [{ slug, label }] }
//   names every integration running its local fake (`<area>.mode = fake`;
//   the server refuses fakes off loopback unless the host is staging).
// GET  {apiBase}/admin/fake/outbox → { outbox, inbox, messages: [OutboxMessage] }
//   every email the fake outbox kept instead of sending (newest first).
// POST {apiBase}/admin/fake/inbox  → 201 { captured, capture_id, trust, verified_by, authority, outreach_email }
//   body { from, subject, body, in_reply_to?, dmarc: 'pass'|'fail', attachments?: [{ name, content_type, data_base64 }] }
//   200 { captured: false, duplicate } | { captured: false, suppressed }; 409 not in fake mode; 422 bad field

import { csrfHeaders } from './config-client.js';
import { authError, readErrorBody } from './http-errors.js';

const API = '/apostille/api/v1';

/**
 * @typedef {{ slug: string, label: string }} FakeIntegration
 * @typedef {{ id: number, at: number, kind: string, to: string, subject: string, headers: Record<string, string>, body: string }} OutboxMessage
 * @typedef {{ outbox: boolean, inbox: boolean, messages: OutboxMessage[] }} Outbox
 * @typedef {{ from: string, subject: string, body: string, in_reply_to: string }} ComposeDraft
 * @typedef {{ name: string, content_type: string, data_base64: string }} ComposeAttachment
 */

/**
 * The FAKE ribbon's text, or null when nothing runs fake. Pure (tested).
 * @param {unknown} fakes the `fakes` array from /admin/version
 * @returns {string | null}
 */
export function fakeRibbon(fakes) {
	const names = fakeSlugs(fakes, 'label');
	if (names.length === 0) return null;
	return `FAKE — ${names.join(', ')}. Demo stand-ins: nothing here sends mail, ships a parcel or charges a card.`;
}

/**
 * One field of every well-formed entry in /admin/version's `fakes`. Pure.
 * @param {unknown} fakes
 * @param {'slug' | 'label'} [field]
 * @returns {string[]}
 */
export function fakeSlugs(fakes, field = 'slug') {
	if (!Array.isArray(fakes)) return [];
	return fakes
		.map((f) => (f && typeof f === 'object' ? /** @type {Record<string, unknown>} */ (f)[field] : null))
		.filter((v) => typeof v === 'string' && v.trim() !== '')
		.map((v) => /** @type {string} */ (v));
}

/** What a kept message was, in staff words. */
export const KIND_LABEL = /** @type {Record<string, string>} */ ({
	mail: 'Sign-in link',
	digest: 'Follower digest',
	outreach: 'Outreach'
});

/**
 * Normalize the outbox list; anything malformed is dropped. Pure (tested).
 * @param {unknown} data
 * @returns {Outbox}
 */
export function normalizeOutbox(data) {
	const d = /** @type {Record<string, unknown>} */ (data && typeof data === 'object' ? data : {});
	const messages = (Array.isArray(d.messages) ? d.messages : [])
		.map((m) => {
			const r = /** @type {Record<string, unknown>} */ (m && typeof m === 'object' ? m : {});
			if (typeof r.id !== 'number' || typeof r.to !== 'string') return null;
			const raw = r.headers && typeof r.headers === 'object' ? r.headers : {};
			/** @type {Record<string, string>} */
			const headers = {};
			for (const [k, v] of Object.entries(raw)) if (typeof v === 'string') headers[k] = v;
			return {
				id: r.id,
				at: typeof r.at === 'number' ? r.at : 0,
				kind: typeof r.kind === 'string' ? r.kind : 'mail',
				to: r.to,
				subject: typeof r.subject === 'string' ? r.subject : '',
				headers,
				body: typeof r.body === 'string' ? r.body : ''
			};
		})
		.filter((m) => m !== null);
	return { outbox: d.outbox === true, inbox: d.inbox === true, messages: /** @type {OutboxMessage[]} */ (messages) };
}

/**
 * Compose prefill answering a kept message as its recipient: From is
 * the address we wrote to, the subject gains one "Re: ", and
 * In-Reply-To names our Message-ID so the reply threads. Pure (tested).
 * @param {OutboxMessage} message
 * @returns {ComposeDraft}
 */
export function replyDraft(message) {
	const subject = /^re:/i.test(message.subject.trim()) ? message.subject.trim() : `Re: ${message.subject.trim()}`;
	return {
		from: message.to,
		subject,
		body: '',
		in_reply_to: message.headers['Message-ID'] ?? ''
	};
}

/**
 * Standard base64 of raw bytes (attachments). Pure (tested).
 * @param {Uint8Array} bytes
 * @returns {string}
 */
export function bytesToBase64(bytes) {
	let binary = '';
	for (let i = 0; i < bytes.length; i += 0x8000) {
		binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
	}
	return btoa(binary);
}

/**
 * Client-side checks mirroring the server (`mail::fake::fake_message`):
 * the first problem, or null. Pure (tested).
 * @param {ComposeDraft & { dmarc: string }} draft
 * @returns {string | null}
 */
export function composeProblem(draft) {
	if (!/^[A-Za-z0-9._+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/.test(draft.from.trim())) {
		return 'From needs one plain address, like ops@example.gov.';
	}
	if (/[\r\n]/.test(draft.subject) || draft.subject.length > 500) return 'Subject is one line of at most 500 characters.';
	if (draft.body.trim() === '') return 'Write the message body.';
	const reply = draft.in_reply_to.trim();
	if (reply && !/^<[^<>\s]+@[^<>\s]+>$/.test(reply)) return 'In-Reply-To is one <message-id> in angle brackets.';
	if (draft.dmarc !== 'pass' && draft.dmarc !== 'fail') return 'Choose DMARC pass or fail.';
	return null;
}

/** A refused or failed fake request, with its HTTP status. */
export class FakeError extends Error {
	/**
	 * @param {string} message
	 * @param {number} status
	 */
	constructor(message, status) {
		super(message);
		this.name = 'FakeError';
		this.status = status;
	}
}

/**
 * @param {'GET' | 'POST'} method
 * @param {string} path
 * @param {unknown} body
 * @param {string} what
 * @param {typeof fetch} fetchFn
 */
async function request(method, path, body, what, fetchFn) {
	let res;
	try {
		res = await fetchFn(`${API}${path}`, {
			method,
			credentials: 'same-origin',
			headers: method === 'GET' ? undefined : { 'content-type': 'application/json', ...csrfHeaders() },
			body: method === 'GET' ? undefined : JSON.stringify(body ?? {})
		});
	} catch {
		throw new FakeError(`${what} could not reach the admin host.`, 0);
	}
	if (!res.ok) {
		const data = await readErrorBody(res);
		const auth = authError(res.status, data);
		if (auth) throw new FakeError(auth, res.status);
		const server = typeof data?.error === 'string' && data.error ? data.error : '';
		throw new FakeError(server ? `${what} failed: ${server}.` : `${what} failed (HTTP ${res.status}).`, res.status);
	}
	return res.json();
}

/**
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<Outbox>}
 */
export async function fetchOutbox(fetchFn = fetch) {
	return normalizeOutbox(await request('GET', '/admin/fake/outbox', undefined, 'The outbox', fetchFn));
}

/**
 * Deliver one composed message into the fake inbox.
 * @param {ComposeDraft & { dmarc: string, attachments?: ComposeAttachment[] }} draft
 * @param {typeof fetch} [fetchFn]
 */
export async function composeMail(draft, fetchFn = fetch) {
	const reply = draft.in_reply_to.trim();
	return request(
		'POST',
		'/admin/fake/inbox',
		{
			from: draft.from.trim(),
			subject: draft.subject.trim(),
			body: draft.body,
			...(reply ? { in_reply_to: reply } : {}),
			dmarc: draft.dmarc,
			attachments: draft.attachments ?? []
		},
		'Delivering',
		fetchFn
	);
}
