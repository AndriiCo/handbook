// GP2-page v2 in CI (plan §7): the scoreboard's live row
// (xtask/ops/scoreboard.mjs gp2Page, operator-approved Nov 3) run
// against the built SPA and Viet Nam's production reads, before
// staging. Same predicate (xtask/ops/gp2-page.mjs gp2PageProblems, fed
// the probe's own text), same pages (/authority/vietnam at 1440x900 and
// 390x844, light), same wait (load + 1.5 s), the same clean() checks
// (HTTP 200, no console errors, no banned words, no horizontal
// overflow), the h1 rule and the live socket. The scoreboard is not
// edited; what it does not export (visit, clean, BANNED) is mirrored
// here line for line.
//
// Plan §7 also asks of the guide's reads: each one answers 200, none
// leaves the page's origin, and no orders call without a send slot.
//
// Viet Nam is read twice: as production served it before R1 (topic-level
// conflict flags) and with R1's field-level flags (compare.js, the
// server rule's twin), so the row holds on either side of R1's deploy.
import { expect, test } from '@playwright/test';
import { gp2PageProblems, probeValueNodes } from '../../xtask/ops/gp2-page.mjs';
import { serveDir } from './helpers/serve.mjs';
import { BUILD, consoleErrors, dossier, liveBus, mockGuide, withR1Flags } from './helpers/guide.mjs';

const MIN_FACTS = 8;
// scoreboard.mjs BANNED, verbatim.
const BANNED = /\b(placeholder|not wired|skeleton|lorem ipsum|todo|coming soon)\b/i;

/** scoreboard.mjs clean(), verbatim but for its inputs. */
function clean(v, label) {
	const p = [];
	if (v.status !== 200) p.push(`${label}: HTTP ${v.status}`);
	if (v.errors.length) p.push(`${label}: ${v.errors.length} console error(s), first: ${v.errors[0]}`);
	const banned = v.text.match(BANNED);
	if (banned) p.push(`${label}: visible "${banned[0]}"`);
	if (v.overflow > 0) p.push(`${label}: ${v.overflow}px horizontal overflow`);
	return p;
}

test.describe('GP2-page v2 (CI port)', () => {
	let srv;
	test.beforeAll(async () => {
		srv = await serveDir(BUILD, { base: '/apostille' });
	});
	test.afterAll(async () => {
		await srv?.close();
	});

	for (const r1 of [false, true]) {
		test(`/authority/vietnam, ${r1 ? "R1's field-level conflict flags" : 'as production served it before R1'}`, async ({ browser }) => {
			const f = dossier('vietnam-prod');
			const facts = r1 ? withR1Flags(f.facts.facts) : f.facts.facts;
			const body = { ...f.facts, facts };
			const problems = [];
			const add = (list) => {
				for (const p of list) if (!problems.includes(p)) problems.push(p);
			};
			const seen = [];
			for (const [width, height] of [
				[1440, 900],
				[390, 844]
			]) {
				const label = String(width);
				const ctx = await browser.newContext({ viewport: { width, height }, colorScheme: 'light' });
				const page = await ctx.newPage();
				const errors = consoleErrors(page);
				const requests = [];
				page.on('request', (r) => requests.push(r.url()));
				const office = await mockGuide(page, { ...f, facts: body });
				const bus = await liveBus(page);
				try {
					// scoreboard.mjs visit(): load, then 1.5 s.
					const res = await page.goto(`${srv.url}/apostille/authority/vietnam`, { waitUntil: 'load', timeout: 20_000 });
					await page.waitForTimeout(1500);
					const dom = await page.evaluate(() => ({
						text: document.body.innerText,
						overflow: document.documentElement.scrollWidth - window.innerWidth
					}));
					add(clean({ status: res?.status() ?? 0, errors, ...dom }, label));
					if (width === 1440) {
						const h1 = (await page.locator('h1').first().innerText().catch(() => '')).trim();
						if (h1.length < 4 || h1.toLowerCase() === 'vietnam') add([`h1 is "${h1}" (want the authority's name)`]);
					}
					// The probe renders content-visibility:auto subtrees
					// first, so its text (not the visit's) is what the
					// raw-key rule reads.
					const probe = await page.evaluate(probeValueNodes);
					const out = gp2PageProblems({ label, probe, text: probe.text, facts, min: MIN_FACTS });
					add(out.problems);
					seen.push(`${out.shown}/${out.total} at ${label}`);
					expect(out.want, 'Viet Nam has enough public facts for the row to mean something').toBe(MIN_FACTS);
					if (width === 1440) {
						const live = bus.urls.some((u) => /\/ws\/bus/.test(u) && /authority(%3A|:)vietnam/i.test(u));
						if (!live) add(['no WebSocket to /ws/bus?topic=authority:vietnam']);
					}
					// Plan §7: every read answers 200, none leaves the origin,
					// and a page with no send slot asks for no order options.
					const failed = office.reads.filter((r) => r.status !== 200).map((r) => `${r.path} -> ${r.status}`);
					if (failed.length) add([`${label}: reads not answered 200: ${failed.join(', ')}`]);
					const foreign = requests.filter((u) => !u.startsWith(`${srv.url}/`) && !/^(data|blob):/.test(u));
					if (foreign.length) add([`${label}: cross-origin request(s): ${foreign.slice(0, 3).join(', ')}`]);
					if (office.reads.some((r) => r.path.startsWith('/p2/orders'))) add([`${label}: orders read without a send slot`]);
					if (!office.reads.some((r) => r.path === '/authorities/vietnam/facts')) add([`${label}: /facts never read`]);
				} finally {
					await ctx.close();
				}
			}
			expect(problems, `facts on visible values ${seen.join(', ')}`).toEqual([]);
		});
	}
});
