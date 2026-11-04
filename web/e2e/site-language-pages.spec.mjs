// The rest of the site in the site language (docs/site-theme.md §6),
// over the built SPA with no API: the masthead, footer and home page
// arrive in French from `?l=fr`, and a switch to Spanish re-renders them
// in place. The security page's signed-out state is translated too.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { BUILD } from './helpers/guide.mjs';

/** @type {{ url: string, close: () => Promise<void> }} */
let srv;

test.beforeAll(async () => {
	srv = await serveDir(BUILD, { base: '/apostille' });
});
test.afterAll(async () => {
	await srv?.close();
});

test('home: French masthead, hero and footer; Spanish in place', async ({ page }) => {
	await page.goto(`${srv.url}/apostille/?l=fr`);
	await expect(page.getByRole('heading', { level: 1 })).toContainText('Les règles de l’apostille');
	await expect(page.getByRole('link', { name: 'Aller au contenu' })).toBeAttached();
	await expect(page.getByText('information générale, pas un conseil juridique')).toBeVisible();
	await expect(page.getByRole('navigation', { name: 'Pied de page' })).toBeVisible();

	await page.getByRole('button', { name: 'Langue: Français' }).click();
	await page.getByRole('menuitem', { name: 'Español' }).click();
	await expect(page.getByText('información general, no asesoramiento jurídico')).toBeVisible();
	await expect(page.getByRole('navigation', { name: 'Pie de página' })).toBeVisible();
	await expect(page.getByRole('heading', { level: 1 })).not.toContainText('Les règles');
});

test('security: the signed-out state in Spanish', async ({ page }) => {
	await page.goto(`${srv.url}/apostille/security?l=es`);
	await expect(page.getByRole('heading', { level: 1 })).toHaveText('Métodos de inicio de sesión');
	await expect(page.getByRole('link', { name: 'Ir a iniciar sesión' })).toBeVisible();
});

test('login: the model certificate names its fields in French', async ({ page }) => {
	await page.goto(`${srv.url}/apostille/login?l=fr`);
	const certificate = page.locator('[data-graphic="certificate"]');
	await expect(certificate).toHaveAttribute('aria-label', /Modèle de certificat/);
	await expect(certificate.getByText('Signé par')).toBeAttached();
});
