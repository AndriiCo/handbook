// The guide's copy in the three site languages (lib/i18n.ts LOCALES).
// en.js is the source; es.js and fr.js translate it key for key. Pure:
// node tests import it directly.

import en from './en.js';
import es from './es.js';
import fr from './fr.js';

/** @typedef {typeof en} GuideCopy */
/** @typedef {'en' | 'es' | 'fr'} GuideLang */

/** @type {Record<GuideLang, GuideCopy>} */
export const GUIDE_COPY = { en, es, fr };

/**
 * The copy for a language; English for anything else.
 * @param {unknown} lang
 * @returns {GuideCopy}
 */
export function guideCopy(lang) {
	return lang === 'es' || lang === 'fr' ? GUIDE_COPY[lang] : en;
}
