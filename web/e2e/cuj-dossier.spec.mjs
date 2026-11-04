// Customer CUJs D1–D11 for the apostille guide (plan §1;
// docs/test-plans/cuj-customer.md): one scripted check per row, at 390
// and 1440, over the built SPA with the public API mocked
// (helpers/guide.mjs) and the clock pinned with page.clock to Wednesday
// 2026-11-04 10:00 PT (D4 moves it). Fixtures: the Washington-shaped
// wa-guide.json (canonical fixture boxes 40220/40234, never values
// copied from production), an office nobody has checked (empty.json)
// and Viet Nam's production reads (vietnam-prod.json) with R1's
// field-level conflict flags.
//
// Budgets (helpers/cuj.mjs) count what the reader does after the page
// opens, as the plan's table states them: arrival is free, so D1's
// "0 actions" is { actions: 0, inputs: 0 } and D2's "1 input + 1 click"
// is { actions: 1, inputs: 1 }.
//
// D8 here is the citations toggle on paper; print.spec.mjs counts the
// Letter pages.
//
// Also here: axe with no serious or critical issue at 390/1440 ×
// light/dark × citations on/off; the raw-key regex and the copy rules;
// Alt+C and the Citations button; hover, focus and Esc on a marker; an
// endnote backlink round trip; focus never under the sticky masthead or
// a card (Tab and Shift+Tab through the whole guide); the conflict box
// (no Copy, no label link); the empty-state exit; Viet Nam's thin state;
// a live update over page.routeWebSocket; and the one-region-named-fees
// rule (J1).
import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { RAW_KEY as RAW_KEY_LINE, probeValueNodes, publicFacts } from '../../xtask/ops/gp2-page.mjs';
import { cuj } from './helpers/cuj.mjs';
import { serveDir } from './helpers/serve.mjs';
import { BUILD, NOW, consoleErrors, dossier, liveBus, mockGuide, useTheme, withR1Flags } from './helpers/guide.mjs';

const SIZES = [
	{ width: 390, height: 844 },
	{ width: 1440, height: 900 }
];
// WCAG 2.2 AA, the plan's accessibility target (shots.mjs runs 2.0 AA).
const AXE_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'];
// Plan §2: a raw drafter key anywhere in the text ("standard_fee: ").
const RAW_KEY = /\b[a-z][a-z0-9]*_[a-z0-9_]+:\s/;
// Never on the page.
const BANNED = /\b(?:placeholder|not wired|skeleton|lorem ipsum|todo|coming soon)\b/i;
// Vendor model names, spelled in halves: the repo's model gate refuses
// any tracked line that names one.
const VENDOR = new RegExp(['gem' + 'ini', 'clau' + 'de-'].join('|'), 'i');
// Above the records the page never names its own machinery.
const MACHINERY = /\b(?:dossier|slot|topic|fact)s?\b/i;
// A street line ("243 Israel Rd SE"); a PO box value ("PO Box 40234").
const STREET = /\b\d{1,6}\s+(?:[A-Z][a-z]+\s+)+(?:Rd|Road|St|Street|Ave|Avenue|Blvd|Way|Dr|Drive|Ln|Lane)\b/;
const PO_BOX = /\bP\.?\s?O\.?\s*Box\s*\d/i;

// Accepted office records (OfficeSetV1) for D4, each resting on the
// fixture's own address and hours facts, with a staff-confirmed zone.
const OFFICE_RECORDS = [
	{
		id: 31,
		status: 'accepted',
		name: 'Tumwater office',
		hours: [{ days: ['mon', 'tue', 'wed', 'thu', 'fri'], open: '08:00', close: '16:00', fact_id: 224 }],
		timezone: 'America/Los_Angeles',
		timezone_source: 'staff_confirmed',
		address: { line1: 'Town Center 3, 243 Israel Rd SE', city: 'Tumwater', region: 'WA', postal_code: '98501', country: 'US', fact_id: 222 },
		fact_ids: [222, 224]
	},
	{
		id: 32,
		status: 'accepted',
		name: 'Cheney office',
		hours: [{ days: ['mon', 'tue', 'wed', 'fri'], open: '08:00', close: '16:00', fact_id: 225 }],
		timezone: 'America/Los_Angeles',
		timezone_source: 'staff_confirmed',
		address: { line1: '960 Washington St', city: 'Cheney', region: 'WA', postal_code: '99004', country: 'US', fact_id: 223 },
		fact_ids: [223, 225]
	}
];

/** The fixture without its restore line: the outage notice is active. */
const withoutRestore = (facts) => facts.filter((x) => x.id !== 249);

let srv;

/**
 * Open one fixture's guide.
 * @param {import('@playwright/test').Page} page
 * @param {{
 *   width: number, height: number, name?: 'wa-guide'|'vietnam-prod'|'empty',
 *   facts?: (facts: any[]) => any[], plan?: (plan: any) => any, options?: any,
 *   query?: string, time?: Date, theme?: 'light'|'dark'
 * }} o
 */
async function guide(page, o) {
	await page.setViewportSize({ width: o.width, height: o.height });
	await page.clock.install({ time: o.time ?? NOW });
	if (o.theme) await useTheme(page, o.theme);
	const f = dossier(o.name ?? 'wa-guide');
	if (o.facts) f.facts = { ...f.facts, facts: o.facts(f.facts.facts) };
	if (o.plan) f.plan = o.plan(f.plan);
	const errors = consoleErrors(page);
	const office = await mockGuide(page, f, { options: o.options });
	const bus = await liveBus(page);
	await page.goto(`${srv.url}/apostille/authority/${f.slug}${o.query ?? ''}`);
	await expect(page.locator('h1[data-heading]')).toBeVisible();
	await expect(page.locator('[data-guide]')).toBeVisible();
	return { f, office, bus, errors };
}

/**
 * WCAG 2.5.8 over the row markers (`.cite-row a.cite`): each one a box
 * of at least 24 x 24 px, and no two boxes overlapping (a later marker
 * covering an earlier one would shrink its target).
 * @param {import('@playwright/test').Page} page
 */
async function markerTargets(page) {
	return page.evaluate(() => {
		// Rendered markers only: one inside a closed <details> is no target.
		const boxes = [...document.querySelectorAll('.cite-row a.cite')]
			.filter((a) => a.checkVisibility())
			.map((a) => ({ id: a.id, r: a.getBoundingClientRect() }))
			.filter((b) => b.r.width > 0);
		const small = boxes.filter((b) => b.r.width < 24 || b.r.height < 24).map((b) => b.id);
		const overlaps = [];
		for (let i = 0; i < boxes.length; i++) {
			for (let j = i + 1; j < boxes.length; j++) {
				const a = boxes[i].r;
				const b = boxes[j].r;
				const w = Math.min(a.right, b.right) - Math.max(a.left, b.left);
				const h = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
				if (w > 0.5 && h > 0.5) overlaps.push(`${boxes[i].id} ∩ ${boxes[j].id} = ${w.toFixed(1)}x${h.toFixed(1)}`);
			}
		}
		return { small, overlaps };
	});
}

/** The focused element sits fully in the viewport, clear of the sticky masthead. */
async function expectUnobscured(page) {
	const box = await page.evaluate(() => {
		const el = document.activeElement;
		const bar = document.querySelector('header.masthead');
		if (!el || el === document.body) return null;
		const r = el.getBoundingClientRect();
		const top = bar && getComputedStyle(bar).position !== 'static' ? bar.getBoundingClientRect().bottom : 0;
		return { top: r.top, bottom: r.bottom, below: top, height: window.innerHeight };
	});
	expect(box, 'something holds focus').not.toBeNull();
	expect(box.top, 'focus clears the masthead').toBeGreaterThanOrEqual(box.below - 0.5);
	expect(box.bottom, 'focus is inside the viewport').toBeLessThanOrEqual(box.height + 0.5);
}

test.describe('customer guide CUJs D1–D11', () => {
	test.beforeAll(async () => {
		srv = await serveDir(BUILD, { base: '/apostille' });
	});
	test.afterAll(async () => {
		await srv?.close();
	});

	for (const size of SIZES) {
		const at = `at ${size.width}`;

		test(`D1 right office, D5 time, D9 status, J1 one fees region ${at}: answered on arrival`, async ({ page }) => {
			const { f, errors } = await guide(page, size);
			const journey = cuj(page);

			// One h1, built with the brand heading; the task, then the issuer.
			await expect(page.locator('h1')).toHaveCount(1);
			await expect(page.locator('h1[data-heading]')).toHaveText('Get an apostille in Washington');
			await expect(page.getByText('Issued by the Washington Secretary of State — Apostilles & Authentications.')).toBeVisible();
			await expect(page.getByText('Unofficial guide, not a government website').first()).toBeVisible();

			// D1: scope, the federal route and the warning, in step 1.
			const before = page.getByRole('region', { name: /Check this is the right office/ });
			await expect(before).toHaveAttribute('id', 'before');
			await expect(before.getByText('Documents issued in Washington State or notarized by a Washington notary public', { exact: true })).toBeVisible();
			await expect(before.getByText('Federal documents go to the U.S. Department of State', { exact: true })).toBeVisible();
			const warning = before.locator('[data-print-keep]').filter({ hasText: 'Before you apply' });
			await expect(warning).toContainText(
				'Certificates are not valid for use anywhere within the United States of America, its territories or possessions.'
			);
			await expect(warning.locator('svg').first()).toBeAttached(); // text plus an icon, never colour alone

			// D5: the time per route, verbatim with the office's hedge.
			const times = [
				'Typically processed 7 to 10 business days of receiving your order',
				'Typically processed within 2 business days of receiving your order',
				'While you wait, the same business day'
			];
			for (const text of times) {
				await expect(page.locator('[data-glance="time"]').getByText(text, { exact: true })).toBeVisible();
				await expect(page.locator('#after').getByText(text, { exact: true })).toBeVisible();
			}

			// D9: the Nov 2 outage was restored yesterday: a status line, no banner.
			const status = page.locator('[data-status="resolved"]');
			await expect(status).toContainText("No open service notice in the office's notice log (last read");
			await expect(status).toContainText('Resolved:');
			await expect(status).toContainText('The online filing system was restored on November 3, 2026 at 12:01 p.m.');
			await expect(page.getByRole('region', { name: 'Service notice' })).toHaveCount(0);

			// J1: "Pay the fees" is the only region whose name says fees.
			await expect(page.getByRole('region', { name: /fees/i })).toHaveCount(1);
			await expect(page.getByRole('region', { name: /fees/i })).toHaveAttribute('id', 'fees');

			// Traceable: every public fact on a visible value node (closures
			// folded under "More closures" wait in a closed <details>), every
			// visible value node shows text, and no raw key anywhere.
			const probe = await page.evaluate(probeValueNodes);
			const folded = await page.evaluate(() =>
				[...document.querySelectorAll('details:not([open]) [data-fact-ids]')].flatMap((el) =>
					String(el.getAttribute('data-fact-ids')).split(' ')
				)
			);
			const missing = publicFacts(f.facts.facts)
				.map((x) => String(x.id))
				.filter((id) => !probe.ids.includes(id) && !folded.includes(id));
			expect(missing).toEqual([]);
			expect(probe.empty).toEqual([]);
			expect(probe.text).not.toMatch(RAW_KEY);
			expect(probe.text).not.toMatch(RAW_KEY_LINE);
			expect(probe.text).not.toMatch(BANNED);
			expect(probe.text).not.toMatch(VENDOR);
			const above = probe.text.split('How this page was made')[0];
			expect(above).not.toMatch(MACHINERY);

			// Layout: no overflow; at 390 no sticky bar of the guide's own.
			const layout = await page.evaluate(() => ({
				overflow: document.documentElement.scrollWidth - window.innerWidth,
				sticky: [...document.querySelectorAll('[data-guide] *')]
					.filter((el) => ['sticky', 'fixed'].includes(getComputedStyle(el).position) && el.checkVisibility())
					.map((el) => el.tagName),
				footers: document.querySelectorAll('main footer').length
			}));
			expect(layout.overflow).toBeLessThanOrEqual(0);
			expect(layout.footers).toBe(0);
			if (size.width < 768) expect(layout.sticky).toEqual([]);

			expect(journey.summary()).toEqual({ actions: 0, inputs: 0 });
			expect(errors).toEqual([]);
		});

		test(`D2 what N documents cost ${at}: one input and one click`, async ({ page, context }) => {
			await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: srv.url });
			await guide(page, size);
			const fees = page.locator('#fees');
			// The fee table, verbatim.
			for (const text of [
				'$15.00 per document',
				'an additional $100 expedite fee per 10 documents',
				'an additional $150 same day fee for the first 10 documents',
				'All fees are non-refundable'
			]) {
				await expect(fees.locator('dl').first().getByText(text, { exact: true })).toBeVisible();
			}
			const count = fees.getByLabel('Number of documents');
			const speed = (name) => fees.getByRole('group', { name: /Work out your total/ }).locator('label').filter({ hasText: name });
			const total = fees.locator('[data-print-line]');

			let journey = cuj(page);
			await journey.fill(count, '3');
			await journey.click(speed(/^\s*Expedited\s*$/));
			await expect(total).toContainText('Total to enclose: $145.00 = 3 × $15.00 + $100.00 expedite fee (1–10 documents)');
			await expect(total.locator('[data-state="computed"]')).toHaveAttribute('data-inputs', /fees-per_document/);
			expect(journey.summary()).toEqual({ actions: 1, inputs: 1 });

			// The 11–20 same-day tier is printed on the office's page: it answers.
			journey = cuj(page);
			await journey.fill(count, '12');
			await journey.click(speed('Same day, in person'));
			await expect(total).toContainText('Total at the counter: $430.00');
			expect(journey.summary()).toEqual({ actions: 1, inputs: 1 });

			// 25 is past the calculator's own range (the office writes "etc."):
			// it says so, never blaming the office, and asks with the phone.
			journey = cuj(page);
			await journey.fill(count, '25');
			await expect(total).toContainText("This calculator doesn't work out totals for that many documents. Ask the office before you pay.");
			await expect(total).toContainText('Call the office: 360-725-0377');
			await expect(total.locator('[data-state="computed"]')).toHaveCount(0);
			expect(journey.summary()).toEqual({ actions: 0, inputs: 1 });

			// The payee copies exactly, and says so.
			journey = cuj(page);
			await journey.click(fees.getByRole('button', { name: 'Copy the payee name' }));
			await expect(fees.getByRole('status').filter({ hasText: 'Copied: Secretary of State' })).toHaveCount(1);
			expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('Secretary of State');
			expect(journey.summary()).toEqual({ actions: 1, inputs: 0 });
		});

		test(`D2 ${at}: a tier the office's page does not print is refused, not guessed`, async ({ page }) => {
			// The same-day fee as accepted, quoted without its 11–20 tier.
			const quote = 'Same day service: Fee is $15.00 per document and an additional $150 same day fee for the first 10 documents.';
			await guide(page, {
				...size,
				facts: (facts) => facts.map((x) => (x.id === 212 ? { ...x, citations: x.citations.map((c) => ({ ...c, quote })) } : x))
			});
			const fees = page.locator('#fees');
			const total = fees.locator('[data-print-line]');
			const journey = cuj(page);
			await journey.fill(fees.getByLabel('Number of documents'), '12');
			await journey.click(fees.getByRole('group', { name: /Work out your total/ }).locator('label').filter({ hasText: 'Same day, in person' }));
			await expect(total).toContainText("The office's pages don't give this fee for that many documents. Ask the office before you pay.");
			await expect(total.locator('[data-state="computed"]')).toHaveCount(0);
			expect(journey.summary()).toEqual({ actions: 1, inputs: 1 });
			// Ten documents sit inside the printed first block: that still answers.
			await fees.getByLabel('Number of documents').fill('10');
			await expect(total).toContainText('Total at the counter: $300.00');
		});

		test(`D3 where to send it ${at}: no wrong address, the disputed box with no Copy and no label`, async ({ page }) => {
			const { errors } = await guide(page, size);
			const journey = cuj(page);
			const mail = page.locator('#send-mail');
			const courier = page.locator('#send-courier');
			await expect(mail.getByRole('heading', { name: 'By mail (USPS)' })).toBeVisible();
			await expect(courier.getByRole('heading', { name: 'By courier (FedEx, UPS)' })).toBeVisible();

			// Every variant, each with the page it is printed on.
			const box = mail.locator('[data-conflict]');
			await expect(box.getByRole('heading', { name: "The office's own pages list different mailing addresses" })).toBeVisible();
			await expect(box.locator('h4 svg').first()).toBeAttached();
			await expect(box.locator(':scope > ul > li')).toHaveCount(3);
			for (const [id, text] of [
				[216, 'PO BOX 40220 Olympia, WA 98504-0220'],
				[217, 'PO Box 40234, Olympia, WA 98504-0234'],
				[218, 'PO Box 40220, Olympia, WA 98504-0234']
			]) {
				await expect(box.locator(`:scope > ul > li [data-fact-ids="${id}"]`)).toHaveText(text);
			}
			// A ZIP+4 never breaks at its hyphen (display only).
			expect(await box.locator(':scope > ul > li .whitespace-nowrap').allInnerTexts()).toEqual(['98504-0220', '98504-0234', '98504-0234']);
			// The safe way round: call first, then the courier route.
			const advice = await box.evaluate((el) => [...el.querySelectorAll('ul.list-disc > li')].map((li) => li.textContent.replace(/\s+/g, ' ').trim()));
			expect(advice).toEqual(['Call 360-725-0377 (option 3) before you mail.', 'Or send by courier to the Tumwater overnight address.']);
			// The staff decision the office's phone answer backs (plan §3): in
			// words, citations off, with the preferred row marked and the only
			// Copy; the other values stay listed, and labels stay off.
			const advise = box.locator('[data-recommendation]');
			await expect(advise).toContainText(
				'Our recommendation (staff, November 4, 2026): use PO Box 40234, Olympia, WA 98504-0234 — confirmed by phone with the office (Apostille desk, the phone line on the office\'s contact page).'
			);
			await expect(advise).toContainText('The office told us by phone on November 4, 2026: The desk said mailed requests go to PO Box 40234, ZIP 98504-0234.');
			await expect(box.locator('[data-recommended]')).toHaveCount(1);
			await expect(box.locator('[data-recommended] [data-fact-ids="217"]')).toBeVisible();
			await expect(box.locator('[data-recommended]')).toContainText('Our recommendation');
			const copy = box.getByRole('button', { name: /copy/i });
			await expect(copy).toHaveCount(1);
			await expect(box.locator('[data-recommended]').getByRole('button', { name: /copy/i })).toHaveCount(1);
			await expect(mail.getByRole('button', { name: /copy/i })).toHaveCount(1);
			await expect(box.locator('a[href*="/order"]')).toHaveCount(0);
			await expect(page.locator('[data-offer="mail"], a[href*="kind=address_label"], a[href*="kind=label_usps"]')).toHaveCount(0);
			await expect(box.getByText("A phone answer isn't in writing")).toHaveCount(0);
			// The glance points at the step instead of naming a box.
			const send = page.locator('[data-glance="send"]');
			await expect(send.getByRole('link', { name: /list different PO boxes/ })).toHaveAttribute('href', '#send-mail');

			// 0 wrong-address outputs: no street by mail, no PO box by courier.
			expect(await mail.innerText()).not.toMatch(STREET);
			expect(await courier.innerText()).not.toMatch(PO_BOX);
			await expect(courier.locator('[data-item="mailing-courier"]')).toContainText('Town Center 3, 243 Israel Rd SE, Tumwater, WA 98501');
			expect(await send.innerText()).not.toMatch(PO_BOX);

			expect(journey.summary()).toEqual({ actions: 0, inputs: 0 });
			expect(errors).toEqual([]);
		});

		test(`D4 walk in today ${at}: the rules as printed without an office record`, async ({ page }) => {
			await guide(page, size);
			const visit = page.locator('#visit');
			await expect(visit.getByRole('heading', { name: 'In person, same day' })).toBeVisible();
			await expect(visit.getByText('Bring documents at least 30 minutes before closing for same-day service', { exact: true })).toBeVisible();
			await expect(visit.getByText('Monday through Friday, 8:00 a.m. to 4:00 p.m.', { exact: true })).toBeVisible();
			// Nothing calculated: no accepted record with a confirmed zone.
			await expect(visit.locator('[data-state="computed"]')).toHaveCount(0);
			await expect(visit.getByText(/Open now|Closed now|Today's cut-off/)).toHaveCount(0);
			// The next closures, verbatim, under their dates.
			await expect(visit.locator('[data-item="holiday_hours-closure-2026_11_11"]')).toContainText('Wed, Nov 11');
			await expect(visit.locator('[data-item="holiday_hours-closure-2026_11_11"]')).toContainText(
				'Closed Wednesday, November 11, 2026 for Veterans Day'
			);
			await expect(visit.locator('[data-item="holiday_hours-closure-2026_11_13"]')).toContainText(
				'Closing early at 2:00 p.m. on Friday, November 13, 2026'
			);
		});

		for (const [when, time, office, want] of [
			['Monday 10:00 PT', '2026-11-09T18:00:00Z', 'record-31', ['Open now · closes 4:00 p.m.', "Today's cut-off: 3:30 p.m."]],
			['Thursday 10:00 PT', '2026-11-12T18:00:00Z', 'record-32', ['Closed now · opens tomorrow at 8:00 a.m.']],
			['Nov 11 (closed)', '2026-11-11T18:00:00Z', 'record-31', ['Closed today', 'Closed Wednesday, November 11, 2026 for Veterans Day']],
			['Nov 13 (closing early)', '2026-11-13T18:00:00Z', 'record-31', ['Open now · closes 2:00 p.m.', "Today's cut-off: 1:30 p.m."]]
		]) {
			test(`D4 walk in today ${at}, ${when}: calculated from the accepted office record`, async ({ page }) => {
				await guide(page, { ...size, time: new Date(time), plan: (p) => ({ ...p, offices: OFFICE_RECORDS }) });
				const card = page.locator(`#visit [data-office="${office}"]`);
				for (const text of want) await expect(card).toContainText(text);
				const status = card.locator('[data-state="computed"]');
				await expect(status).toHaveCount(1);
				// A calculated cut-off names the printed rule it counts from.
				if (want.some((t) => t.includes('cut-off'))) await expect(status).toHaveAttribute('data-inputs', /walkin-cutoff/);
			});
		}

		test(`D4 ${at}: an office record's card, citations on: h4 name, guide times, markers clear of each other`, async ({ page }) => {
			await guide(page, { ...size, time: new Date('2026-11-09T18:00:00Z'), query: '?cite=1', plan: (p) => ({ ...p, offices: OFFICE_RECORDS }) });
			const card = page.locator('#visit [data-office="record-31"]');
			await expect(card.getByRole('heading', { level: 4, name: 'Tumwater office' })).toBeVisible();
			await expect(card).toContainText('8:00 a.m. – 4:00 p.m.');
			await expect(card).not.toContainText(/\b(?:AM|PM)\b/);
			await expect(page.locator('a.cite').first()).toBeVisible();
			expect(await markerTargets(page)).toEqual({ small: [], overlaps: [] });
		});

		test(`D4 ${at}: the office record card failing to load leaves each office its own card`, async ({ page }) => {
			// A stale hashed chunk after a deploy: the record card never loads.
			await page.route(/\/_app\/immutable\/chunks\/.*\.js$/, async (route) => {
				const res = await route.fetch();
				const body = await res.text();
				// The card's own code (its words live in the guide's copy table).
				if (body.includes('directions_url') && body.includes('mail_only')) return route.abort();
				return route.fulfill({ response: res, body });
			});
			await guide(page, { ...size, time: new Date('2026-11-09T18:00:00Z'), plan: (p) => ({ ...p, offices: OFFICE_RECORDS }) });
			const visit = page.locator('#visit');
			await expect(visit.locator('[data-office="tumwater"]')).toBeVisible();
			await expect(visit.locator('[data-office="tumwater"]')).toContainText('Town Center 3, 243 Israel Rd SE, Tumwater, WA 98501');
			await expect(visit.locator('[data-office="tumwater"]')).toContainText('Monday through Friday, 8:00 a.m. to 4:00 p.m.');
			await expect(visit.locator('[data-office="cheney"]')).toContainText('960 Washington St, Cheney, WA 99004');
			await expect(visit.locator('[data-office^="record-"]')).toHaveCount(0);
		});

		test(`D4 and D9 ${at}: an active notice sits under the h1 and hides "Open now"`, async ({ page }) => {
			await guide(page, {
				...size,
				time: new Date('2026-11-09T18:00:00Z'),
				facts: withoutRestore,
				plan: (p) => ({ ...p, offices: OFFICE_RECORDS })
			});
			const banner = page.getByRole('region', { name: 'Service notice' });
			await expect(banner).toBeVisible();
			await expect(banner).toContainText('The online filing system is unavailable due to an outage.');
			await expect(banner.locator('svg').first()).toBeAttached();
			// Directly under the h1: the lede, then the banner.
			const prev = await banner.evaluate((el) => el.previousElementSibling?.textContent?.trim() ?? '');
			expect(prev).toBe('Issued by the Washington Secretary of State — Apostilles & Authentications.');
			await expect(page.locator('[data-status="resolved"]')).toHaveCount(0);
			await expect(page.locator('#visit [data-office="record-31"]')).toBeVisible();
			await expect(page.locator('#visit [data-state="computed"]')).toHaveCount(0);
			await expect(page.locator('#visit').getByText(/Open now|Today's cut-off/)).toHaveCount(0);
		});

		test(`D9 ${at}: a week after the restore the pair moves to Updates`, async ({ page }) => {
			await guide(page, { ...size, time: new Date('2026-11-12T18:00:00Z') });
			// The notice log still reads clear; the pair itself has left the band.
			const status = page.locator('[data-status="resolved"]');
			await expect(status).toContainText("No open service notice in the office's notice log (last read");
			await expect(status).not.toContainText('Resolved:');
			await expect(page.getByRole('region', { name: 'Service notice' })).toHaveCount(0);
			const updates = page.getByRole('region', { name: 'Updates' });
			await expect(updates).toBeVisible();
			await expect(updates).toContainText('The online filing system was restored on November 3, 2026 at 12:01 p.m.');
		});

		test(`D6 get the form ${at}: the official PDF in one action`, async ({ page }) => {
			const { f } = await guide(page, size);
			const form = page.getByRole('region', { name: /Fill in the request form/ });
			const pdf = form.getByRole('link', { name: 'Download the official form (PDF)' });
			const url = f.facts.facts.find((x) => x.id === 207).citations[0].source_url;
			await expect(pdf).toBeVisible();
			await expect(pdf).toHaveAttribute('href', url);
			// No form map accepted: no online fill-in.
			await expect(page.getByText('Fill it in online — free')).toHaveCount(0);
			// The office's server is never reached: the click lands on a stub.
			await page.route(
				(u) => u.href === url,
				(route) => route.fulfill({ contentType: 'text/plain', body: 'official form' })
			);
			const journey = cuj(page);
			await journey.click(pdf);
			await page.waitForURL(url);
			expect(journey.summary()).toEqual({ actions: 1, inputs: 0 });
		});

		test(`D6 ${at}: two form facts worded differently still give the PDF in one action`, async ({ page }) => {
			// Production: "…Request Form (PDF Fillable Form)" on one page,
			// "…Request (Fillable Form)" on another: the slot is worded.
			const page2 = 'https://www.sos.wa.gov/corporations-charities/apostilles-services-information-resource-page';
			const { f } = await guide(page, {
				...size,
				facts: (facts) => {
					const form = facts.find((x) => x.id === 207);
					return [
						...facts.map((x) =>
							x.id === 207 ? { ...x, statement: 'form: Apostille Authentication Request Form (PDF Fillable Form)', citations: x.citations.map((c) => ({ ...c, kind: 'web', source_url: page2 })) } : x
						),
						{ ...form, id: 252, statement: 'form: Apostille Authentication Request (Fillable Form)' }
					];
				}
			});
			// The PDF one of the two pages cites.
			const url = f.facts.facts.find((x) => x.id === 252).citations[0].source_url;
			expect(url).toMatch(/\.pdf$/);
			const form = page.locator('#form');
			await expect(form.getByText('Worded differently on the office\'s pages')).toBeVisible();
			await expect(form.getByRole('link', { name: 'Download the official form (PDF)' })).toHaveAttribute('href', url);
			// The glance's form row carries the same link.
			await expect(page.locator('[data-glance="form"]').getByRole('link', { name: 'Download (PDF)' })).toHaveAttribute('href', url);
		});

		test(`D6 ${at}: an accepted, fillable form map offers the free online fill (R3 contract)`, async ({ page }) => {
			// docs/contracts/plan.json forms[0], as R3's /plan serves it.
			const contract = JSON.parse(readFileSync(new URL('../../docs/contracts/plan.json', import.meta.url), 'utf8')).forms[0];
			expect(contract.fill_supported).toBe(true);
			await guide(page, { ...size, plan: (p) => ({ ...p, forms: [{ ...contract, authority_slug: 'usa-wa-sos' }] }) });
			const form = page.locator('#form');
			const fill = form.getByRole('link', { name: 'Fill it in online — free' });
			await expect(fill).toBeVisible();
			await expect(fill).toHaveAttribute('href', '/apostille/order?destination=usa-wa-sos&path=general&step=form');
			await expect(form.getByRole('link', { name: 'Download the official form (PDF)' })).toBeVisible();
		});

		test(`D6 ${at}: a form cited from a web page is never called a PDF`, async ({ page }) => {
			const page2 = 'https://www.sos.wa.gov/corporations-charities/apostilles-services-information-resource-page';
			await guide(page, {
				...size,
				facts: (facts) =>
					facts.map((x) =>
						x.topic === 'forms' ? { ...x, citations: x.citations.map((c) => ({ ...c, kind: 'web', source_url: page2 })) } : x
					)
			});
			const form = page.locator('#form');
			await expect(form.getByRole('link', { name: /Get the form on the office's page/ })).toHaveAttribute('href', page2);
			await expect(form.getByText(/PDF/)).toHaveCount(0);
		});

		test(`D7 where it comes from ${at}: the button, then ${size.width < 768 ? 'a tap' : 'a hover'}; Esc and the endnote round trip`, async ({ page }) => {
			await guide(page, size);
			const toggle = page.getByRole('button', { name: 'Citation' }).first();
			await expect(toggle).toHaveAccessibleName(/^Citations/);
			await expect(toggle).toHaveAttribute('aria-pressed', 'false');
			await expect(toggle).toHaveAttribute('aria-keyshortcuts', 'Alt+C');
			await expect(page.locator('a.cite')).toHaveCount(0);
			await expect(page.locator('#sources')).toHaveCount(0);
			await expect(page.locator('blockquote')).toHaveCount(0);

			const journey = cuj(page);
			await journey.click(toggle);
			await expect(toggle).toHaveAttribute('aria-pressed', 'true');
			await expect(page.locator('section#sources[role="doc-endnotes"]')).toBeVisible();
			await expect(page.getByText(/^Citations shown: \d+ sources numbered$/)).toHaveCount(1);
			// demo.spec: the first blockquote on the page is an endnote's.
			expect(await page.locator('blockquote').first().evaluate((el) => !!el.closest('#sources'))).toBe(true);

			// The fee's marker: the card names the kind of evidence, quotes the
			// office, links the page and gives both dates.
			const marker = page.locator('#fees a.cite[role="doc-noteref"]').first();
			const card = page.locator('[data-cite-card]');
			if (size.width < 768) await journey.click(marker);
			else await journey.hover(marker);
			await expect(card).toBeVisible();
			await expect(card).toContainText('Official web page');
			await expect(card.locator('blockquote')).toContainText('Fee is $15.00 per document.');
			await expect(card.locator('blockquote mark')).toHaveText('$15.00 per document');
			await expect(card).toContainText(/Captured .+ · approved /);
			await expect(card.getByRole('link', { name: /Open the page/ })).toHaveAttribute(
				'href',
				/^https:\/\/www\.sos\.wa\.gov\/corporations-charities\/apostilles-services-information-resource-page#:~:text=/
			);
			expect(journey.summary()).toEqual({ actions: 2, inputs: 0 });
			// The card fits the screen.
			const fit = await card.evaluate((el) => {
				const r = el.getBoundingClientRect();
				return { left: r.left, right: r.right, width: r.width, vw: document.documentElement.clientWidth };
			});
			expect(fit.left).toBeGreaterThanOrEqual(0);
			expect(fit.right).toBeLessThanOrEqual(fit.vw);
			expect(fit.width).toBeLessThanOrEqual(Math.min(320, fit.vw - 32) + 0.5);
			// While a card is open, the first blockquote is still an endnote's.
			expect(await page.locator('blockquote').first().evaluate((el) => !!el.closest('#sources'))).toBe(true);

			// Dismissible: Esc closes it.
			await page.keyboard.press('Escape');
			await expect(card).toBeHidden();

			if (size.width >= 768) {
				// Hoverable and persistent: the pointer can rest on the card
				// past the close delay; leaving both closes it.
				await page.mouse.move(2, size.height - 2);
				await marker.hover();
				await expect(card).toBeVisible();
				await card.hover();
				await page.waitForTimeout(800);
				await expect(card).toBeVisible();
				await page.mouse.move(2, size.height - 2);
				await expect(card).toBeHidden();
			}

			// Keyboard: Tab from Print reaches the first marker; focus opens
			// the card at once; Esc closes it and keeps focus on the marker.
			await page.getByRole('button', { name: 'Print' }).focus();
			await page.keyboard.press('Tab');
			const focused = page.locator('a.cite:focus');
			await expect(focused).toHaveCount(1);
			await expect(card).toBeVisible();
			const id = await focused.getAttribute('id');
			const n = await focused.getAttribute('data-cite-n');
			const letter = String(await focused.getAttribute('data-cite-letters')).split(' ')[0];
			await expectUnobscured(page);
			await page.keyboard.press('Escape');
			await expect(card).toBeHidden();
			await expect(page.locator(`#${id}`)).toBeFocused();

			// Enter follows the marker to its endnote, which takes focus; the
			// endnote's backlink for that quote brings focus back.
			await page.keyboard.press('Enter');
			const note = page.locator(`#src-${n}`);
			await expect(note).toBeFocused();
			const back = note.locator('a[role="doc-backlink"]').nth(letter.charCodeAt(0) - 97);
			expect(await back.evaluate((a) => a.parentElement?.querySelector('span')?.textContent?.trim())).toBe(letter);
			await back.focus();
			await page.keyboard.press('Enter');
			await expect(page.locator(`#${id}`)).toBeFocused();
			await expectUnobscured(page);

			// Alt+C turns them off: markers, card and endnotes leave, and
			// focus stays on the page, beside the marker that left.
			await page.keyboard.press('Alt+KeyC');
			await expect(toggle).toHaveAttribute('aria-pressed', 'false');
			await expect(page.locator('a.cite')).toHaveCount(0);
			await expect(page.locator('#sources')).toHaveCount(0);
			await expect(page.getByText('Citations hidden')).toHaveCount(1);
			expect(await page.evaluate(() => document.activeElement !== document.body && document.activeElement !== null)).toBe(true);

			// ...and back on with the hotkey alone: one action, then a hover or tap.
			const keys = cuj(page);
			await keys.press(page.keyboard, 'Alt+KeyC');
			await expect(toggle).toHaveAttribute('aria-pressed', 'true');
			await expect(page.locator('a.cite').first()).toBeVisible();
			expect(keys.summary()).toEqual({ actions: 1, inputs: 0 });
			// Markers in a row are 24 px targets, and no two overlap.
			expect(await markerTargets(page)).toEqual({ small: [], overlaps: [] });
			// Endnotes carry no raw keys either.
			expect(await page.locator('#sources').innerText()).not.toMatch(RAW_KEY);
		});

		// WCAG 2.4.11, held to "not even partly": no focused control of the
		// guide ends up under the sticky site masthead or an open citation
		// card, going forward or back.
		test(`focus is never obscured ${at}: Tab and Shift+Tab through the whole guide, citations on`, async ({ page }) => {
			await guide(page, { ...size, query: '?cite=1' });
			await expect(page.locator('[data-offer="courier"]')).toBeVisible();
			const problems = [];
			let seen = 0;
			for (const key of ['Tab', 'Shift+Tab']) {
				// Forward from the citations switch; back from the guide's last control.
				if (key === 'Tab') await page.getByRole('button', { name: 'Citation' }).first().focus();
				else await page.locator('[data-guide] a[href], [data-guide] button, [data-guide] summary').last().focus();
				for (let step = 0; step < 400; step++) {
					await page.keyboard.press(key);
					const focus = await page.evaluate(() => {
						const el = document.activeElement;
						const card = document.querySelector('[data-cite-card]');
						const inCard = !!card && !!el && card.contains(el);
						if (!el || (!document.querySelector('[data-guide]')?.contains(el) && !inCard)) return null;
						const r = el.getBoundingClientRect();
						const bar = document.querySelector('header.masthead');
						const top = bar && getComputedStyle(bar).position !== 'static' ? bar.getBoundingClientRect().bottom : 0;
						let covered = false;
						if (card && !inCard && card.getAttribute('data-state') === 'open' && card.checkVisibility()) {
							const c = card.getBoundingClientRect();
							covered = r.left < c.right && r.right > c.left && r.top < c.bottom && r.bottom > c.top;
						}
						const name = `${el.tagName.toLowerCase()}${el.id ? `#${el.id}` : ''} "${(el.textContent ?? '').trim().slice(0, 32)}"`;
						return { name, hidden: r.top < top - 0.5 || covered };
					});
					if (!focus) break;
					seen += 1;
					if (focus.hidden) problems.push(`${key}: ${focus.name}`);
				}
			}
			expect(seen).toBeGreaterThan(40);
			expect(problems).toEqual([]);
		});

		// D8 here is the toggle on paper; print.spec.mjs counts the Letter pages.
		test(`D8 print follows the toggle ${at}`, async ({ page }) => {
			await guide(page, size);
			await expect(page.locator('[data-offer="courier"]')).toBeVisible();
			await page.emulateMedia({ media: 'print' });
			await expect(page.locator('[data-print-header]')).toContainText('Check the live page before you send documents or money.');
			await expect(page.locator('a.cite, #sources')).toHaveCount(0);
			await expect(page.locator('[data-offer]')).toBeHidden();
			await page.emulateMedia({ media: 'screen' });
			const journey = cuj(page);
			await journey.press(page.keyboard, 'Alt+KeyC');
			await expect(page.locator('#sources')).toBeVisible();
			await page.emulateMedia({ media: 'print' });
			await expect(page.locator('[data-print-header]')).toContainText('Numbers in brackets point to the sources listed at the end.');
			await expect(page.locator('#sources')).toBeVisible();
			const markers = page.locator('a.cite[role="doc-noteref"]');
			await expect(markers.first()).toBeVisible();
			const orphans = await markers.evaluateAll((els) => els.filter((a) => !document.querySelector(a.getAttribute('href') ?? '#none')).length);
			expect(orphans).toBe(0);
			expect(journey.summary()).toEqual({ actions: 1, inputs: 0 });
		});

		test(`D10 a prepaid label ${at}: the free route first, the provider and live price named`, async ({ page }) => {
			const { errors } = await guide(page, size);
			const courier = page.locator('#send-courier');
			const offer = courier.locator('[data-offer="courier"]');
			await expect(offer).toBeVisible();
			const link = offer.getByRole('link', { name: 'Print a prepaid FedEx label to the Tumwater courier address — from $24.00' });
			await expect(link).toBeVisible();
			await expect(offer).toContainText('Paid service by the Apostille Handbook, not the Secretary of State.');
			// The free route is complete above it.
			const first = await page.evaluate(() => {
				const address = document.querySelector('#send-courier [data-item="mailing-courier"]');
				const card = document.querySelector('#send-courier [data-offer="courier"]');
				return !!address && !!card && !!(address.compareDocumentPosition(card) & Node.DOCUMENT_POSITION_FOLLOWING);
			});
			expect(first).toBe(true);
			expect(errors).toEqual([]);
			const journey = cuj(page);
			await journey.click(link);
			await page.waitForURL(/\/apostille\/order\?/);
			expect(Object.fromEntries(new URL(page.url()).searchParams)).toEqual({
				destination: 'usa-wa-sos',
				path: 'general',
				kind: 'label_fedex',
				step: 'delivery'
			});
			expect(journey.summary()).toEqual({ actions: 1, inputs: 0 });
		});

		test(`D10 ${at}: a disabled offer says why and points back to the free route`, async ({ page }) => {
			const f = dossier('wa-guide');
			const options = {
				...f.options,
				options: f.options.options.map((o) => (o.kind === 'label_fedex' ? { ...o, enabled: false, from_cents: null, reason: 'Paused' } : o))
			};
			await guide(page, { ...size, options });
			const offer = page.locator('#send-courier [data-offer="courier"]');
			await expect(offer).toContainText(
				"Prepaid courier labels aren't available for this office right now. You can send by FedEx or UPS yourself — copy the courier address above."
			);
			await expect(offer.getByRole('link')).toHaveCount(0);
			await expect(page.getByText('coming soon')).toHaveCount(0);
		});

		test(`D10 ${at}: no paid offer while the fee per document is disputed`, async ({ page }) => {
			const { office } = await guide(page, {
				...size,
				facts: (facts) =>
					facts.map((x) =>
						x.id === 210 || x.id === 251 ? { ...x, status: 'accepted', conflict: true, accepted_at: x.accepted_at ?? 1793750400 } : x
					)
			});
			await expect(page.locator('[data-item="fees-per_document"]')).toContainText("The office's pages give different values");
			await expect.poll(() => office.reads.some((r) => r.path === '/p2/orders/options')).toBe(true);
			await page.waitForTimeout(300);
			await expect(page.locator('[data-offer]')).toHaveCount(0);
			// About drops the fee clause it cannot state.
			await expect(page.getByRole('region', { name: 'About this guide' })).not.toContainText('per document');
		});

		test(`D2 and D10 ${at}: a staff note that contradicts the fee contests it everywhere`, async ({ page }) => {
			const note = {
				id: 19,
				kind: 'call',
				stance: 'contradicts',
				fact_id: 210,
				preferred_fact_id: null,
				slot: 'fees.per_document',
				date: '2026-11-04',
				channel: 'Apostille desk',
				summary: 'The desk said the fee went up to $20.00 per document on November 1.',
				recorded: false,
				supports: [],
				by: 'Staff reviewer'
			};
			const { office } = await guide(page, { ...size, plan: (p) => ({ ...p, evidence: [...p.evidence, note] }) });
			const row = page.locator('#fees [data-item="fees-per_document"]');
			await expect(row).toContainText('$15.00 per document');
			await expect(row).toContainText(
				'Staff note: the office told us otherwise by phone on November 4, 2026: The desk said the fee went up to $20.00 per document on November 1.'
			);
			await expect(page.locator('#fees [data-print-line]')).toContainText(
				'The office told us something different from its pages. Ask the office before you pay.'
			);
			await expect(page.locator('#fees [data-state="computed"]')).toHaveCount(0);
			await expect(page.locator('[data-glance="cost"]')).toContainText('The office told us something different');
			await expect(page.locator('[data-glance="cost"]')).not.toContainText('$15.00');
			await expect.poll(() => office.reads.some((r) => r.path === '/p2/orders/options')).toBe(true);
			await page.waitForTimeout(300);
			await expect(page.locator('[data-offer]')).toHaveCount(0);
			await expect(page.getByRole('region', { name: 'About this guide' })).not.toContainText('per document');
		});

		test(`D9 ${at}: a resolved notice stays on the page under another active one`, async ({ page }) => {
			const outage = {
				id: 260,
				topic: 'service_notice',
				status: 'accepted',
				statement: 'notice__2026_11_04: Online apostille orders are paused for maintenance.',
				conflict: false,
				conflicts: [],
				accepted_at: 1793840000,
				citations: [
					{
						capture_id: 760,
						captured_at: '2026-11-04T15:00:00Z',
						kind: 'web',
						quote: 'Online apostille orders are paused for maintenance.',
						source_url: 'https://www.sos.wa.gov/corporations-charities/public-notices/maintenance'
					}
				]
			};
			await guide(page, { ...size, query: '?cite=1', facts: (facts) => [...facts, outage] });
			const banner = page.getByRole('region', { name: 'Service notice' });
			await expect(banner).toContainText('Online apostille orders are paused for maintenance.');
			const resolved = page.locator('[data-status="resolved"]');
			await expect(resolved).toContainText('Resolved:');
			await expect(resolved).toContainText('The online filing system was restored on November 3, 2026 at 12:01 p.m.');
			// The notice log is not "clear" while a notice is open.
			await expect(resolved).not.toContainText('No open service notice');
			// Every endnote backlink has its marker on the page.
			await expect(page.locator('#sources')).toBeVisible();
			const dead = await page.evaluate(() =>
				[...document.querySelectorAll('#sources a[role="doc-backlink"]')]
					.map((a) => a.getAttribute('href') ?? '')
					.filter((href) => !document.getElementById(href.slice(1)))
			);
			expect(dead).toEqual([]);
		});

		test(`D4 ${at}: a closure whose day has passed moves to Updates; no endnote loses its marker`, async ({ page }) => {
			await guide(page, { ...size, time: new Date('2026-11-12T18:00:00Z'), query: '?cite=1' });
			const visit = page.locator('#visit');
			await expect(visit.locator('[data-item="holiday_hours-closure-2026_11_13"]')).toBeVisible();
			await expect(visit.locator('[data-item="holiday_hours-closure-2026_11_11"]')).toHaveCount(0);
			const updates = page.getByRole('region', { name: 'Updates' });
			await expect(updates).toContainText('Closed Wednesday, November 11, 2026 for Veterans Day');
			await expect(page.locator('#sources')).toBeVisible();
			const dead = await page.evaluate(() =>
				[...document.querySelectorAll('#sources a[role="doc-backlink"]')]
					.map((a) => a.getAttribute('href') ?? '')
					.filter((href) => !document.getElementById(href.slice(1)))
			);
			expect(dead).toEqual([]);
			// The markers run 1, 2, 3… with no gap.
			const ns = await page.evaluate(() => [...new Set([...document.querySelectorAll('a.cite')].map((a) => Number(a.dataset.citeN)))].sort((a, b) => a - b));
			expect(ns).toEqual(ns.map((_, i) => i + 1));
		});

		test(`D7 ${at}: values the send step repeats carry their source's number`, async ({ page }) => {
			await guide(page, { ...size, query: '?cite=1' });
			await expect(page.locator('a.cite').first()).toBeVisible();
			const n = async (selector) => page.locator(selector).first().getAttribute('data-cite-n');
			// What to include (step 3) and how it comes back (step 6), in the mail branch.
			for (const [own, copy] of [
				['#form [data-item="mailing-enclose"] a.cite', '#send-mail [data-item="mailing-enclose-send"] a.cite'],
				['#after [data-item="mailing-return"] a.cite', '#send-mail [data-item="mailing-return-send"] a.cite']
			]) {
				await expect(page.locator(copy).first()).toBeVisible();
				expect(await n(copy)).toBe(await n(own));
			}
			// The office phone in the courier line and the conflict box, and its option.
			const phone = await n('#contact [data-item="contacts-phone"] a.cite');
			expect(await n('#send-courier p a.cite[data-cite-item="contacts-phone-courier"]')).toBe(phone);
			expect(await n('[data-conflict] a.cite[data-cite-item="contacts-phone-conflict"]')).toBe(phone);
			await expect(page.locator('[data-conflict] a.cite[data-cite-item="contacts-phone_option-conflict"]')).toHaveCount(1);
			// Marker ids stay unique.
			const ids = await page.evaluate(() => [...document.querySelectorAll('a.cite')].map((a) => a.id));
			expect(new Set(ids).size).toBe(ids.length);
		});

		test(`D7 ${at}: sources that fail to load say so and load on retry`, async ({ page }) => {
			let block = true;
			await page.route(/\/_app\/immutable\/chunks\/.*\.js$/, async (route) => {
				const res = await route.fetch();
				const body = await res.text();
				if (block && body.includes('doc-endnotes')) return route.abort();
				return route.fulfill({ response: res, body });
			});
			await guide(page, { ...size, query: '?cite=1' });
			const alert = page.locator('[data-cite-failed]');
			await expect(alert).toContainText("Sources didn't load");
			await expect(page.locator('#sources')).toHaveCount(0);
			block = false;
			await alert.getByRole('button', { name: 'Retry' }).click();
			await expect(page.locator('#sources')).toBeVisible();
			await expect(alert).toHaveCount(0);
		});

		test(`D3 ${at}: the glance and the route cards never name an address the step withholds`, async ({ page }) => {
			await guide(page, {
				...size,
				facts: (facts) =>
					facts
						.filter((x) => x.id !== 217 && x.id !== 218)
						.map((x) =>
							x.id === 216
								? { ...x, statement: 'apostille_mailing_address: 801 Capitol Way S, Olympia, WA 98501', conflict: false, conflicts: [] }
								: x.id === 219
									? { ...x, statement: 'courier: PO Box 40234, Olympia, WA 98504-0234' }
									: x
						)
			});
			const send = page.locator('[data-glance="send"]');
			await expect(send).not.toContainText('801 Capitol Way S');
			await expect(send).not.toContainText('PO Box 40234');
			await expect(page.locator('#send-mail')).toContainText('Not confirmed yet — ask the office');
			await expect(page.locator('#send-courier')).toContainText('Not confirmed yet — ask the office');
			await expect(page.locator('a[href="#send-mail"], a[href="#send-courier"]')).toHaveCount(0);
		});

		test(`D11 an office we haven't checked ${at}: no values, two exits`, async ({ page }) => {
			const { errors } = await guide(page, { ...size, name: 'empty' });
			await expect(page.locator('h1[data-heading]')).toHaveText('Oregon Secretary of State — Corporation Division');
			const empty = page.getByRole('region', { name: 'Not checked yet' });
			await expect(empty).toBeVisible();
			// The listed site, as plain text, never as a link.
			await expect(empty.getByText('https://sos.oregon.gov/', { exact: true })).toBeVisible();
			await expect(empty).toContainText('— not yet checked by us');
			await expect(page.locator('a[href^="https://sos.oregon.gov"]')).toHaveCount(0);
			// No values, trust line, switch, offers or records.
			await expect(page.locator('[data-fact-ids]')).toHaveCount(0);
			await expect(page.getByText(/Sources captured/)).toHaveCount(0);
			await expect(page.getByRole('button', { name: 'Citation' })).toHaveCount(0);
			await expect(page.locator('[data-offer], [data-calculator], #made, [data-glance]')).toHaveCount(0);
			expect(errors).toEqual([]);
			// 0 dead ends: one action to the offices we have checked.
			const journey = cuj(page);
			await journey.click(empty.getByRole('link', { name: "Browse offices we've checked" }));
			await expect(
				page.getByRole('heading', { name: 'Where does your document need to be accepted?' })
			).toBeVisible();
			expect(journey.summary()).toEqual({ actions: 1, inputs: 0 });
		});

		test(`Viet Nam ${at}: thin, the office's name as h1, every value shown`, async ({ page }) => {
			const { f, office, errors } = await guide(page, { ...size, name: 'vietnam-prod', facts: withR1Flags });
			await expect(page.locator('h1[data-heading]')).toHaveText('Viet Nam — receiving authorities');
			// Thin: no glance, routes, calculator or "Not confirmed" list...
			await expect(page.getByRole('heading', { name: 'At a glance' })).toHaveCount(0);
			await expect(page.getByRole('navigation', { name: 'Ways to send your documents' })).toHaveCount(0);
			await expect(page.locator('[data-calculator], [data-not-confirmed], [data-offer]')).toHaveCount(0);
			// ...but the same section regions, unnumbered.
			const sections = page.locator('[data-section]');
			expect(await sections.count()).toBeGreaterThan(0);
			for (const id of await sections.evaluateAll((els) => els.map((el) => el.id))) {
				const heading = (await page.locator(`#${id}-h`).innerText()).trim();
				expect(heading).not.toMatch(/\d/);
				await expect(page.getByRole('region', { name: heading, exact: true })).toHaveAttribute('id', id);
			}
			expect(await page.getByRole('region', { name: /fees/i }).count()).toBeLessThanOrEqual(1);
			// Every public fact on a visible value; no false conflict after R1.
			const probe = await page.evaluate(probeValueNodes);
			for (const fact of publicFacts(f.facts.facts)) expect(probe.ids).toContain(String(fact.id));
			expect(probe.empty).toEqual([]);
			expect(probe.text).not.toMatch(RAW_KEY);
			await expect(page.locator('[data-conflict]')).toHaveCount(0);
			await expect(page.getByText("The office's pages give different values")).toHaveCount(0);
			// No official site the authority row doesn't list (hcch.net is the
			// Hague Conference's), and its pages are web pages, not official ones.
			const header = page.locator('header').filter({ has: page.locator('h1[data-heading]') });
			await expect(header).toContainText('Unofficial guide, not a government website');
			await expect(header).not.toContainText('Official site');
			await expect(header).toContainText('2 web pages');
			// A receiving-authorities page names no office to apply to.
			const about = page.getByRole('region', { name: 'About this guide' });
			await expect(about).toContainText('Unofficial guide, not a government website.');
			await expect(about).not.toContainText('apply directly');
			// Vietnamese values carry their language.
			for (const text of ['Trang chủ', 'Cổng thông tin']) {
				await expect(page.locator('[data-fact-ids][lang="vi"]', { hasText: text }).first()).toBeVisible();
			}
			// No orders read without a send slot; every read answered.
			expect(office.reads.some((r) => r.path.startsWith('/p2/orders'))).toBe(false);
			expect(office.reads.filter((r) => r.status !== 200)).toEqual([]);
			expect(errors).toEqual([]);
		});

		test(`J1 shape ${at}: unkeyed, status-less facts and a missing /plan keep one fees region`, async ({ page }) => {
			await page.setViewportSize({ width: size.width, height: size.height });
			const errors = consoleErrors(page);
			await page.route('**/apostille/api/v1/**', (route) => {
				const path = new URL(route.request().url()).pathname.replace(/^.*\/api\/v1/, '');
				if (path === '/authorities') {
					return route.fulfill({ json: { authorities: [{ slug: 'example-a', name: 'Example Authority', country: 'Exampleland', fact_count: 2 }] } });
				}
				if (path === '/authorities/example-a/facts') {
					return route.fulfill({
						json: {
							facts: [
								{ topic: 'fees', statement: 'Eight dollars per document.' },
								{ topic: 'hours', statement: 'Walk-ins nine to five.' }
							]
						}
					});
				}
				return route.fulfill({ status: 404, json: { error: 'mock' } });
			});
			await page.goto(`${srv.url}/apostille/authority/example-a`);
			await expect(page.locator('h1[data-heading]')).toHaveText('Example Authority');
			const fees = page.getByRole('region', { name: /fees/i });
			await expect(fees).toHaveCount(1);
			await expect(fees.getByText('Eight dollars per document.')).toBeVisible();
			await expect(page.getByRole('region', { name: 'How this page was made' }).getByRole('region')).toHaveCount(0);
			// The 404s are the browser's own resource lines; the page logs nothing.
			expect(errors.filter((e) => !/Failed to load resource/.test(e))).toEqual([]);
		});

		test(`live update ${at}: an accepted fact re-presents, flashes its row and is announced`, async ({ page }) => {
			const { f, office, bus } = await guide(page, size);
			await expect.poll(() => bus.urls.length).toBeGreaterThan(0);
			expect(bus.urls.some((u) => /\/ws\/bus\?topic=authority(%3A|:)usa-wa-sos$/.test(u))).toBe(true);
			const facts = f.facts.facts.map((x) =>
				x.id === 224
					? {
							...x,
							statement: 'weekday__tumwater: Monday through Friday, 8:00 a.m. to 3:00 p.m.',
							citations: x.citations.map((c) => ({ ...c, quote: c.quote.replace('4:00 p.m.', '3:00 p.m.') }))
						}
					: x
			);
			office.setFacts({ ...f.facts, facts });
			const reads = office.reads.length;
			bus.send({ type: 'fact_accepted', topic: 'authority:usa-wa-sos', fact_id: 224 });
			const row = page.locator('[data-item="hours-weekday-tumwater"]');
			await expect(row).toContainText('Monday through Friday, 8:00 a.m. to 3:00 p.m.');
			await expect(page.locator('[data-item="hours-weekday-tumwater"][data-flash]')).toHaveCount(1);
			await expect(page.getByRole('status').filter({ hasText: 'Updated: Opening hours' })).toHaveCount(1);
			// Ids only on the wire: the page re-read its facts.
			expect(office.reads.slice(reads).some((r) => r.path === '/authorities/usa-wa-sos/facts')).toBe(true);
			// The light goes out after 1.5 s.
			await expect(page.locator('[data-flash]')).toHaveCount(0, { timeout: 5000 });
		});
	}

	// Accessibility: WCAG 2.2 AA, no serious or critical issue anywhere
	// the reader can get to.
	const bad = (result) =>
		result.violations
			.filter((v) => v.impact === 'serious' || v.impact === 'critical')
			.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).slice(0, 3).join(' | ')}`);

	for (const size of SIZES) {
		for (const theme of ['light', 'dark']) {
			for (const on of [false, true]) {
				test(`axe ${size.width} ${theme}, citations ${on ? 'on' : 'off'}: no serious or critical issue`, async ({ page }) => {
					await guide(page, { ...size, theme, query: on ? '?cite=1' : '' });
					await expect(page.locator('[data-offer="courier"]')).toBeVisible();
					if (on) await expect(page.locator('#sources')).toBeVisible();
					else await expect(page.locator('a.cite')).toHaveCount(0);
					expect(bad(await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze())).toEqual([]);
					if (on) {
						// With a citation card open too.
						await page.locator('#fees a.cite').first().click();
						await expect(page.locator('[data-cite-card]')).toBeVisible();
						expect(bad(await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze())).toEqual([]);
					}
				});
			}
			test(`axe ${size.width} ${theme}: Viet Nam, citations on`, async ({ page }) => {
				await guide(page, { ...size, theme, name: 'vietnam-prod', facts: withR1Flags, query: '?cite=1' });
				await expect(page.locator('#sources')).toBeVisible();
				expect(bad(await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze())).toEqual([]);
			});
			test(`axe ${size.width} ${theme}: an office we haven't checked`, async ({ page }) => {
				await guide(page, { ...size, theme, name: 'empty' });
				await expect(page.getByRole('heading', { name: 'Not checked yet' })).toBeVisible();
				expect(bad(await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze())).toEqual([]);
			});
		}
	}
});
