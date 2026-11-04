import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	b64urlDecode,
	b64urlEncode,
	confirmPasskey,
	cookieValue,
	credentialRequestArgs,
	enrolErrorCopy,
	enrolPasskey,
	isTransportFailure,
	loginErrorCopy,
	loginPasskey,
	serializeAuthentication,
	serializeRegistration,
	validInviteShape,
	validSetupKeyShape
} from '../src/lib/admin/staff-auth.js';

describe('staff-auth buffers', () => {
	it('round-trips base64url without padding', () => {
		const bytes = new Uint8Array([255, 0, 128, 1, 2, 3]);
		assert.equal(b64urlDecode(b64urlEncode(bytes)).join(','), bytes.join(','));
		assert.doesNotMatch(b64urlEncode(bytes), /=/);
	});

	it('builds credential args from server options', () => {
		const args = credentialRequestArgs({
			challenge: b64urlEncode(new Uint8Array([1, 2])),
			user: { id: b64urlEncode(new Uint8Array([3])), name: 'jdoe' },
			allowCredentials: [{ id: b64urlEncode(new Uint8Array([4])), type: 'public-key' }]
		});
		assert.ok(args.publicKey.challenge instanceof ArrayBuffer);
		assert.ok(args.publicKey.user.id instanceof ArrayBuffer);
		assert.ok(args.publicKey.allowCredentials[0].id instanceof ArrayBuffer);
	});

	it('serializes registration and authentication credentials', () => {
		const reg = serializeRegistration({
			id: 'x',
			rawId: new Uint8Array([1]).buffer,
			response: {
				attestationObject: new Uint8Array([2]).buffer,
				clientDataJSON: new Uint8Array([3]).buffer
			},
			type: 'public-key'
		});
		assert.equal(reg.rawId, b64urlEncode(new Uint8Array([1])));
		const auth = serializeAuthentication({
			id: 'x',
			rawId: new Uint8Array([1]).buffer,
			response: {
				authenticatorData: new Uint8Array([2]).buffer,
				clientDataJSON: new Uint8Array([3]).buffer,
				signature: new Uint8Array([4]).buffer
			},
			type: 'public-key'
		});
		assert.ok(!('userHandle' in auth.response));
	});

	it('reads cookie values', () => {
		assert.equal(cookieValue('a=1; hb_csrf=tok; b=2', 'hb_csrf'), 'tok');
		assert.equal(cookieValue('', 'hb_csrf'), '');
	});
});

describe('staff-auth shapes', () => {
	it('validates invite shape without logging the value', () => {
		assert.equal(validInviteShape('a'.repeat(64)), true);
		assert.equal(validInviteShape('A'.repeat(64)), true);
		assert.equal(validInviteShape('a'.repeat(63)), false);
		assert.equal(validInviteShape('not-hex!'.padEnd(64, '0')), false);
		assert.equal(validInviteShape(null), false);
	});

	it('validates setup-key shape without logging the value', () => {
		assert.equal(validSetupKeyShape(`setup_${'a'.repeat(64)}`), true);
		assert.equal(validSetupKeyShape(`setup_${'a'.repeat(63)}`), false);
		assert.equal(validSetupKeyShape('a'.repeat(64)), false);
		assert.equal(validSetupKeyShape(`setup_${'A'.repeat(64)}`), false);
		assert.equal(validSetupKeyShape('setup_short'), false);
		assert.equal(validSetupKeyShape(null), false);
	});

	it('detects transport failures only', () => {
		assert.equal(isTransportFailure(new TypeError('fetch failed')), true);
		assert.equal(isTransportFailure(new Error('HTTP 500')), false);
	});

	it('maps enrol failures, carrying denial bodies verbatim', () => {
		assert.match(enrolErrorCopy(410, 'invite invalid or expired'), /ask your admin/);
		assert.match(
			enrolErrorCopy(403, 'admin access is limited — your IP is 198.51.100.9'),
			/198\.51\.100\.9/
		);
		assert.match(
			enrolErrorCopy(403, 'certificate does not match invitation'),
			/does not match/
		);
		assert.match(enrolErrorCopy(409, 'credential already registered'), /already registered/);
		assert.match(enrolErrorCopy(422, 'verification failed'), /Verification failed/);
		assert.match(enrolErrorCopy(500, '<script>'), /Enrolment failed/);
	});

	it('maps setup-mode enrol failures to the window', () => {
		assert.match(
			enrolErrorCopy(410, 'invite invalid or expired', true),
			/window already closed/
		);
		assert.match(enrolErrorCopy(400, 'x', true), /setup page/);
		assert.match(enrolErrorCopy(429, 'rate limited', true), /wait a minute/);
	});

	it('maps login failures, carrying denial bodies verbatim', () => {
		assert.match(loginErrorCopy(401, 'unknown account or no passkeys'), /Unknown account/);
		assert.match(loginErrorCopy(403, 'use the customer login'), /customer login/);
		assert.match(
			loginErrorCopy(403, 'admin access is limited — your IP is 198.51.100.9'),
			/198\.51\.100\.9/
		);
		assert.match(loginErrorCopy(404, 'credential unknown'), /fresh invite/);
		assert.match(loginErrorCopy(410, 'challenge invalid or expired'), /expired/);
		assert.match(loginErrorCopy(500, '<script>'), /Sign-in failed/);
	});
});

describe('staff-auth ceremonies with fakes', () => {
	function credential() {
		return {
			id: 'x',
			rawId: new Uint8Array([1]).buffer,
			response: {
				attestationObject: new Uint8Array([2]).buffer,
				clientDataJSON: new Uint8Array([3]).buffer,
				authenticatorData: new Uint8Array([2]).buffer,
				signature: new Uint8Array([4]).buffer
			},
			type: 'public-key'
		};
	}

	it('enrols: start → create → finish with the invite echoed', async () => {
		const seen = [];
		const io = {
			post: async (path, body) => {
				seen.push(path);
				if (path.endsWith('/enrol/start')) {
					assert.equal(body.invite, 'i'.repeat(64));
					assert.equal(body.display_name, 'J Doe');
					return {
						ok: true,
						body: {
							challenge_id: 'c1',
							options: { challenge: b64urlEncode(new Uint8Array([9])) },
							role: 'staff',
							cn: 'jdoe'
						}
					};
				}
				assert.equal(body.challenge_id, 'c1');
				assert.equal(body.invite, 'i'.repeat(64));
				assert.ok(body.credential.rawId);
				return { ok: true, body: { user_id: 'u', methods: ['passkey'] } };
			},
			credentials: { create: async () => credential(), get: async () => credential() }
		};
		const res = await enrolPasskey(
			{ invite: 'i'.repeat(64), displayName: 'J Doe' },
			io
		);
		assert.equal(res.ok, true);
		assert.equal(res.body.role, 'staff');
		assert.equal(res.body.user_id, 'u');
		assert.deepEqual(seen, [
			'/auth/passkey/enrol/start',
			'/auth/passkey/enrol/finish'
		]);
	});

	it('enrols in setup mode with the key instead of an invite', async () => {
		const seen = [];
		const io = {
			post: async (path, body) => {
				seen.push(path);
				assert.ok(!('invite' in body), 'no invite in setup mode');
				assert.equal(body.setup_key, `setup_${'b'.repeat(64)}`);
				if (path.endsWith('/enrol/start')) {
					return {
						ok: true,
						body: {
							challenge_id: 'c9',
							options: { challenge: b64urlEncode(new Uint8Array([9])) },
							role: 'admin',
							cn: 'setup'
						}
					};
				}
				assert.equal(body.challenge_id, 'c9');
				return { ok: true, body: { user_id: 'u', methods: ['passkey'] } };
			},
			credentials: { create: async () => credential(), get: async () => credential() }
		};
		const res = await enrolPasskey(
			{ setupKey: `setup_${'b'.repeat(64)}`, displayName: 'Op' },
			io
		);
		assert.equal(res.ok, true);
		assert.equal(res.body.role, 'admin');
		assert.equal(res.body.user_id, 'u');
		assert.deepEqual(seen, [
			'/auth/passkey/enrol/start',
			'/auth/passkey/enrol/finish'
		]);
	});

	it('enrol short-circuits a failed start without touching the authenticator', async () => {
		let touched = false;
		const io = {
			post: async () => ({ ok: false, status: 410, body: {} }),
			credentials: {
				create: async () => {
					touched = true;
					return credential();
				},
				get: async () => credential()
			}
		};
		const res = await enrolPasskey({ invite: 'i'.repeat(64), displayName: 'J' }, io);
		assert.equal(res.ok, false);
		assert.equal(touched, false);
	});

	it('logs in: start → get → finish with the account', async () => {
		const seen = [];
		const io = {
			post: async (path, body) => {
				seen.push(path);
				if (path.endsWith('/login/start')) {
					assert.equal(body.account, 'u'.repeat(32));
					return {
						ok: true,
						body: {
							challenge_id: 'c2',
							options: { challenge: b64urlEncode(new Uint8Array([9])) }
						}
					};
				}
				assert.equal(body.challenge_id, 'c2');
				assert.ok(body.credential.response.signature);
				return { ok: true, body: { user_id: 'u', methods: ['passkey'] } };
			},
			credentials: { create: async () => credential(), get: async () => credential() }
		};
		const res = await loginPasskey('u'.repeat(32), io);
		assert.equal(res.ok, true);
		assert.deepEqual(seen, [
			'/auth/passkey/login/start',
			'/auth/passkey/login/finish'
		]);
	});

	it('confirms a step-up for the session’s own staff id', async () => {
		const seen = [];
		const io = {
			get: async (path) => {
				seen.push(path);
				return { ok: true, status: 200, body: { user_id: 'a'.repeat(32), role: 'admin' } };
			},
			post: async (path, body) => {
				seen.push(path);
				if (path.endsWith('/login/start')) {
					assert.equal(body.account, 'a'.repeat(32));
					return {
						ok: true,
						body: {
							challenge_id: 'c3',
							options: { challenge: b64urlEncode(new Uint8Array([7])) }
						}
					};
				}
				return { ok: true, status: 200, body: { user_id: 'a'.repeat(32) } };
			},
			credentials: { create: async () => credential(), get: async () => credential() }
		};
		const res = await confirmPasskey(io);
		assert.equal(res.ok, true);
		assert.deepEqual(seen, ['/me', '/auth/passkey/login/start', '/auth/passkey/login/finish']);
	});

	it('stops at /me when there is no staff session', async () => {
		let posted = false;
		const io = {
			get: async () => ({ ok: false, status: 401, body: { error: 'unauthorized' } }),
			post: async () => {
				posted = true;
				return { ok: true, body: {} };
			},
			credentials: { create: async () => credential(), get: async () => credential() }
		};
		const res = await confirmPasskey(io);
		assert.equal(res.ok, false);
		assert.equal(res.status, 401);
		assert.equal(posted, false);
	});
});
