import test from 'node:test';
import assert from 'node:assert/strict';
import {
	APPLIES_TEXT,
	CONFIG_GUIDE,
	OTHER_GROUP,
	guideEntry,
	guideKeys,
	groupForKey,
	richText
} from '../src/lib/admin/config-guide.ts';

// Every registry key with its type, from crates/config/src/lib.rs keys::ALL
// (registry_holds_96_typed_keys). Regenerate when the registry changes.
const REGISTRY = [
	['acme.directory', 'text'], ['acme.domains', 'text'], ['acme.eab_hmac', 'secret'], ['acme.eab_kid', 'text'], ['acme.email', 'text'], ['acme.enabled', 'bool'],
	['admin.allow_ips', 'text'], ['admin.mtls_off', 'bool'],
	['apple.client_id', 'text'], ['apple.key_id', 'text'], ['apple.private_key', 'secret'], ['apple.team_id', 'text'],
	['auth.origin', 'text'], ['auth.rp_id', 'text'], ['auth.rp_name', 'text'], ['compile.daily_runs', 'text'],
	['google.client_id', 'text'], ['google.client_secret', 'secret'],
	['labels.customer_daily_cap', 'text'], ['labels.customer_live', 'bool'], ['labels.customer_max_usd', 'text'], ['labels.fedex_account', 'text'], ['labels.fedex_base_url', 'text'], ['labels.fedex_client_id', 'secret'], ['labels.fedex_client_secret', 'secret'], ['labels.live', 'bool'], ['labels.live_daily_cap', 'text'], ['labels.live_max_usd', 'text'], ['labels.live_ship_to_allow', 'text'],
	['mail.base_url', 'text'], ['mail.client_id', 'text'], ['mail.client_secret', 'secret'], ['mail.refresh_token', 'secret'], ['mail.token_url', 'text'],
	['mailer.dir', 'text'], ['mailer.from', 'text'], ['mailer.host', 'text'], ['mailer.password', 'secret'], ['mailer.port', 'text'], ['mailer.provider', 'text'], ['mailer.username', 'text'],
	['media.terms_read', 'bool'], ['meta.api_key', 'secret'], ['meta.base_url', 'text'], ['meta.cache_mode', 'text'], ['meta.client_id', 'text'], ['meta.client_secret', 'secret'], ['meta.daily_call_cap', 'text'],
	['net.admin_port', 'text'], ['net.bind', 'text'], ['net.bind6', 'text'], ['net.http_port', 'text'], ['net.https_port', 'text'], ['net.port', 'text'], ['net.public_host', 'text'],
	['notify.daily_cap', 'text'], ['notify.enabled', 'bool'], ['notify.postal_address', 'text'],
	['orders.currency', 'text'], ['orders.daily_cap', 'text'], ['orders.enabled', 'bool'], ['orders.fee_label_cents', 'text'], ['orders.kit_price_cents', 'text'], ['orders.max_order_cents', 'text'], ['orders.ship_from', 'text'],
	['paths.acme_challenge_dir', 'text'], ['paths.admin_spa_dir', 'text'], ['paths.fetch_cache_dir', 'text'], ['paths.meta_cache_dir', 'text'], ['paths.spa_dir', 'text'], ['paths.static_dir', 'text'],
	['plan.auto_compose', 'bool'], ['quic.enabled', 'bool'], ['setup.key_hash', 'secret'], ['stripe.secret_key', 'secret'], ['stripe.webhook_secret', 'secret'],
	['telnyx.api_key', 'secret'], ['telnyx.base_url', 'text'], ['telnyx.connection_id', 'text'], ['telnyx.from_number', 'text'], ['telnyx.webhook_public_key', 'text'],
	['tls.cert_file', 'text'], ['tls.key_file', 'text'], ['tracking.poll_enabled', 'bool'],
	['usps.base_url', 'text'], ['usps.certified', 'bool'], ['usps.client_id', 'secret'], ['usps.client_secret', 'secret'], ['usps.crid', 'text'], ['usps.epa_account', 'secret'], ['usps.live', 'bool'], ['usps.live_daily_cap', 'text'], ['usps.live_max_usd', 'text'], ['usps.manifest_mid', 'text'], ['usps.mid', 'text'],
	['worker.token', 'secret']
];

test('guide covers every registry key once, with the registry type', () => {
	const seen = guideKeys()
		.map((k) => [k.key, k.type])
		.sort((a, b) => a[0].localeCompare(b[0]));
	assert.equal(REGISTRY.length, 96);
	assert.deepEqual(seen, REGISTRY);
	assert.equal(CONFIG_GUIDE.length, 18);
});

test('every group and key has full text', () => {
	for (const g of CONFIG_GUIDE) {
		assert.ok(g.id && g.title && g.summary && g.setup.length > 0, `group ${g.id}`);
		assert.match(g.id, /^[a-z][a-z-]*$/, `group id ${g.id} is a clean anchor`);
		for (const k of g.keys) {
			for (const f of ['label', 'what', 'how', 'format']) assert.ok(k[f], `${k.key}.${f}`);
			assert.ok(APPLIES_TEXT[k.applies], `${k.key}.applies`);
		}
	}
	assert.equal(new Set(CONFIG_GUIDE.map((g) => g.id)).size, CONFIG_GUIDE.length);
});

test('no banned words or credential-shaped strings', () => {
	const text = JSON.stringify([CONFIG_GUIDE, OTHER_GROUP, APPLIES_TEXT]);
	for (const w of ['placeholder', 'not wired', 'skeleton', 'lorem', 'todo', 'coming soon']) {
		assert.ok(!text.toLowerCase().includes(w), w);
	}
	assert.ok(!text.includes('setup_'));
	for (const re of [
		/-----BEGIN /,
		/(sk|rk)_(live|test)_[A-Za-z0-9_-]{8}/,
		/LLM_[A-Za-z0-9]{8,}/,
		/whsec_[A-Za-z0-9]{8}/,
		/AIza[A-Za-z0-9_-]{35}/,
		/AKIA[A-Z0-9]{16}/,
		/xox[bpars]-/
	]) {
		assert.doesNotMatch(text, re);
	}
});

test('unknown keys fall back by prefix, then to Other', () => {
	assert.equal(guideEntry('stripe.new_key', 'secret').group.id, 'stripe');
	assert.equal(guideEntry('stripe.new_key', 'secret').entry.type, 'secret');
	assert.equal(groupForKey('labels.fedex_new').id, 'fedex');
	assert.equal(groupForKey('labels.other').id, 'labels');
	assert.equal(guideEntry('zzz.key').group, OTHER_GROUP);
	assert.equal(guideEntry('zzz.key').known, false);
	assert.equal(guideEntry('net.bind').known, true);
	assert.equal(guideEntry('setup.key_hash').entry.readOnly, true);
	assert.equal(guideEntry('setup.other', 'secret').entry.readOnly, true);
});

test('rich text links vendor consoles and codes our own endpoints', () => {
	const runs = richText(
		'Open https://dashboard.stripe.com/webhooks: URL https://andrii.ai/apostille/api/v1/stripe/webhook, then set stripe.webhook_secret.'
	);
	assert.deepEqual(
		runs.map((r) => r.kind),
		['text', 'link', 'text', 'code', 'text', 'key', 'text']
	);
	assert.equal(runs[1].href, 'https://dashboard.stripe.com/webhooks');
	assert.equal(runs[3].text, 'https://andrii.ai/apostille/api/v1/stripe/webhook');
	assert.equal(runs[5].href, '#key-stripe.webhook_secret');
	assert.deepEqual(
		richText('Set it to https://apis-sandbox.fedex.com.').map((r) => [r.kind, r.text]),
		[
			['text', 'Set it to '],
			['code', 'https://apis-sandbox.fedex.com'],
			['text', '.']
		]
	);
	assert.deepEqual(
		richText('Callback https://{net.public_host}/auth/google/callback and /auth/<provider>/callback').map(
			(r) => r.kind
		),
		['text', 'code', 'text', 'code']
	);
	// Dotted words that are not registry keys stay plain text.
	assert.deepEqual(richText('andrii.ai and e.g. A/AAAA'), [{ kind: 'text', text: 'andrii.ai and e.g. A/AAAA' }]);
});

test('rich text round-trips every guide string', () => {
	for (const g of CONFIG_GUIDE) {
		for (const s of [g.summary, ...g.setup, ...g.keys.flatMap((k) => [k.what, k.how, k.format])]) {
			assert.equal(
				richText(s)
					.map((r) => r.text)
					.join(''),
				s
			);
			for (const r of richText(s)) {
				if (r.kind === 'link') assert.match(r.href ?? '', /^https:\/\//, r.text);
			}
		}
	}
});
