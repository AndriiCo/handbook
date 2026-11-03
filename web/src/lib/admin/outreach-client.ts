// First source client (P358 server, `crates/web/src/first_source.rs`):
// staff asking the office itself. Admin-only: never imported from
// (public) or (customer) routes.
//
// GET  /admin/first-source/{slug}/questions  → { slug, name, questions, contacts }
// POST /admin/first-source/{slug}/contacts   → 201 { id, contacts }
// GET  /admin/first-source/emails?slug=      → { emails, daily_cap }
// POST /admin/first-source/emails            → 201 { email }   (a live Muse Spark call)
// PUT  /admin/first-source/emails/{id}       → { email }       (draft only, else 409)
// POST /admin/first-source/emails/{id}/send  → { email } | 502 { error, email } | 503 | 429 | 409
// POST /admin/first-source/emails/{id}/discard → { email }
//
// The pure helpers below mirror the server's bounds (questions 1–12
// per email and 1–20 per call, 280 chars each; subject one line of
// 140; body 8000; nothing that looks like customer data), so the page
// explains a refusal before anything is sent. Node tests import this
// file directly (type stripping): keep it free of `.ts` imports.
import { parsePhoneNumberFromString } from 'libphonenumber-js';
import { csrfHeaders } from './config-client.js';
import { authError, readErrorBody } from './http-errors.js';

const API = '/apostille/api/v1';

export const MAX_CALL_QUESTIONS = 20;
export const MAX_EMAIL_QUESTIONS = 12;
export const MAX_QUESTION_CHARS = 280;
export const MAX_SUBJECT = 140;
export const MAX_BODY = 8000;
/** The batch "Add top 6" adds (TOPICS.md batching rule). */
export const TOP_N = 6;

export type Priority = 'high' | 'medium' | 'low';

export interface OpenQuestion {
	topic: string;
	question: string;
	reason: string;
	why: string;
	priority: Priority;
	fact_ids: number[];
}

export interface Contact {
	id: number;
	kind: 'email' | 'phone' | 'webform';
	value: string;
	details: string;
	visibility: string;
}

export interface QuestionSet {
	slug: string;
	name: string;
	questions: OpenQuestion[];
	contacts: Contact[];
}

export type EmailStatus = 'draft' | 'sending' | 'sent' | 'failed' | 'discarded' | 'replied';

export interface OutreachEmail {
	id: number;
	slug: string;
	to: string;
	subject: string;
	body: string;
	questions: string[];
	status: EmailStatus | string;
	model: string;
	note: string;
	error: string;
	drafted_at: number;
	sent_at: number | null;
	reply_capture_id: number | null;
}

export interface EmailEdit {
	to: string;
	subject: string;
	body: string;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Loose = any;

// ------------------------------------------------------------ labels

export const PRIORITY_LABEL: Record<Priority, string> = {
	high: 'High',
	medium: 'Medium',
	low: 'Low'
};

export const REASON_LABEL: Record<string, string> = {
	contradicted: 'Contradicted',
	open_task: 'Open task',
	missing_core: 'Missing core topic',
	unconfirmed: 'Unconfirmed',
	not_found: 'Not in source',
	stale: 'Stale',
	missing: 'Missing'
};

const TOPIC_LABEL: Record<string, string> = {
	po_box: 'PO box',
	processing_time: 'Processing time',
	legalization_chain: 'Legalization chain',
	id_requirements: 'ID requirements',
	walkin: 'Walk-in',
	holiday_hours: 'Holiday hours',
	dropbox: 'Drop box'
};

/** `processing_time` → "Processing time". */
export function topicLabel(topic: string): string {
	if (topic in TOPIC_LABEL) return TOPIC_LABEL[topic];
	const s = String(topic ?? '').replaceAll('_', ' ').trim();
	return s ? s.charAt(0).toUpperCase() + s.slice(1) : 'General';
}

export function reasonLabel(reason: string): string {
	return REASON_LABEL[reason] ?? topicLabel(reason);
}

export const STATUS_LABEL: Record<EmailStatus, string> = {
	draft: 'Draft',
	sending: 'Sending',
	sent: 'Sent',
	failed: 'Failed',
	discarded: 'Discarded',
	replied: 'Replied'
};

export function statusLabel(status: string): string {
	return (STATUS_LABEL as Record<string, string>)[status] ?? topicLabel(status);
}

/** Tone for a status pill: verified green, changed amber, neutral. */
export function statusTone(status: string): 'done' | 'active' | 'attention' | 'muted' | 'error' {
	if (status === 'sent' || status === 'replied') return 'done';
	if (status === 'sending') return 'active';
	if (status === 'draft') return 'attention';
	if (status === 'failed') return 'error';
	return 'muted';
}

/** "Muse Spark" for a model draft, "Template" for the fixed fallback. */
export function modelLabel(model: string): string {
	if (model === 'template' || !model) return 'Template';
	if (model.startsWith('muse-spark')) return 'Muse Spark';
	return model;
}

// ------------------------------------------------------------- shapes

const PRIORITIES = new Set(['high', 'medium', 'low']);

function str(v: unknown): string {
	return typeof v === 'string' ? v : '';
}

/** Normalize one question row; null for unknown shapes. Pure (tested). */
export function normalizeQuestion(row: Loose): OpenQuestion | null {
	if (!row || typeof row.topic !== 'string' || typeof row.question !== 'string') return null;
	const question = row.question.trim();
	if (!question || question.length > MAX_QUESTION_CHARS) return null;
	if (!PRIORITIES.has(row.priority)) return null;
	return {
		topic: row.topic,
		question,
		reason: str(row.reason),
		why: str(row.why),
		priority: row.priority as Priority,
		fact_ids: Array.isArray(row.fact_ids)
			? row.fact_ids.filter((n: unknown) => typeof n === 'number' && Number.isInteger(n))
			: []
	};
}

export function normalizeContact(row: Loose): Contact | null {
	if (!row || typeof row.id !== 'number' || typeof row.value !== 'string') return null;
	if (row.kind !== 'email' && row.kind !== 'phone' && row.kind !== 'webform') return null;
	return {
		id: row.id,
		kind: row.kind,
		value: row.value,
		details: str(row.details),
		visibility: str(row.visibility)
	};
}

/** The questions endpoint's body; null for unknown shapes. Pure (tested). */
export function normalizeQuestionSet(data: Loose): QuestionSet | null {
	if (!data || typeof data.slug !== 'string' || !Array.isArray(data.questions)) return null;
	const seen = new Set<string>();
	const questions: OpenQuestion[] = [];
	for (const row of data.questions) {
		const q = normalizeQuestion(row);
		if (!q || seen.has(q.question)) continue;
		seen.add(q.question);
		questions.push(q);
	}
	const contacts = (Array.isArray(data.contacts) ? data.contacts : [])
		.map(normalizeContact)
		.filter((c: Contact | null): c is Contact => c !== null);
	return {
		slug: data.slug,
		name: typeof data.name === 'string' && data.name ? data.name : data.slug,
		questions,
		contacts
	};
}

export function normalizeEmail(row: Loose): OutreachEmail | null {
	if (!row || typeof row.id !== 'number' || typeof row.status !== 'string') return null;
	return {
		id: row.id,
		slug: str(row.slug),
		to: str(row.to),
		subject: str(row.subject),
		body: str(row.body),
		questions: Array.isArray(row.questions)
			? row.questions.filter((q: unknown): q is string => typeof q === 'string')
			: [],
		status: row.status,
		model: str(row.model),
		note: str(row.note),
		error: str(row.error),
		drafted_at: typeof row.drafted_at === 'number' ? row.drafted_at : 0,
		sent_at: typeof row.sent_at === 'number' ? row.sent_at : null,
		reply_capture_id: typeof row.reply_capture_id === 'number' ? row.reply_capture_id : null
	};
}

/** `{emails, daily_cap}`; newest first as the server sends them. */
export function normalizeEmailList(data: Loose): { emails: OutreachEmail[]; dailyCap: number } {
	const emails = (Array.isArray(data?.emails) ? data.emails : [])
		.map(normalizeEmail)
		.filter((e: OutreachEmail | null): e is OutreachEmail => e !== null);
	const dailyCap = typeof data?.daily_cap === 'number' ? data.daily_cap : 20;
	return { emails, dailyCap };
}

/** Sends counted against the rolling 24 h cap (sent or replied). */
export function sentInLastDay(emails: OutreachEmail[], now: number): number {
	return emails.filter(
		(e) => (e.status === 'sent' || e.status === 'replied') && e.sent_at !== null && e.sent_at >= now - 86_400
	).length;
}

// ---------------------------------------------------------- questions

/** Non-empty trimmed lines of the call's question box. */
export function questionLines(text: string): string[] {
	return String(text ?? '')
		.split('\n')
		.map((line) => line.trim())
		.filter((line) => line.length > 0);
}

const norm = (q: string) => q.trim().replace(/\s+/g, ' ').toLowerCase();

/**
 * Append questions to the call box: skips lines already there (case
 * and spacing ignored) and stops at the cap. Pure (tested).
 */
export function appendQuestions(
	existingText: string,
	lines: string[],
	cap = MAX_CALL_QUESTIONS
): { text: string; added: number; skipped: number; full: boolean } {
	const have = questionLines(existingText);
	const seen = new Set(have.map(norm));
	const out = [...have];
	let added = 0;
	let skipped = 0;
	let full = false;
	for (const raw of lines) {
		const line = String(raw ?? '').trim();
		if (!line || seen.has(norm(line))) {
			skipped += 1;
			continue;
		}
		if (out.length >= cap) {
			full = true;
			skipped += 1;
			continue;
		}
		seen.add(norm(line));
		out.push(line);
		added += 1;
	}
	return { text: out.join('\n'), added, skipped, full };
}

/** True when the box already holds this question (case/spacing ignored). */
export function hasQuestion(existingText: string, question: string): boolean {
	const want = norm(question);
	return questionLines(existingText).some((line) => norm(line) === want);
}

const SLUG = /^[a-z0-9-]{1,80}$/;

/**
 * `/email?authority=<slug>&q=…&q=…` for "Email these questions
 * instead": at most 12 questions, each within the server bound.
 */
export function emailHref(base: string, slug: string, questions: string[]): string {
	const params = new URLSearchParams();
	if (SLUG.test(slug)) params.set('authority', slug);
	const seen = new Set<string>();
	for (const raw of questions) {
		const q = String(raw ?? '').trim();
		if (!q || q.length > MAX_QUESTION_CHARS || seen.has(norm(q))) continue;
		seen.add(norm(q));
		params.append('q', q);
		if (seen.size >= MAX_EMAIL_QUESTIONS) break;
	}
	const query = params.toString();
	return query ? `${base}?${query}` : base;
}

/** The /email deep link back: authority slug plus prefilled questions. */
export function parseEmailQuery(search: string): { authority: string; questions: string[] } {
	const params = new URLSearchParams(search);
	const slug = params.get('authority') ?? '';
	const seen = new Set<string>();
	const questions: string[] = [];
	for (const raw of params.getAll('q')) {
		const q = raw.trim();
		if (!q || q.length > MAX_QUESTION_CHARS || seen.has(norm(q))) continue;
		seen.add(norm(q));
		questions.push(q);
		if (questions.length >= MAX_EMAIL_QUESTIONS) break;
	}
	return { authority: SLUG.test(slug) ? slug : '', questions };
}

// --------------------------------------------------------- validation

/** Mirror of `first_source::valid_email`: one bare mailbox. */
export function validEmail(addr: unknown): boolean {
	if (typeof addr !== 'string') return false;
	const a = addr.trim();
	const at = a.indexOf('@');
	if (at < 1 || a.indexOf('@', at + 1) !== -1) return false;
	const local = a.slice(0, at);
	const domain = a.slice(at + 1);
	return (
		local.length <= 64 &&
		domain.length <= 253 &&
		domain.includes('.') &&
		!domain.startsWith('.') &&
		!domain.endsWith('.') &&
		/^[A-Za-z0-9@._%+-]+$/.test(a)
	);
}

/**
 * Mirror of `first_source::looks_personal`: an email, an order id or
 * a run of 7+ digits (spaces, dashes, dots and brackets do not break
 * the run) never leaves in an outreach question.
 */
export function looksPersonal(text: string): boolean {
	let run = 0;
	for (const c of String(text ?? '')) {
		if (c >= '0' && c <= '9') {
			run += 1;
			if (run >= 7) return true;
		} else if (!' -.()'.includes(c)) {
			run = 0;
		}
	}
	return text.includes('@') || text.includes('ord_');
}

/** Why a custom question cannot go out, or null when it can. */
export function questionProblem(text: string): string | null {
	const q = String(text ?? '').trim();
	if (!q) return 'Write the question first.';
	if (q.length > MAX_QUESTION_CHARS) return `Keep it under ${MAX_QUESTION_CHARS} characters (${q.length} now).`;
	if (looksPersonal(q)) return 'Outreach asks about public procedure only: no emails, order ids or long numbers.';
	return null;
}

/** POST body for a draft, or the reason it cannot be sent. Pure (tested). */
export function draftBody(
	slug: string,
	to: string,
	questions: string[]
): { ok: true; body: { slug: string; to: string; questions: string[] } } | { ok: false; error: string } {
	if (!SLUG.test(slug)) return { ok: false, error: 'Choose an authority first.' };
	const addr = String(to ?? '').trim().toLowerCase();
	if (!validEmail(addr)) return { ok: false, error: 'Choose or add the office email address.' };
	const qs = questions.map((q) => String(q ?? '').trim()).filter((q) => q.length > 0);
	if (qs.length === 0) return { ok: false, error: 'Tick at least one question.' };
	if (qs.length > MAX_EMAIL_QUESTIONS) {
		return { ok: false, error: `An email asks at most ${MAX_EMAIL_QUESTIONS} questions.` };
	}
	for (const q of qs) {
		const problem = questionProblem(q);
		if (problem) return { ok: false, error: problem };
	}
	return { ok: true, body: { slug, to: addr, questions: qs } };
}

/** PUT body for a staff edit, or the reason it cannot be saved. Pure (tested). */
export function editProblem(edit: EmailEdit): string | null {
	if (!validEmail(edit.to)) return 'The recipient must be one email address.';
	const subject = edit.subject.trim();
	if (!subject) return 'Write a subject.';
	if (/[\r\n]/.test(edit.subject)) return 'The subject must be one line.';
	if (subject.length > MAX_SUBJECT) return `Keep the subject under ${MAX_SUBJECT} characters.`;
	if (!edit.body.trim()) return 'The body is empty.';
	if (edit.body.length > MAX_BODY) return `Keep the body under ${MAX_BODY} characters.`;
	return null;
}

/** True when the editor differs from the stored draft. */
export function isDirty(email: OutreachEmail | null, edit: EmailEdit): boolean {
	if (!email) return false;
	return (
		email.to !== edit.to.trim().toLowerCase() ||
		email.subject !== edit.subject.trim() ||
		email.body !== edit.body.trim()
	);
}

export function emailContacts(contacts: Contact[]): Contact[] {
	return contacts.filter((c) => c.kind === 'email');
}

export function phoneContacts(contacts: Contact[]): Contact[] {
	return contacts.filter((c) => c.kind === 'phone');
}

/** `+15184744429` → "+1 518 474 4429" (falls back to the raw value). */
export function phoneLabel(e164: string): string {
	return parsePhoneNumberFromString(String(e164 ?? ''))?.formatInternational() ?? String(e164 ?? '');
}

/**
 * What staff typed → E.164 for the dial box ("(360) 555-0100" →
 * "+13605550100"; bare ten digits read as US/Canada). Null when it
 * cannot be a phone number, so the box keeps what was typed.
 */
export function toE164(raw: string): string | null {
	const text = String(raw ?? '').trim();
	if (!text) return null;
	const parsed = parsePhoneNumberFromString(text, 'US');
	return parsed && parsed.isPossible() ? parsed.number : null;
}

// -------------------------------------------------------------- fetch

/** Error carrying the HTTP status (0 = network) and, for a failed send, the row. */
export class OutreachError extends Error {
	status: number;
	email: OutreachEmail | null;
	constructor(message: string, status: number, email: OutreachEmail | null = null) {
		super(message);
		this.status = status;
		this.email = email;
	}
}

const sentence = (s: string) => {
	const t = s.trim();
	if (!t) return t;
	const up = t.charAt(0).toUpperCase() + t.slice(1);
	return /[.!?]$/.test(up) ? up : `${up}.`;
};

async function request(
	method: 'GET' | 'POST' | 'PUT',
	path: string,
	body: unknown,
	what: string,
	fetchFn: typeof fetch
): Promise<Loose> {
	let res: Response;
	try {
		res = await fetchFn(`${API}${path}`, {
			method,
			credentials: 'same-origin',
			headers:
				method === 'GET' ? undefined : { 'content-type': 'application/json', ...csrfHeaders() },
			body: method === 'GET' ? undefined : JSON.stringify(body ?? {})
		});
	} catch {
		throw new OutreachError(`${what} could not reach the admin host.`, 0);
	}
	if (!res.ok) {
		const data = await readErrorBody(res);
		const auth = authError(res.status, data);
		if (auth) throw new OutreachError(auth, res.status);
		const server = typeof data?.error === 'string' && data.error ? data.error : '';
		const email = normalizeEmail((data as Loose)?.email);
		throw new OutreachError(
			server ? sentence(server) : `${what} failed (HTTP ${res.status}).`,
			res.status,
			email
		);
	}
	try {
		return await res.json();
	} catch {
		throw new OutreachError(`${what} is unavailable on this host.`, 0);
	}
}

function emailOf(data: Loose, what: string): OutreachEmail {
	const email = normalizeEmail(data?.email);
	if (!email) throw new OutreachError(`${what} returned an unexpected shape.`, 0);
	return email;
}

export async function fetchQuestions(slug: string, fetchFn: typeof fetch = fetch): Promise<QuestionSet> {
	const data = await request(
		'GET',
		`/admin/first-source/${encodeURIComponent(slug)}/questions`,
		undefined,
		'Open questions',
		fetchFn
	);
	const set = normalizeQuestionSet(data);
	if (!set) throw new OutreachError('Open questions returned an unexpected shape.', 0);
	return set;
}

export async function addContact(
	slug: string,
	contact: { kind: 'email' | 'phone'; value: string; details?: string },
	fetchFn: typeof fetch = fetch
): Promise<{ id: number; contacts: Contact[] }> {
	const data = await request(
		'POST',
		`/admin/first-source/${encodeURIComponent(slug)}/contacts`,
		{ kind: contact.kind, value: contact.value.trim(), details: (contact.details ?? '').trim() },
		'Adding the contact',
		fetchFn
	);
	if (typeof data?.id !== 'number') throw new OutreachError('Adding the contact returned an unexpected shape.', 0);
	const contacts = (Array.isArray(data.contacts) ? data.contacts : [])
		.map(normalizeContact)
		.filter((c: Contact | null): c is Contact => c !== null);
	return { id: data.id, contacts };
}

export async function fetchEmails(
	slug: string | null = null,
	fetchFn: typeof fetch = fetch
): Promise<{ emails: OutreachEmail[]; dailyCap: number }> {
	const query = slug ? `?slug=${encodeURIComponent(slug)}` : '';
	return normalizeEmailList(await request('GET', `/admin/first-source/emails${query}`, undefined, 'Outreach emails', fetchFn));
}

export async function draftEmail(
	body: { slug: string; to: string; questions: string[] },
	fetchFn: typeof fetch = fetch
): Promise<OutreachEmail> {
	return emailOf(await request('POST', '/admin/first-source/emails', body, 'Drafting', fetchFn), 'Drafting');
}

export async function saveEmail(id: number, edit: EmailEdit, fetchFn: typeof fetch = fetch): Promise<OutreachEmail> {
	const body = { to: edit.to.trim().toLowerCase(), subject: edit.subject.trim(), body: edit.body.trim() };
	return emailOf(await request('PUT', `/admin/first-source/emails/${id}`, body, 'Saving', fetchFn), 'Saving');
}

export async function sendEmail(id: number, fetchFn: typeof fetch = fetch): Promise<OutreachEmail> {
	return emailOf(await request('POST', `/admin/first-source/emails/${id}/send`, {}, 'Sending', fetchFn), 'Sending');
}

export async function discardEmail(id: number, fetchFn: typeof fetch = fetch): Promise<OutreachEmail> {
	return emailOf(await request('POST', `/admin/first-source/emails/${id}/discard`, {}, 'Discarding', fetchFn), 'Discarding');
}
