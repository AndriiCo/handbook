// "Quote a page" on /review: a deep link opens one capture's text; Find
// selects the span (whitespace-flexible, any case) and hands focus to
// the text; the topic, a key chip and the value (the selection, by
// default) make the statement; Propose posts the quote with its UTF-8
// byte offsets into the sidecar (CRLF breaks and multibyte characters
// ahead of it, so they differ from the textarea's indices) and the
// form clears for the next span. Prepared quotes post one at a time
// without offsets, each row with its own answer. API and bus are
// route-mocked with synthetic values; the page is the built admin SPA.
import { expect, test } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { serveDir } from './helpers/serve.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const authorities = JSON.parse(readFileSync(join(HERE, 'fixtures', 'admin-authorities.json'), 'utf8'));

const SLUG = 'usa-wa-sos';
const TEXT =
	'Corporations & Charities — Apostilles (synthetic)\r\n' +
	'Café counter: Town Center 3, Tumwater 😀\r\n' +
	'Office hours: Monday through Friday,\r\n8 a.m. to 5 p.m.\r\n' +
	'Fee: $15 per document.\r\n' +
	'Expedite fee: $15 per document.\r\n';
const QUOTE = 'Monday through Friday,\r\n8 a.m. to 5 p.m.';
const VALUE = 'Monday through Friday, 8 a.m. to 5 p.m.';
const BANNED = ['placeholder', 'not wired', 'skeleton', 'lorem', 'todo', 'coming soon'];

const at = (path) => (url) => url.pathname === `/apostille/api/v1${path}`;

/**
 * Mocks the review page's reads and the three quote routes; returns
 * the recorded state. `answer` maps a POST body to the reply (default
 * 201 with the next fact id).
 */
async function mockQuote(page, { answer } = {}) {
	const state = { posts: [], inFlight: 0, maxInFlight: 0, errors: [], nextId: 512 };
	page.on('console', (msg) => {
		if (msg.type() === 'error') state.errors.push(msg.text());
	});
	page.on('pageerror', (err) => state.errors.push(err.message));
	await page.routeWebSocket('**/ws/bus*', (ws) => {
		ws.send(JSON.stringify({ type: 'subscribed', topic: new URL(ws.url()).searchParams.get('topic') ?? '' }));
	});
	// Fallback first: Playwright runs the newest matching route first.
	await page.route('**/apostille/api/v1/**', (route) => route.fulfill({ json: {} }));
	await page.route(at('/me'), (route) => route.fulfill({ json: { user_id: 'staff-1', role: 'staff' } }));
	await page.route(at('/admin/authorities'), (route) => route.fulfill({ json: authorities }));
	await page.route(at('/queue'), (route) => route.fulfill({ json: { items: [], count: 0 } }));
	await page.route(at(`/admin/authorities/${SLUG}/captures`), (route) =>
		route.fulfill({
			json: {
				slug: SLUG,
				captures: [
					{ id: 130, source_id: 12, uri: 'https://sos.example/apostille/hours', kind: 'warc', trust: 'official', captured_at: 1793700000, bytes: 27298 },
					{ id: 118, source_id: 12, uri: 'https://sos.example/', kind: 'warc', trust: 'official', captured_at: 1793000000, bytes: 940 }
				]
			}
		})
	);
	await page.route(/\/apostille\/api\/v1\/admin\/captures\/\d+\/text$/, (route) => {
		const id = Number(new URL(route.request().url()).pathname.split('/').at(-2));
		if (id !== 130) return route.fulfill({ status: 404, json: { error: 'capture not found' } });
		return route.fulfill({
			json: { id, slug: SLUG, uri: 'https://sos.example/apostille/hours', kind: 'warc', trust: 'official', captured_at: 1793700000, text: TEXT }
		});
	});
	await page.route(/\/apostille\/api\/v1\/admin\/captures\/\d+\/quote$/, async (route) => {
		const req = route.request();
		const id = Number(new URL(req.url()).pathname.split('/').at(-2));
		const body = req.postDataJSON();
		state.posts.push({ id, body, csrf: await req.headerValue('x-csrf') });
		state.inFlight += 1;
		state.maxInFlight = Math.max(state.maxInFlight, state.inFlight);
		await new Promise((resolve) => setTimeout(resolve, 150));
		state.inFlight -= 1;
		const reply = answer?.(body) ?? {
			status: 201,
			json: { fact_id: state.nextId++, status: 'proposed', capture_id: id, slug: SLUG, ...body }
		};
		return route.fulfill(reply);
	});
	return state;
}

/** No banned copy, no native select, no console error but the `expected` ones. */
async function clean(page, state, expected = []) {
	const text = (await page.evaluate(() => document.body.innerText)).toLowerCase();
	for (const word of BANNED) expect(text, `banned "${word}"`).not.toContain(word);
	await expect(page.locator('select')).toHaveCount(0);
	expect(state.errors.filter((e) => !expected.some((re) => re.test(e)))).toEqual([]);
}

test('find a span, key it and propose it with byte offsets into the sidecar', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const state = await mockQuote(page);
		await page.goto(`${srv.url}/review?tab=quote&slug=${SLUG}&capture=130`);
		await expect(page.getByRole('tab', { name: 'Quote a page' })).toHaveAttribute('aria-selected', 'true');
		await expect(page.getByRole('button', { name: /\/apostille\/hours/ })).toHaveAttribute('aria-current', 'true');
		const text = page.getByRole('textbox', { name: 'Capture text', exact: true });
		// The textarea holds the CRLF breaks as LF.
		await expect(text).toHaveValue(TEXT.replaceAll('\r\n', '\n'));

		const find = page.getByRole('searchbox', { name: 'Find in the text' });
		await find.fill('monday THROUGH friday, 8 a.m. to 5 p.m.');
		await find.press('Enter');
		await expect(page.locator('#quote-find-status')).toHaveText('1 of 1');
		await expect(text).toBeFocused();

		const form = page.getByRole('complementary', { name: 'Propose a fact' });
		const key = form.getByRole('textbox', { name: 'Key', exact: true });
		const value = form.getByRole('textbox', { name: 'Value', exact: true });
		await expect(form.locator('blockquote')).toHaveText(VALUE);
		await expect(form.getByText(`${[...QUOTE].length} characters`)).toBeVisible();
		await expect(form.getByText('Propose is off: Pick a topic.')).toBeVisible();

		await form.getByRole('combobox', { name: /Topic/ }).click();
		await page.getByRole('option', { name: /^hours/ }).click();
		await form.getByRole('button', { name: 'weekday', exact: true }).click();
		await expect(key).toHaveValue('weekday');
		await key.fill('weekday__tumwater');
		// The value starts as the selection, on one line.
		await expect(value).toHaveValue(VALUE);
		await expect(form.getByText('The value is in the quote.')).toBeVisible();
		await expect(form.getByText(`weekday__tumwater: ${VALUE}`, { exact: true })).toBeVisible();

		const violations = (await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze()).violations
			.filter((v) => v.impact === 'serious' || v.impact === 'critical')
			.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`);
		expect(violations).toEqual([]);

		await form.getByRole('button', { name: 'Propose', exact: true }).click();
		await expect(page.getByText('Proposed #512 — accept it on the Facts tab')).toBeVisible();
		await expect(form.getByRole('status').filter({ hasText: 'Proposed #512.' })).toBeVisible();
		const from = TEXT.indexOf(QUOTE);
		expect(state.posts).toEqual([
			{
				id: 130,
				csrf: expect.stringMatching(/\S{16,}/),
				body: {
					quote: QUOTE,
					start: Buffer.byteLength(TEXT.slice(0, from)),
					end: Buffer.byteLength(TEXT.slice(0, from + QUOTE.length)),
					topic: 'hours',
					statement: `weekday__tumwater: ${VALUE}`
				}
			}
		]);
		// The byte offsets are not the UTF-16 indices: the em dash, é and the emoji come first.
		expect(state.posts[0].body.start).toBeGreaterThan(from);

		// The capture stays open for the next span; key and value clear.
		await expect(key).toHaveValue('');
		await expect(value).toHaveValue('');
		await expect(text).toHaveValue(TEXT.replaceAll('\r\n', '\n'));
		await expect(page).toHaveURL(/[?&]capture=130/);
		await clean(page, state);
	} finally {
		await srv.close();
	}
});

test('prepared quotes post one at a time without offsets, each with its answer', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		const state = await mockQuote(page, {
			answer: (body) =>
				body.topic === 'payment'
					? { status: 409, json: { error: 'this statement is already on file', fact_id: 88, status: 'accepted' } }
					: body.topic === 'fees'
						? {
								status: 409,
								json: { error: 'quote appears 2 times in this capture; select the one you mean in the text', count: 2 }
							}
						: undefined
		});
		await page.goto(`${srv.url}/review?tab=quote`);
		await expect(page.getByText('Pick an authority to list its captured pages.')).toBeVisible();
		await page.getByRole('button', { name: 'Prepared quotes' }).click();
		const lines = [
			{ capture_id: 130, quote: 'Town Center 3, Tumwater', topic: 'address', statement: 'office__tumwater: Town Center 3, Tumwater' },
			{ capture_id: 130, quote: 'Corporations & Charities', topic: 'payment', statement: 'payee: Corporations & Charities' },
			{ capture_id: 130, quote: '$15 per document', topic: 'fees', statement: 'per_document: $15 per document' }
		];
		await page.getByLabel('JSON Lines').fill([...lines.map((l) => JSON.stringify(l)), '{"capture_id": 130,'].join('\n'));
		const rows = page.getByRole('list', { name: 'Prepared rows' }).getByRole('listitem');
		await expect(rows).toHaveCount(4);
		await expect(rows.nth(3)).toContainText('Line 4: not JSON');

		await page.getByRole('button', { name: 'Propose all ready rows (3)' }).click();
		await expect(page.getByText('1 of 3 proposed — accept them on the Facts tab.')).toBeVisible();
		await expect(rows.nth(0)).toContainText('Proposed #512.');
		await expect(rows.nth(1)).toContainText('Already on file as #88 (accepted).');
		await expect(rows.nth(2)).toContainText('Appears 2 times in this capture: select the one you mean in the text.');
		expect(state.posts.map((p) => p.body)).toEqual(lines.map(({ capture_id, ...body }) => body));
		expect(state.maxInFlight).toBe(1);

		// The ambiguous row opens its capture with the form filled and Find on the quote.
		await rows.nth(2).getByRole('button', { name: 'Find in the text' }).click();
		await expect(page.locator('#quote-find-status')).toHaveText('1 of 2');
		const form = page.getByRole('complementary', { name: 'Propose a fact' });
		await expect(form.getByRole('textbox', { name: 'Key', exact: true })).toHaveValue('per_document');
		await expect(form.getByRole('textbox', { name: 'Value', exact: true })).toHaveValue('$15 per document');
		await expect(form.getByRole('combobox', { name: /Topic/ })).toContainText('fees');
		await clean(page, state, [/status of 409/]);
	} finally {
		await srv.close();
	}
});
