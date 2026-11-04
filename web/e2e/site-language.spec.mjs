// The masthead language switcher (docs/site-theme.md §6) over the built
// SPA: `?l=` sets the page language on arrival, and picking another
// language re-renders the page in place (no reload) with `?l=` kept in
// the address. The login form is fully translated; the authority guide
// is English with a translation-pending notice in Spanish or French,
// and its records view reads stored translations in that language.
import { expect, test } from '@playwright/test';
import { serveDir } from './helpers/serve.mjs';
import { BUILD, NOW, dossier, liveBus, mockGuide } from './helpers/guide.mjs';

/** @type {{ url: string, close: () => Promise<void> }} */
let srv;

test.beforeAll(async () => {
	srv = await serveDir(BUILD, { base: '/apostille' });
});
test.afterAll(async () => {
	await srv?.close();
});

test('login: ?l=fr on arrival, then Español in place', async ({ page }) => {
	await page.goto(`${srv.url}/apostille/login?l=fr`);
	await expect(page.getByLabel('Identifiant ou adresse e-mail')).toBeVisible();
	await expect(page.getByRole('button', { name: 'Se connecter avec une clé d’accès' })).toBeVisible();

	// A reload would lose this marker: the switch must not reload.
	await page.evaluate(() => ((/** @type {any} */ (window)).__sameDocument = true));
	await page.getByRole('button', { name: 'Langue: Français' }).click();
	await page.getByRole('menuitem', { name: 'Español' }).click();

	await expect(page.getByLabel('ID de usuario o correo electrónico')).toBeVisible();
	await expect(page.getByRole('tab', { name: 'Crear cuenta' })).toBeVisible();
	await expect(page).toHaveURL(/[?&]l=es\b/);
	await expect(page.locator('html')).toHaveAttribute('lang', 'es');
	expect(await page.evaluate(() => (/** @type {any} */ (window)).__sameDocument)).toBe(true);

	await page.getByRole('tab', { name: 'Crear cuenta' }).click();
	await expect(page.getByLabel('Nombre completo')).toBeVisible();
	await expect(page.getByRole('button', { name: 'Registrar llave de acceso' })).toBeVisible();
});

test('authority: ?l=es shows the pending notice and asks for Spanish records', async ({ page }) => {
	await page.clock.install({ time: NOW });
	const f = dossier('wa-guide');
	await mockGuide(page, f);
	await liveBus(page);
	const spanish = page.waitForRequest((r) => r.url().includes('/translations?lang=es'));
	await page.goto(`${srv.url}/apostille/authority/${f.slug}?l=es`);

	await expect(page.getByText('Traducción pendiente: se muestra el original en inglés.')).toBeVisible();
	await expect(page.locator('[lang="en"] [data-guide]')).toBeVisible();
	await spanish;

	// English: no notice, no records-language request needed.
	await page.getByRole('button', { name: 'Idioma: Español' }).click();
	await page.getByRole('menuitem', { name: 'English' }).click();
	await expect(page.getByText('Traducción pendiente')).toHaveCount(0);
	await expect(page.locator('[data-guide]')).toBeVisible();
});
