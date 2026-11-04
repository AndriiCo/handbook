// The customer guide (R2, plan §3–§4) over the built SPA: the WA-shaped
// fixture (web/e2e/fixtures/dossier/wa-guide.json, the same file the
// presenter's node tests read), Viet Nam's production shape and an
// office nobody has checked. API reads are mocked with page.route, the
// live bus with page.routeWebSocket; the clock is pinned to Wednesday
// 2026-11-04 10:00 PT.
import { expect, test } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { serveDir } from './helpers/serve.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const fixture = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', 'dossier', `${name}.json`), 'utf8'));
const NOW = new Date('2026-11-04T18:00:00Z');
const RAW_KEY = /^\s*[a-z][a-z0-9_]*:\s/m;

/**
 * Mock one office's public reads. Returns the list of API paths read,
 * and a setter for the facts body (live updates).
 */
async function mockOffice(page, f, { authorityInPlan = true, changes = [] } = {}) {
	const reads = [];
	let facts = f.facts;
	const plan = f.plan ?? {
		slug: f.slug,
		authority: null,
		plan: null,
		facts: [],
		offices: [],
		forms: [],
		provenance: null,
		evidence: []
	};
	const row = f.authority ?? f.row;
	await page.route('**/apostille/api/v1/**', (route) => {
		const url = new URL(route.request().url());
		const path = url.pathname.replace('/apostille/api/v1', '');
		reads.push(path);
		if (path === `/authorities/${f.slug}/facts`) return route.fulfill({ json: facts });
		if (path === `/authorities/${f.slug}/plan`) {
			return route.fulfill({ json: authorityInPlan ? { ...plan, authority: plan.authority ?? row } : { ...plan, authority: null } });
		}
		if (path === '/authorities') return route.fulfill({ json: { authorities: [row] } });
		if (path === `/authorities/${f.slug}/changes`) return route.fulfill({ json: { authority: f.slug, changes } });
		if (path === '/p2/orders/options') return route.fulfill({ json: f.options ?? { options: [] } });
		if (path === '/notify/status') return route.fulfill({ json: { enabled: false } });
		if (path === '/authorities/resolve') {
			const canonical = row?.path ?? f.slug;
			return route.fulfill({
				json: {
					slug: f.slug,
					canonical,
					aliases: [],
					crumbs: canonical.includes('/')
						? [
								{ path: 'usa', label: 'United States', slug: null, location: 'usa' },
								{ path: canonical, label: row?.name ?? f.slug, slug: f.slug }
							]
						: [{ path: canonical, label: row?.name ?? f.slug, slug: f.slug }]
				}
			});
		}
		return route.fulfill({ json: {} });
	});
	return {
		reads,
		setFacts: (next) => {
			facts = next;
		}
	};
}

async function open(page, srv, f, query = '') {
	await page.goto(`${srv.url}/apostille/authority/${f.slug}${query}`);
	await expect(page.locator('h1[data-heading]')).toBeVisible();
}

test.describe('customer guide', () => {
	let srv;
	test.beforeAll(async () => {
		srv = await serveDir(join(HERE, '..', 'build'), { base: '/apostille' });
	});
	test.afterAll(async () => {
		await srv?.close();
	});

	for (const width of [390, 1440]) {
		test(`WA guide at ${width}: one h1, one fees region, traced values, no overflow`, async ({ page }) => {
			await page.setViewportSize({ width, height: 900 });
			await page.clock.install({ time: NOW });
			const errors = [];
			page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
			page.on('pageerror', (e) => errors.push(String(e)));
			const f = fixture('wa-guide');
			await mockOffice(page, f);
			await open(page, srv, f);

			await expect(page.locator('h1')).toHaveCount(1);
			await expect(page.locator('h1')).toHaveText('Get an apostille in Washington');
			await expect(page.getByRole('region', { name: /fees/i })).toHaveCount(1);
			await expect(page.getByRole('region', { name: /fees/i })).toContainText('$15.00 per document');
			// D3: the disputed box lists every variant and no label; the staff
			// decision the office's phone answer backs gives its row alone Copy.
			const conflict = page.locator('[data-conflict]');
			await expect(conflict).toContainText('PO Box 40234, Olympia, WA 98504-0234');
			await expect(conflict.locator(':scope > ul').first().locator(':scope > li')).toHaveCount(3);
			await expect(conflict.getByRole('button', { name: /Copy/ })).toHaveCount(1);
			await expect(conflict.locator('[data-recommended]').getByRole('button', { name: /Copy/ })).toHaveCount(1);
			await expect(page.locator('[data-offer="mail"]')).toHaveCount(0);
			// The courier label names its provider and live price.
			await expect(page.locator('[data-offer="courier"]')).toContainText('from $24.00');
			await expect(page.locator('[data-offer="courier"]')).toContainText('not the Secretary of State');
			// Traceable: every visible value node shows text; no raw key text.
			const { empty, text, overflow } = await page.evaluate(() => {
				const empty = [];
				for (const el of document.querySelectorAll('[data-fact-ids]')) {
					const r = el.getBoundingClientRect();
					if (r.width > 1 && r.height > 1 && !el.closest('details:not([open])') && !el.textContent.trim()) {
						empty.push(el.getAttribute('data-fact-ids'));
					}
				}
				return {
					empty,
					text: document.body.innerText,
					overflow: document.documentElement.scrollWidth - window.innerWidth
				};
			});
			expect(empty).toEqual([]);
			expect(text).not.toMatch(RAW_KEY);
			expect(text.toLowerCase()).not.toMatch(/\b(?:dossier|not wired|skeleton|lorem ipsum|coming soon)\b/);
			expect(overflow).toBeLessThanOrEqual(0);
			// Records stay closed: the details element holds them.
			await expect(page.locator('#made details')).not.toHaveAttribute('open', '');
			await expect(page.getByRole('region', { name: 'How this page was made' }).getByRole('region')).toHaveCount(0);
			// No footer inside the page; top-level bands carry the page padding.
			await expect(page.locator('main footer')).toHaveCount(0);
			const thin = await page.evaluate(
				(w) =>
					[...document.querySelectorAll('main > div > section, main > section')].filter((s) => {
						const cs = getComputedStyle(s);
						const pad = w >= 1440 ? 64 : 40;
						return parseFloat(cs.paddingTop) < pad || parseFloat(cs.paddingBottom) < pad;
					}).length,
				width
			);
			expect(thin).toBe(0);
			const axe = await new AxeBuilder({ page }).analyze();
			const bad = axe.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
			expect(bad.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
			expect(errors).toEqual([]);
		});
	}

	test('citations: off by default, Alt+C shows markers and endnotes, the first blockquote is an endnote', async ({ page }) => {
		await page.setViewportSize({ width: 1440, height: 900 });
		await page.clock.install({ time: NOW });
		const f = fixture('wa-guide');
		await mockOffice(page, f);
		await open(page, srv, f);
		const toggle = page.getByRole('button', { name: 'Citation' }).first();
		await expect(toggle).toHaveAttribute('aria-pressed', 'false');
		await expect(toggle).toHaveAttribute('aria-keyshortcuts', 'Alt+C');
		await expect(page.locator('a.cite')).toHaveCount(0);
		await expect(page.locator('blockquote')).toHaveCount(0);
		await page.locator('body').click({ position: { x: 5, y: 300 } });
		await page.keyboard.press('Alt+KeyC');
		await expect(toggle).toHaveAttribute('aria-pressed', 'true');
		await expect(page.locator('a.cite[role="doc-noteref"]').first()).toBeVisible();
		await expect(page.locator('section#sources[role="doc-endnotes"]')).toBeVisible();
		const first = page.locator('blockquote').first();
		await expect(first).toBeVisible();
		expect(await first.evaluate((el) => !!el.closest('#sources'))).toBe(true);
		// Every marker points at an endnote that exists.
		const missing = await page.evaluate(() =>
			[...document.querySelectorAll('a.cite')]
				.map((a) => a.getAttribute('href'))
				.filter((href) => !document.querySelector(href ?? '#none'))
		);
		expect(missing).toEqual([]);
		// The staff call prints its day, not a clock time.
		await expect(page.locator('#sources')).toContainText('Phone call');
		await expect(page.locator('#sources')).not.toContainText('a.m. PT');
		// With markers and endnotes on, still no serious or critical issue.
		const axe = await new AxeBuilder({ page }).analyze();
		const bad = axe.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
		expect(bad.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
		// The button turns them off again.
		await toggle.click();
		await expect(page.locator('a.cite')).toHaveCount(0);
		await expect(page.locator('#sources')).toHaveCount(0);
	});

	test('live: an accepted fact re-presents, flashes its row and is announced', async ({ page }) => {
		await page.setViewportSize({ width: 1440, height: 900 });
		await page.clock.install({ time: NOW });
		const f = fixture('wa-guide');
		const office = await mockOffice(page, f);
		let socket = null;
		await page.routeWebSocket('**/apostille/ws/bus*', (ws) => {
			socket = ws;
		});
		await open(page, srv, f);
		await expect.poll(() => socket !== null).toBe(true);
		const facts = f.facts.facts.map((x) =>
			x.id === 224 ? { ...x, statement: 'weekday__tumwater: Monday through Friday, 8:00 a.m. to 3:00 p.m.' } : x
		);
		office.setFacts({ ...f.facts, facts });
		socket.send(JSON.stringify({ type: 'fact_accepted', topic: 'authority:usa-wa-sos', fact_id: 224 }));
		await expect(page.locator('[data-item="hours-weekday-tumwater"]')).toContainText('3:00 p.m.');
		await expect(page.locator('[data-item="hours-weekday-tumwater"][data-flash]')).toHaveCount(1);
		await expect(page.getByRole('status').filter({ hasText: 'Updated: Opening hours' })).toHaveCount(1);
	});

	test('records open on request; change notes show under Updates once read', async ({ page }) => {
		await page.setViewportSize({ width: 1440, height: 900 });
		await page.clock.install({ time: NOW });
		const f = fixture('wa-guide');
		const office = await mockOffice(page, f, {
			changes: [
				{
					source_url: 'https://www.sos.wa.gov/corporations-charities/contact-us',
					captured_at: '2026-11-04T16:00:00Z',
					previous_captured_at: '2026-11-03T16:00:00Z',
					added: ['Thursday, November 26, 2026: closed for Thanksgiving Day.'],
					removed: [],
					more_added: 0,
					more_removed: 0
				}
			]
		});
		await open(page, srv, f);
		await expect(page.locator('#updates')).toHaveCount(0);
		await page.locator('#made').scrollIntoViewIfNeeded();
		await expect(page.getByRole('heading', { name: 'Updates' })).toBeVisible();
		await expect(page.locator('#updates')).toContainText('Changes on sos.wa.gov');
		const before = office.reads.filter((p) => p.endsWith('/changes')).length;
		expect(before).toBe(1);
		await page.locator('#made summary').click();
		await expect(page.locator('#made')).toContainText('#210 · per_document: $15.00 per document');
		await expect(page.locator('#made')).toContainText('What changed on');
		// The records reuse the notes the guide already read.
		expect(office.reads.filter((p) => p.endsWith('/changes')).length).toBe(before);
	});

	test('Viet Nam: thin, the office name as h1, no offers read, every value visible', async ({ page }) => {
		await page.setViewportSize({ width: 1440, height: 900 });
		const errors = [];
		page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
		const f = fixture('vietnam-prod');
		const office = await mockOffice(page, { ...f, facts: f.facts, plan: null, authority: f.row }, { authorityInPlan: false });
		await open(page, srv, f);
		await expect(page.locator('h1')).toHaveText('Viet Nam — receiving authorities');
		await expect(page.getByRole('heading', { name: 'At a glance' })).toHaveCount(0);
		const ids = await page.evaluate(() =>
			[...document.querySelectorAll('[data-fact-ids]')].flatMap((el) => el.getAttribute('data-fact-ids').split(' '))
		);
		for (const fact of f.facts.facts) expect(ids).toContain(String(fact.id));
		expect(office.reads.some((p) => p.startsWith('/p2/orders/options'))).toBe(false);
		expect(office.reads).toContain('/authorities');
		expect(errors).toEqual([]);
	});

	test('an office we have not checked: no values, the listed site as plain text', async ({ page }) => {
		await page.setViewportSize({ width: 390, height: 844 });
		const f = fixture('empty');
		await mockOffice(page, f);
		await open(page, srv, f);
		await expect(page.locator('h1')).toHaveText('Oregon Secretary of State — Corporation Division');
		await expect(page.getByRole('heading', { name: 'Not checked yet' })).toBeVisible();
		await expect(page.getByText('https://sos.oregon.gov/')).toBeVisible();
		await expect(page.locator('a[href^="https://sos.oregon.gov"]')).toHaveCount(0);
		await expect(page.getByRole('button', { name: 'Citation' })).toHaveCount(0);
		await expect(page.locator('#made')).toHaveCount(0);
	});
});
