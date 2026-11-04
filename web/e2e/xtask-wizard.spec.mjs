// `cargo xtask setup wizard` page (xtask/src/setup/wizard.html), not
// an SPA build: the page is the real single file, served with the
// API route-mocked on a fake origin, so no server or port is needed.
// Covers the SSH box probe (W-5).
import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const PAGE = readFileSync(join(HERE, '..', '..', 'xtask', 'src', 'setup', 'wizard.html'), 'utf8')
	.replace('__CSRF__', 'csrf-test');
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

function state() {
	return {
		path: '/synthetic/private/config.yaml',
		operator: 'Synthetic Operator',
		default_targets: ['local'],
		keys: [],
		shared: {},
		targets: {
			local: target('local'),
			staging: target('plant', { ssh: 'plant' })
		},
		checks: [],
		phases: ['doctor'],
		brand: [],
		brand_dir: '/synthetic/private/brand',
		problems: [],
		running: false
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
