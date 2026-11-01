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
	'/': { name: 'gp1-home', ready: 'Apostille rules' },
	'/authority/vietnam': { name: 'gp2-dossier', ready: 'Authority dossier' },
	'/authority/usa-wa-sos': { name: 'gp2-dossier-wa', ready: 'Authority dossier' },
	'/authority/hcch': { name: 'gp2-dossier-hcch', ready: 'Authority dossier' },
	'/login': { name: 'gp3-login', ready: 'Log in' },
	'/order': { name: 'gp4-order', ready: 'Destination' },
	'/orders': { name: 'gp4-orders', ready: 'newest first' },
	'/precheck': { name: 'a2-precheck', ready: 'Document precheck' },
	'/notify/confirm': { name: 'notify-confirm', ready: 'invalid or expired' },
	'/notify/unsubscribe': { name: 'notify-unsubscribe', ready: 'Unsubscribe from updates' }
};
const SIZES = [
	{ w: 390, h: 844 },
	{ w: 1440, h: 900 }
];
const THEMES = ['light', 'dark'];
const BANNED = ['placeholder', 'not wired', 'skeleton', 'lorem', 'todo'];
// Routes whose redesign slice has landed (each screen gets one graphic
// drawn from its own data). Extend as slices 2–5 land.
const GRAPHIC_ROUTES = ['/', '/authority/vietnam', '/authority/usa-wa-sos', '/authority/hcch', '/order', '/precheck', '/login'];
// Routes still allowed a native <select>: none — slice 4 replaced the
// order destination select with a combobox.
const NATIVE_SELECT_OK = [];

const wanted = process.argv.slice(2);
const routes = Object.keys(ROUTES).filter((r) => wanted.length === 0 || wanted.includes(r));
if (routes.length === 0) {
	console.error(`shots: unknown route(s): ${wanted.join(' ')} (known: ${Object.keys(ROUTES).join(' ')})`);
	process.exit(2);
}

const authorities = JSON.parse(readFileSync(join(root, 'e2e/fixtures/authorities.json'), 'utf8'));
const factsVietnam = JSON.parse(readFileSync(join(root, 'e2e/fixtures/facts-vietnam.json'), 'utf8'));
const factsBySlug = {
	vietnam: factsVietnam,
	'usa-wa-sos': JSON.parse(
		readFileSync(join(root, 'e2e/fixtures/facts-usa-wa-sos.json'), 'utf8')
	),
	hcch: JSON.parse(readFileSync(join(root, 'e2e/fixtures/facts-hcch.json'), 'utf8'))
};
const quality = JSON.parse(readFileSync(join(root, 'e2e/fixtures/quality.json'), 'utf8'));
const providers = JSON.parse(readFileSync(join(root, 'e2e/fixtures/providers.json'), 'utf8'));
const precheckSamples = JSON.parse(
	readFileSync(join(root, 'e2e/fixtures/precheck-samples.json'), 'utf8')
);
const p2Submissions = JSON.parse(
	readFileSync(join(root, 'e2e/fixtures/p2-submissions.json'), 'utf8')
);
const p2Forms = JSON.parse(
	readFileSync(join(root, 'e2e/fixtures/p2-forms.json'), 'utf8')
);

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
				// Any console error is fatal: the anonymous session
				// probes answer 200, so nothing expected remains.
				page.on('console', (m) => {
					if (m.type() !== 'error') return;
					errors.push(m.text());
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
						const body = factsBySlug[m[1]];
						return r.fulfill({
							status: body ? 200 : 404,
							json: body ?? { error: 'unknown authority' }
						});
					}
					if (path.endsWith('/api/v1/quality')) {
						return r.fulfill({ json: quality });
					}
					if (path.endsWith('/api/v1/notify/status')) {
						return r.fulfill({ json: { enabled: true } });
					}
					if (path.endsWith('/api/v1/auth/providers')) {
						return r.fulfill({ json: providers });
					}
					if (path.endsWith('/api/v1/demo/precheck/samples')) {
						return r.fulfill({ json: precheckSamples });
					}
					if (path.endsWith('/api/v1/p2/submissions')) {
						return r.fulfill({ json: p2Submissions });
					}
					if (path.endsWith('/api/v1/p2/forms')) {
						return r.fulfill({ json: p2Forms });
					}
					// Logged-out shell: anonymous session probes answer
					// 200 {"user": null}, like the real API — no console
					// error. Everything else unmocked is 404.
					if (
						path.endsWith('/api/v1/me') ||
						path.endsWith('/api/v1/auth/me')
					) {
						return r.fulfill({ json: { user: null } });
					}
					return r.fulfill({
						status: 404,
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
				// Brand/graphic audit (redesign brief): one evaluate per
				// capture, each violation is a failure line.
				const audit = await page.evaluate(({ w, theme, wantGraphic, selectOk }) => {
					const out = [];
					for (const b of document.querySelectorAll('button')) {
						if (b.scrollWidth - b.clientWidth > 1)
							out.push(`clipped button ${JSON.stringify(b.textContent.trim().slice(0, 24))}`);
					}
					if (!selectOk && document.querySelector('select'))
						out.push('native <select> present');
					const h1s = [...document.querySelectorAll('h1')];
					if (h1s.length !== 1) out.push(`${h1s.length} h1 elements (want 1)`);
					else if (!h1s[0].hasAttribute('data-heading'))
						out.push('h1 not built with the heading component');
					// Layout rhythm checks apply to redesigned routes only;
					// slices 3–5 bring the remaining pages up.
					const pad = w >= 1440 ? 64 : 40;
					if (wantGraphic) {
						// Major blocks only: page headers, topic sections and
						// home sections — not cards nested in asides.
						for (const s of document.querySelectorAll(
							'main > section, main [data-topic], main > div > section'
						)) {
							const cs = getComputedStyle(s);
							if (parseFloat(cs.paddingTop) < pad || parseFloat(cs.paddingBottom) < pad)
								out.push(`section padding below ${pad}px`);
						}
					}
					const hero = document.querySelector('#start');
					if (wantGraphic && hero && w >= 1440) {
						const cols = getComputedStyle(hero).gridTemplateColumns.split(' ').length;
						if (cols < 2) out.push('hero is a single column at 1440');
					}
					if (wantGraphic && !document.querySelector('[data-graphic]'))
						out.push('no data-graphic element');
					for (const el of document.querySelectorAll('body *')) {
						const cs = getComputedStyle(el);
						const vogun = cs.fontFamily.startsWith('Vogun');
						if (vogun && parseFloat(cs.fontSize) < 24)
							out.push(`Vogun below 24px: ${el.tagName} ${cs.fontSize}`);
						if (
							vogun &&
							el.closest(':lang(vi), :lang(uk), :lang(ru)') &&
							el.textContent.trim()
						)
							out.push(`Vogun under lang: ${JSON.stringify(el.textContent.trim().slice(0, 24))}`);
						if (
							theme === 'dark' &&
							!['IMG', 'SVG', 'PATH', 'CIRCLE', 'RECT', 'LINE', 'TEXT'].includes(el.tagName) &&
							cs.backgroundColor === 'rgb(255, 255, 255)'
						)
							out.push(`white background in dark mode: ${el.tagName}`);
					}
					const h1 = document.querySelector('h1');
					const h1Height = h1 ? h1.getBoundingClientRect().height : 0;
					return { out, h1Height };
				},
				{
					w,
					theme,
					wantGraphic: GRAPHIC_ROUTES.includes(route),
					selectOk: NATIVE_SELECT_OK.includes(route)
				});
				for (const line of audit.out.slice(0, 6)) failures.push(`${file}: ${line}`);
				shots.push({ file, route, w, h, theme, h1Height: Math.round(audit.h1Height) });

				const axe = await new AxeBuilder({ page })
					.withTags(['wcag2a', 'wcag2aa'])
					.analyze();
				for (const v of axe.violations) {
					if (v.impact === 'serious' || v.impact === 'critical') {
						failures.push(`${file}: axe ${v.impact} ${v.id} (${v.nodes.length} node(s))`);
					}
				}
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
writeFileSync(join(shotsDir, 'meta.json'), JSON.stringify(shots, null, 1));
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
