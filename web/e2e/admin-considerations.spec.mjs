// Considerations and retire on /review (plan §5.4, §5.5): the Published
// tab lists an office's accepted facts; each drawer shows the staff
// notes about the fact (Withdraw needs a reason), the "Add a
// consideration" form (channel prefilled from the office's accepted
// phone, live summary count, the server's 422 reason inline, a decision
// picks the preferred fact among the conflicting ones and rests on
// published notes), and "Retire this fact…" behind a reason and a
// confirmation. `evidence_updated` / `fact_retired` on the office's
// channel re-read the lists. The queue drawer keeps its one-click
// Accept where it was; its considerations open on demand. API and bus
// are route-mocked with synthetic values; the page is the built admin SPA.
import { expect, test } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { serveDir } from './helpers/serve.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const fixture = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', name), 'utf8'));
const authorities = fixture('admin-authorities.json');
const queue = fixture('admin-queue.json');

const SLUG = 'usa-wa-sos';
const TOPIC = `authority:${SLUG}`;
const LINE =
	'Notes are public on the guide. Write what the office said; never names or personal numbers; calls are not recorded.';
// Mon 2026-11-02 09:10 Pacific (the browser runs on Los Angeles time).
const NOV_02_0910 = 1793639400;
const BANNED = ['placeholder', 'not wired', 'skeleton', 'lorem', 'todo', 'coming soon'];

test.use({ timezoneId: 'America/Los_Angeles' });

const at = (path) => (url) => url.pathname === `/apostille/api/v1${path}`;

function fact(id, topic, statement, quote, conflicts = []) {
	return {
		id,
		authority_slug: SLUG,
		topic,
		statement,
		status: 'accepted',
		created_at: 1793600000,
		accepted_at: 1793700000,
		citations: [
			{
				quote,
				source_url: 'https://sos.example/apostille/contact',
				captured_at: '2026-11-02T17:00:00Z',
				kind: 'web',
				capture_id: 11
			}
		],
		conflict: conflicts.length > 0,
		conflicts
	};
}

const BOX_A = 'apostille_mailing_address: PO Box 40220, Olympia, WA 98504-0220';
const BOX_B = 'apostille_mailing_address: PO Box 40234, Olympia, WA 98504-0234';

function publishedFacts() {
	return [
		fact(201, 'mailing', BOX_A, 'Mail requests to PO Box 40220, Olympia, WA 98504-0220.', [
			{ id: 202, topic: 'mailing', statement: BOX_B, status: 'accepted' }
		]),
		fact(202, 'mailing', BOX_B, 'Apostille requests go to PO Box 40234, Olympia, WA 98504-0234.', [
			{ id: 201, topic: 'mailing', statement: BOX_A, status: 'accepted' }
		]),
		fact(203, 'contacts', 'phone: (360) 555-0142', 'Call (360) 555-0142 for apostille questions.'),
		fact(204, 'contacts', 'email: apostille@sos.example', 'Write to apostille@sos.example.'),
		fact(205, 'fees', 'per_document: $15 per document', 'The fee is $15 per document.')
	];
}

function note(id, fields) {
	return {
		id,
		authority_slug: SLUG,
		kind: 'call',
		fact_id: null,
		preferred_fact_id: null,
		slot: '',
		stance: 'context',
		occurred_at: NOV_02_0910,
		date: '2026-11-02',
		channel: '',
		summary: '',
		recorded: false,
		capture_id: null,
		supports: [],
		status: 'published',
		created_by: 'staff-1',
		created_at: NOV_02_0910 + 60,
		...fields
	};
}

function seedNotes() {
	return [
		note(31, {
			kind: 'call',
			fact_id: 202,
			slot: 'mailing.usps',
			stance: 'confirms',
			channel: '(360) 555-0142 option 3',
			summary: 'The desk said PO Box 40234 takes apostille mail (synthetic).'
		}),
		note(32, {
			kind: 'absence',
			slot: 'fees.same_day',
			stance: 'context',
			channel: 'FAQ and fee pages',
			summary: 'Neither page names a same-day fee (synthetic).'
		})
	];
}

/**
 * Mocks every read the review page makes; returns the recorded state.
 * `create` answers POST /admin/evidence (default: 201 with the note).
 * `eventFirst` sends a retire's `fact_retired` before its response, as
 * the server may (it publishes, then answers).
 */
async function mockReview(page, { facts = publishedFacts(), notes = seedNotes(), create, eventFirst = false } = {}) {
	const state = {
		facts,
		notes,
		posts: [],
		withdrawals: [],
		retires: [],
		noteReads: [],
		factReads: [],
		sockets: new Map(),
		errors: [],
		nextId: 40
	};
	page.on('console', (msg) => {
		if (msg.type() === 'error') state.errors.push(msg.text());
	});
	page.on('pageerror', (err) => state.errors.push(err.message));
	await page.routeWebSocket('**/ws/bus*', (ws) => {
		const topic = new URL(ws.url()).searchParams.get('topic') ?? '';
		state.sockets.set(topic, ws);
		ws.send(JSON.stringify({ type: 'subscribed', topic }));
	});
	// Fallback first: Playwright runs the newest matching route first.
	await page.route('**/apostille/api/v1/**', (route) => route.fulfill({ json: {} }));
	await page.route(at('/me'), (route) => route.fulfill({ json: { user_id: 'staff-1', role: 'staff' } }));
	await page.route(at('/admin/authorities'), (route) => route.fulfill({ json: authorities }));
	await page.route(at('/queue'), (route) => route.fulfill({ json: queue }));
	await page.route(at('/facts'), (route) => {
		const url = new URL(route.request().url());
		if (url.searchParams.get('status') !== 'accepted') return route.fulfill({ json: { facts: [], count: 0 } });
		const slug = url.searchParams.get('authority');
		state.factReads.push(slug);
		const rows = state.facts.filter((f) => !slug || f.authority_slug === slug);
		return route.fulfill({ json: { facts: rows, count: rows.length } });
	});
	await page.route(at('/admin/evidence'), async (route) => {
		const req = route.request();
		if (req.method() === 'POST') {
			const body = req.postDataJSON();
			state.posts.push({ body, csrf: await req.headerValue('x-csrf') });
			const answer = create?.(body, state) ?? {
				status: 201,
				json: note(state.nextId++, { ...body, date: '2026-11-02', status: 'published' })
			};
			if (answer.status === 201) state.notes.unshift(answer.json);
			return route.fulfill(answer);
		}
		const url = new URL(req.url());
		const slug = url.searchParams.get('slug');
		state.noteReads.push(slug);
		const rows = state.notes.filter((n) => n.authority_slug === slug && n.status === 'published');
		return route.fulfill({ json: { notes: rows, count: rows.length } });
	});
	await page.route(/\/apostille\/api\/v1\/admin\/evidence\/\d+\/withdraw$/, async (route) => {
		const id = Number(new URL(route.request().url()).pathname.split('/').at(-2));
		state.withdrawals.push({ id, body: route.request().postDataJSON(), csrf: await route.request().headerValue('x-csrf') });
		const row = state.notes.find((n) => n.id === id);
		row.status = 'withdrawn';
		return route.fulfill({ json: row });
	});
	await page.route(/\/apostille\/api\/v1\/admin\/facts\/\d+\/retire$/, async (route) => {
		const id = Number(new URL(route.request().url()).pathname.split('/').at(-2));
		state.retires.push({ id, body: route.request().postDataJSON(), csrf: await route.request().headerValue('x-csrf') });
		state.facts = state.facts.filter((f) => f.id !== id);
		if (eventFirst) {
			state.sockets.get(TOPIC)?.send(JSON.stringify({ type: 'fact_retired', topic: TOPIC, fact_id: id, at: 3 }));
			await new Promise((resolve) => setTimeout(resolve, 300));
		}
		return route.fulfill({ json: { id, authority_slug: SLUG, status: 'superseded' } });
	});
	return state;
}

/** Open the Published tab on fact `id`; returns its drawer. */
async function openPublished(page, srv, id) {
	await page.goto(`${srv.url}/review?tab=published&authority=${SLUG}`);
	await expect(page.getByRole('tab', { name: 'Published' })).toHaveAttribute('aria-selected', 'true');
	await page.getByRole('button', { name: new RegExp(`^#${id} · `) }).click();
	const drawer = page.getByRole('complementary', { name: 'Selected published fact' });
	await expect(drawer.getByRole('heading', { name: `Fact #${id}` })).toBeVisible();
	return drawer;
}

/** No banned copy, no native select, no console error but the `expected` ones. */
async function clean(page, state, expected = []) {
	const text = (await page.evaluate(() => document.body.innerText)).toLowerCase();
	for (const word of BANNED) expect(text, `banned "${word}"`).not.toContain(word);
	await expect(page.locator('select')).toHaveCount(0);
	expect(state.errors.filter((e) => !expected.some((re) => re.test(e)))).toEqual([]);
}

test('publish a call note: notes for the fact, prefilled channel, live count, list re-read', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const state = await mockReview(page);
		const drawer = await openPublished(page, srv, 202);
		const notes = drawer.getByRole('list', { name: 'Published notes' });
		await expect(notes.getByText('The desk said PO Box 40234 takes apostille mail (synthetic).')).toBeVisible();
		// The office's same-day absence is about another field.
		await expect(drawer.getByText('Neither page names a same-day fee (synthetic).')).toHaveCount(0);

		await drawer.getByRole('button', { name: 'Add a consideration' }).click();
		await expect(drawer.getByText(LINE, { exact: true })).toBeVisible();
		const form = drawer.getByRole('form', { name: 'Add a consideration' });
		await expect(form.getByRole('combobox', { name: /Kind/ })).toContainText('Phone call');
		await expect(form.getByText('This fact (#202)')).toBeVisible();
		// From the office's accepted phone fact (#203).
		const channel = form.getByLabel('Phone line');
		await expect(channel).toHaveValue('(360) 555-0142');
		await channel.fill('(360) 555-0142 option 3');
		const publish = form.getByRole('button', { name: 'Publish note' });
		await expect(publish).toBeDisabled();
		await expect(form.getByText('Pick a stance.')).toBeVisible();
		await form.getByRole('radio', { name: 'Confirms' }).check();
		await form.getByLabel('Date and time').fill('2026-11-02T09:10');
		const summary = 'The desk confirmed PO Box 40234 for apostille mail.';
		await form.getByLabel('Summary').fill('Too few');
		await expect(form.getByText('7/280 characters · at least 8')).toBeVisible();
		await expect(publish).toBeDisabled();
		await form.getByLabel('Summary').fill(summary);
		await expect(form.getByText(`${summary.length}/280 characters`, { exact: true })).toBeVisible();
		await publish.click();

		await expect(drawer.getByText('Note #40 published.')).toBeVisible();
		await expect(notes.getByText(summary)).toBeVisible();
		expect(state.posts).toHaveLength(1);
		expect(state.posts[0].body).toEqual({
			authority_slug: SLUG,
			kind: 'call',
			stance: 'confirms',
			occurred_at: NOV_02_0910,
			channel: '(360) 555-0142 option 3',
			summary,
			fact_id: 202,
			slot: 'mailing.usps'
		});
		expect(state.posts[0].csrf).toBeTruthy();
		// Read once on open and once after the publish.
		expect(state.noteReads.length).toBeGreaterThanOrEqual(2);
		await clean(page, state);
	} finally {
		await srv.close();
	}
});

test("a 422 refusal shows the server's reason inline and keeps the text", async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const reason = "summary has the number 2065550199, which is not in this office's accepted facts";
		const state = await mockReview(page, { create: () => ({ status: 422, json: { error: reason } }) });
		const drawer = await openPublished(page, srv, 202);
		await drawer.getByRole('button', { name: 'Add a consideration' }).click();
		const form = drawer.getByRole('form', { name: 'Add a consideration' });
		await form.getByRole('radio', { name: 'Context' }).check();
		await form.getByLabel('Date and time').fill('2026-11-02T09:10');
		const summary = 'The clerk said to call her direct line, 206-555-0199, next time.';
		await form.getByLabel('Summary').fill(summary);
		await form.getByRole('button', { name: 'Publish note' }).click();

		await expect(form.getByRole('alert')).toHaveText(`Not published: ${reason}`);
		await expect(form.getByLabel('Summary')).toHaveValue(summary);
		await expect(drawer.getByText(/Note #\d+ published\./)).toHaveCount(0);
		expect(state.posts).toHaveLength(1);
		expect(state.notes.map((n) => n.id)).toEqual([31, 32]);
		// The browser logs the refused POST itself; the page logs nothing.
		await clean(page, state, [/status of 422/]);
	} finally {
		await srv.close();
	}
});

test('withdraw needs a reason, then the note leaves the list', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const state = await mockReview(page);
		const drawer = await openPublished(page, srv, 202);
		await drawer.getByRole('button', { name: 'Withdraw… note #31' }).click();
		const confirm = drawer.getByRole('button', { name: 'Withdraw note #31' });
		await expect(confirm).toBeDisabled();
		await drawer.getByLabel('Withdraw reason (required)').fill('The desk called back: the box is not settled (synthetic).');
		await confirm.click();

		await expect(drawer.getByText('No published notes for this fact yet.')).toBeVisible();
		expect(state.withdrawals).toEqual([
			{ id: 31, body: { reason: 'The desk called back: the box is not settled (synthetic).' }, csrf: expect.any(String) }
		]);
		await clean(page, state);
	} finally {
		await srv.close();
	}
});

test('a decision picks the preferred fact among the conflicting ones and rests on notes', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const state = await mockReview(page);
		const drawer = await openPublished(page, srv, 202);
		await drawer.getByRole('button', { name: 'Add a consideration' }).click();
		const form = drawer.getByRole('form', { name: 'Add a consideration' });
		await form.getByRole('combobox', { name: /Kind/ }).click();
		await page.getByRole('option', { name: 'Staff decision' }).click();

		const preferred = form.getByRole('group', { name: 'Preferred fact' });
		await expect(preferred.getByRole('radio')).toHaveCount(2);
		await expect(preferred.getByRole('radio', { name: /#202/ })).toBeChecked();
		await expect(preferred.getByRole('radio', { name: /#201/ })).not.toBeChecked();
		// Decisions rest on call, email, visit or absence notes only.
		const basis = form.getByRole('group', { name: 'Rests on these published notes' });
		await expect(basis.getByRole('checkbox')).toHaveCount(2);
		await expect(form.getByLabel(/Phone line/)).toHaveCount(0);
		await basis.getByRole('checkbox', { name: /#31/ }).check();
		await form.getByRole('radio', { name: 'Confirms' }).check();
		await form.getByLabel('Date and time').fill('2026-11-02T09:40');
		await form.getByLabel('Summary').fill('Use PO Box 40234 until the office says otherwise.');
		await form.getByRole('button', { name: 'Publish note' }).click();

		await expect(drawer.getByText('Note #40 published.')).toBeVisible();
		expect(state.posts[0].body).toEqual({
			authority_slug: SLUG,
			kind: 'decision',
			stance: 'confirms',
			occurred_at: NOV_02_0910 + 30 * 60,
			channel: '',
			summary: 'Use PO Box 40234 until the office says otherwise.',
			slot: 'mailing.usps',
			preferred_fact_id: 202,
			supports: [31]
		});
		await expect(drawer.getByText('Prefers fact #202 · rests on #31')).toBeVisible();
		await clean(page, state);
	} finally {
		await srv.close();
	}
});

test('retire asks for a reason and a confirmation, then the row reads Retired', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const state = await mockReview(page, { eventFirst: true });
		const drawer = await openPublished(page, srv, 202);
		await expect.poll(() => state.sockets.has(TOPIC)).toBe(true);
		await drawer.getByRole('button', { name: /Retire this fact/ }).click();
		const dialog = page.getByRole('dialog');
		await expect(dialog.getByRole('heading', { name: 'Retire fact #202?' })).toBeVisible();
		await expect(dialog).toContainText('disappears from the public guide');
		const confirm = dialog.getByRole('button', { name: 'Retire fact #202' });
		await expect(confirm).toBeDisabled();
		await dialog.getByLabel('Reason (required)').fill('short');
		await expect(dialog.getByText('5/500 characters · at least 8')).toBeVisible();
		await expect(confirm).toBeDisabled();
		const reason = 'Accepted from the wrong page; the box is not the office’s (synthetic).';
		await dialog.getByLabel('Reason (required)').fill(reason);
		await confirm.click();

		await expect(dialog).toHaveCount(0);
		expect(state.retires).toEqual([{ id: 202, body: { reason }, csrf: expect.any(String) }]);
		const row = page.getByRole('row').filter({ has: page.getByRole('button', { name: /^#202 · / }) });
		await expect(row.getByText('Retired', { exact: true }).first()).toBeVisible();
		await expect(row.getByText(reason)).toBeVisible();
		await expect(drawer.getByText('It no longer shows on the public guide.', { exact: false })).toBeVisible();
		await expect(drawer.getByRole('button', { name: /Retire this fact/ })).toHaveCount(0);
		// Its own `fact_retired` (here ahead of the response) is no news.
		await expect(drawer.getByRole('heading', { name: 'Fact #202' })).toBeVisible();
		await expect(page.getByText(/was retired elsewhere/)).toHaveCount(0);
		await clean(page, state);
	} finally {
		await srv.close();
	}
});

test('evidence_updated and fact_retired on the office channel re-read the lists', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const state = await mockReview(page);
		const drawer = await openPublished(page, srv, 202);
		await expect(drawer.getByRole('list', { name: 'Published notes' })).toBeVisible();
		await expect.poll(() => state.sockets.has(TOPIC)).toBe(true);

		state.notes.unshift(
			note(41, {
				kind: 'email',
				fact_id: 201,
				slot: 'mailing.usps',
				stance: 'contradicts',
				channel: 'apostille@sos.example',
				summary: 'The office wrote that PO Box 40220 is for other filings (synthetic).'
			})
		);
		state.sockets.get(TOPIC).send(JSON.stringify({ type: 'evidence_updated', topic: TOPIC, note_id: 41, at: 1 }));
		await expect(drawer.getByText('The office wrote that PO Box 40220 is for other filings (synthetic).')).toBeVisible();

		state.facts = state.facts.filter((f) => f.id !== 201);
		state.sockets.get(TOPIC).send(JSON.stringify({ type: 'fact_retired', topic: TOPIC, fact_id: 201, at: 2 }));
		await expect(page.getByText('Fact #201 was retired elsewhere; the list is up to date.')).toBeVisible();
		await expect(page.getByRole('button', { name: /^#201 · / })).toHaveCount(0);
		// The note on #201 stays staff-visible and says it left the guide.
		await expect(drawer.getByText('About fact #201, which is no longer accepted', { exact: false })).toBeVisible();
		await clean(page, state);
	} finally {
		await srv.close();
	}
});

test('the queue drawer keeps its one-click Accept; considerations open on demand', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		// Queue #9 (riskiest: it changes live fact #41 of vietnam).
		const live = {
			...fact(41, 'processing_time', 'Legalization takes 5 working days from receipt.', 'Results within 5 working days.'),
			authority_slug: 'vietnam'
		};
		const state = await mockReview(page, { facts: [live, ...publishedFacts()] });
		await page.route(at('/facts/9/accept'), (route) => route.fulfill({ json: { status: 'accepted' } }));
		await page.goto(`${srv.url}/review`);
		const pane = page.getByRole('complementary', { name: 'Selected fact' });
		await expect(pane.getByRole('heading', { name: 'Fact #9' })).toBeVisible();
		const accept = page.getByRole('button', { name: 'Accept', exact: true });
		await expect(accept).toHaveCount(1);
		const toggle = pane.getByRole('button', { name: 'Considerations' });
		await expect(toggle).toHaveAttribute('aria-expanded', 'false');
		// Accept stays above the panel, and a closed panel reads nothing.
		const acceptFirst = await page.evaluate(() => {
			const buttons = [...document.querySelectorAll('aside button')];
			const a = buttons.find((b) => b.textContent?.trim() === 'Accept');
			const c = buttons.find((b) => b.textContent?.trim() === 'Considerations');
			return !!a && !!c && (a.compareDocumentPosition(c) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
		});
		expect(acceptFirst).toBe(true);
		expect(state.noteReads).toEqual([]);
		expect(state.factReads).toEqual([]);

		await toggle.click();
		await expect(toggle).toHaveAttribute('aria-expanded', 'true');
		await expect(pane.getByText('No published notes for this fact yet.')).toBeVisible();
		expect(state.noteReads).toContain('vietnam');
		await pane.getByRole('button', { name: 'Add a consideration' }).click();
		await expect(pane.getByText(LINE, { exact: true })).toBeVisible();
		// A proposal is not public yet: a call is about the live fact it would change.
		await expect(pane.getByRole('combobox', { name: /About this accepted fact/ })).toContainText('#41');

		await accept.click();
		await expect(page.getByText('Fact #9 decided: accepted.')).toBeVisible();
		await clean(page, state);
	} finally {
		await srv.close();
	}
});

for (const theme of ['light', 'dark']) {
	test(`axe finds nothing serious in the drawers with the forms open (${theme})`, async ({ page }) => {
		await page.emulateMedia({ colorScheme: theme });
		await page.addInitScript((t) => {
			try {
				localStorage.setItem('theme', t);
			} catch {}
		}, theme);
		const srv = await serveDir(join(HERE, '..', 'build-admin'));
		try {
			const state = await mockReview(page);
			const drawer = await openPublished(page, srv, 202);
			await drawer.getByRole('button', { name: 'Withdraw… note #31' }).click();
			await drawer.getByRole('button', { name: 'Add a consideration' }).click();
			await expect(drawer.getByRole('form', { name: 'Add a consideration' })).toBeVisible();
			const scan = async () =>
				(await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze()).violations
					.filter((v) => v.impact === 'serious' || v.impact === 'critical')
					.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`);
			expect(await scan()).toEqual([]);

			await page.getByRole('tab', { name: /^Facts/ }).click();
			const pane = page.getByRole('complementary', { name: 'Selected fact' });
			await pane.getByRole('button', { name: 'Considerations' }).click();
			await pane.getByRole('button', { name: 'Add a consideration' }).click();
			await expect(pane.getByRole('form', { name: 'Add a consideration' })).toBeVisible();
			expect(await scan()).toEqual([]);
			await clean(page, state);
		} finally {
			await srv.close();
		}
	});
}

test('the published drawer fits a phone screen with the form open', async ({ page }) => {
	await page.setViewportSize({ width: 390, height: 844 });
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const state = await mockReview(page);
		const drawer = await openPublished(page, srv, 202);
		await drawer.getByRole('button', { name: 'Add a consideration' }).click();
		await expect(drawer.getByRole('form', { name: 'Add a consideration' })).toBeVisible();
		const overflow = await page.evaluate(
			() => document.documentElement.scrollWidth - document.documentElement.clientWidth
		);
		expect(overflow).toBeLessThanOrEqual(1);
		await clean(page, state);
	} finally {
		await srv.close();
	}
});
