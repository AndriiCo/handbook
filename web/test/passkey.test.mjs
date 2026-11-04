import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	b64urlDecode,
	b64urlEncode,
	cookieValue,
	credentialRequestArgs,
	deviceLabel,
	deviceName,
	loginPasskey,
	registerPasskey,
	serializeAuthentication,
	serializeRegistration
} from '../src/lib/customer/passkey.js';

describe('passkey buffers', () => {
	it('round-trips bytes through base64url', () => {
		const bytes = new Uint8Array([0, 1, 2, 250, 255]);
		assert.equal(b64urlEncode(bytes), b64urlEncode(bytes.buffer));
		assert.deepEqual([...b64urlDecode(b64urlEncode(bytes))], [...bytes]);
		// URL-safe alphabet, no padding.
		assert.match(b64urlEncode(new Uint8Array([251, 255, 190])), /^[A-Za-z0-9\-_]+$/);
	});

	it('converts server options to credential args', () => {
		const args = credentialRequestArgs({
			publicKey: {
				challenge: 'AAEC',
				user: { id: 'AAED', name: 'u', displayName: 'U' },
				excludeCredentials: [{ id: 'AAEE', type: 'public-key' }]
			}
		});
		assert.ok(args.publicKey.challenge instanceof ArrayBuffer);
		assert.ok(args.publicKey.user.id instanceof ArrayBuffer);
		assert.ok(args.publicKey.excludeCredentials[0].id instanceof ArrayBuffer);
		assert.equal(args.publicKey.user.name, 'u');
	});

	it('serializes credentials for finish posts', () => {
		const reg = serializeRegistration({
			id: 'x',
			rawId: new Uint8Array([1]).buffer,
			response: {
				attestationObject: new Uint8Array([2]).buffer,
				clientDataJSON: new Uint8Array([3]).buffer
			},
			type: 'public-key'
		});
		assert.equal(reg.rawId, 'AQ');
		assert.equal(reg.response.attestationObject, 'Ag');
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
		assert.equal(auth.response.signature, 'BA');
		assert.ok(!('userHandle' in auth.response));
	});

	it('reads cookies for the CSRF echo', () => {
		assert.equal(cookieValue('a=1; hb_csrf=t-9; b=2', 'hb_csrf'), 't-9');
		assert.equal(cookieValue('', 'hb_csrf'), '');
	});
});

describe('passkey ceremonies with fakes', () => {
	const options = { publicKey: { challenge: 'AAEC', user: { id: 'AAED' } } };

	it('register threads start → create → finish', async () => {
		const seen = [];
		const io = {
			credentials: {
				create: async (args) => {
					seen.push(['create', args.publicKey.challenge.byteLength]);
					return {
						id: 'c',
						rawId: new ArrayBuffer(1),
						response: { attestationObject: new ArrayBuffer(1), clientDataJSON: new ArrayBuffer(1) },
						type: 'public-key'
					};
				}
			},
			post: async (path, body) => {
				seen.push([path, body.challenge_id ?? body.display_name]);
				if (path.endsWith('/start')) return { ok: true, body: { challenge_id: 'ch-1', options } };
				return { ok: true, status: 200, body: { user_id: 'u-1', methods: ['passkey'] } };
			}
		};
		const res = await registerPasskey({ displayName: 'A', label: 'K', email: 'a@x.test' }, io);
		assert.equal(res.ok, true);
		assert.deepEqual(
			seen.map(([kind]) => kind),
			['/auth/passkey/register/start', 'create', '/auth/passkey/register/finish']
		);
	});

	it('register short-circuits a failed start', async () => {
		const io = {
			credentials: {
				create: async () => {
					throw new Error('must not run');
				}
			},
			post: async () => ({ ok: false, status: 400, body: { error: 'display_name required' } })
		};
		const res = await registerPasskey({ displayName: '' }, io);
		assert.equal(res.ok, false);
	});

	it('login threads start → get → finish', async () => {
		const seen = [];
		const io = {
			credentials: {
				get: async () => ({
					id: 'c',
					rawId: new ArrayBuffer(1),
					response: {
						authenticatorData: new ArrayBuffer(1),
						clientDataJSON: new ArrayBuffer(1),
						signature: new ArrayBuffer(1)
					},
					type: 'public-key'
				})
			},
			post: async (path, body) => {
				seen.push(path);
				if (path.endsWith('/start')) {
					assert.equal(body.account, 'u-9');
					return { ok: true, body: { challenge_id: 'ch-2', options } };
				}
				assert.equal(body.challenge_id, 'ch-2');
				return { ok: true, status: 200, body: { user_id: 'u-9' } };
			}
		};
		const res = await loginPasskey('u-9', io);
		assert.equal(res.body.user_id, 'u-9');
		assert.deepEqual(seen, ['/auth/passkey/login/start', '/auth/passkey/login/finish']);
	});
});

describe('passkey device label', () => {
	const UA = {
		macSafari:
			'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15',
		macChrome:
			'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36',
		winEdge:
			'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0',
		winFirefox: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) Gecko/20100101 Firefox/132.0',
		iphoneChrome:
			'Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0.6723.90 Mobile/15E148 Safari/604.1',
		androidSamsung:
			'Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/26.0 Chrome/122.0.0.0 Mobile Safari/537.36',
		linuxFirefox: 'Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0'
	};

	it('names the OS and browser', () => {
		assert.equal(deviceName(UA.macSafari), 'macOS Safari');
		assert.equal(deviceName(UA.macChrome), 'macOS Chrome');
		assert.equal(deviceName(UA.winEdge), 'Windows Edge');
		assert.equal(deviceName(UA.winFirefox), 'Windows Firefox');
		assert.equal(deviceName(UA.iphoneChrome), 'iPhone Chrome');
		assert.equal(deviceName(UA.androidSamsung), 'Android Samsung Internet');
		assert.equal(deviceName(UA.linuxFirefox), 'Linux Firefox');
		// iPadOS asks for the desktop site and says Macintosh.
		assert.equal(deviceName(UA.macSafari, 5), 'iPad Safari');
		assert.equal(deviceName(''), 'this device');
	});

	it('labels the key with the site, within the server limit', () => {
		assert.equal(deviceLabel('andrii.ai', UA.macSafari), 'andrii.ai - macOS Safari');
		assert.equal(deviceLabel('x'.repeat(80), UA.macSafari).length, 64);
	});
});
