// The masthead language switcher (docs/site-theme.md §6) over the built
// SPA: `?l=` sets the page language on arrival, and picking another
// language re-renders the page in place (no reload) with `?l=` kept in
// the address. The login form and the authority guide are translated;
// the guide's values stay as the office's pages write them (tagged
// English), and its records view reads stored translations in the
// site language.
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

test('authority: ?l=es renders the guide in Spanish, values as the office writes them', async ({ page }) => {
	await page.clock.install({ time: NOW });
	const f = dossier('wa-guide');
	await mockGuide(page, f);
	await liveBus(page);
	const spanish = page.waitForRequest((r) => r.url().includes('/translations?lang=es'));
	await page.goto(`${srv.url}/apostille/authority/${f.slug}?l=es`);

	await expect(page.getByRole('heading', { level: 1, name: 'Obtener una apostilla en Washington' })).toBeVisible();
	await expect(page.getByRole('heading', { name: 'De un vistazo' })).toBeVisible();
	await expect(page.getByRole('heading', { name: /Pague las tarifas$/ })).toBeVisible();
	await expect(page.getByRole('heading', { name: /Envíe o lleve sus documentos$/ })).toBeVisible();
	await expect(page.getByRole('heading', { name: 'Cómo se hizo esta página' })).toBeVisible();
	await expect(page.getByText('Traducción pendiente')).toHaveCount(0);
	await expect(page.locator('[lang="en"] [data-guide]')).toHaveCount(0);
	// A value stays as the office's page writes it, tagged English.
	const payee = page.locator('[data-section="fees"] [data-fact-ids="214"]').first();
	await expect(payee).toHaveText('Secretary of State');
	await expect(payee).toHaveAttribute('lang', 'en');
	await spanish;

	// French in place, then English: one document, no reload.
	await page.evaluate(() => ((/** @type {any} */ (window)).__sameDocument = true));
	await page.getByRole('button', { name: 'Idioma: Español' }).click();
	await page.getByRole('menuitem', { name: 'Français' }).click();
	await expect(page.getByRole('heading', { level: 1, name: /^Obtenir une apostille\s:\sWashington$/ })).toBeVisible();
	await expect(page.getByRole('heading', { name: 'En bref' })).toBeVisible();
	await expect(page.getByRole('heading', { name: /Réglez les frais$/ })).toBeVisible();
	await expect(page.getByRole('heading', { name: /Envoyez ou déposez vos documents$/ })).toBeVisible();
	await expect(page.locator('html')).toHaveAttribute('lang', 'fr');
	await expect(payee).toHaveText('Secretary of State');

	await page.getByRole('button', { name: 'Langue: Français' }).click();
	await page.getByRole('menuitem', { name: 'English' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Get an apostille in Washington' })).toBeVisible();
	await expect(page.getByRole('heading', { name: /Pay the fees$/ })).toBeVisible();
	await expect(payee).not.toHaveAttribute('lang');
	expect(await page.evaluate(() => (/** @type {any} */ (window)).__sameDocument)).toBe(true);
});
