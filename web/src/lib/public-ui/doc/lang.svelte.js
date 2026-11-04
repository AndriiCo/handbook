// The guide's language: the shared site language (lib/locale.svelte.ts),
// so the masthead switcher re-renders the guide in place. Components
// read `guide.t` (the copy) and `guide.lang`; pure helpers take the
// language as an argument instead.

import { site } from '../../locale.svelte.js';
import { guideCopy } from './copy/index.js';

export const guide = {
	/** @returns {import('./copy/index.js').GuideLang} */
	get lang() {
		return site.locale === 'es' || site.locale === 'fr' ? site.locale : 'en';
	},
	get t() {
		return guideCopy(site.locale);
	}
};
