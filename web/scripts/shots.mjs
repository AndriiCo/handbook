// shots.mjs — visual gate (AGENTS.md §4). Serves the fresh public
// `build/` over 127.0.0.1 (port 0), intercepts /api/v1 with
// e2e/fixtures, and captures each route at 390×844 + 1440×900 in
// light + dark. Fails on: console errors, axe serious/critical,
// horizontal scroll at 390, banned strings in rendered text.
// Writes web/shots/*.png + web/shots/index.html.
//
// Usage: npm run shots -- [route ...]   (default: / /authority/vietnam /login)
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { serveDir } from '../e2e/helpers/serve.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const shotsDir = join(root, 'shots');
mkdirSync(shotsDir, { recursive: true });

const BASE = '/apostille';
const ROUTES = {
	'/': { name: 'gp1-home', ready: 'AUTHORITY INDEX' },
	'/authority/vietnam': { name: 'gp2-dossier', ready: 'Authority dossier' },
	'/login': { name: 'gp3-login', ready: 'Log in' }
};
const SIZES = [
	{ w: 390, h: 844 },
	{ w: 1440, h: 900 }
];
const THEMES = ['light', 'dark'];
const BANNED = ['placeholder', 'not wired', 'skeleton', 'lorem', 'todo'];

const wanted = process.argv.slice(2);
const routes = Object.keys(ROUTES).filter((r) => wanted.length === 0 || wanted.includes(r));
if (routes.length === 0) {
	console.error(`shots: unknown route(s): ${wanted.join(' ')} (known: ${Object.keys(ROUTES).join(' ')})`);
	process.exit(2);
}

const authorities = JSON.parse(readFileSync(join(root, 'e2e/fixtures/authorities.json'), 'utf8'));
const factsVietnam = JSON.parse(readFileSync(join(root, 'e2e/fixtures/facts-vietnam.json'), 'utf8'));

const failures = [];
const shots = [];

const { url, close } = await serveDir(join(root, 'build'), { base: BASE });
const browser = await chromium.launch();
try {
	for (const route of routes) {
		const { name, ready } = ROUTES[route];
		for (const { w, h } of SIZES) {
			for (const theme of THEMES) {
				const file = `${name}-${w}x${h}-${theme}.png`;
				const context = await browser.newContext({
					viewport: { width: w, height: h },
					colorScheme: theme
				});
				const page = await context.newPage();
				const errors = [];
				// Expected logged-out noise: /me + /auth/me genuinely 401
				// (see me_without_cookie_is_unauthorized + passkey::me),
				// and Chromium logs each failed resource as a console
				// error. Discount exactly those; all else is fatal.
				let expected401 = 0;
				page.on('response', (r) => {
					const u = new URL(r.request().url());
					if (
						r.status() === 401 &&
						(u.pathname.endsWith('/api/v1/me') || u.pathname.endsWith('/api/v1/auth/me'))
					) {
						expected401 += 1;
					}
				});
				page.on('console', (m) => {
					if (m.type() !== 'error') return;
					if (expected401 > 0 && m.text().includes('401')) expected401 -= 1;
					else errors.push(m.text());
				});
				page.on('pageerror', (e) => errors.push(String(e)));
				await page.addInitScript((t) => {
					try {
						localStorage.setItem('theme', t);
					} catch {}
				}, theme);
				await page.route('**/api/v1/**', async (r) => {
					const path = new URL(r.request().url()).pathname;
					if (path.endsWith('/api/v1/authorities')) {
						return r.fulfill({ json: authorities });
					}
					const m = path.match(/\/api\/v1\/authorities\/([^/]+)\/facts$/);
					if (m) {
						return r.fulfill({
							status: m[1] === 'vietnam' ? 200 : 404,
							json: m[1] === 'vietnam' ? factsVietnam : { error: 'unknown authority' }
						});
					}
					// Logged-out shell: /me + /auth/me 401, everything else 404.
					return r.fulfill({
						status: path.endsWith('/me') ? 401 : 404,
						json: { error: 'mock' }
					});
				});
				await page.goto(`${url}${BASE}${route}`, { waitUntil: 'networkidle' });
				try {
					await page.getByText(ready, { exact: false }).first().waitFor({ timeout: 8000 });
				} catch {
					failures.push(`${file}: ready text ${JSON.stringify(ready)} never appeared`);
				}
				await page.waitForTimeout(400);
				await page.screenshot({ path: join(shotsDir, file) });

				if (errors.length > 0) {
					failures.push(`${file}: ${errors.length} console error(s): ${errors[0]}`);
				}
				if (w === 390) {
					const overflow = await page.evaluate(
						() => document.documentElement.scrollWidth - document.documentElement.clientWidth
					);
					if (overflow > 1) failures.push(`${file}: horizontal overflow ${overflow}px`);
				}
				const text = ((await page.evaluate(() => document.body.innerText)) ?? '').toLowerCase();
				for (const word of BANNED) {
					if (text.includes(word)) failures.push(`${file}: banned string ${JSON.stringify(word)}`);
				}
				const axe = await new AxeBuilder({ page })
					.withTags(['wcag2a', 'wcag2aa'])
					.analyze();
				for (const v of axe.violations) {
					if (v.impact === 'serious' || v.impact === 'critical') {
						failures.push(`${file}: axe ${v.impact} ${v.id} (${v.nodes.length} node(s))`);
					}
				}
				shots.push({ file, route, w, h, theme });
				await context.close();
			}
		}
	}
} finally {
	await browser.close();
	await close();
}

const cards = shots
	.map(
		(s) =>
			`<figure><img src="${s.file}" alt="${s.file}" loading="lazy" width="360"/><figcaption>${s.route} · ${s.w}×${s.h} · ${s.theme}</figcaption></figure>`
	)
	.join('\n');
writeFileSync(
	join(shotsDir, 'index.html'),
	`<!doctype html><html lang="en"><head><meta charset="utf-8"><title>shots</title><style>body{font-family:system-ui,sans-serif;background:#111;color:#eee}main{display:flex;flex-wrap:wrap;gap:16px}figure{margin:0}figcaption{font-size:12px;margin-top:4px}</style></head><body><main>${cards}</main></body></html>`
);

if (failures.length > 0) {
	console.error(`shots FAILED (${failures.length}):`);
	for (const f of failures) console.error(`  ${f}`);
	process.exit(1);
}
console.log(`shots OK: ${shots.length} captures, 0 console errors, 0 axe serious/critical.`);
