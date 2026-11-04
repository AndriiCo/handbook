// `cargo xtask setup wizard` page (xtask/src/setup/wizard.html), not
// an SPA build: the page is the real single file, served with the
// API route-mocked on a fake origin, so no server or port is needed.
// Covers the SSH box probe (W-5) and the system map (W-4, W-6). The
// map JSON is fixtures/wizard-map.json, which a Rust test pins to what
// the wizard serves (setup::map::tests).
import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const PAGE = readFileSync(join(HERE, '..', '..', 'xtask', 'src', 'setup', 'wizard.html'), 'utf8')
	.replace('__CSRF__', 'csrf-test');
const MAPS = JSON.parse(readFileSync(join(HERE, 'fixtures', 'wizard-map.json'), 'utf8'));
const ORIGIN = 'http://wizard.test';

const PROBE = {
	via: 'plant',
	at: '2026-11-04T09:00:00Z',
	hostname: 'plant',
	user: 'andrii',
	os: 'Ubuntu 24.04.1 LTS',
	kernel: 'Linux 6.8.0-45-generic x86_64',
	arch: 'x86_64',
	cpus: 16,
	ram_mb: 64238,
	disk_free_mb: 890962,
	gpus: ['GPU 0: NVIDIA GeForce RTX 3060'],
	docker: 'Docker version 27.3.1, build ce12230'
};

function target(kind, extra = {}) {
	return {
		kind,
		ssh: null,
		ssh_admin: null,
		public_host: null,
		bind: null,
		bind6: null,
		dns: [],
		timers: [],
		platform: null,
		deploy_keys: [],
		probe: null,
		config: {},
		derived: {},
		...extra
	};
}

/** The operator's file: production beside plant (the template). */
function state() {
	return {
		path: '/synthetic/private/config.yaml',
		operator: 'Synthetic Operator',
		default_targets: ['local'],
		keys: [],
		shared: {},
		targets: {
			docker: target('docker', { public_host: 'rehearsal.localhost' }),
			local: target('local'),
			production: target('server', { ssh: 'andrii.ai', public_host: 'andrii.ai' }),
			staging: target('plant', { ssh: 'plant' })
		},
		checks: [],
		phases: ['doctor'],
		brand: [],
		brand_dir: '/synthetic/private/brand',
		problems: [],
		running: false,
		map: structuredClone(MAPS.operator)
	};
}

/** A judge's plain local file, running in Docker. */
function judgeState() {
	return {
		...state(),
		path: '/synthetic/.local/setup.yaml',
		targets: { local: target('local', { runtime: 'docker' }) },
		map: structuredClone(MAPS.judge)
	};
}

/** Serve the page and a mutable state; returns the posted bodies. */
async function mount(page, s, routes = {}) {
	const posts = [];
	await page.route(`${ORIGIN}/**`, async (route) => {
		const url = new URL(route.request().url());
		const method = route.request().method();
		if (url.pathname === '/') return route.fulfill({ contentType: 'text/html', body: PAGE });
		if (url.pathname === '/api/events')
			return route.fulfill({ contentType: 'text/event-stream', body: '' });
		if (url.pathname === '/api/state') return route.fulfill({ json: s });
		if (method === 'POST') {
			const body = route.request().postDataJSON();
			posts.push({ path: url.pathname, body, csrf: await route.request().headerValue('x-setup-csrf') });
			const handler = routes[url.pathname];
			if (handler) return handler(route, body);
			return route.fulfill({ json: { saved: true } });
		}
		return route.fulfill({ status: 404, json: { error: 'not mocked' } });
	});
	await page.goto(`${ORIGIN}/`);
	return posts;
}

test('a plant target probes its box over SSH and shows what it found', async ({ page }) => {
	const s = state();
	const posts = await mount(page, s, {
		'/api/probe': (route, body) => {
			s.targets.staging.probe = { ...PROBE, via: body.host };
			return route.fulfill({ json: { probe: s.targets.staging.probe } });
		}
	});
	await page.getByRole('tab', { name: 'staging' }).click();
	const host = page.getByLabel('SSH host or alias');
	await expect(host).toHaveValue('plant');
	await page.getByRole('button', { name: 'Probe over SSH' }).click();
	await expect(page.getByRole('cell', { name: 'Ubuntu 24.04.1 LTS' })).toBeVisible();
	await expect(page.getByRole('cell', { name: 'GPU 0: NVIDIA GeForce RTX 3060' })).toBeVisible();
	await expect(page.getByRole('cell', { name: '62.7 GB' })).toBeVisible();
	expect(posts).toEqual([{ path: '/api/probe', body: { target: 'staging', host: 'plant' }, csrf: 'csrf-test' }]);
});

test('an unknown host key shows ssh’s words and how to accept the key', async ({ page }) => {
	const s = state();
	const said =
		'No ED25519 host key is known for plant and you have requested strict checking.\nHost key verification failed.\nrun `ssh plant` once in a terminal to accept its key';
	await mount(page, s, {
		'/api/probe': (route) => route.fulfill({ status: 502, json: { error: said } })
	});
	await page.getByRole('tab', { name: 'staging' }).click();
	await page.getByRole('button', { name: 'Probe over SSH' }).click();
	await expect(page.locator('pre.fail')).toHaveText(said);
	await expect(page.getByRole('button', { name: 'Probe over SSH' })).toBeEnabled();
});

test('the system map draws the operator’s deployment: hosts, links, models, modes', async ({ page }) => {
	await mount(page, state());
	await expect(page.getByRole('tab', { name: 'System map' })).toHaveAttribute('aria-selected', 'true');
	const fe = page.locator('#map-front_end');
	await expect(fe).toContainText('«node» Server');
	await expect(fe).toContainText('production · andrii.ai');
	await expect(fe.locator('a', { hasText: 'Open the public site' })).toHaveAttribute('href', 'https://andrii.ai/apostille');
	await expect(fe.locator('a', { hasText: 'Open the admin console' })).toHaveAttribute('href', 'https://admin.andrii.ai');
	const workers = page.locator('#map-workers');
	await expect(workers.locator('a', { hasText: 'Work queue' })).toHaveAttribute('href', 'https://admin.andrii.ai/work');
	await expect(workers).toContainText('andrii@plant');
	await expect(workers).toContainText('Ubuntu 24.04.1 LTS');
	await expect(workers).toContainText('x86_64 · 16 CPUs · 62.7 GB RAM');
	const staging = page.locator('#map-staging');
	await expect(staging.locator('a', { hasText: 'Staging' })).toHaveAttribute('href', 'http://localhost:18318/apostille');
	await expect(staging).toContainText('ssh -L 18318:127.0.0.1:18318 plant');
	// Paths and dependencies.
	await expect(page.locator('.path-https')).toContainText('HTTPS');
	await expect(page.locator('.path-wss')).toContainText('WSS work queue');
	await expect(page.locator('.path-ssh')).toHaveCount(2);
	await expect(page.locator('.use-meta').first()).toContainText('muse-wonder-1.0');
	await expect(page.locator('.use-meta line').first()).toHaveAttribute('marker-end', 'url(#open)');
	await expect(page.locator('.deploy-weights')).toContainText('«deploy»');
	await expect(page.locator('#map-weights')).toContainText('→ plant:');
	await expect(page.locator('#map-weights')).toContainText('/srv/scratch/apostille/models');
	await expect(workers).toContainText('xtask worker on plant');
	await expect(page.locator('#map-meta')).toContainText('key set (target production)');
	await expect(page.locator('#map-svc-stripe')).toContainText('Stripe · sandbox');
	await expect(page.locator('#map-svc-usps')).toContainText('USPS · live');
	await expect(page.locator('#map-svc-gmail')).toContainText('Gmail · not set');
	await expect(page.locator('#map-svc-stripe a')).toHaveAttribute('href', 'https://dashboard.stripe.com/');
	// The page never holds a secret.
	expect(await page.content()).not.toContain('LLM_1');
});

test('map controls post the placement, test Meta through the wizard, save the weights repo', async ({ page }) => {
	const s = state();
	const posts = await mount(page, s, {
		'/api/meta': (route) =>
			route.fulfill({ json: { models: ['muse-spark-1.3', 'muse-wonder-1.0', 'muse-voice-transcribe-1.0'] } }),
		// What the wizard would answer next, so each refresh shows the change.
		'/api/map': (route, body) => {
			if (body.node) Object.assign(s.map.nodes[body.node], { target: 'local', kind: 'local', place: body.place });
			if ('weights_repo' in body) s.map.weights.repo = body.weights_repo;
			return route.fulfill({ json: { saved: true, problems: [] } });
		}
	});
	await page.getByRole('button', { name: 'Test' }).click();
	await expect(page.getByText('The key works: 3 models.')).toBeVisible();
	const front = page.getByRole('group', { name: 'Front end: Local or Server' });
	await expect(front.getByRole('button', { name: 'Server' })).toHaveAttribute('aria-pressed', 'true');
	await front.getByRole('button', { name: 'Local' }).click();
	await expect(front.getByRole('button', { name: 'Local' })).toHaveAttribute('aria-pressed', 'true');
	await expect(page.locator('#map-front_end')).toContainText('«node» Local · Native');
	const repo = page.getByLabel('Repo the workers serve');
	await repo.fill('synthetic-org/glimmer-gguf');
	await repo.blur();
	await expect(page.locator('#map-weights a')).toHaveAttribute('href', 'https://huggingface.co/synthetic-org/glimmer-gguf');
	expect(posts.map((p) => [p.path, p.body])).toEqual([
		['/api/meta', { target: 'production' }],
		['/api/map', { node: 'front_end', place: 'native' }],
		['/api/map', { weights_repo: 'synthetic-org/glimmer-gguf' }]
	]);
	// A server node probes its box from the map, too.
	await expect(page.locator('#probe-workers-staging')).toHaveValue('plant');
});

test('a judge’s local Docker map fits a 390 px phone in dark mode', async ({ page }) => {
	await page.setViewportSize({ width: 390, height: 844 });
	await page.emulateMedia({ colorScheme: 'dark' });
	const posts = await mount(page, judgeState());
	const fe = page.locator('#map-front_end');
	await expect(fe).toContainText('«node» Local · Docker');
	await expect(fe.locator('a', { hasText: 'Open the public site' })).toHaveAttribute('href', 'http://localhost:18318/apostille');
	await expect(page.locator('#map-workers a', { hasText: 'Work queue' })).toHaveAttribute('href', 'http://localhost:18319/work');
	await expect(page.locator('.path-https')).toContainText('HTTP · loopback');
	await expect(page.locator('.path-ssh')).toHaveCount(0);
	await expect(page.locator('#map-staging')).toContainText('not set up');
	// A local-only file starts on the fakes (file::LOCAL_DEFAULTS).
	await expect(page.locator('#map-svc-fedex')).toContainText('FedEx · fake');
	await expect(page.locator('#map-svc-stripe')).toContainText('Stripe · fake');
	// The intro says what needs a host on the internet.
	await expect(page.getByRole('region', { name: 'What runs where' })).toContainText('Customer passkeys');
	const staging = page.getByRole('group', { name: 'Staging: Local or Server' });
	await expect(staging.getByRole('button', { name: 'Server' })).toBeDisabled();
	await expect(page.getByText(/needs the sops file/).first()).toBeVisible();
	// The page never scrolls sideways; the map box does.
	const widths = await page.evaluate(() => ({
		page: document.scrollingElement.scrollWidth,
		view: window.innerWidth,
		box: document.querySelector('.map-box').scrollWidth,
		boxView: document.querySelector('.map-box').clientWidth
	}));
	expect(widths.page).toBeLessThanOrEqual(widths.view);
	expect(widths.box).toBeGreaterThan(widths.boxView);
	// Dark mode reaches the drawing.
	const fill = await page.locator('#map-front_end rect').evaluate((r) => getComputedStyle(r).fill);
	expect(fill).toBe('rgb(23, 27, 33)');
	// Docker | Native on the front end (last: the answer re-renders the map).
	const runtime = page.getByRole('group', { name: 'Front end: Docker or Native' });
	await expect(runtime.getByRole('button', { name: 'Docker' })).toHaveAttribute('aria-pressed', 'true');
	await runtime.getByRole('button', { name: 'Native' }).click();
	await expect.poll(() => posts.map((p) => p.body)).toEqual([{ node: 'front_end', place: 'native' }]);
});

const GOOGLE_ZIP = 'https://developers.google.com/static/identity/images/signin-assets.zip';
const GOOGLE_PICK = 'Theme=Neutral, Show text=No, Shape=Square, Platform=Android+Web.svg';

function brandState() {
	const asset = (id, owner, file, source) => ({
		id,
		owner,
		terms: `${owner} terms`,
		terms_url: `https://${id}.example/terms`,
		permits: 'the logo in its sign-in button',
		get: `https://${id}.example/download (by hand)`,
		files: [{ name: file, present: false, licensed: false, source }],
		accepted: null
	});
	return {
		...state(),
		brand: [
			asset('google', 'Google', 'google.svg', { url: GOOGLE_ZIP, pick: GOOGLE_PICK }),
			asset('apple', 'Apple', 'apple.svg', null)
		]
	};
}

test('brand Fetch waits for acceptance, says where to download by hand, then licenses the file', async ({ page }) => {
	const s = brandState();
	let fetches = 0;
	const posts = await mount(page, s, {
		'/api/licence': (route, body) => {
			s.brand.find((a) => a.id === body.asset).accepted = {
				by: 'Synthetic Operator',
				at: '2026-11-04T10:00:00Z',
				version: body.terms_version
			};
			return route.fulfill({ json: { saved: true } });
		},
		'/api/brand-fetch': (route) => {
			fetches += 1;
			if (fetches === 1)
				return route.fulfill({
					status: 502,
					json: { error: 'refused google.svg: it carries a <foreignObject>' }
				});
			Object.assign(s.brand[0].files[0], { present: true, licensed: true });
			return route.fulfill({ json: { saved: 'google.svg', sha256: 'ab'.repeat(32) } });
		}
	});
	await page.getByRole('tab', { name: 'Brand & licences' }).click();
	const fetchGoogle = page.getByRole('button', { name: 'Fetch google.svg' });
	await expect(page.getByLabel('Source URL').first()).toHaveValue(GOOGLE_ZIP);
	await expect(fetchGoogle).toBeDisabled();
	await expect(page.getByRole('button', { name: 'Fetch apple.svg' })).toBeDisabled();
	await expect(page.getByText('No direct download is known').first()).toBeVisible();

	await page.locator('#acc-google').check();
	await expect(fetchGoogle).toBeEnabled();
	await fetchGoogle.click();
	await expect(page.getByText('refused google.svg: it carries a <foreignObject>')).toBeVisible();
	await expect(page.getByText('Download it by hand instead: https://google.example/download (by hand)')).toBeVisible();

	await fetchGoogle.click();
	await expect(page.locator('.badge.ok', { hasText: 'licensed' })).toBeVisible();
	expect(posts.map((p) => [p.path, p.body])).toEqual([
		['/api/licence', { asset: 'google', terms_version: '', accept: true }],
		['/api/brand-fetch', { file: 'google.svg', url: GOOGLE_ZIP, pick: GOOGLE_PICK }],
		['/api/brand-fetch', { file: 'google.svg', url: GOOGLE_ZIP, pick: GOOGLE_PICK }]
	]);
});
