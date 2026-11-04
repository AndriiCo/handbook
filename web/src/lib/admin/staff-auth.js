// Staff passkey ceremonies (spec customer-auth.md §5). Admin-only:
// never imported from (public) or (customer) routes. Self-contained
// (no customer-lib imports — the admin bundle owns its copies).
//
// Pure buffer conversions + injectable ceremony runners (no direct
// navigator/fetch references, so node tests drive them with fakes).

/**
 * base64url (no pad) bytes → string.
 * @param {ArrayBuffer|Uint8Array} buf
 */
export function b64urlEncode(buf) {
	const bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
	let binary = '';
	for (const byte of bytes) binary += String.fromCharCode(byte);
	return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

/**
 * base64url string → bytes (throws on malformed input).
 * @param {string} text
 */
export function b64urlDecode(text) {
	const padded = text.replaceAll('-', '+').replaceAll('_', '/');
	const binary = atob(padded);
	const out = new Uint8Array(binary.length);
	for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
	return out;
}

/**
 * Convert server `options.publicKey` into `navigator.credentials`
 * args (challenge/user.id/excludeCredentials/allowCredentials).
 * @param {any} options CreationChallengeResponse publicKey wrapper or request options
 */
export function credentialRequestArgs(options) {
	const pk = options.publicKey ?? options;
	const mapId = (/** @type {any} */ entry) => ({
		...entry,
		id: b64urlDecode(entry.id).buffer
	});
	const args = { ...pk, challenge: b64urlDecode(pk.challenge).buffer };
	if (pk.user?.id) args.user = { ...pk.user, id: b64urlDecode(pk.user.id).buffer };
	if (pk.excludeCredentials) args.excludeCredentials = pk.excludeCredentials.map(mapId);
	if (pk.allowCredentials) args.allowCredentials = pk.allowCredentials.map(mapId);
	return { publicKey: args };
}

/**
 * Serialize a registration credential for POST finish.
 * @param {any} cred PublicKeyCredential from navigator.credentials.create
 */
export function serializeRegistration(cred) {
	return {
		id: cred.id,
		rawId: b64urlEncode(cred.rawId),
		response: {
			attestationObject: b64urlEncode(cred.response.attestationObject),
			clientDataJSON: b64urlEncode(cred.response.clientDataJSON)
		},
		type: cred.type
	};
}

/**
 * Serialize an authentication credential for POST finish.
 * @param {any} cred PublicKeyCredential from navigator.credentials.get
 */
export function serializeAuthentication(cred) {
	/** @type {any} */
	const out = {
		id: cred.id,
		rawId: b64urlEncode(cred.rawId),
		response: {
			authenticatorData: b64urlEncode(cred.response.authenticatorData),
			clientDataJSON: b64urlEncode(cred.response.clientDataJSON),
			signature: b64urlEncode(cred.response.signature)
		},
		type: cred.type
	};
	if (cred.response.userHandle) {
		out.response.userHandle = b64urlEncode(cred.response.userHandle);
	}
	return out;
}

/**
 * Read one cookie value (CSRF echo).
 * @param {string} jar document.cookie
 * @param {string} name
 */
export function cookieValue(jar, name) {
	for (const pair of jar.split(';')) {
		const trimmed = pair.trim();
		if (trimmed.startsWith(name + '=')) return trimmed.slice(name.length + 1);
	}
	return '';
}

/**
 * Invite query-param shape (64 hex). The server re-validates;
 * this only decides link-vs-form rendering. Never logs the value.
 * @param {string | null} value
 */
export function validInviteShape(value) {
	return typeof value === 'string' && /^[0-9a-fA-F]{64}$/.test(value);
}

/**
 * Setup-key shape (`setup_` + 64 lowercase hex). The server
 * re-validates; this only decides render-vs-error. Never logs it.
 * @param {string | null} value
 */
export function validSetupKeyShape(value) {
	return typeof value === 'string' && /^setup_[0-9a-f]{64}$/.test(value);
}

/**
 * True when a fetch threw at the network layer (the machine is
 * offline or the admin host is unreachable — surfaces as a
 * TypeError). There is no client certificate anymore: the IP
 * allowlist gate answers with a 403 body instead.
 * @param {any} err
 */
export function isTransportFailure(err) {
	return err instanceof TypeError;
}

export const CANCELLED_COPY = 'Cancelled — no changes were made.';

/**
 * Run staff enrolment: start → navigator.create → finish.
 * @param {any} input {invite?, setupKey?, displayName, label?} —
 * exactly one of invite/setupKey (setup mode burns the key).
 * @param {any} io injectable browser surface ({credentials, post})
 */
export async function enrolPasskey(input, io) {
	const gate = input.setupKey ? { setup_key: input.setupKey } : { invite: input.invite };
	const started = await io.post('/auth/passkey/enrol/start', {
		...gate,
		display_name: input.displayName,
		...(input.label ? { label: input.label } : {})
	});
	if (!started.ok) return started;
	const args = credentialRequestArgs(started.body.options);
	const cred = await io.credentials.create(args);
	const credential = serializeRegistration(cred);
	const done = await io.post('/auth/passkey/enrol/finish', {
		challenge_id: started.body.challenge_id,
		credential,
		...gate,
		...(input.label ? { label: input.label } : {})
	});
	// Carry the grant echo forward so the page can confirm the
	// role without another round trip.
	if (done.ok) done.body = { ...done.body, role: started.body.role, cn: started.body.cn };
	return done;
}

/**
 * Run staff login: start → navigator.get → finish.
 * @param {string} account user_id hex (no emails on this branch)
 * @param {any} io
 */
export async function loginPasskey(account, io) {
	const started = await io.post('/auth/passkey/login/start', { account });
	if (!started.ok) return started;
	const args = credentialRequestArgs(started.body.options);
	const cred = await io.credentials.get(args);
	const credential = serializeAuthentication(cred);
	return io.post('/auth/passkey/login/finish', {
		challenge_id: started.body.challenge_id,
		credential
	});
}

/**
 * Usernameless staff sign-in: start with no account, so the server
 * sends a discoverable challenge (empty allow-list) and the browser
 * offers the staff passkeys it holds for this console. The server
 * resolves the account from the credential (spec §5.3).
 * @param {any} io {credentials, post}
 */
export async function loginDiscoverable(io) {
	const started = await io.post('/auth/passkey/login/start', { account: '' });
	if (!started.ok) return started;
	// No `mediation`: a button click opens the passkey sheet now
	// rather than waiting for an autofill pick.
	const cred = await io.credentials.get(credentialRequestArgs(started.body.options));
	return io.post('/auth/passkey/login/finish', {
		challenge_id: started.body.challenge_id,
		credential: serializeAuthentication(cred)
	});
}

/**
 * Passkey step-up for the signed-in staffer (config writes need one
 * from the last 10 minutes): read the session's own user id from the
 * admin `/me`, then run the same UV ceremony as login, which stamps
 * `step_up_at`. Returns the login result, or the `/me` failure.
 * @param {any} io {get, post, credentials}
 */
export async function confirmPasskey(io) {
	const me = await io.get('/me');
	if (!me.ok) return me;
	const account = typeof me.body?.user_id === 'string' ? me.body.user_id : '';
	if (!account) return { ok: false, status: 401, body: { error: 'no staff session' } };
	return loginPasskey(account, io);
}

/**
 * Friendly copy for enrol failures (status + server error mapped).
 * `serverError` is the exact `error` string; denial bodies pass
 * through verbatim (they name the refused IP by design).
 * @param {number} status
 * @param {string} serverError
 */
export function enrolErrorCopy(status, serverError, setupMode = false) {
	if (status === 400) {
		return setupMode
			? 'Start over from the setup page — the request was malformed.'
			: 'This invite link is malformed — ask your admin for a new one.';
	}
	if (status === 403 && serverError === 'certificate does not match invitation') {
		return 'This device certificate does not match the invite — enrol on the device holding the invited certificate.';
	}
	// Denial bodies (IP allowlist, host guard) are written to be
	// shown: carry them verbatim, IP included.
	if (status === 403 && serverError) return serverError;
	if (status === 403) return 'Session expired — reload and try again.';
	if (status === 409) return 'This security key is already registered — use a different key or ask your admin.';
	if (status === 410) {
		return setupMode
			? 'Setup key invalid, or the window already closed (a passkey was enrolled).'
			: 'Invite invalid, expired, or already used — ask your admin for a new one.';
	}
	if (status === 422) return 'Verification failed — try again.';
	if (status === 429) return 'Too many attempts — wait a minute and try again.';
	return 'Enrolment failed — try again.';
}

/**
 * Friendly copy for login failures (status + server error mapped).
 * Denial bodies pass through verbatim (they name the refused IP
 * by design).
 * @param {number} status
 * @param {string} serverError
 */
export function loginErrorCopy(status, serverError) {
	// 401 from finish: the authenticator answered but the signature,
	// user verification or account did not check out. From start: no
	// usable passkey exists for the account given.
	if (status === 401 && serverError === 'authentication failed') {
		return 'The passkey check did not pass — try again with the passkey you enrolled on this server.';
	}
	if (status === 401) {
		return 'No staff passkey matches that account — check the user ID, or ask an admin for an invite link.';
	}
	if (status === 403 && serverError === 'use the customer login') {
		return 'This branch is for staff — use the customer login.';
	}
	// Denial bodies (IP allowlist, host guard) are written to be
	// shown: carry them verbatim, IP included.
	if (status === 403 && serverError) return serverError;
	if (status === 403) return 'Session expired — reload and try again.';
	if (status === 404) return 'Credential unknown — ask your admin for a fresh invite link.';
	if (status === 410) return 'The sign-in expired — try again.';
	if (status === 429) return 'Too many attempts — wait a minute and try again.';
	return 'Sign-in failed — try again.';
}
