// Header-label strings only (EN/ES/FR). Day-1 stub: no dictionary chunks,
// no Accept-Language negotiation yet. See docs/site-theme.md §6.

export type Locale = 'en' | 'es' | 'fr';

export const LOCALES: Locale[] = ['en', 'es', 'fr'];
const STORE_KEY = 'locale';

export interface HeaderStrings {
	searchLabel: string;
	searchPlaceholder: string;
	searchButton: string;
	themeLabel: string;
	langLabel: string;
	orderLabel: string;
	billingLabel: string;
	loginLabel: string;
	logoutLabel: string;
}

export const STRINGS: Record<Locale, HeaderStrings> = {
	en: {
		searchLabel: 'Search the handbook',
		searchPlaceholder: 'Country, state, or authority…',
		searchButton: 'Search',
		themeLabel: 'Theme',
		langLabel: 'Language',
		orderLabel: 'Order',
		billingLabel: 'Billing',
		loginLabel: 'Log in',
		logoutLabel: 'Log out'
	},
	es: {
		searchLabel: 'Buscar en el manual',
		searchPlaceholder: 'País, estado o autoridad…',
		searchButton: 'Buscar',
		themeLabel: 'Tema',
		langLabel: 'Idioma',
		orderLabel: 'Pedido',
		billingLabel: 'Facturación',
		loginLabel: 'Iniciar sesión',
		logoutLabel: 'Cerrar sesión'
	},
	fr: {
		searchLabel: 'Rechercher dans le manuel',
		searchPlaceholder: 'Pays, État ou autorité…',
		searchButton: 'Rechercher',
		themeLabel: 'Thème',
		langLabel: 'Langue',
		orderLabel: 'Commande',
		billingLabel: 'Facturation',
		loginLabel: 'Se connecter',
		logoutLabel: 'Se déconnecter'
	}
};

function normalize(code: string | null | undefined): Locale | null {
	if (!code) return null;
	const base = code.toLowerCase().split('-')[0];
	return (LOCALES as string[]).includes(base) ? (base as Locale) : null;
}

/** Pure detection: `?l=` override wins, then stored choice, then browser. */
export function detectLocale(search: string, languages: readonly string[], stored: string | null): Locale {
	const override = normalize(new URLSearchParams(search).get('l'));
	if (override) return override;
	const saved = normalize(stored);
	if (saved) return saved;
	for (const lang of languages) {
		const match = normalize(lang);
		if (match) return match;
	}
	return 'en';
}

/** Browser binding: detect + persist the `?l=` override when present. */
export function resolveLocale(): Locale {
	const stored =
		typeof localStorage === 'undefined' ? null : localStorage.getItem(STORE_KEY);
	const locale = detectLocale(
		typeof location === 'undefined' ? '' : location.search,
		typeof navigator === 'undefined' ? [] : navigator.languages,
		stored
	);
	persistLocale(locale);
	return locale;
}

export function persistLocale(locale: Locale): void {
	try {
		localStorage.setItem(STORE_KEY, locale);
	} catch {
		// Private mode etc: locale just won't persist.
	}
}
