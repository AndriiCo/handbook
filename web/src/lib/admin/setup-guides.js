// Setup-wizard steps + provider guides (secure-config.md §9).
// Admin-only: never imported from (public) or (customer) routes.
//
// Static content, reviewed with the SPA: one step per group, each
// key with its type, label, and hint, plus a retrieval guide with
// numbered console steps. Synthetic placeholders only — never real
// credentials. A test asserts every registry key except `setup.*`
// appears in exactly one step, so a new key without a wizard slot
// fails loudly.

/**
 * @typedef {object} GuideKey
 * @property {string} key
 * @property {'bool' | 'text' | 'secret'} type
 * @property {string} label
 * @property {string} hint
 * @property {string} [placeholder]
 */

/**
 * @typedef {object} GuideStep
 * @property {string} id
 * @property {string} title
 * @property {string} blurb
 * @property {GuideKey[]} keys
 * @property {{ intro: string, steps: string[] }} guide
 */

/** @type {GuideStep[]} */
export const STEPS = [
	{
		id: 'site',
		title: 'Site & network',
		blurb: 'Where the server listens and where its files live. The compiled defaults match the standard deploy — change only what differs.',
		keys: [
			{ key: 'net.public_host', type: 'text', label: 'Public host', hint: 'Required. The apex domain clients use. Must match DNS and the TLS certificate.', placeholder: 'andrii.ai' },
			{ key: 'net.bind', type: 'text', label: 'Bind (IPv4)', hint: 'Required on a public server: the compiled default is loopback 127.0.0.1. Use 0.0.0.0 for all interfaces.', placeholder: '0.0.0.0' },
			{ key: 'net.bind6', type: 'text', label: 'Bind (IPv6)', hint: 'Optional. Unset means IPv4-only; :: listens dual-stack.', placeholder: '::' },
			{ key: 'net.https_port', type: 'text', label: 'HTTPS port', hint: 'Default 443. HTTP/1.1 + QUIC share this port (single-443 routing).', placeholder: '443' },
			{ key: 'net.http_port', type: 'text', label: 'HTTP port', hint: 'Default 80. Redirects to HTTPS and answers ACME challenges.', placeholder: '80' },
			{ key: 'net.port', type: 'text', label: 'Legacy port', hint: 'Deprecated alias of the HTTPS port; leave unset on new installs.', placeholder: '' },
			{ key: 'net.admin_https_port', type: 'text', label: 'Legacy admin port', hint: 'Deprecated: admin is Host-routed on 443 now. Leave unset.', placeholder: '' },
			{ key: 'net.admin_port', type: 'text', label: 'Legacy admin (plain)', hint: 'Deprecated alongside the split-port layout. Leave unset.', placeholder: '' },
			{ key: 'quic.enabled', type: 'bool', label: 'Enable QUIC', hint: 'Optional. HTTP/3 over UDP/443 via quinn+h3. Safe to enable.', placeholder: '' },
			{ key: 'paths.static_dir', type: 'text', label: 'Static dir', hint: 'Required on prod: the compiled default is a repo-relative dev path. Immutable release payload.', placeholder: '/srv/apostille/current/static' },
			{ key: 'paths.spa_dir', type: 'text', label: 'Public app dir', hint: 'Required on prod: the compiled default is a repo-relative dev path. Immutable release payload.', placeholder: '/srv/apostille/current/spa' },
			{ key: 'paths.admin_spa_dir', type: 'text', label: 'Admin app dir', hint: 'Required on prod: the compiled default is a repo-relative dev path. Contains this wizard.', placeholder: '/srv/apostille/current/spa-admin' },
			{ key: 'paths.acme_challenge_dir', type: 'text', label: 'ACME challenge dir', hint: 'Required on prod: the compiled default is a repo-relative dev path. Must be writable by the service user.', placeholder: '/srv/apostille/current/acme' },
			{ key: 'tls.cert_file', type: 'text', label: 'TLS cert file', hint: 'Required on prod (no compiled default). The ACME manager writes the issued cert here.', placeholder: '/var/lib/apostille/tls/cert.pem' },
			{ key: 'tls.key_file', type: 'text', label: 'TLS key file', hint: 'Required on prod (no compiled default). Never leaves the server.', placeholder: '/var/lib/apostille/tls/key.pem' },
			{ key: 'tls.client_ca_file', type: 'text', label: 'Client CA file', hint: 'Required on prod (no compiled default). Verifies staff mTLS certs.', placeholder: '/var/lib/apostille/admin-certs/ca.pem' }
		],
		guide: {
			intro: 'No consoles to visit — but do not trust the compiled defaults: they are loopback/dev (127.0.0.1, repo paths, no TLS). A public server sets every field in this step except the deprecated ports. Values apply at boot — restart to reload.',
			steps: [
				'Set the public host to the apex domain (DNS A/AAAA already pointing at this server).',
				'Set the IPv4 bind to 0.0.0.0 (all interfaces); add :: for IPv6 only if the host routes it.',
				'Point all four paths at the release tree (/srv/apostille/current/…) and the three TLS files at /var/lib/apostille/… — the ACME step issues into those TLS paths.',
				'Enable QUIC if UDP/443 is open; browsers fall back to TCP silently when it is not.'
			]
		}
	},
	{
		id: 'certs',
		title: 'Certificates (ACME)',
		blurb: 'The server issues and renews its own TLS certificates via ACME. Provide the directory, domains, contact email, and EAB credentials.',
		keys: [
			{ key: 'acme.enabled', type: 'bool', label: 'Enable ACME', hint: 'Optional. On for any public host; off only for loopback dev.', placeholder: '' },
			{ key: 'acme.directory', type: 'text', label: 'ACME directory', hint: 'Default the Google Public CA production directory. Staging URL for dry runs.', placeholder: 'https://dv.acme-v02.api.pki.goog/directory' },
			{ key: 'acme.domains', type: 'text', label: 'Domains', hint: 'Required. Comma-separated SANs: apex + admin + www.', placeholder: 'andrii.ai, admin.andrii.ai, www.andrii.ai' },
			{ key: 'acme.email', type: 'text', label: 'Contact email', hint: 'Required. The CA requires at least one contact for the account.', placeholder: 'ops@example.com' },
			{ key: 'acme.eab_kid', type: 'text', label: 'EAB key ID', hint: 'External Account Binding ID from the CA console (binds ACME to your account).', placeholder: '' },
			{ key: 'acme.eab_hmac', type: 'secret', label: 'EAB HMAC key', hint: 'The EAB secret. Write-only: shown as Set, never back.', placeholder: '' }
		],
		guide: {
			intro: 'Google Public CA requires External Account Binding: create the EAB credentials in the Google Cloud console, then paste them here.',
			steps: [
				'Open the Google Cloud console → Certificate Manager → ACME external accounts (or search “Public CA EAB”).',
				'Create an external account: copy the Key ID into EAB key ID and the HMAC secret into EAB HMAC key.',
				'List every name the server answers on (apex, admin, www) in Domains — a missing SAN fails closed in browsers.',
				'Set the contact email, enable ACME, and save. Issuance runs at boot; watch the service log for the order state.'
			]
		}
	},
	{
		id: 'staff',
		title: 'Staff identity',
		blurb: 'Passkey relying-party coordinates for staff login. They must match the admin host exactly, or ceremonies fail.',
		keys: [
			{ key: 'auth.rp_id', type: 'text', label: 'RP ID', hint: 'Required. The effective domain of the admin host (no scheme, no port).', placeholder: 'admin.andrii.ai' },
			{ key: 'auth.rp_name', type: 'text', label: 'RP name', hint: 'Required. Human name shown by authenticators during ceremonies.', placeholder: 'Apostille Handbook (staff)' },
			{ key: 'auth.origin', type: 'text', label: 'Origin', hint: 'Required. Staff WebAuthn origin ONLY (exact admin origin over TLS) — public links use net.public_host.', placeholder: 'https://admin.andrii.ai' },
			{ key: 'admin.mtls_off', type: 'bool', label: 'Disable staff mTLS', hint: 'Keep OFF (mTLS on). Break-glass only — enrol and sessions normally require certs.', placeholder: '' }
		],
		guide: {
			intro: 'Three strings, no console: they must describe the staff WebAuthn origin precisely (never the public host). The wizard cannot check them — a typo fails at enrolment.',
			steps: [
				'Set the RP ID to the admin host bare domain and the origin to its full https URL (trailing slash never).',
				'Pick an RP display name your staff will recognize on their authenticator prompts.',
				'Leave staff mTLS enabled: the first enrolment is the only flow that skips the client cert (setup key instead).'
			]
		}
	},
	{
		id: 'login',
		title: 'Customer login',
		blurb: 'OAuth clients customers use to sign in. All optional in v1 — configure a provider only if its login button should work.',
		keys: [
			{ key: 'google.client_id', type: 'text', label: 'Google client ID', hint: 'OAuth client ID (…apps.googleusercontent.com).', placeholder: '' },
			{ key: 'google.client_secret', type: 'secret', label: 'Google client secret', hint: 'Client secret. Write-only.', placeholder: '' },
			{ key: 'apple.client_id', type: 'text', label: 'Apple Services ID', hint: 'The Services ID identifier (reverse-DNS), used as client_id.', placeholder: 'ai.andrii.signin' },
			{ key: 'apple.team_id', type: 'text', label: 'Apple Team ID', hint: '10-character Team ID from Membership details.', placeholder: 'A1B2C3D4E5' },
			{ key: 'apple.key_id', type: 'text', label: 'Apple Key ID', hint: '10-character ID of the Sign-in key.', placeholder: 'K1Y2E3Y4I5' },
			{ key: 'apple.private_key', type: 'secret', label: 'Apple private key', hint: 'Full .p8 file contents, including its first/last marker lines. Write-only.', placeholder: '(paste the .p8 file contents)' },
			{ key: 'meta.client_id', type: 'text', label: 'Meta App ID', hint: 'Numeric app ID from the Meta App dashboard.', placeholder: '' },
			{ key: 'meta.client_secret', type: 'secret', label: 'Meta App Secret', hint: 'App secret (show once, rotate on leak). Write-only.', placeholder: '' }
		],
		guide: {
			intro: 'Each provider needs an app registration with the login callback. Register only the providers you will offer.',
			steps: [
				'Google: console → APIs & Services → Credentials → Create OAuth client (Web). Add the callback https://<public host>/auth/google/callback as an authorized redirect URI, then paste the client ID + secret.',
				'Google consent screen: publish the app (Testing mode expires grants weekly) and request only profile + email.',
				'Apple: developer portal → Certificates, Identifiers & Profiles → Identifiers → register a Services ID; enable Sign in with Apple with the callback https://<public host>/auth/apple/callback as a return URL.',
				'Apple: Membership → copy the Team ID; Keys → create a Sign in with Apple key → note the Key ID and download the .p8 once — paste its full contents (it cannot be re-downloaded).',
				'Meta: developers.facebook.com → My Apps → create an app → add Facebook Login → set the callback https://<public host>/auth/meta/callback as a valid OAuth redirect URI; paste the App ID + App Secret.'
			]
		}
	},
	{
		id: 'models',
		title: 'AI models',
		blurb: 'The Meta model API key that powers analysis. Read the model terms first — the gate records that you did.',
		keys: [
			{ key: 'meta.base_url', type: 'text', label: 'Meta API base URL', hint: 'Default the production endpoint. Override for proxies only.', placeholder: 'https://api.meta.ai/v1' },
			{ key: 'meta.api_key', type: 'secret', label: 'Meta API key', hint: 'Required. Format LLM_{id}_{secret}. Write-only.', placeholder: 'LLM_' },
			{ key: 'media.terms_read', type: 'bool', label: 'Model terms read', hint: 'Required. Analysis refuses to run until this is explicitly true.', placeholder: '' }
		],
		guide: {
			intro: 'Create the key in the Meta developer portal (dev.meta.ai docs), then acknowledge the model terms in the wizard.',
			steps: [
				'Open the Meta developer portal → API keys → create a key for the handbook server; copy the LLM_… value once.',
				'Paste it as the Meta API key. A wrong key fails at the first analysis run (401), never silently.',
				'Read the model terms linked from the portal, then set Model terms read. The runner checks this gate on every call.'
			]
		}
	},
	{
		id: 'voice',
		title: 'Voice (Telnyx)',
		blurb: 'Outbound verification calls. All five keys come from the Telnyx portal: API key, number, connection, webhook key.',
		keys: [
			{ key: 'telnyx.base_url', type: 'text', label: 'Telnyx base URL', hint: 'Default https://api.telnyx.com. Override for private interconnects.', placeholder: 'https://api.telnyx.com' },
			{ key: 'telnyx.from_number', type: 'text', label: 'From number', hint: 'Required. Caller ID in E.164, must be a number on your account.', placeholder: '+13605550100' },
			{ key: 'telnyx.connection_id', type: 'text', label: 'Connection ID', hint: 'Required. The Call Control (Telnyx IQ SIP) connection dials from.', placeholder: '' },
			{ key: 'telnyx.webhook_public_key', type: 'text', label: 'Webhook public key', hint: 'Required. Verifies Telnyx-signed status callbacks (Ed25519).', placeholder: '' },
			{ key: 'telnyx.api_key', type: 'secret', label: 'Telnyx API key', hint: 'Required. Shown once at creation. Write-only.', placeholder: '' }
		],
		guide: {
			intro: 'Four portal stops: API key, phone number, Call Control connection, webhook signing key.',
			steps: [
				'Telnyx portal → API Keys → create a key (restrict to Call Control + messaging if offered) → paste as the API key.',
				'Numbers → buy or assign a number with voice + SMS in the regions you call → paste in E.164 as the From number.',
				'Voice → Connections → create/use a Call Control connection → paste its ID.',
				'Voice → Preferences (or the connection webhooks) → copy the public signing key → paste as the webhook key so forged status callbacks fail verification.'
			]
		}
	},
	{
		id: 'mail',
		title: 'Mail',
		blurb: 'Outbound SMTP (magic links, receipts) plus the Gmail inbox poll (closures, notices). Both halves are required.',
		keys: [
			{ key: 'mailer.provider', type: 'text', label: 'SMTP provider', hint: 'Label only (for example ses, postmark, local).', placeholder: 'ses' },
			{ key: 'mailer.host', type: 'text', label: 'SMTP host', hint: 'Required. Submission host of the provider.', placeholder: 'email-smtp.us-east-1.amazonaws.com' },
			{ key: 'mailer.port', type: 'text', label: 'SMTP port', hint: 'Usually 587 (STARTTLS).', placeholder: '587' },
			{ key: 'mailer.username', type: 'text', label: 'SMTP username', hint: 'Required. Provider SMTP credentials, not mailbox login.', placeholder: '' },
			{ key: 'mailer.password', type: 'secret', label: 'SMTP password', hint: 'Required. Write-only.', placeholder: '' },
			{ key: 'mailer.from', type: 'text', label: 'From address', hint: 'Required. Must be a verified sender identity at the provider.', placeholder: 'no-reply@example.com' },
			{ key: 'mailer.dir', type: 'text', label: 'Mail queue dir', hint: 'Local outbox spool. Default under the store dir.', placeholder: '' },
			{ key: 'mail.base_url', type: 'text', label: 'Gmail API base', hint: 'Default https://gmail.googleapis.com. Override for proxies only.', placeholder: 'https://gmail.googleapis.com' },
			{ key: 'mail.token_url', type: 'text', label: 'OAuth token URL', hint: 'Default oauth2.googleapis token endpoint.', placeholder: 'https://oauth2.googleapis.com/token' },
			{ key: 'mail.client_id', type: 'text', label: 'Gmail client ID', hint: 'Required. Desktop-app OAuth client (loopback, no pre-registration).', placeholder: '' },
			{ key: 'mail.client_secret', type: 'secret', label: 'Gmail client secret', hint: 'Required. Write-only.', placeholder: '' },
			{ key: 'mail.refresh_token', type: 'secret', label: 'Gmail refresh token', hint: 'Required. From the one-time loopback authorization below. Write-only.', placeholder: '' }
		],
		guide: {
			intro: 'SMTP first (provider console), then a dedicated Gmail mailbox authorized once via loopback PKCE — no helper tooling, browser + curl.',
			steps: [
				'SMTP: create SMTP credentials at your provider, verify the From address (SPF/DKIM at minimum), paste host/username/password/from.',
				'Gmail: Cloud project → enable Gmail API → OAuth client, type Desktop app → paste client ID + secret. Publish the consent screen (Testing mode re-consent expires weekly).',
				'Generate a PKCE pair: python3 -c "import secrets,hashlib,base64; v=secrets.token_urlsafe(64); c=base64.urlsafe_b64encode(hashlib.sha256(v.encode()).digest()).rstrip(b\'=\').decode(); print(v); print(c)" — keep both lines.',
				'Open in a browser (one line, replace CLIENT/CHALLENGE): https://accounts.google.com/o/oauth2/v2/auth?client_id=CLIENT&redirect_uri=http://127.0.0.1:8085/&response_type=code&scope=https://www.googleapis.com/auth/gmail.readonly&access_type=offline&prompt=consent&code_challenge=CHALLENGE&code_challenge_method=S256 — approve, and copy code=… from the (failing) localhost address bar.',
				'Exchange it: curl -X POST https://oauth2.googleapis.com/token -d code=CODE -d client_id=CLIENT -d client_secret=SECRET -d redirect_uri=http://127.0.0.1:8085/ -d grant_type=authorization_code -d code_verifier=VERIFIER — paste refresh_token. A later invalid_grant means: repeat from the browser step.'
			]
		}
	},
	{
		id: 'postage',
		title: 'Postage (USPS)',
		blurb: 'Digital-stamp label purchases. Optional in v1 — configure when label buying goes live.',
		keys: [
			{ key: 'usps.base_url', type: 'text', label: 'USPS base URL', hint: 'Default the test environment (TEM). Production URL only after certification.', placeholder: 'https://apis-tem.usps.com' },
			{ key: 'usps.certified', type: 'bool', label: 'USPS certified', hint: 'Purchases stay sandboxed until this is explicitly true.', placeholder: '' }
		],
		guide: {
			intro: 'Nothing to retrieve yet: keep the test environment until the USPS certification round completes.',
			steps: [
				'Leave the base URL on the test environment (TEM) for all pre-certification work.',
				'After USPS certifies the integration, switch to production and set Certified — the labels client refuses live purchases otherwise.'
			]
		}
	}
];

/**
 * Keys that must be set (bools: explicitly true) before the wizard
 * calls the setup complete (secure-config.md §9.3). Guidance, not a
 * lock: enrolment may run first.
 * @type {string[]}
 */
export const REQUIRED_KEYS = [
	'net.public_host',
	'net.bind',
	'paths.static_dir',
	'paths.spa_dir',
	'paths.admin_spa_dir',
	'paths.acme_challenge_dir',
	'tls.cert_file',
	'tls.key_file',
	'tls.client_ca_file',
	'acme.email',
	'acme.domains',
	'auth.rp_id',
	'auth.rp_name',
	'auth.origin',
	'meta.api_key',
	'media.terms_read',
	'telnyx.api_key',
	'telnyx.from_number',
	'telnyx.connection_id',
	'telnyx.webhook_public_key',
	'mailer.host',
	'mailer.username',
	'mailer.password',
	'mailer.from',
	'mail.client_id',
	'mail.client_secret',
	'mail.refresh_token'
];

/**
 * Every wizard key with its step id. Pure (tested).
 * @returns {Map<string, string>}
 */
export function keyStepIndex() {
	const index = new Map();
	for (const step of STEPS) {
		for (const field of step.keys) {
			index.set(field.key, step.id);
		}
	}
	return index;
}
