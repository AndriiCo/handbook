// D8 "Print it" (plan §1 and §4 "Print follows the toggle", budget per
// §12): the Washington-shaped guide (wa-guide.json) printed to US
// Letter from the built SPA, with page.emulateMedia({ media: 'print' })
// for the DOM checks and page.pdf for the page count.
//
// - Citations off: no marker and no endnotes on paper.
// - Citations on: every marker points at an endnote that carries a URL
//   or the staff contact and its dates, and the markers' quotes equal
//   the endnotes' backlinks (one backlink per numbered quote, each
//   pointing back at a marker of that source).
// - Both: the letterhead (title, canonical address, captured/approved
//   dates, "Data as of", "Printed", the warning to check the live page;
//   no record ids), every critical value, nothing screen-only (masthead,
//   buttons, offers, "How this page was made"), light ink on white even
//   from a dark screen.
// - At most 4 Letter pages off and 8 on, counting the PDF's
//   `/Type /Page` objects.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { BUILD, NOW, dossier, liveBus, mockGuide, useTheme } from './helpers/guide.mjs';

const BUDGET = { off: 4, on: 8 };
// Critical values (the us_state profile's fee, payee, mailing address,
// standard time and form) and the courier street.
const CRITICAL = [210, 214, 216, 217, 218, 219, 236, 207];

/** Pages in a PDF: one `/Type /Page` object each (`/Type /Pages` is the tree). */
function pdfPages(bytes) {
	return (Buffer.from(bytes).toString('latin1').match(/\/Type\s*\/Page(?![A-Za-z])/g) ?? []).length;
}

/** sRGB relative luminance of a computed `rgb(...)` colour. */
function luminance(css) {
	const [r, g, b] = (css.match(/[\d.]+/g) ?? ['0', '0', '0']).slice(0, 3).map((v) => {
		const c = Number(v) / 255;
		return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
	});
	return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

test.describe('print (D8)', () => {
	let srv;
	test.beforeAll(async () => {
		srv = await serveDir(BUILD, { base: '/apostille' });
	});
	test.afterAll(async () => {
		await srv?.close();
	});

	/** The guide, settled (late offers read), then switched to print media. */
	async function printed(page, { cite, theme }) {
		await page.setViewportSize({ width: 1440, height: 900 });
		await page.clock.install({ time: NOW });
		await useTheme(page, theme);
		const f = dossier('wa-guide');
		await mockGuide(page, f);
		await liveBus(page);
		await page.goto(`${srv.url}/apostille/authority/${f.slug}?cite=${cite ? 1 : 0}`);
		await expect(page.locator('h1[data-heading]')).toBeVisible();
		// The offer read lands after first paint: wait, so paper provably drops it.
		await expect(page.locator('[data-offer="courier"]')).toBeVisible();
		await page.emulateMedia({ media: 'print' });
		const header = page.locator('[data-print-header]');
		await expect(header).toContainText('Check the live page before you send documents or money.');
		return { f, header };
	}

	/** What both settings print the same way. */
	async function paper(page, header) {
		// The letterhead: title, canonical address (no ?cite), dates; no record ids.
		await expect(header).toContainText('Get an apostille in Washington');
		await expect(header).toContainText(`${srv.url}/apostille/authority/usa/washington`);
		await expect(header).toContainText(/Sources captured .+2026/);
		await expect(header).toContainText(/Data as of .+ · Printed /);
		expect(await header.innerText()).not.toMatch(/#\d|\?cite/);
		// Every critical value prints.
		for (const id of CRITICAL) await expect(page.locator(`[data-fact-ids~="${id}"]`).first()).toBeVisible();
		// Screen-only things don't.
		await expect(page.locator('header.masthead')).toBeHidden();
		await expect(page.locator('[data-offer]')).toBeHidden();
		await expect(page.locator('#made')).toBeHidden();
		expect(await page.locator('main button').evaluateAll((els) => els.filter((b) => b.checkVisibility()).length)).toBe(0);
		// Light ink on white paper, whatever the screen's theme.
		const ink = await page.evaluate(() => ({
			bg: getComputedStyle(document.body).backgroundColor,
			h1: getComputedStyle(document.querySelector('h1')).color,
			value: getComputedStyle(document.querySelector('[data-fact-ids~="210"]')).color
		}));
		expect(ink.bg).toBe('rgb(255, 255, 255)');
		expect(luminance(ink.h1)).toBeLessThan(0.1);
		expect(luminance(ink.value)).toBeLessThan(0.1);
	}

	test('citations off: clean paper, critical values in ink, at most 4 Letter pages', async ({ page }) => {
		const { header } = await printed(page, { cite: false, theme: 'dark' });
		await expect(page.locator('a.cite')).toHaveCount(0);
		await expect(page.locator('#sources')).toHaveCount(0);
		await expect(header).not.toContainText('Numbers in brackets');
		await paper(page, header);
		const pages = pdfPages(await page.pdf({ format: 'Letter' }));
		expect(pages).toBeGreaterThan(0);
		expect(pages, `Letter pages with citations off (budget ${BUDGET.off})`).toBeLessThanOrEqual(BUDGET.off);
	});

	test('citations on: every marker has its endnote and backlink, at most 8 Letter pages', async ({ page }) => {
		const { header } = await printed(page, { cite: true, theme: 'light' });
		await expect(header).toContainText('Numbers in brackets point to the sources listed at the end.');
		await expect(page.locator('#sources')).toBeVisible();
		await expect(page.locator('a.cite').first()).toBeVisible();
		await paper(page, header);
		const audit = await page.evaluate(() => {
			const problems = [];
			const marks = new Set();
			const markers = [...document.querySelectorAll('a.cite[role="doc-noteref"]')];
			for (const m of markers) {
				const n = m.getAttribute('data-cite-n');
				if (m.getAttribute('href') !== `#src-${n}` || !document.getElementById(`src-${n}`)) {
					problems.push(`marker ${m.id} has no endnote`);
				}
				if (!m.checkVisibility()) problems.push(`marker ${m.id} does not print`);
				for (const l of String(m.getAttribute('data-cite-letters') ?? '').split(/\s+/).filter(Boolean)) marks.add(`${n}${l}`);
			}
			const backs = new Set();
			const notes = [...document.querySelectorAll('#sources li[id^="src-"]')];
			for (const li of notes) {
				const n = li.id.slice(4);
				for (const a of li.querySelectorAll('a[role="doc-backlink"]')) {
					const letter = a.parentElement?.querySelector('span')?.textContent?.trim() ?? '';
					backs.add(`${n}${letter}`);
					const target = document.querySelector(a.getAttribute('href') ?? '#none');
					if (!target?.matches('a.cite') || target.getAttribute('data-cite-n') !== n) {
						problems.push(`backlink ${n}${letter} points at no marker of source ${n}`);
					}
				}
				// A URL or the staff contact, and its dates; a calculation
				// names the numbered values it reads.
				const head = li.querySelector('p')?.textContent ?? '';
				const text = li.textContent ?? '';
				if (/^\s*Calculated on this page/.test(head)) {
					if (!/\[\d+[a-z]\]/.test(text)) problems.push(`endnote ${n}: a calculation without its inputs`);
				} else if (/https:\/\//.test(head)) {
					if (!/Captured .+20\d\d|approved .+20\d\d/i.test(head)) problems.push(`endnote ${n}: a page without dates`);
				} else if (/Phone call|Email|Visit|Staff note|Not found/.test(head)) {
					if (!/20\d\d/.test(head)) problems.push(`endnote ${n}: a staff note without its day`);
				} else problems.push(`endnote ${n}: no URL or contact ("${head.trim().slice(0, 60)}")`);
			}
			return { markers: markers.length, notes: notes.length, marks: [...marks].sort(), backs: [...backs].sort(), problems };
		});
		expect(audit.markers).toBeGreaterThan(0);
		expect(audit.notes).toBeGreaterThan(0);
		expect(audit.backs.length).toBeGreaterThan(0);
		expect(audit.problems).toEqual([]);
		expect(audit.marks).toEqual(audit.backs);
		const pages = pdfPages(await page.pdf({ format: 'Letter' }));
		expect(pages).toBeGreaterThan(0);
		expect(pages, `Letter pages with citations on (budget ${BUDGET.on})`).toBeLessThanOrEqual(BUDGET.on);
	});
});
