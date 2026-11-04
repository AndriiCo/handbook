// Staff management API client (/staff page, customer-auth.md §5.6).
// Admin-only: never imported from (public) or (customer) routes.
//
// GET  {apiBase}/staff                          → { me, accounts, invites }
// POST {apiBase}/staff/invites                  → { url, role, cn, expires_at }
// POST {apiBase}/staff/invites/{id}/withdraw    → { ok }
// POST {apiBase}/staff/{user_id}/revoke         → { ok, sessions, passkeys }
// POST {apiBase}/staff/passkeys/revoke          → { ok }
// POST {apiBase}/staff/passkeys/add/start|finish (see addPasskey)
//
// Mutations ride the double-submit CSRF header and need a passkey
// step-up from the last 10 minutes: refusals come back as
// AdminRequestError with `stepUp` set, so the page can confirm in
// place and retry.

import { csrfHeaders } from './config-client.js';
import { adminRequestError } from './http-errors.js';
import { credentialRequestArgs, serializeRegistration } from './staff-auth.js';

/**
 * @typedef {{ id: string, label: string, created_at: number, last_used_at: number | null }} StaffPasskey
 * @typedef {{ user_id: string, role: 'staff' | 'admin', name: string, email: string, created_at: number, locked_at: number | null, passkeys: StaffPasskey[] }} StaffAccount
 * @typedef {{ id: string, role: 'staff' | 'admin', cn: string, created_by: string, created_at: number, expires_at: number }} PendingInvite
 * @typedef {{ me: { user_id: string, role: string }, accounts: StaffAccount[], invites: PendingInvite[] }} StaffList
 */

/** Invite names: ASCII word characters, dots and dashes, 1 to 64. */
export const CN_RULE = /^[A-Za-z0-9._-]{1,64}$/;

/**
 * True for a name the server accepts on an invite. Pure (tested).
 * @param {unknown} cn
 */
export function validCn(cn) {
	return typeof cn === 'string' && CN_RULE.test(cn);
}

/**
 * Invite lifetime in whole days, clamped to the server's 1..30.
 * @param {unknown} days
 */
export function inviteDays(days) {
	const n = Math.trunc(Number(days));
	if (!Number.isFinite(n)) return 1;
	return Math.min(30, Math.max(1, n));
}

/**
 * First 8 characters of an opaque id (ids are never secrets, but the
 * full 32 hex adds nothing on screen).
 * @param {string} id
 */
export function shortId(id) {
	return typeof id === 'string' ? id.slice(0, 8) : '';
}

/**
 * Normalise the list body, dropping rows that do not fit. Pure.
 * @param {any} data
 * @returns {StaffList}
 */
export function normalizeStaffList(data) {
	const num = (/** @type {unknown} */ v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
	const role = (/** @type {unknown} */ v) => (v === 'admin' ? 'admin' : 'staff');
	const accounts = Array.isArray(data?.accounts) ? data.accounts : [];
	const invites = Array.isArray(data?.invites) ? data.invites : [];
	return {
		me: {
			user_id: typeof data?.me?.user_id === 'string' ? data.me.user_id : '',
			role: typeof data?.me?.role === 'string' ? data.me.role : ''
		},
		accounts: accounts
			.filter((/** @type {any} */ a) => typeof a?.user_id === 'string')
			.map((/** @type {any} */ a) => ({
				user_id: a.user_id,
				role: role(a.role),
				name: typeof a.name === 'string' ? a.name : '',
				email: typeof a.email === 'string' ? a.email : '',
				created_at: num(a.created_at) ?? 0,
				locked_at: num(a.locked_at),
				passkeys: (Array.isArray(a.passkeys) ? a.passkeys : [])
					.filter((/** @type {any} */ p) => typeof p?.id === 'string')
					.map((/** @type {any} */ p) => ({
						id: p.id,
						label: typeof p.label === 'string' ? p.label : '',
						created_at: num(p.created_at) ?? 0,
						last_used_at: num(p.last_used_at)
					}))
			})),
		invites: invites
			.filter((/** @type {any} */ i) => typeof i?.id === 'string')
			.map((/** @type {any} */ i) => ({
				id: i.id,
				role: role(i.role),
				cn: typeof i.cn === 'string' ? i.cn : '',
				created_by: typeof i.created_by === 'string' ? i.created_by : '',
				created_at: num(i.created_at) ?? 0,
				expires_at: num(i.expires_at) ?? 0
			}))
	};
}

/**
 * @param {string} apiBase
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<StaffList>}
 */
export async function fetchStaff(apiBase, fetchFn = fetch) {
	const res = await fetchFn(`${apiBase}/staff`, { credentials: 'same-origin' });
	if (!res.ok) throw await adminRequestError(res, 'Staff list failed.');
	let data;
	try {
		data = await res.json();
	} catch {
		throw new Error('Staff list returned invalid JSON.');
	}
	return normalizeStaffList(data);
}

/**
 * @param {string} apiBase
 * @param {string} path
 * @param {unknown} body
 * @param {string} fallback
 * @param {typeof fetch} fetchFn
 */
async function postJson(apiBase, path, body, fallback, fetchFn) {
	const res = await fetchFn(`${apiBase}${path}`, {
		method: 'POST',
		headers: { 'content-type': 'application/json', ...csrfHeaders() },
		credentials: 'same-origin',
		body: JSON.stringify(body ?? {})
	});
	if (!res.ok) throw await adminRequestError(res, fallback);
	return res.json().catch(() => ({}));
}

/**
 * @param {string} apiBase
 * @param {{ role: 'staff' | 'admin', cn: string, days: number }} input
 * @param {typeof fetch} [fetchFn]
 * @returns {Promise<{ url: string, role: string, cn: string, expires_at: number }>}
 */
export async function createInvite(apiBase, input, fetchFn = fetch) {
	if (!validCn(input.cn)) throw new Error('Name: letters, digits, dots, dashes or underscores, up to 64.');
	return postJson(
		apiBase,
		'/staff/invites',
		{ role: input.role === 'admin' ? 'admin' : 'staff', cn: input.cn, days: inviteDays(input.days) },
		'Invite failed.',
		fetchFn
	);
}

/**
 * @param {string} apiBase
 * @param {string} id pending invite id
 * @param {typeof fetch} [fetchFn]
 */
export function withdrawInvite(apiBase, id, fetchFn = fetch) {
	return postJson(apiBase, `/staff/invites/${encodeURIComponent(id)}/withdraw`, {}, 'Withdraw failed.', fetchFn);
}

/**
 * @param {string} apiBase
 * @param {string} userId
 * @param {typeof fetch} [fetchFn]
 */
export function revokeStaff(apiBase, userId, fetchFn = fetch) {
	return postJson(apiBase, `/staff/${encodeURIComponent(userId)}/revoke`, {}, 'Revoke failed.', fetchFn);
}

/**
 * @param {string} apiBase
 * @param {string} credentialId
 * @param {typeof fetch} [fetchFn]
 */
export function revokePasskey(apiBase, credentialId, fetchFn = fetch) {
	return postJson(apiBase, '/staff/passkeys/revoke', { credential_id: credentialId }, 'Remove failed.', fetchFn);
}

/**
 * Add a passkey to the signed-in staffer's own account (a second
 * device): start → navigator.credentials.create → finish.
 * @param {string} apiBase
 * @param {{ label: string }} input
 * @param {{ credentials: { create: (args: any) => Promise<any> }, fetchFn?: typeof fetch }} io
 */
export async function addPasskey(apiBase, input, io) {
	const fetchFn = io.fetchFn ?? fetch;
	const label = input.label.trim();
	const started = await postJson(
		apiBase,
		'/staff/passkeys/add/start',
		label ? { label } : {},
		'Adding a passkey failed.',
		fetchFn
	);
	const cred = await io.credentials.create(credentialRequestArgs(started.options));
	return postJson(
		apiBase,
		'/staff/passkeys/add/finish',
		{ challenge_id: started.challenge_id, credential: serializeRegistration(cred), ...(label ? { label } : {}) },
		'Adding a passkey failed.',
		fetchFn
	);
}
