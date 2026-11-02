// Machine-translation chrome (SA-20): static es/fr catalogs for the
// dossier `?lang` view and the TranslationChip. No model calls here —
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
 * Dossier URL with the language chosen (`null` clears back to English).
 * Keeps the path; drops any other query (the dossier has none).
 * @param {string} pathname
 * @param {string | null} lang
 */
export function translateHref(pathname, lang) {
	if (!isSupportedLang(lang)) return String(pathname ?? '');
	return `${pathname}?lang=${lang}`;
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
