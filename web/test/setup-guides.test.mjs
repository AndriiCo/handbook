import test from 'node:test';
import assert from 'node:assert/strict';
import { STEPS, REQUIRED_KEYS, keyStepIndex } from '../src/lib/admin/setup-guides.js';

// Every registry key except `setup.*`, generated from
// crates/config/src/lib.rs (keys::ALL). A new registry key without
// a wizard slot fails this test on purpose: slot it or justify it.
const REGISTRY_KEYS = [
	'acme.directory',
	'acme.domains',
	'acme.eab_hmac',
	'acme.eab_kid',
	'acme.email',
	'acme.enabled',
	'admin.mtls_off',
	'apple.client_id',
	'apple.key_id',
	'apple.private_key',
	'apple.team_id',
	'auth.origin',
	'auth.rp_id',
	'auth.rp_name',
	'google.client_id',
	'google.client_secret',
	'labels.fedex_account',
	'labels.fedex_client_id',
	'labels.fedex_client_secret',
	'labels.live',
	'labels.live_daily_cap',
	'labels.live_max_usd',
	'labels.live_ship_to_allow',
	'mail.base_url',
	'mail.client_id',
	'mail.client_secret',
	'mail.refresh_token',
	'mail.token_url',
	'mailer.dir',
	'mailer.from',
	'mailer.host',
	'mailer.password',
	'mailer.port',
	'mailer.provider',
	'mailer.username',
	'media.terms_read',
	'meta.api_key',
	'meta.base_url',
	'meta.client_id',
	'meta.client_secret',
	'net.admin_port',
	'net.bind',
	'net.bind6',
	'net.http_port',
	'net.https_port',
	'net.port',
	'net.public_host',
	'paths.acme_challenge_dir',
	'paths.admin_spa_dir',
	'paths.spa_dir',
	'paths.static_dir',
	'quic.enabled',
	'telnyx.api_key',
	'telnyx.base_url',
	'telnyx.connection_id',
	'telnyx.from_number',
	'telnyx.webhook_public_key',
	'tls.cert_file',
	'tls.key_file',
	'usps.base_url',
	'usps.certified'
];

test('steps cover every wizard key exactly once', () => {
	const seen = [];
	for (const step of STEPS) {
		assert.ok(step.id && step.title && step.blurb, `step ${step.id} has chrome`);
		assert.ok(step.guide.intro && step.guide.steps.length > 0, `step ${step.id} has a guide`);
		for (const field of step.keys) {
			assert.ok(field.key && field.label && field.hint, `field ${field.key} has chrome`);
			assert.ok(
				['bool', 'text', 'secret'].includes(field.type),
				`field ${field.key} has a known type`
			);
			seen.push(field.key);
		}
	}
	assert.deepEqual([...seen].sort(), REGISTRY_KEYS);
	assert.equal(new Set(seen).size, seen.length);
});

test('required keys are covered and non-empty', () => {
	assert.ok(REQUIRED_KEYS.length > 0);
	const index = keyStepIndex();
	for (const key of REQUIRED_KEYS) {
		assert.ok(index.has(key), `${key} is slotted`);
	}
	assert.equal(new Set(REQUIRED_KEYS).size, REQUIRED_KEYS.length);
});

test('no guide leaks real credentials or endpoints', () => {
	const banned = [/ LLM_[A-Za-z0-9]{8,}/, /sk-[A-Za-z0-9]{8,}/, /xox[bap]-/];
	for (const step of STEPS) {
		const text = JSON.stringify(step);
		assert.ok(!text.includes('setup_'), `step ${step.id} names no live key`);
		for (const pattern of banned) {
			assert.doesNotMatch(text, pattern, `step ${step.id}`);
		}
	}
});
