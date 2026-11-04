// Machine-translation chrome (SA-20): static es/fr catalogs for the
// dossier's translated records view and the TranslationChip. The view
// follows the site language (masthead switcher, `?l=`); an older
// `?lang=es|fr` link still opens it. No model calls here —
// the page reads stored guard-passing rows from
// GET /api/v1/authorities/{slug}/translations?lang=es|fr and falls back
// to the English original whenever a row is missing. Pure: no
// window/fetch, so node tests drive it directly.

/** Languages with stored machine translations. */
export const TRANSLATE_LANGS = ['es', 'fr'];

/** Chrome strings per language (English + the two served targets). */
export const CHROME = {
	en: {
		langName: 'English',
		readIn: 'Read in',
		machineNote: 'Machine translation',
		seeOriginal: 'See original',
		showTranslation: 'Show translation',
		translatedFrom: 'Translated from English',
		modelBy: 'by',
		unavailable: 'No translation yet — showing the original.'
	},
	es: {
		langName: 'Español',
		readIn: 'Leer en',
		machineNote: 'Traducción automática',
		seeOriginal: 'Ver original',
		showTranslation: 'Ver traducción',
		translatedFrom: 'Traducido del inglés',
		modelBy: 'por',
		unavailable: 'Aún no hay traducción — se muestra el original.'
	},
	fr: {
		langName: 'Français',
		readIn: 'Lire en',
		machineNote: 'Traduction automatique',
		seeOriginal: 'Voir l’original',
		showTranslation: 'Voir la traduction',
		translatedFrom: 'Traduit de l’anglais',
		modelBy: 'par',
		unavailable: 'Pas encore de traduction — l’original est affiché.'
	}
};

/**
 * The authority page's own words (states around the guide; the guide's
 * own are doc/copy).
 * @type {Record<'en' | 'es' | 'fr', {
 *   unknownHeading: (address: string) => string,
 *   unknownBody: string,
 *   backToIndex: string,
 *   unavailableHeading: string,
 *   unavailableBody: string,
 *   loading: string,
 *   updated: string
 * }>}
 */
export const PAGE_COPY = {
	en: {
		unknownHeading: (address) => `Unknown authority “${address}”.`,
		unknownBody: 'This address is not in the handbook index.',
		backToIndex: 'Back to the authority index',
		unavailableHeading: 'This guide is unavailable.',
		unavailableBody: 'Could not load this guide — reload the page to try again.',
		loading: 'Loading the guide',
		updated: 'Updated'
	},
	es: {
		unknownHeading: (address) => `Autoridad desconocida «${address}».`,
		unknownBody: 'Esta dirección no figura en el índice del manual.',
		backToIndex: 'Volver al índice de autoridades',
		unavailableHeading: 'Esta guía no está disponible.',
		unavailableBody: 'No se pudo cargar esta guía: vuelva a cargar la página para intentarlo de nuevo.',
		loading: 'Cargando la guía',
		updated: 'Actualizado'
	},
	fr: {
		unknownHeading: (address) => `Autorité inconnue « ${address} ».`,
		unknownBody: 'Cette adresse ne figure pas dans l’index du manuel.',
		backToIndex: 'Retour à l’index des autorités',
		unavailableHeading: 'Ce guide n’est pas disponible.',
		unavailableBody: 'Impossible de charger ce guide : rechargez la page pour réessayer.',
		loading: 'Chargement du guide',
		updated: 'Mis à jour'
	}
};

/**
 * `true` for the two served translation languages.
 * @param {unknown} value
 */
export function isSupportedLang(value) {
	return value === 'es' || value === 'fr';
}

/**
 * Read `?lang=` from a query string (`location.search` shape).
 * Returns `'es'`, `'fr'`, or `null` (English).
 * @param {string} search
 * @returns {'es' | 'fr' | null}
 */
export function parseLangParam(search) {
	const q = String(search ?? '');
	const m = /[?&]lang=(es|fr)(?:&|$)/.exec(q);
	if (!m) return null;
	return m[1] === 'fr' ? 'fr' : 'es';
}

/**
 * Dossier URL with the site language chosen (`null` is English). It
 * sets `?l=`, the masthead switcher's parameter, so picking a language
 * here switches the whole site. Keeps the path; drops any other query.
 * @param {string} pathname
 * @param {string | null} lang
 */
export function translateHref(pathname, lang) {
	return `${String(pathname ?? '')}?l=${isSupportedLang(lang) ? lang : 'en'}`;
}

/**
 * The records view's language: the site language, unless the address
 * carries only an older `?lang=es|fr` (no `?l=`), which still wins.
 * @param {string} search `location.search` shape
 * @param {string} siteLocale
 * @returns {'es' | 'fr' | null}
 */
export function recordsLang(search, siteLocale) {
	const legacy = parseLangParam(search);
	if (legacy && !new URLSearchParams(String(search ?? '')).has('l')) return legacy;
	return isSupportedLang(siteLocale) ? /** @type {'es' | 'fr'} */ (siteLocale) : null;
}

/**
 * Index stored translation rows by fact id (first row wins).
 * @param {unknown} rows
 */
export function indexTranslations(rows) {
	/** @type {Record<number, { statement: string, model: string }>} */
	const map = {};
	for (const row of Array.isArray(rows) ? rows : []) {
		if (typeof row?.fact_id !== 'number' || typeof row?.statement !== 'string') continue;
		if (map[row.fact_id]) continue;
		map[row.fact_id] = { statement: row.statement, model: String(row?.model ?? '') };
	}
	return map;
}
