import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { googleErrorCopy, googleStartError, startGoogle } from '../src/lib/customer/google.js';

describe('google error copy', () => {
	it('maps known codes and never leaks raw text', () => {
		assert.equal(
			googleErrorCopy(new URLSearchParams('error=oauth_denied')),
			'Google sign-in was cancelled — try again.'
		);
		assert.match(
			googleErrorCopy(new URLSearchParams('error=link_required')),
			/already in use/
		);
		assert.equal(
			googleErrorCopy(new URLSearchParams('error=<script>')),
			'Google sign-in failed — try again.'
		);
		assert.equal(googleErrorCopy(new URLSearchParams('')), null);
	});

	it('maps start failures by status', () => {
		assert.match(googleStartError(429), /Too many attempts/);
		assert.match(googleStartError(403), /fresh session/);
		assert.match(googleStartError(503), /unavailable/);
		assert.match(googleStartError(500), /failed/);
	});
});

describe('google start with fakes', () => {
	it('posts start then redirects to auth_url', async () => {
		const seen = [];
		const io = {
			post: async (path, body) => {
				seen.push([path, body.mode]);
				return { ok: true, body: { auth_url: 'https://accounts.google.com/o/oauth2/v2/auth?x=1' } };
			},
			redirect: (url) => seen.push(['redirect', url.startsWith('https://')])
		};
		const res = await startGoogle(io, 'login');
		assert.equal(res.ok, true);
		assert.deepEqual(
			seen.map(([kind]) => kind),
			['/auth/google/start', 'redirect']
		);
	});

	it('short-circuits a failed start without redirecting', async () => {
		let redirected = false;
		const io = {
			post: async () => ({ ok: false, status: 429, body: {} }),
			redirect: () => {
				redirected = true;
			}
		};
		const res = await startGoogle(io, 'link');
		assert.equal(res.ok, false);
		assert.equal(redirected, false);
	});

	it('rejects non-https auth_url without redirecting', async () => {
		let redirected = false;
		const io = {
			post: async () => ({ ok: true, body: { auth_url: 'javascript:alert(1)' } }),
			redirect: () => {
				redirected = true;
			}
		};
		const res = await startGoogle(io, 'login');
		assert.equal(res.ok, false);
		assert.equal(redirected, false);
	});
});
