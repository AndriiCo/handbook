// Admin config guide: one entry per registry key (crates/config/src/lib.rs
// `keys::ALL`), grouped so each vendor's setup manual shows once.
// Admin-only: never imported from (public) or (customer) routes.
//
// Static, reviewed text. Synthetic example shapes only, never a real
// credential (web/test/config-guide.test.mjs bans secret-shaped
// strings and the shots banned words). Facts come from the code paths
// cited in the commit; vendor console paths marked "(verify)" were not
// checked against the vendor's current UI.

export type ConfigType = 'bool' | 'text' | 'secret';

/**
 * When a saved value takes effect (from the reading code, not the spec):
 * boot     RuntimeConfig::load at startup, restart the service.
 * live     CachedReader (30 s TTL; the admin PUT clears the cache).
 * call     re-read on every request or model call.
 * poll     read by each `xtask mail-poll` run (apostille-mail.timer).
 * locked   CLI-managed; the admin PUT answers 422.
 */
export type Applies = 'boot' | 'live' | 'call' | 'poll' | 'locked';

export interface GuideKey {
	key: string;
	type: ConfigType;
	label: string;
	what: string;
	how: string;
	format: string;
	default?: string;
	applies: Applies;
	readOnly?: boolean;
}

export interface GuideGroup {
	id: string;
	title: string;
	summary: string;
	setup: string[];
	keys: GuideKey[];
}

export const APPLIES_TEXT: Record<Applies, string> = {
	boot: 'Read at boot. Restart the service to apply.',
	live: 'Applies within seconds. No restart.',
	call: 'Read on every call. No restart.',
	poll: 'Read by the next mail-poll run.',
	locked: 'Managed from the server CLI. This page cannot change it.'
};

export const CONFIG_GUIDE: GuideGroup[] = [
	{
		id: 'setup',
		title: 'First-boot setup',
		summary:
			'The gate for the first-boot wizard. Written only by xtask setup-key-mint on the server and erased when the first staff passkey enrols. This page cannot change it.',
		setup: [
			'Mint a key on the server with xtask setup-key-mint; it prints once and stops working 72 hours later.',
			'Enrol the first staff passkey from the wizard; that erases the gate and this page takes over.'
		],
		keys: [
			{
				key: 'setup.key_hash',
				type: 'secret',
				label: 'Setup key hash',
				what: 'SHA-256 of the minted setup key. Present means the setup window is open.',
				how: 'Server CLI only.',
				format: '64 hex characters. Never shown.',
				applies: 'locked',
				readOnly: true
			}
		]
	},
	{
		id: 'net',
		title: 'Network & listeners',
		summary:
			'Which IPs and ports the server binds and which host customers use. With TLS (pinned files or ACME) one listener on the HTTPS port serves andrii.ai and admin.andrii.ai by Host header, plus a port-80 redirector. With neither, the server runs plain HTTP on two ports (local dev and plant staging).',
		setup: [
			'Point DNS A/AAAA records for the public host, admin.andrii.ai and www at the server. The admin host name is fixed in code (admin.andrii.ai).',
			'Set net.bind to the server address (or 0.0.0.0). The default 127.0.0.1 is loopback only. Add net.bind6 only if the host routes IPv6.',
			'Leave net.https_port 443 and net.http_port 80 unless a proxy sits in front. Port 80 must stay reachable: it serves ACME http-01 challenges.',
			'Turn on quic.enabled only when UDP on the HTTPS port is open in the firewall.',
			'Restart the service, then check the boot log (journalctl -u apostille) for the "listening on" lines and any config warnings.'
		],
		keys: [
			{
				key: 'net.public_host',
				type: 'text',
				label: 'Public host',
				what: 'Customer-facing host. Every customer URL is built as https://{host}: OAuth callbacks (/auth/<provider>/callback), magic sign-in links, follow and unsubscribe links, Stripe success and cancel URLs, and the port-80 redirect target. It is not the passkey origin (see auth.origin).',
				how: 'Your apex domain, already pointing at this server in DNS.',
				format: 'Bare host name: no scheme, port or slash. Example: andrii.ai',
				default: 'andrii.ai',
				applies: 'boot'
			},
			{
				key: 'net.bind',
				type: 'text',
				label: 'Bind address (IPv4)',
				what: 'IP every listener binds. A dev KEK or admin.mtls_off in a release build is allowed only while every bind IP is loopback.',
				how: 'The server address from your hosting panel, or 0.0.0.0 for all interfaces.',
				format: 'One IP literal. Example: 0.0.0.0 or 203.0.113.10. An unparsable value falls back to 127.0.0.1 with a boot warning.',
				default: '127.0.0.1',
				applies: 'boot'
			},
			{
				key: 'net.bind6',
				type: 'text',
				label: 'Bind address (IPv6)',
				what: 'Optional second bind IP. Empty means IPv4 only.',
				how: 'The server IPv6 address from your hosting panel, or :: for all IPv6 interfaces.',
				format: 'One IPv6 literal. Example: 2001:db8::10. Empty = unset.',
				default: 'unset',
				applies: 'boot'
			},
			{
				key: 'net.https_port',
				type: 'text',
				label: 'HTTPS port',
				what: 'The single TLS listener for both hosts (Host-routed). QUIC binds UDP on the same port.',
				how: 'Keep 443 unless a proxy terminates TLS on 443 for you.',
				format: 'Integer 1-65535. Example: 443',
				default: '443',
				applies: 'boot'
			},
			{
				key: 'net.http_port',
				type: 'text',
				label: 'HTTP port (redirect + ACME)',
				what: 'Plain-HTTP listener in TLS and ACME modes: redirects to https://{net.public_host} and serves /.well-known/acme-challenge/. It is bound before ACME issuance starts.',
				how: 'Keep 80: certificate authorities validate http-01 on port 80.',
				format: 'Integer 1-65535. Example: 80',
				default: '80',
				applies: 'boot'
			},
			{
				key: 'net.port',
				type: 'text',
				label: 'Plain-HTTP public port',
				what: 'Public listener used only when no TLS files are set and ACME is off (local dev; plant staging uses 18318).',
				how: 'Pick a free local port. Leave unset on a TLS server.',
				format: 'Integer 1-65535. Example: 3000',
				default: '3000',
				applies: 'boot'
			},
			{
				key: 'net.admin_port',
				type: 'text',
				label: 'Plain-HTTP admin port',
				what: 'Admin listener in the same no-TLS mode (plant staging uses 18319).',
				how: 'Pick a free local port next to net.port. Leave unset on a TLS server.',
				format: 'Integer 1-65535. Example: 3001',
				default: '3001',
				applies: 'boot'
			},
			{
				key: 'quic.enabled',
				type: 'bool',
				label: 'HTTP/3 (QUIC)',
				what: 'Adds an HTTP/3 listener on UDP net.https_port and sends Alt-Svc so browsers upgrade. TLS modes only; browsers fall back to TCP when UDP is blocked.',
				how: 'Open UDP 443 in the firewall first.',
				format: 'On or off.',
				default: 'off',
				applies: 'boot'
			}
		]
	},
	{
		id: 'paths',
		title: 'Release paths',
		summary:
			'Where the server reads the built web apps and static files. The defaults are repo-relative dev paths; production points at the release tree behind /srv/apostille/current.',
		setup: [
			'Each deploy unpacks spa/, spa-admin/ and static/ into a release directory and points /srv/apostille/current at it, so set the three app paths through the current symlink and deploys need no config change.',
			'Set paths.acme_challenge_dir to a directory the service user can write, for example /var/lib/apostille/acme.',
			'Restart and open /apostille and the admin host. A wrong path shows up as a 404 for the app shell.'
		],
		keys: [
			{
				key: 'paths.static_dir',
				type: 'text',
				label: 'Static files',
				what: 'Files served at /apostille/static/; robots.txt answers /robots.txt and privacy.html answers /privacy.',
				how: 'The static/ directory of the live release.',
				format: 'Absolute path. Example: /srv/apostille/current/static',
				default: 'crates/web/static',
				applies: 'boot'
			},
			{
				key: 'paths.spa_dir',
				type: 'text',
				label: 'Public app build',
				what: 'The public SvelteKit build; its index.html answers every /apostille page.',
				how: 'The spa/ directory of the live release.',
				format: 'Absolute path. Example: /srv/apostille/current/spa',
				default: 'web/build',
				applies: 'boot'
			},
			{
				key: 'paths.admin_spa_dir',
				type: 'text',
				label: 'Admin app build',
				what: 'The admin SvelteKit build served on the admin host (this page).',
				how: 'The spa-admin/ directory of the live release.',
				format: 'Absolute path. Example: /srv/apostille/current/spa-admin',
				default: 'web/build-admin',
				applies: 'boot'
			},
			{
				key: 'paths.acme_challenge_dir',
				type: 'text',
				label: 'ACME challenge directory',
				what: 'Where ACME writes http-01 tokens; served at /.well-known/acme-challenge/ on port 80 and on HTTPS.',
				how: 'A directory the service user can write, outside the release tree.',
				format: 'Absolute path. Example: /var/lib/apostille/acme',
				default: 'acme (relative to the working directory)',
				applies: 'boot'
			}
		]
	},
	{
		id: 'tls',
		title: 'TLS certificates & ACME',
		summary:
			'One certificate covers andrii.ai, admin.andrii.ai and www.andrii.ai. Either pin files you issued (tls.cert_file + tls.key_file) or let the server issue and renew from Google Public CA over ACME. Pinned files always win: while both are set, ACME stays off.',
		setup: [
			'Choose a mode. ACME: leave tls.cert_file and tls.key_file empty. Pinned files: set both (Google PKI certificates only, per docs/ops/acme.md).',
			'Enable the Public CA API once: gcloud services enable publicca.googleapis.com --project=<your project>. Guide: https://cloud.google.com/certificate-manager/docs/public-ca-tutorial (verify).',
			'Mint an EAB key: gcloud publicca external-account-keys create --project=<your project> --key-output-file=<file>. The output holds keyId and b64MacKey. A key registers one ACME account and expires 7 days after creation if unused.',
			'Paste keyId into acme.eab_kid and b64MacKey into acme.eab_hmac, then delete the local file.',
			'List every served name in acme.domains (boot warns when the certificate lacks admin.andrii.ai) and set acme.email.',
			'Make sure port 80 is reachable from the internet, turn on acme.enabled and restart. The first boot runs the order; certificates persist under <STORE_DIR>/acme/ and renew in the background without restarts.',
			'Dry run: set acme.directory to staging (untrusted certificates; a staging EAB key needs the preprod endpoint override in docs/ops/acme.md).'
		],
		keys: [
			{
				key: 'acme.enabled',
				type: 'bool',
				label: 'Issue certificates with ACME',
				what: 'Turns on in-binary ACME when no pinned files are set. Off with no files means plain-HTTP dev mode.',
				how: 'Turn on last, after the EAB pair and domains are saved.',
				format: 'On or off.',
				default: 'off',
				applies: 'boot'
			},
			{
				key: 'acme.directory',
				type: 'text',
				label: 'ACME directory',
				what: 'Which CA directory to order from.',
				how: 'prod for real certificates, staging for dry runs.',
				format: 'prod, staging, or a full https:// directory URL. prod = https://dv.acme-v02.api.pki.goog/directory, staging = https://dv.acme-v02.test-api.pki.goog/directory',
				default: 'prod',
				applies: 'boot'
			},
			{
				key: 'acme.domains',
				type: 'text',
				label: 'Certificate names',
				what: 'Names on the certificate (one order, one certificate).',
				how: 'Every host the server answers on. Keep admin.andrii.ai on the list.',
				format: 'Comma-separated DNS names. Example: andrii.ai,admin.andrii.ai,www.andrii.ai',
				default: 'andrii.ai,admin.andrii.ai,www.andrii.ai',
				applies: 'boot'
			},
			{
				key: 'acme.email',
				type: 'text',
				label: 'ACME contact email',
				what: 'Optional contact mailbox on the ACME account.',
				how: 'An operations mailbox you read.',
				format: 'user@host, without mailto:. Example: ops@example.com',
				default: 'unset',
				applies: 'boot'
			},
			{
				key: 'acme.eab_kid',
				type: 'text',
				label: 'EAB key ID',
				what: 'External Account Binding key ID: binds the ACME account to your Google Cloud project. A public identifier (it is logged at registration). Required when ACME is on.',
				how: 'Setup step 3: the keyId field of the gcloud output.',
				format: 'Opaque ID string exactly as printed by gcloud.',
				applies: 'boot'
			},
			{
				key: 'acme.eab_hmac',
				type: 'secret',
				label: 'EAB HMAC key',
				what: 'The secret half of the EAB pair. Required when ACME is on.',
				how: 'Setup step 3: the b64MacKey field of the gcloud output.',
				format: 'Base64, standard or URL-safe, padding optional. Write-only.',
				applies: 'boot'
			},
			{
				key: 'tls.cert_file',
				type: 'text',
				label: 'Pinned certificate file',
				what: 'PEM certificate chain to serve. While this and tls.key_file are both set, ACME is skipped.',
				how: 'A path on the server to a chain you issued. Save an empty value to hand over to ACME.',
				format: 'Absolute path. Example: /var/lib/apostille/tls/cert.pem. Empty = unset. Set both files or neither; one alone refuses to boot.',
				default: 'unset',
				applies: 'boot'
			},
			{
				key: 'tls.key_file',
				type: 'text',
				label: 'Pinned private key file',
				what: 'PEM private key for tls.cert_file. The key never leaves the server.',
				how: 'A path on the server readable by the service user only.',
				format: 'Absolute path. Example: /var/lib/apostille/tls/key.pem. Empty = unset.',
				default: 'unset',
				applies: 'boot'
			}
		]
	},
	{
		id: 'admin',
		title: 'Admin access & passkeys',
		summary:
			'Who can reach admin.andrii.ai and how passkeys verify. Every admin request must come from an allow-listed IP and then carry a staff passkey session. One WebAuthn configuration serves customer passkeys on andrii.ai and staff passkeys on admin.andrii.ai.',
		setup: [
			'Find your public IP: a refused browser gets a 403 page that names the detected IP. For IPv6 allow your /64, since privacy addresses rotate.',
			'Set admin.allow_ips before you need remote access, from loopback or with the server CLI (xtask config-set --key admin.allow_ips --type text --value <ranges>), then restart. Unset or unparsable means loopback only.',
			'Keep auth.rp_id = andrii.ai and auth.origin = https://andrii.ai in production. https://admin.andrii.ai is always accepted as an extra origin. Changing auth.rp_id invalidates every passkey already enrolled. (verify against the sealed production values before changing)',
			'Local dev (xtask up) seals auth.rp_id localhost, auth.origin http://localhost:<admin port> and admin.mtls_off on. Never turn admin.mtls_off on for a public server.'
		],
		keys: [
			{
				key: 'admin.allow_ips',
				type: 'text',
				label: 'Admin IP allowlist',
				what: 'IP ranges allowed to reach anything on the admin host: pages, API and the setup wizard. The Telnyx webhook is exempt (it is signature-checked instead).',
				how: 'Your office or VPN egress addresses. The 403 page shows the address the server saw.',
				format: 'Comma-separated IPs or CIDRs, IPv4 and IPv6. Example: 203.0.113.0/24, 198.51.100.7, 2001:db8:1234:5678::/64. A bare IP is one host. 0.0.0.0/0 would allow everyone.',
				default: 'loopback only (127.0.0.0/8, ::1)',
				applies: 'boot'
			},
			{
				key: 'admin.mtls_off',
				type: 'bool',
				label: 'Local-dev admin seam',
				what: 'Lets the admin host guard accept localhost and 127.0.0.1 and relaxes origin checks for loopback development. A release build refuses to boot with it on unless every bind IP is loopback.',
				how: 'Set only by xtask up on a laptop.',
				format: 'On or off. Keep off on any public server.',
				default: 'off',
				applies: 'boot'
			},
			{
				key: 'auth.rp_id',
				type: 'text',
				label: 'Passkey RP ID',
				what: 'WebAuthn relying-party ID for every passkey, customer and staff. Subdomains of it are accepted.',
				how: 'Your registrable domain. Changing it orphans existing passkeys.',
				format: 'Bare domain, no scheme or port. Production: andrii.ai. Loopback dev: localhost.',
				default: 'andrii.ai',
				applies: 'boot'
			},
			{
				key: 'auth.rp_name',
				type: 'text',
				label: 'Passkey display name',
				what: 'Name authenticators show in passkey prompts.',
				how: 'A name your customers and staff recognise.',
				format: 'Free text. Example: Apostille Handbook',
				default: 'Apostille Handbook',
				applies: 'boot'
			},
			{
				key: 'auth.origin',
				type: 'text',
				label: 'Passkey origin',
				what: 'Primary WebAuthn origin; https://admin.andrii.ai is always added and subdomains are accepted. It feeds passkeys only; customer links use net.public_host. The server refuses to boot when this and auth.rp_id do not form a valid pair.',
				how: 'The https origin of the public host in production; the loopback admin origin in dev.',
				format: 'Scheme + host (+ port), no path or trailing slash. Production: https://andrii.ai. Dev: http://localhost:3001',
				default: 'https://andrii.ai',
				applies: 'boot'
			}
		]
	},
	{
		id: 'login',
		title: 'Customer sign-in (Google, Apple, Meta)',
		summary:
			'OAuth apps behind the sign-in buttons. A provider button renders only when all of its keys are set (GET /apostille/api/v1/auth/providers). Callbacks live at the domain root, not under /apostille.',
		setup: [
			'Google: https://console.cloud.google.com/apis/credentials > Create credentials > OAuth client ID > Web application. Authorized redirect URI: https://andrii.ai/auth/google/callback. On the OAuth consent screen request openid, email and profile, then publish the app.',
			'Apple: https://developer.apple.com/account/resources/identifiers/list/serviceId > + > Services IDs. Enable Sign in with Apple > Configure: primary App ID, domain andrii.ai, return URL https://andrii.ai/auth/apple/callback.',
			'Apple: https://developer.apple.com/account/resources/authkeys/list > + > enable Sign in with Apple > download the .p8 file (one download only) and note its Key ID. The Team ID is under Membership details at https://developer.apple.com/account.',
			'Meta: https://developers.facebook.com/apps > Create app > add Facebook Login. Facebook Login > Settings > Valid OAuth Redirect URIs: https://andrii.ai/auth/meta/callback. App settings > Basic: copy App ID and App Secret, and set the data deletion callback URL to https://andrii.ai/auth/meta/delete-request. Switch the app to Live.',
			'Optional, Google Cross-Account Protection (RISC): the receiver is https://andrii.ai/auth/google/risc (verify the registration steps in Google RISC docs).',
			'Restart. The login page shows one button per fully configured provider.'
		],
		keys: [
			{
				key: 'google.client_id',
				type: 'text',
				label: 'Google client ID',
				what: 'Google OAuth web client. With google.client_secret it enables the Google button (scopes openid, email, profile).',
				how: 'Setup step 1: the Client ID of the web client.',
				format: '<digits>-<random>.apps.googleusercontent.com',
				applies: 'boot'
			},
			{
				key: 'google.client_secret',
				type: 'secret',
				label: 'Google client secret',
				what: 'Secret for the token exchange on /auth/google/callback.',
				how: 'Setup step 1: the Client secret of the same client.',
				format: 'String starting GOCSPX- . Write-only.',
				applies: 'boot'
			},
			{
				key: 'apple.client_id',
				type: 'text',
				label: 'Apple Services ID',
				what: 'Services ID used as client_id. Apple sign-in needs all four apple.* keys.',
				how: 'Setup step 2: the Services ID identifier.',
				format: 'Reverse-DNS identifier. Example: ai.andrii.signin',
				applies: 'boot'
			},
			{
				key: 'apple.team_id',
				type: 'text',
				label: 'Apple Team ID',
				what: 'Issuer of the ES256 client-secret JWT the server signs for each token exchange.',
				how: 'Setup step 3: Membership details.',
				format: '10 uppercase letters and digits. Example: A1B2C3D4E5',
				applies: 'boot'
			},
			{
				key: 'apple.key_id',
				type: 'text',
				label: 'Apple Key ID',
				what: 'ID of the Sign in with Apple key; goes in the JWT header.',
				how: 'Setup step 3: shown next to the key.',
				format: '10 uppercase letters and digits. Example: K1Y2E3Y4I5',
				applies: 'boot'
			},
			{
				key: 'apple.private_key',
				type: 'secret',
				label: 'Apple private key (.p8)',
				what: 'Signs the client-secret JWT.',
				how: 'Setup step 3: open the downloaded AuthKey_<Key ID>.p8 file in a text editor and paste all of it.',
				format: 'The whole PEM block, from its BEGIN PRIVATE KEY line to its END PRIVATE KEY line. Write-only; Apple never shows it again.',
				applies: 'boot'
			},
			{
				key: 'meta.client_id',
				type: 'text',
				label: 'Meta App ID',
				what: 'Facebook Login app behind the Meta button. Not the Meta model API (that is meta.api_key).',
				how: 'Setup step 4: App settings > Basic > App ID.',
				format: 'Numeric. Example: 1234567890123456',
				applies: 'boot'
			},
			{
				key: 'meta.client_secret',
				type: 'secret',
				label: 'Meta App Secret',
				what: 'Token exchange for Meta login; also verifies the signed_request on data deletion callbacks.',
				how: 'Setup step 4: App settings > Basic > App Secret > Show.',
				format: '32 hex characters. Write-only.',
				applies: 'boot'
			}
		]
	},
	{
		id: 'meta-model',
		title: 'Meta model API',
		summary:
			'Key and budget for every Meta model call: fact drafts, page plans, offices, cross-checks, form maps, translations, call transcripts and the xtask analyze and atlas runs. Only synthetic or public text may be sent, and every call follows .agents/meta-api-findings.md.',
		setup: [
			'Create an API key on the Meta developer platform (docs: https://dev.meta.ai/docs) and copy the LLM_ value once. (verify the console menu name)',
			'Paste it into meta.api_key. A wrong key fails the first call with 401. Staging on plant uses its own staging-only key, never this one.',
			'Read the model terms and data-handling pages on dev.meta.ai, then turn on media.terms_read and restart.',
			'Set meta.daily_call_cap to your budget and watch usage in the admin trace console (/trace).'
		],
		keys: [
			{
				key: 'meta.api_key',
				type: 'secret',
				label: 'Meta API key',
				what: 'Bearer key for the model API. Read on each call; a META_API_KEY variable in the process environment takes precedence (staging and one-shot runs).',
				how: 'Setup step 1.',
				format: 'LLM_<numeric id>_<secret>. Write-only.',
				applies: 'call'
			},
			{
				key: 'meta.base_url',
				type: 'text',
				label: 'Meta API base URL',
				what: 'API root for the model clients. Override only for a proxy or a test stub.',
				how: 'Leave unset for production.',
				format: 'https URL including /v1, no trailing slash. Example: https://api.meta.ai/v1',
				default: 'https://api.meta.ai/v1',
				applies: 'call'
			},
			{
				key: 'meta.daily_call_cap',
				type: 'text',
				label: 'Daily model-call cap',
				what: 'Hard daily budget counted from recorded model calls since 00:00 UTC across every pipeline. Once reached, staff draft, plan, offices, cross-check and form-map runs answer 429. 0 stops every run.',
				how: 'Your daily call budget.',
				format: 'Integer, 0 or more. Example: 400',
				default: '400',
				applies: 'live'
			},
			{
				key: 'media.terms_read',
				type: 'bool',
				label: 'Model terms read',
				what: 'Records that the operator read the model terms. Off: uploads are limited to synthetic fixtures and call recordings stay untranscribed.',
				how: 'Setup step 3.',
				format: 'On or off.',
				default: 'off',
				applies: 'boot'
			}
		]
	},
	{
		id: 'mailer',
		title: 'Outbound mail (SMTP)',
		summary:
			'Sends magic sign-in links and follow, confirm and digest emails. mailer.provider picks the transport: smtp sends over STARTTLS, file writes messages to a local folder (dev). Unset or unknown disables email sign-in.',
		setup: [
			'Gmail or Google Workspace: turn on 2-Step Verification, create an app password at https://myaccount.google.com/apppasswords, then use host smtp.gmail.com, port 587 and the full mailbox address as the username.',
			'Another provider, for example Amazon SES: https://console.aws.amazon.com/ses > SMTP settings > Create SMTP credentials, and use its submission host on port 587. Port 465 (implicit TLS) is not supported; the client speaks STARTTLS.',
			'Verify the From domain at the provider (SPF, DKIM, DMARC).',
			'Set mailer.provider to smtp and fill the five SMTP fields, restart, then request a sign-in link on /apostille/login to test.'
		],
		keys: [
			{
				key: 'mailer.provider',
				type: 'text',
				label: 'Mail transport',
				what: 'Switch between real SMTP and the dev file spool.',
				how: 'smtp on servers, file on a laptop.',
				format: 'smtp or file. Any other value disables email sign-in with a boot warning.',
				default: 'unset (email sign-in off)',
				applies: 'boot'
			},
			{
				key: 'mailer.host',
				type: 'text',
				label: 'SMTP host',
				what: 'Submission host for the smtp transport.',
				how: 'Setup step 1 or 2.',
				format: 'Host name. Example: smtp.gmail.com or email-smtp.us-east-1.amazonaws.com',
				applies: 'boot'
			},
			{
				key: 'mailer.port',
				type: 'text',
				label: 'SMTP port',
				what: 'Submission port; the connection upgrades with STARTTLS.',
				how: 'Use 587.',
				format: 'Integer. Example: 587',
				applies: 'boot'
			},
			{
				key: 'mailer.username',
				type: 'text',
				label: 'SMTP username',
				what: 'SMTP login.',
				how: 'Gmail: the mailbox address. SES: the SMTP user name from step 2 (not an IAM access key).',
				format: 'Text. Example: no-reply@example.com',
				applies: 'boot'
			},
			{
				key: 'mailer.password',
				type: 'secret',
				label: 'SMTP password',
				what: 'SMTP password or app password.',
				how: 'Gmail: the 16-letter app password. SES: the SMTP password shown once at creation.',
				format: 'Text. Write-only.',
				applies: 'boot'
			},
			{
				key: 'mailer.from',
				type: 'text',
				label: 'From address',
				what: 'From header on every message. It must parse as a mailbox, or the mailer is disabled at boot.',
				how: 'A sender address verified at the provider.',
				format: 'addr@domain or Name <addr@domain>. Example: Apostille Handbook <no-reply@example.com>',
				applies: 'boot'
			},
			{
				key: 'mailer.dir',
				type: 'text',
				label: 'File spool directory',
				what: 'file transport only: where messages are written instead of sent.',
				how: 'Leave the default.',
				format: 'Path relative to STORE_DIR. Example: mail',
				default: 'mail',
				applies: 'boot'
			}
		]
	},
	{
		id: 'gmail',
		title: 'Inbound mail (Gmail)',
		summary:
			'xtask mail-poll (apostille-mail.timer) reads one dedicated Gmail inbox, read-only, into email captures. Separate from outbound SMTP. Values are read on each poll run.',
		setup: [
			'In https://console.cloud.google.com: APIs & Services > Library > enable the Gmail API.',
			'APIs & Services > Credentials > Create credentials > OAuth client ID, type Desktop app. Copy the client ID and secret.',
			'OAuth consent screen: add the scope https://www.googleapis.com/auth/gmail.readonly and publish the app (in Testing mode refresh tokens expire after 7 days).',
			'Mint a refresh token once with loopback PKCE: open the consent URL with access_type=offline and prompt=consent, approve, then exchange the code at https://oauth2.googleapis.com/token (the commands are in the setup wizard Mail step and docs/gmail-shapes.md).',
			'Paste the three values. The next timer run logs mail-poll: listed=... created=... ; an invalid_grant error later means repeat step 4.'
		],
		keys: [
			{
				key: 'mail.client_id',
				type: 'text',
				label: 'Gmail client ID',
				what: 'Desktop-app OAuth client the poller refreshes tokens with.',
				how: 'Setup step 2.',
				format: '<digits>-<random>.apps.googleusercontent.com',
				applies: 'poll'
			},
			{
				key: 'mail.client_secret',
				type: 'secret',
				label: 'Gmail client secret',
				what: 'Secret of the same Desktop-app client.',
				how: 'Setup step 2.',
				format: 'String starting GOCSPX- . Write-only.',
				applies: 'poll'
			},
			{
				key: 'mail.refresh_token',
				type: 'secret',
				label: 'Gmail refresh token',
				what: 'Long-lived grant for the dedicated mailbox (gmail.readonly).',
				how: 'Setup step 4: the refresh_token field of the token response.',
				format: 'Opaque string, usually starting 1// . Write-only.',
				applies: 'poll'
			},
			{
				key: 'mail.base_url',
				type: 'text',
				label: 'Gmail API root',
				what: 'Gmail API root for the poller. Override for a test stub only.',
				how: 'Leave unset.',
				format: 'https URL, no trailing slash.',
				default: 'https://gmail.googleapis.com',
				applies: 'poll'
			},
			{
				key: 'mail.token_url',
				type: 'text',
				label: 'OAuth token endpoint',
				what: 'Where the poller exchanges the refresh token. Override for a test stub only.',
				how: 'Leave unset.',
				format: 'https URL.',
				default: 'https://oauth2.googleapis.com/token',
				applies: 'poll'
			}
		]
	},
	{
		id: 'notify',
		title: 'Follower notifications',
		summary:
			'Email digests for people who follow an authority (subscribe, confirm and unsubscribe under /apostille/api/v1/notify/). Needs a working outbound mailer and a postal address.',
		setup: [
			'Configure Outbound mail (SMTP) first.',
			'Set notify.postal_address to the operator mailing address. Every confirm and digest email carries it (CAN-SPAM), and digests refuse to send while it is empty.',
			'Pick notify.daily_cap, turn on notify.enabled and restart.'
		],
		keys: [
			{
				key: 'notify.enabled',
				type: 'bool',
				label: 'Notifications on',
				what: 'Master switch for subscribe and the digest worker. Off: subscribe and confirm answer unavailable and nothing sends.',
				how: 'Turn on after the mailer and postal address work.',
				format: 'On or off.',
				default: 'off',
				applies: 'boot'
			},
			{
				key: 'notify.daily_cap',
				type: 'text',
				label: 'Digests per subscriber per day',
				what: 'Max digest emails one subscriber receives per day.',
				how: 'Keep it low; digests batch facts.',
				format: 'Integer 1-50. Example: 3',
				default: '3',
				applies: 'boot'
			},
			{
				key: 'notify.postal_address',
				type: 'text',
				label: 'Postal address footer',
				what: 'Physical mailing address printed in every notification email.',
				how: 'The operator business or PO box address.',
				format: 'One line. Example: Example Co, 1 Example Way, Seattle, WA 98101',
				default: 'empty (digests do not send)',
				applies: 'boot'
			}
		]
	},
	{
		id: 'stripe',
		title: 'Payments (Stripe)',
		summary:
			'Hosted Checkout for paid order tiers plus the signed webhook. Card data never reaches this server. Both keys are read per request, so no restart. A live key (sk_live_ or rk_live_ prefix) switches orders to live mode; any other key is test mode.',
		setup: [
			'Stripe Dashboard > Developers > API keys (https://dashboard.stripe.com/apikeys; test mode https://dashboard.stripe.com/test/apikeys). Copy the secret key, or create a restricted key with write access to Checkout Sessions and Refunds (verify permission names).',
			'Developers > Webhooks > Add endpoint (https://dashboard.stripe.com/webhooks; newer dashboards call it Workbench > Event destinations): URL https://andrii.ai/apostille/api/v1/stripe/webhook, events checkout.session.completed and checkout.session.async_payment_succeeded.',
			'Open the endpoint > Signing secret > Reveal, and paste it into stripe.webhook_secret. Test and live endpoints have different secrets.',
			'Staging cannot receive webhooks; the reconciler re-reads open Checkout sessions every minute instead.',
			'Then set Orders & fees and turn on orders.enabled.'
		],
		keys: [
			{
				key: 'stripe.secret_key',
				type: 'secret',
				label: 'Stripe secret key',
				what: 'Creates and re-reads Checkout Sessions and issues refunds. Without it paid tiers show as paused and the webhook answers 503.',
				how: 'Setup step 1.',
				format: 'sk_test_…, sk_live_…, rk_test_… or rk_live_… . Write-only.',
				applies: 'call'
			},
			{
				key: 'stripe.webhook_secret',
				type: 'secret',
				label: 'Webhook signing secret',
				what: 'Verifies the Stripe-Signature header (HMAC-SHA256, 5-minute tolerance) on POST /apostille/api/v1/stripe/webhook. Without it the webhook answers 503.',
				how: 'Setup step 3.',
				format: 'whsec_… . Write-only.',
				applies: 'call'
			}
		]
	},
	{
		id: 'orders',
		title: 'Orders & fees',
		summary:
			'Customer order switches, prices and caps. Amounts are integer cents in orders.currency. Saving here applies within seconds.',
		setup: [
			'Configure Payments (Stripe) first: paid tiers need both orders.enabled and stripe.secret_key.',
			'Mailing kits: set orders.kit_price_cents and orders.ship_from (the kit card stays paused until both are valid).',
			'Label tiers: set orders.fee_label_cents, then turn on labels.customer_live in Label limits & tracking and set the carrier keys (FedEx API, USPS). Each label order is priced at the carrier for both legs, plus the fee.',
			'Set orders.max_order_cents and orders.daily_cap, turn on orders.enabled, then open /apostille/order: the delivery cards show prices and the mode (test or live).',
			'To pause paid orders at once, turn orders.enabled off. The free address label keeps working.'
		],
		keys: [
			{
				key: 'orders.enabled',
				type: 'bool',
				label: 'Paid orders on',
				what: 'Master switch for paid tiers. Off: paid options show as paused and checkout answers 503 paid_tiers_paused.',
				how: 'Turn on after Stripe and prices are set.',
				format: 'On or off.',
				default: 'off',
				applies: 'live'
			},
			{
				key: 'orders.kit_price_cents',
				type: 'text',
				label: 'Mailing kit price',
				what: 'All-in price of the mailing kit (kit plus prepaid envelopes both ways). Unset or 0: the kit card stays paused.',
				how: 'Your retail price, in cents.',
				format: 'Positive integer cents. Example: 4900 = $49.00',
				default: 'unset (kit paused)',
				applies: 'live'
			},
			{
				key: 'orders.currency',
				type: 'text',
				label: 'Checkout currency',
				what: 'Currency for every Checkout session; checked against the paid session at settlement.',
				how: 'The currency your Stripe account settles in.',
				format: 'ISO 4217 code, lowercase. Example: usd',
				default: 'usd',
				applies: 'live'
			},
			{
				key: 'orders.max_order_cents',
				type: 'text',
				label: 'Max order total',
				what: 'Per-order ceiling: a quote or checkout above it answers paused.',
				how: 'A little above your most expensive option.',
				format: 'Integer cents. Example: 15000 = $150.00',
				default: '15000',
				applies: 'live'
			},
			{
				key: 'orders.daily_cap',
				type: 'text',
				label: 'Paid orders per day',
				what: 'Soft cap per UTC day in the current mode: once this many payments are captured, no new Checkout opens; sessions already open may still complete.',
				how: 'Your daily fulfilment capacity.',
				format: 'Integer. Example: 20',
				default: '20',
				applies: 'live'
			},
			{
				key: 'orders.fee_label_cents',
				type: 'text',
				label: 'Label service fee',
				what: 'Service fee added to every paid FedEx or USPS label order, on top of the carrier price for both legs. It shows as its own "Service fee" line; 0 or unset adds none.',
				how: 'Your fee per order, in cents.',
				format: 'Integer cents, 0 to 100000. Example: 500 = $5.00',
				default: '0',
				applies: 'live'
			},
			{
				key: 'orders.ship_from',
				type: 'text',
				label: 'Kit ship-from address',
				what: 'Fulfilment address mailing kits ship from (the sender on the kit leg), and the contact phone carriers print on any label whose address has none. The kit card stays paused until this is a valid US address.',
				how: 'Your packing location, with a phone number carriers can call.',
				format: 'One JSON object: name, line1, optional line2, city, region (two capital letters), postal_code, optional phone. Example: {"name":"Example Co","line1":"1 Example Way","city":"Seattle","region":"WA","postal_code":"98101","phone":"+13605550100"}',
				default: 'unset (kit paused)',
				applies: 'live'
			}
		]
	},
	{
		id: 'fedex',
		title: 'FedEx API',
		summary:
			'OAuth credentials and shipper account for live FedEx label buys: staff buys (/apostille/api/v1/fedex-live/ on the admin host) and customer-paid label orders. labels.fedex_base_url picks sandbox or production; customer orders only buy when it matches the Stripe mode (test key with sandbox, live key with production).',
		setup: [
			'https://developer.fedex.com > sign in with your fedex.com user > My Projects > Create API Project. Select the Ship API and the Rate API (the code calls /ship/v1/shipments, /ship/v1/shipments/cancel and /rate/v1/rates/quotes). (verify menu names)',
			'Start in sandbox: the project test key gives an API Key and a Secret Key. Set labels.fedex_base_url to https://apis-sandbox.fedex.com, because an unset base URL means production.',
			'Link your FedEx shipping account to the project and save its number in labels.fedex_account.',
			'For production, request production keys in the project (FedEx may require label certification first, verify), replace both secrets and set the base URL to https://apis.fedex.com.',
			'Restart, then check caps and quotes on the admin Labels page.'
		],
		keys: [
			{
				key: 'labels.fedex_client_id',
				type: 'secret',
				label: 'FedEx API key',
				what: 'Sent as client_id to {base}/oauth/token for a bearer token.',
				how: 'Setup step 2: API Key.',
				format: 'Opaque string. Write-only.',
				applies: 'boot'
			},
			{
				key: 'labels.fedex_client_secret',
				type: 'secret',
				label: 'FedEx secret key',
				what: 'Client secret for the same token request.',
				how: 'Setup step 2: Secret Key.',
				format: 'Opaque string. Write-only.',
				applies: 'boot'
			},
			{
				key: 'labels.fedex_account',
				type: 'text',
				label: 'FedEx account number',
				what: 'Shipper account billed for live labels.',
				how: 'Setup step 3.',
				format: '9 digits. Example: 123456789',
				default: 'unset',
				applies: 'boot'
			},
			{
				key: 'labels.fedex_base_url',
				type: 'text',
				label: 'FedEx API root',
				what: 'Which FedEx environment credentials go to. Only the two FedEx roots are honoured; unset or any other value means production.',
				how: 'Sandbox while testing, production after go-live.',
				format: 'https://apis-sandbox.fedex.com or https://apis.fedex.com',
				default: 'https://apis.fedex.com',
				applies: 'live'
			}
		]
	},
	{
		id: 'labels',
		title: 'Label limits & tracking',
		summary:
			'Money guards for real label purchases. Staff live FedEx buys need labels.live plus the caps, the ZIP allowlist and the FedEx API keys. Customer-paid labels (labels.customer_*) are bought automatically after payment, each leg once, only to that order\'s own two addresses; test-mode payments only buy sandbox or TEM postage. Any missing piece fails closed.',
		setup: [
			'Save the FedEx API keys (and the USPS group for USPS labels) and run quotes in sandbox or TEM.',
			'Staff buys: list the destination ZIP codes in labels.live_ship_to_allow, set labels.live_daily_cap and labels.live_max_usd, then turn on labels.live and restart.',
			'Customer buys: set labels.customer_daily_cap and labels.customer_max_usd, then turn on labels.customer_live. With a Stripe test key, buy one label order end to end and check both PDFs in the cabinet.',
			'Turn on tracking.poll_enabled so bought labels report their scans to the order timeline.'
		],
		keys: [
			{
				key: 'labels.live',
				type: 'bool',
				label: 'Live FedEx buys',
				what: 'Master switch for staff live FedEx purchases. Off: purchases answer 503.',
				how: 'Turn on after every other row in this group and the FedEx group is set.',
				format: 'On or off.',
				default: 'off',
				applies: 'boot'
			},
			{
				key: 'labels.live_daily_cap',
				type: 'text',
				label: 'Live buys per day',
				what: 'Max live label buys per UTC day.',
				how: 'Start at 2.',
				format: 'Integer 1-10. Example: 2',
				default: '2',
				applies: 'boot'
			},
			{
				key: 'labels.live_max_usd',
				type: 'text',
				label: 'Max cost per label',
				what: 'A quote above this amount is refused.',
				how: 'Your overnight rate plus a margin.',
				format: 'US dollars 1-10000, decimals allowed. Example: 60 or 42.50',
				default: '60',
				applies: 'boot'
			},
			{
				key: 'labels.live_ship_to_allow',
				type: 'text',
				label: 'Destination ZIP allowlist',
				what: 'Destination postal codes allowed for live buys. Matching is exact after trimming, so list a ZIP+4 as written.',
				how: 'The ZIP codes of the offices you mail to.',
				format: 'Comma-separated ZIP codes. Example: 98101, 98004',
				default: 'empty (nothing buyable)',
				applies: 'boot'
			},
			{
				key: 'labels.customer_live',
				type: 'bool',
				label: 'Customer-paid labels',
				what: 'When on, the FedEx and USPS label cards quote both legs at the carrier and, after payment, the label buyer purchases them (a carrier refusal refunds in full; anything uncertain goes to staff). Off: those cards are paused. Separate from the staff switch labels.live.',
				how: 'Turn on after one sandbox or TEM order worked end to end.',
				format: 'On or off.',
				default: 'off',
				applies: 'live'
			},
			{
				key: 'labels.customer_daily_cap',
				type: 'text',
				label: 'Customer label buys per day',
				what: 'Customer label buys per UTC day; each leg counts, and so does any buy that may have happened. Reached: new label quotes pause and paid orders wait for staff.',
				how: 'Your daily label budget.',
				format: 'Integer 0 to 1000. Example: 10. 0 stops customer buys.',
				default: '10',
				applies: 'live'
			},
			{
				key: 'labels.customer_max_usd',
				type: 'text',
				label: 'Max cost per customer label',
				what: 'Per-label ceiling for customer labels: a leg quoted above it pauses that route, and a paid leg above it goes to staff instead of being bought.',
				how: 'Your most expensive expected route plus a margin.',
				format: 'US dollars 1 to 10000, decimals allowed. Example: 60 or 42.50',
				default: '60',
				applies: 'live'
			},
			{
				key: 'tracking.poll_enabled',
				type: 'bool',
				label: 'Carrier tracking poller',
				what: 'Every 5 minutes checks up to 20 bought labels with USPS Tracking v3 and FedEx Track (each number at most hourly, never again once delivered). New scans land on the order timeline and move it to in transit, then delivered when the return leg arrives.',
				how: 'Turn on once the carrier keys are set.',
				format: 'On or off.',
				default: 'off',
				applies: 'live'
			}
		]
	},
	{
		id: 'usps',
		title: 'USPS',
		summary:
			'USPS APIs v3: an OAuth client, payment authorization from your CRID, Mailer IDs and Enterprise Payment Account, then Labels, Prices and Tracking. usps.base_url picks the test environment (TEM) or production; usps.certified lets the server call the label API at all; usps.live allows production postage, for paid live-mode orders only, under its own daily and per-label caps. The unpaid /shipping prepaid labels never leave the test environment.',
		setup: [
			'Register at https://developers.usps.com and create an app: its Consumer Key and Consumer Secret are usps.client_id and usps.client_secret. Request Labels API access (it needs separate approval and USPS Ship enrollment). (verify menu names)',
			'In the Customer Onboarding Portal or Business Customer Gateway (https://gateway.usps.com) find your CRID, Mailer ID, manifest Mailer ID and Enterprise Payment Account number. (verify)',
			'Keep usps.base_url on https://apis-tem.usps.com, save the keys, turn on usps.certified and restart. With a Stripe test key and labels.customer_live on, buy one USPS label order end to end.',
			'For production postage: set usps.base_url to https://apis.usps.com and restart, set usps.live_daily_cap and usps.live_max_usd, then turn on usps.live. Only live-mode (paid) orders buy there.'
		],
		keys: [
			{
				key: 'usps.base_url',
				type: 'text',
				label: 'USPS API root',
				what: 'Root for every USPS call. The test environment buys test postage for test-mode orders and the /shipping prepaid labels; production buys real postage, only with usps.live on and a live Stripe key.',
				how: 'TEM while testing, production after go-live.',
				format: 'https://apis-tem.usps.com or https://apis.usps.com',
				default: 'https://apis-tem.usps.com',
				applies: 'boot'
			},
			{
				key: 'usps.certified',
				type: 'bool',
				label: 'USPS API certified',
				what: 'Off: USPS label requests queue for manual handling (carrier api not certified yet). On: the server buys through the API.',
				how: 'Turn on after USPS certifies the integration.',
				format: 'On or off.',
				default: 'off',
				applies: 'boot'
			},
			{
				key: 'usps.client_id',
				type: 'secret',
				label: 'USPS consumer key',
				what: 'OAuth client for Labels, Payments, Prices and Tracking v3; every quote, buy and tracking check mints a fresh token with it.',
				how: 'Setup step 1.',
				format: 'Opaque string. Write-only.',
				applies: 'call'
			},
			{
				key: 'usps.client_secret',
				type: 'secret',
				label: 'USPS consumer secret',
				what: 'Secret of the same OAuth client.',
				how: 'Setup step 1.',
				format: 'Opaque string. Write-only.',
				applies: 'call'
			},
			{
				key: 'usps.crid',
				type: 'text',
				label: 'Customer Registration ID (CRID)',
				what: 'Your USPS business customer ID, sent in both payment-authorization roles (payer and label owner).',
				how: 'Setup step 2.',
				format: 'Digits. Example: 12345678',
				applies: 'live'
			},
			{
				key: 'usps.mid',
				type: 'text',
				label: 'Mailer ID (MID)',
				what: 'Mailer ID for the payment-authorization roles.',
				how: 'Setup step 2.',
				format: '6 or 9 digits. Example: 123456',
				applies: 'live'
			},
			{
				key: 'usps.manifest_mid',
				type: 'text',
				label: 'Manifest Mailer ID',
				what: 'Mailer ID used on manifests in the payment-authorization roles. Empty uses usps.mid.',
				how: 'Setup step 2.',
				format: '6 or 9 digits. Example: 123456',
				default: 'usps.mid',
				applies: 'live'
			},
			{
				key: 'usps.epa_account',
				type: 'secret',
				label: 'Enterprise Payment Account',
				what: 'Account USPS postage is paid from (EPS), in the payment roles and the price searches.',
				how: 'Setup step 2.',
				format: 'Account number. Write-only.',
				applies: 'call'
			},
			{
				key: 'usps.live',
				type: 'bool',
				label: 'Live USPS postage',
				what: 'Production postage at all. On, a paid live-mode order may buy USPS postage on https://apis.usps.com (usps.base_url must point there). Off, only the test environment is ever called.',
				how: 'Turn on last, after a full TEM order.',
				format: 'On or off.',
				default: 'off',
				applies: 'live'
			},
			{
				key: 'usps.live_daily_cap',
				type: 'text',
				label: 'Live USPS labels per day',
				what: 'Live USPS labels per UTC day, on top of labels.customer_daily_cap. An order that would not fit both legs waits for staff.',
				how: 'Your daily postage budget.',
				format: 'Integer 0 to 1000. Example: 10. 0 stops live USPS postage.',
				default: '10',
				applies: 'live'
			},
			{
				key: 'usps.live_max_usd',
				type: 'text',
				label: 'Max cost per USPS label',
				what: 'Per-label ceiling for live USPS postage: a Prices v3 quote above it is refused before payment.',
				how: 'Your Priority Mail rate plus a margin.',
				format: 'US dollars 1 to 10000, decimals allowed. Example: 40',
				default: '40',
				applies: 'live'
			}
		]
	},
	{
		id: 'voice',
		title: 'Voice calls (Telnyx)',
		summary:
			'Outbound verification calls from the admin Calls page plus signed call webhooks. Dialling needs the API key, connection ID and from number; webhooks need the public key.',
		setup: [
			'https://portal.telnyx.com > API Keys > Create API Key. Copy it once into telnyx.api_key. (verify menu names)',
			'Voice > Programmable Voice > Voice API Applications (Call Control): create an application with webhook URL https://admin.andrii.ai/apostille/api/v1/calls/webhook, method POST, API v2. Copy its ID into telnyx.connection_id.',
			'Numbers > My Numbers: assign a voice-enabled number to that application and save it in E.164 as telnyx.from_number.',
			'Keys & Credentials > Public Key: paste the base64 key into telnyx.webhook_public_key. Webhooks with a bad signature or a stale timestamp are rejected. (verify menu names)',
			'The webhook path skips admin.allow_ips but still needs the admin host name. Restart and place a test call from /calls.'
		],
		keys: [
			{
				key: 'telnyx.api_key',
				type: 'secret',
				label: 'Telnyx API key',
				what: 'Bearer key for Call Control (POST /v2/calls and call actions). Without it the Calls page cannot dial.',
				how: 'Setup step 1.',
				format: 'String starting KEY. Write-only.',
				applies: 'boot'
			},
			{
				key: 'telnyx.connection_id',
				type: 'text',
				label: 'Call Control application ID',
				what: 'The application (connection) every call dials from; its webhook URL receives call events.',
				how: 'Setup step 2.',
				format: 'Numeric string. Example: 1234567890123456789',
				applies: 'boot'
			},
			{
				key: 'telnyx.from_number',
				type: 'text',
				label: 'Caller ID number',
				what: 'Number calls come from; it must belong to your account.',
				how: 'Setup step 3.',
				format: 'E.164. Example: +13605550100',
				applies: 'boot'
			},
			{
				key: 'telnyx.webhook_public_key',
				type: 'text',
				label: 'Webhook public key',
				what: 'Ed25519 key that verifies the telnyx-signature-ed25519 header over timestamp|body.',
				how: 'Setup step 4.',
				format: 'Base64 of 32 bytes (44 characters ending in =).',
				applies: 'boot'
			},
			{
				key: 'telnyx.base_url',
				type: 'text',
				label: 'Telnyx API root',
				what: 'API root for Call Control. Override for a test stub only.',
				how: 'Leave unset.',
				format: 'https URL, no trailing slash.',
				default: 'https://api.telnyx.com',
				applies: 'boot'
			}
		]
	},
	{
		id: 'pipeline',
		title: 'AI pipeline automation',
		summary:
			'Automation for the Meta pipeline. Auto-compose drafts a page-plan proposal after accepts; a Compile run (Dashboard, Compile an authority) walks capture, draft, cross-check, offices and plan. Nothing is accepted automatically: every output lands in Review as a proposal.',
		setup: [
			'Run one Compile from the dashboard and review what it proposed.',
			'Turn on plan.auto_compose for a fresh plan proposal after each accept batch (about two model calls per run).',
			'Set compile.daily_runs to your budget; meta.daily_call_cap still caps every model call.'
		],
		keys: [
			{
				key: 'plan.auto_compose',
				type: 'bool',
				label: 'Auto-compose page plans',
				what: 'After a single or bulk accept, compose a fresh page-plan proposal for that authority. One run per authority at a time; accepts during a run fold into one follow-up. Skipped (logged) below 3 accepted facts or at the model-call cap.',
				how: 'Turn on when reviewers want a plan after every approval batch.',
				format: 'On or off.',
				default: 'off',
				applies: 'live'
			},
			{
				key: 'compile.daily_runs',
				type: 'text',
				label: 'Compile runs per day',
				what: 'Compile runs per UTC day across all authorities; one run per authority at a time. Reached: the Compile button answers with the cap. 0 stops every run.',
				how: 'Your daily pipeline budget.',
				format: 'Integer 0 or more. Example: 6',
				default: '6',
				applies: 'live'
			}
		]
	}
];

/** Prefix routes for keys the guide does not list (longest prefix wins). */
const PREFIX_GROUP: [string, string][] = [
	['labels.fedex_', 'fedex'],
	['meta.client_', 'login'],
	['net.', 'net'],
	['quic.', 'net'],
	['paths.', 'paths'],
	['acme.', 'tls'],
	['tls.', 'tls'],
	['admin.', 'admin'],
	['auth.', 'admin'],
	['google.', 'login'],
	['apple.', 'login'],
	['meta.', 'meta-model'],
	['media.', 'meta-model'],
	['mailer.', 'mailer'],
	['mail.', 'gmail'],
	['notify.', 'notify'],
	['stripe.', 'stripe'],
	['orders.', 'orders'],
	['labels.', 'labels'],
	['tracking.', 'labels'],
	['usps.', 'usps'],
	['telnyx.', 'voice'],
	['plan.', 'pipeline'],
	['compile.', 'pipeline'],
	['setup.', 'setup']
];

/** Group for keys no prefix claims. */
export const OTHER_GROUP: GuideGroup = {
	id: 'other',
	title: 'Other keys',
	summary: 'Keys the server holds that this guide does not describe. They save and audit like any other key.',
	setup: [
		'Read the doc comment for the key in crates/config/src/lib.rs (keys module) before changing it.',
		'Most runtime keys are read at boot: restart the service after saving.'
	],
	keys: []
};

const FORMAT_BY_TYPE: Record<ConfigType, string> = {
	bool: 'On or off.',
	text: 'UTF-8 text up to 4 KiB. An empty value means unset for optional keys.',
	secret: 'Write-only, 1 byte to 8 KiB. Saving overwrites; the server never shows it back.'
};

const BY_KEY = new Map<string, { group: GuideGroup; entry: GuideKey }>();
for (const group of CONFIG_GUIDE) {
	for (const entry of group.keys) BY_KEY.set(entry.key, { group, entry });
}

/** Group a key belongs to: listed key, else prefix, else Other. */
export function groupForKey(key: string): GuideGroup {
	const listed = BY_KEY.get(key);
	if (listed) return listed.group;
	let best: [string, string] | undefined;
	for (const pair of PREFIX_GROUP) {
		if (key.startsWith(pair[0]) && (!best || pair[0].length > best[0].length)) best = pair;
	}
	return CONFIG_GUIDE.find((g) => g.id === best?.[1]) ?? OTHER_GROUP;
}

/** Guide entry for a key; unknown keys get a generic entry of the server type. */
export function guideEntry(
	key: string,
	type: ConfigType = 'text'
): { group: GuideGroup; entry: GuideKey; known: boolean } {
	const listed = BY_KEY.get(key);
	if (listed) return { ...listed, known: true };
	return {
		group: groupForKey(key),
		known: false,
		entry: {
			key,
			type,
			label: key,
			what: 'Registered on the server; this guide has no description for it.',
			how: 'See the doc comment for this key in crates/config/src/lib.rs.',
			format: FORMAT_BY_TYPE[type],
			applies: key.startsWith('setup.') ? 'locked' : 'boot',
			readOnly: key.startsWith('setup.')
		}
	};
}

/** Every listed key in guide order (tests compare this to keys::ALL). */
export function guideKeys(): GuideKey[] {
	return CONFIG_GUIDE.flatMap((g) => g.keys);
}

/** One run of guide text: plain, a vendor console link, code, or a key. */
export interface RichToken {
	kind: 'text' | 'link' | 'code' | 'key';
	text: string;
	href?: string;
}

// Vendor consoles and docs open as links; API roots, our own hosts
// and templated URLs are values to paste, so they render as code.
const CONSOLE_HOST =
	/^(?:dashboard|console|developer|developers|portal|myaccount|gateway|dev)\.|^cloud\.google\.com$/;
const RICH =
	/(https?:\/\/[^\s,;"'<>()]+)|((?<![\w.:/])\/[\w.{}<>*/-]*[a-z][\w.{}<>*/-]*)|(\b[a-z][a-z0-9]*\.[a-z][a-z0-9_]*\b)/g;

/** Split guide text into runs the page renders (pure; tested). */
export function richText(text: string): RichToken[] {
	const out: RichToken[] = [];
	const push = (kind: RichToken['kind'], s: string, href?: string) => {
		if (!s) return;
		const last = out[out.length - 1];
		if (kind === 'text' && last?.kind === 'text') last.text += s;
		else out.push(href ? { kind, text: s, href } : { kind, text: s });
	};
	let at = 0;
	for (const m of text.matchAll(RICH)) {
		const start = m.index ?? 0;
		let token = m[0];
		let tail = '';
		if (m[1] || m[2]) {
			const trimmed = token.replace(/[.:,]+$/, '');
			tail = token.slice(trimmed.length);
			token = trimmed;
		}
		push('text', text.slice(at, start));
		if (m[1]) {
			let host = '';
			try {
				host = new URL(token).hostname;
			} catch {
				host = '';
			}
			if (host && !token.includes('{') && CONSOLE_HOST.test(host)) push('link', token, token);
			else push('code', token);
		} else if (m[2]) {
			push('code', token);
		} else if (BY_KEY.has(token)) {
			push('key', token, `#key-${token}`);
		} else {
			push('text', token);
		}
		push('text', tail);
		at = start + m[0].length;
	}
	push('text', text.slice(at));
	return out;
}
