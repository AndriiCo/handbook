// W-3 fake modes in the staff console: the FAKE ribbon names every
// integration running its local fake (from /admin/version) on every
// screen; the Outbox shows mail kept instead of sent; Compose answers a
// kept outreach email as the office, threaded on our Message-ID, with
// the DMARC result the composer picks. API is route-mocked.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const FAKES = [
	{ slug: 'mail_out', label: 'Outgoing email' },
	{ slug: 'mail_in', label: 'Incoming email' },
	{ slug: 'labels', label: 'FedEx/USPS labels' }
];
const OUTBOX = {
	outbox: true,
	inbox: true,
	messages: [
		{
			id: 2,
			at: 1_793_750_100,
			kind: 'outreach',
			to: 'apostille@example.gov',
			subject: 'Two questions',
			headers: { 'Message-ID': '<outreach-7-ab12@andrii.ai>', 'Reply-To': 'notices@example.com' },
			body: 'Hello,\n\n1. What is the fee per document?\n\nThanks.'
		},
		{
			id: 1,
			at: 1_793_750_000,
			kind: 'mail',
			to: 'customer@example.test',
			subject: 'Your sign-in link',
			headers: {},
			body: 'Sign in: http://localhost:28418/apostille/login#token=abc'
		}
	]
};

/** Signed-in staff shell with the given `fakes` on the version badge. */
async function mockShell(page, fakes) {
	await page.route('**/apostille/api/v1/setup/status', (route) =>
		route.fulfill({ json: { setup_required: false } })
	);
	await page.route('**/apostille/api/v1/me', (route) =>
		route.fulfill({ json: { user_id: 'staff-1', role: 'staff' } })
	);
	await page.route('**/apostille/api/v1/admin/version', (route) =>
		route.fulfill({
			json: { version: '0.1.0+test', env: 'judge', started_at: 1, schema: 13, p2_schema: 2, fakes }
		})
	);
}

test('the FAKE ribbon names every fake integration', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await mockShell(page, FAKES);
		await page.goto(`${srv.url}/dashboard`);
		const ribbon = page.getByTestId('fake-ribbon');
		await expect(ribbon).toBeVisible();
		await expect(ribbon).toContainText('FAKE — Outgoing email, Incoming email, FedEx/USPS labels.');
		await expect(ribbon).toContainText('nothing here sends mail, ships a parcel or charges a card');
		await expect(page.getByRole('link', { name: 'Outbox' })).toBeVisible();
		await expect(page.getByRole('link', { name: 'Compose' })).toBeVisible();
	} finally {
		await srv.close();
	}
});

test('no fakes, no ribbon and no fake screens in the nav', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await mockShell(page, []);
		await page.goto(`${srv.url}/dashboard`);
		await expect(page.getByText('judge', { exact: true })).toBeVisible();
		await expect(page.getByTestId('fake-ribbon')).toHaveCount(0);
		await expect(page.getByRole('link', { name: 'Email' })).toBeVisible();
		await expect(page.getByRole('link', { name: 'Outbox' })).toHaveCount(0);
		await expect(page.getByRole('link', { name: 'Compose' })).toHaveCount(0);
	} finally {
		await srv.close();
	}
});

test('the outbox shows kept mail, and a reply composed from it threads with its DMARC result', async ({
	page
}) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	/** @type {unknown[]} */
	const delivered = [];
	try {
		await mockShell(page, FAKES);
		await page.route('**/apostille/api/v1/admin/fake/outbox', (route) => route.fulfill({ json: OUTBOX }));
		await page.route('**/apostille/api/v1/admin/fake/inbox', (route) => {
			delivered.push({ body: route.request().postDataJSON(), csrf: route.request().headers()['x-csrf'] });
			return route.fulfill({
				status: 201,
				json: {
					captured: true,
					capture_id: 42,
					trust: 'official',
					verified_by: 'dmarc',
					authority: 'synthetic-ny-dos',
					outreach_email: 7
				}
			});
		});
		await page.goto(`${srv.url}/outbox`);
		await expect(page.getByRole('heading', { name: 'Outbox' })).toBeVisible();
		const kept = page.getByTestId('outbox-message');
		await expect(kept).toHaveCount(2);
		await expect(kept.first()).toContainText('Two questions');
		await expect(kept.first()).toContainText('never sent');
		await expect(kept.first()).toContainText('<outreach-7-ab12@andrii.ai>');
		await expect(kept.nth(1)).toContainText('Sign-in link');
		await expect(kept.nth(1)).toContainText('http://localhost:28418/apostille/login#token=abc');

		await page.getByRole('link', { name: 'Reply as apostille@example.gov' }).click();
		await expect(page.getByRole('heading', { name: 'Compose' })).toBeVisible();
		await expect(page.getByLabel('From')).toHaveValue('apostille@example.gov');
		await expect(page.getByLabel('Subject')).toHaveValue('Re: Two questions');
		await expect(page.getByLabel('In reply to (optional)')).toHaveValue('<outreach-7-ab12@andrii.ai>');
		// The client refuses an empty body before anything is sent.
		await page.getByRole('button', { name: 'Deliver to the inbox' }).click();
		await expect(page.getByText('Write the message body.')).toBeVisible();
		expect(delivered).toHaveLength(0);

		await page.getByLabel('Message', { exact: true }).fill('The fee is $15 per document.');
		await page.getByLabel('Fail').check();
		await page.getByLabel('Pass').check();
		await page.getByRole('button', { name: 'Deliver to the inbox' }).click();
		await expect(page.getByTestId('compose-result')).toContainText(
			'Filed under synthetic-ny-dos with official trust (authenticated by dmarc).'
		);
		await expect(page.getByTestId('compose-result')).toContainText('Threaded to outreach email #7.');
		expect(delivered).toEqual([
			{
				body: {
					from: 'apostille@example.gov',
					subject: 'Re: Two questions',
					body: 'The fee is $15 per document.',
					in_reply_to: '<outreach-7-ab12@andrii.ai>',
					dmarc: 'pass',
					attachments: []
				},
				csrf: expect.any(String)
			}
		]);
	} finally {
		await srv.close();
	}
});

test('compose is disabled while incoming email is not fake', async ({ page }) => {
	const srv = await serveDir(join(HERE, '..', 'build-admin'));
	try {
		await mockShell(page, [{ slug: 'mail_out', label: 'Outgoing email' }]);
		await page.route('**/apostille/api/v1/admin/fake/outbox', (route) =>
			route.fulfill({ json: { ...OUTBOX, inbox: false } })
		);
		await page.goto(`${srv.url}/compose`);
		await expect(page.getByText('Incoming email is not in fake mode')).toBeVisible();
		await expect(page.getByRole('button', { name: 'Deliver to the inbox' })).toBeDisabled();
	} finally {
		await srv.close();
	}
});
