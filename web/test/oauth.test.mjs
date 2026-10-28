import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	OAUTH_LABELS,
	OAUTH_PROVIDERS,
	oauthErrorCopy,
	oauthStartError,
	startOAuth
} from '../src/lib/customer/oauth.js';

describe('oauth error copy', () => {
	it('maps known codes and never leaks raw text', () => {
		assert.equal(
			oauthErrorCopy(new URLSearchParams('error=oauth_denied')),
			'Sign-in was cancelled — try again.'
		);
		assert.match(oauthErrorCopy(new URLSearchParams('error=link_required')), /already in use/);
		assert.equal(
			oauthErrorCopy(new URLSearchParams('error=<script>')),
			'Sign-in failed — try again.'
		);
		assert.equal(oauthErrorCopy(new URLSearchParams('')), null);
	});

	it('stays provider-agnostic (one map for all providers)', () => {
		for (const code of [
			'oauth_denied',
			'oauth_expired',
			'oauth_failed',
			'oauth_unavailable',
			'oauth_rate_limited',
			'link_required',
			'already_linked',
			'account_locked'
		]) {
			const copy = oauthErrorCopy(new URLSearchParams(`error=${code}`));
			assert.doesNotMatch(copy, /Google|Apple|Meta/, code);
		}
	});

	it('maps start failures by status with the provider label', () => {
		assert.match(oauthStartError(429, 'Apple'), /Too many attempts/);
		assert.match(oauthStartError(403, 'Meta'), /fresh session/);
		assert.match(oauthStartError(503, 'Google'), /Google sign-in is unavailable/);
		assert.match(oauthStartError(401, 'Apple'), /link Apple/);
		assert.match(oauthStartError(500, 'Meta'), /Meta sign-in failed/);
	});
});

describe('oauth start with fakes', () => {
	for (const provider of OAUTH_PROVIDERS) {
		it(`posts the ${provider} start then redirects to auth_url`, async () => {
			const seen = [];
			const io = {
				post: async (path, body) => {
					seen.push([path, body.mode]);
					return { ok: true, body: { auth_url: `https://provider.test/auth?x=1` } };
				},
				redirect: (url) => seen.push(['redirect', url.startsWith('https://')])
			};
			const res = await startOAuth(io, provider, 'login');
			assert.equal(res.ok, true);
			assert.deepEqual(
				seen.map(([kind]) => kind),
				[`/auth/${provider}/start`, 'redirect']
			);
			assert.equal(OAUTH_LABELS[provider] !== undefined, true);
		});
	}

	it('rejects an unknown provider without posting', async () => {
		let posted = false;
		let redirected = false;
		const io = {
			post: async () => {
				posted = true;
				return { ok: true, body: {} };
			},
			redirect: () => {
				redirected = true;
			}
		};
		const res = await startOAuth(io, '../admin', 'login');
		assert.equal(res.ok, false);
		assert.equal(res.status, 400);
		assert.equal(posted, false);
		assert.equal(redirected, false);
	});

	it('short-circuits a failed start without redirecting', async () => {
		let redirected = false;
		const io = {
			post: async () => ({ ok: false, status: 429, body: {} }),
			redirect: () => {
				redirected = true;
			}
		};
		const res = await startOAuth(io, 'meta', 'link');
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
		const res = await startOAuth(io, 'apple', 'login');
		assert.equal(res.ok, false);
		assert.equal(redirected, false);
	});
});
