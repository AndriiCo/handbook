// The site language as one shared reactive value: the masthead switcher
// writes it and every page reads it, so a switch re-renders the page in
// place (no reload). Detected once when first imported (`?l=` > stored
// choice > browser, lib/i18n.ts); the app is client-only (ssr = false).

import { goto } from '$app/navigation';
import { LOCALES, persistLocale, resolveLocale, type Locale } from './i18n';

const state = $state<{ value: Locale }>({ value: resolveLocale() });

export const site = {
	get locale(): Locale {
		return state.value;
	}
};

/** `true` for the codes the switcher offers. */
export function isLocale(value: unknown): value is Locale {
	return typeof value === 'string' && (LOCALES as string[]).includes(value);
}

/**
 * Switch the site language: persist it and keep `?l=` in the address.
 * The address changes through the router (same page, no scroll or focus
 * reset) so `page.url` stays truthful: a raw `history.replaceState`
 * would leave it on the old `?l=`, and links built from it would carry
 * the old language back.
 */
export function setLocale(value: Locale): void {
	state.value = value;
	persistLocale(value);
	document.documentElement.lang = value;
	const url = new URL(location.href);
	if (url.searchParams.get('l') === value) return;
	url.searchParams.set('l', value);
	void goto(url, { replace: true, reset: false });
}
