// Public passkey ceremonies (spec customer-auth.md §2).
// Pure buffer conversions + injectable ceremony runners (no
// direct navigator/fetch references, so node tests can drive
// them with fakes).

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
 * Run passkey registration: start → navigator.create → finish.
 * @param {any} input {displayName, label?, email?}
 * @param {any} io injectable browser surface ({credentials, post})
 */
export async function registerPasskey(input, io) {
	const started = await io.post('/auth/passkey/register/start', {
		display_name: input.displayName,
		...(input.label ? { label: input.label } : {}),
		...(input.email ? { email: input.email } : {})
	});
	if (!started.ok) return started;
	const args = credentialRequestArgs(started.body.options);
	const cred = await io.credentials.create(args);
	const credential = serializeRegistration(cred);
	return io.post('/auth/passkey/register/finish', {
		challenge_id: started.body.challenge_id,
		credential,
		...(input.label ? { label: input.label } : {}),
		...(input.email ? { email: input.email } : {})
	});
}

/**
 * Run passkey login: start → navigator.get → finish.
 * @param {string} account user_id hex or linked email
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
