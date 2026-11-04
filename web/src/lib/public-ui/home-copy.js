// The home page's words (and the Hague map caption it shows) in the
// three site languages (lib/i18n.ts LOCALES). The page picks one set by
// the shared site locale (lib/locale.svelte.ts), so the masthead switcher
// re-renders it in place. Authority names, countries, regions and fact
// values come from the API and are never translated here.

/** @typedef {'en' | 'es' | 'fr'} Locale */

/** BCP 47 tag for `Intl` per site language. */
export const INTL_TAG = { en: 'en-US', es: 'es', fr: 'fr' };

/**
 * A count in the locale's digits (English keeps the bare number, as
 * before).
 * @param {number} n
 * @param {Locale} locale
 */
export function num(n, locale) {
	return locale === 'en' ? String(n) : n.toLocaleString(INTL_TAG[locale]);
}

/**
 * A capture date for display: English keeps the ISO day (`2026-11-03`);
 * Spanish and French read it as a medium date in that language. Only the
 * day is used (formatted in UTC) so the zone never shifts it.
 * @param {string} iso
 * @param {Locale} locale
 */
export function shortDate(iso, locale) {
	const day = String(iso ?? '').split('T')[0] || '';
	if (locale === 'en' || !/^\d{4}-\d{2}-\d{2}$/.test(day)) return day;
	const [y, m, d] = day.split('-').map(Number);
	return new Intl.DateTimeFormat(INTL_TAG[locale], { dateStyle: 'medium', timeZone: 'UTC' }).format(
		new Date(Date.UTC(y, m - 1, d))
	);
}

/**
 * @typedef {{
 *   headTitle: string,
 *   metaDescription: string,
 *   evalLine: (pct: number) => string,
 *   heroA: string,
 *   heroEm: string,
 *   heroB: string,
 *   intro: string,
 *   searchLabel: string,
 *   searchPlaceholder: string,
 *   matchingLabel: string,
 *   noMatch: (query: string) => string,
 *   featuredLabel: string,
 *   featuredDossier: (n: number) => string,
 *   flagOf: (country: string) => string,
 *   howEyebrow: string,
 *   howTitle: string,
 *   steps: { model: string, title: string, text: string }[],
 *   featuredEyebrow: string,
 *   featuredTitle: string,
 *   featuredUnavailable: string,
 *   lastVerified: (date: string) => string,
 *   verifiedAgo: (ago: string) => string,
 *   openDossier: string,
 *   indexEyebrow: string,
 *   indexTitle: string,
 *   filterLabel: string,
 *   all: string,
 *   lanes: { hague: string, legalization: string, states: string, regions: string, other: string },
 *   kinds: { hague: string, states: string, legalization: string },
 *   searchUnavailable: string,
 *   loadingIndex: string,
 *   noOthers: string,
 *   noInGroup: string,
 *   newAppear: string,
 *   tryAll: string,
 *   startFeatured: string,
 *   matchCount: (n: number) => string,
 *   matchCap: (cap: number, n: number) => string,
 *   matchingQuery: (query: string) => string,
 *   indexSummary: (authorities: number, countries: number) => string,
 *   byCountry: string,
 *   authorityCount: (n: number) => string,
 *   whichOffice: (country: string) => string,
 *   international: string
 * }} HomeCopy
 */

/** @type {Record<Locale, HomeCopy>} */
export const HOME_COPY = {
	en: {
		headTitle: 'Apostille Handbook — Start with your destination',
		metaDescription: 'Find apostille and authentication requirements by destination authority.',
		evalLine: (pct) => `Eval: ${pct}% of Muse Spark drafts quote the source verbatim`,
		heroA: 'Apostille rules,',
		heroEm: 'quoted',
		heroB: 'from the source.',
		intro:
			"Every requirement comes from the office's own pages. On each guide, citations are off by default: turn them on with Alt+C or the Citations button to see the quote, source and date.",
		searchLabel: 'Search the handbook',
		searchPlaceholder: 'Country, state, or authority…',
		matchingLabel: 'Matching authorities',
		noMatch: (query) => `No authority matches “${query}”.`,
		featuredLabel: 'Featured authorities',
		featuredDossier: (n) => `Featured dossier ${n}`,
		flagOf: (country) => `${country} flag`,
		howEyebrow: 'How it works',
		howTitle: 'Three checks before anything is published',
		steps: [
			{
				model: 'Muse Spark',
				title: 'Spark drafts from official pages',
				text: 'Every fact starts as a quote copied from a watched government page — never from memory.'
			},
			{
				model: 'Quote check',
				title: 'The quote is checked byte-exact',
				text: 'A refused or missing quote goes back to the model once; unverifiable drafts never publish.'
			},
			{
				model: 'Human review',
				title: 'Staff approve, the dossier updates live',
				text: 'Only accepted facts reach the public dossier, seconds after approval.'
			}
		],
		featuredEyebrow: 'Featured',
		featuredTitle: 'Start with a destination',
		featuredUnavailable: 'Featured destinations are unavailable.',
		lastVerified: (date) => `Last verified ${date}`,
		verifiedAgo: (ago) => `verified ${ago}`,
		openDossier: 'Open the dossier',
		indexEyebrow: 'Index',
		indexTitle: 'All authorities',
		filterLabel: 'Filter authorities',
		all: 'All',
		lanes: {
			hague: 'Hague members',
			legalization: 'Legalization',
			states: 'US states',
			regions: 'States & provinces',
			other: 'Destinations'
		},
		kinds: { hague: 'Hague member', states: 'US state', legalization: 'Legalization' },
		searchUnavailable: 'Search is unavailable.',
		loadingIndex: 'Loading the authority index',
		noOthers: 'No other authorities yet.',
		noInGroup: 'No authorities in this group yet.',
		newAppear: 'New destinations appear here as staff verify them.',
		tryAll: 'Try the “All” filter to see every destination.',
		startFeatured: 'Start with a featured dossier:',
		matchCount: (n) => `${n} ${n === 1 ? 'match' : 'matches'}`,
		matchCap: (cap, n) => `Showing ${cap} of ${n} matches — refine the search.`,
		matchingQuery: (query) => `Authorities matching “${query}”`,
		indexSummary: (authorities, countries) =>
			`${authorities} authorities in ${countries} ${countries === 1 ? 'country' : 'countries'}. Open a country to see its authorities.`,
		byCountry: 'Authorities by country',
		authorityCount: (n) => `${n} ${n === 1 ? 'authority' : 'authorities'}`,
		whichOffice: (country) => `Which office handles my document? All offices in ${country}`,
		international: 'International'
	},
	es: {
		headTitle: 'Apostille Handbook — Empiece por su destino',
		metaDescription:
			'Encuentre los requisitos de apostilla y legalización según la autoridad de destino.',
		evalLine: (pct) =>
			`Evaluación: el ${num(pct, 'es')} % de los borradores de Muse Spark cita la fuente textualmente`,
		heroA: 'Las reglas de la apostilla,',
		heroEm: 'citadas',
		heroB: 'de la fuente.',
		intro:
			'Cada requisito procede de las páginas de la propia oficina. En cada guía, las citas están ocultas por defecto: actívelas con Alt+C o con el botón Citas para ver la cita, la fuente y la fecha.',
		searchLabel: 'Buscar en el manual',
		searchPlaceholder: 'País, estado o autoridad…',
		matchingLabel: 'Autoridades que coinciden',
		noMatch: (query) => `Ninguna autoridad coincide con «${query}».`,
		featuredLabel: 'Autoridades destacadas',
		featuredDossier: (n) => `Expediente destacado ${num(n, 'es')}`,
		flagOf: (country) => `Bandera de ${country}`,
		howEyebrow: 'Cómo funciona',
		howTitle: 'Tres comprobaciones antes de publicar nada',
		steps: [
			{
				model: 'Muse Spark',
				title: 'Spark redacta a partir de páginas oficiales',
				text: 'Cada dato parte de una cita copiada de una página oficial vigilada, nunca de memoria.'
			},
			{
				model: 'Comprobación de la cita',
				title: 'La cita se comprueba byte a byte',
				text: 'Una cita rechazada o ausente vuelve una vez al modelo; los borradores no verificables nunca se publican.'
			},
			{
				model: 'Revisión humana',
				title: 'El personal aprueba y el expediente se actualiza al instante',
				text: 'Solo los datos aceptados llegan al expediente público, segundos después de su aprobación.'
			}
		],
		featuredEyebrow: 'Destacados',
		featuredTitle: 'Empiece por un destino',
		featuredUnavailable: 'Los destinos destacados no están disponibles.',
		lastVerified: (date) => `Última verificación: ${date}`,
		verifiedAgo: (ago) => `verificado ${ago}`,
		openDossier: 'Abrir el expediente',
		indexEyebrow: 'Índice',
		indexTitle: 'Todas las autoridades',
		filterLabel: 'Filtrar autoridades',
		all: 'Todas',
		lanes: {
			hague: 'Miembros de La Haya',
			legalization: 'Legalización',
			states: 'Estados de EE. UU.',
			regions: 'Estados y provincias',
			other: 'Destinos'
		},
		kinds: { hague: 'Miembro de La Haya', states: 'Estado de EE. UU.', legalization: 'Legalización' },
		searchUnavailable: 'La búsqueda no está disponible.',
		loadingIndex: 'Cargando el índice de autoridades',
		noOthers: 'Todavía no hay otras autoridades.',
		noInGroup: 'Todavía no hay autoridades en este grupo.',
		newAppear: 'Los nuevos destinos aparecen aquí a medida que el personal los verifica.',
		tryAll: 'Pruebe el filtro «Todas» para ver todos los destinos.',
		startFeatured: 'Empiece por un expediente destacado:',
		matchCount: (n) => `${num(n, 'es')} ${n === 1 ? 'coincidencia' : 'coincidencias'}`,
		matchCap: (cap, n) =>
			`Se muestran ${num(cap, 'es')} de ${num(n, 'es')} coincidencias: afine la búsqueda.`,
		matchingQuery: (query) => `Autoridades que coinciden con «${query}»`,
		indexSummary: (authorities, countries) =>
			`${num(authorities, 'es')} ${authorities === 1 ? 'autoridad' : 'autoridades'} en ${num(countries, 'es')} ${countries === 1 ? 'país' : 'países'}. Abra un país para ver sus autoridades.`,
		byCountry: 'Autoridades por país',
		authorityCount: (n) => `${num(n, 'es')} ${n === 1 ? 'autoridad' : 'autoridades'}`,
		whichOffice: (country) =>
			`¿Qué oficina se ocupa de mi documento? Todas las oficinas de ${country}`,
		international: 'Internacional'
	},
	fr: {
		headTitle: 'Apostille Handbook — Commencez par votre destination',
		metaDescription:
			'Trouvez les exigences d’apostille et de légalisation selon l’autorité de destination.',
		evalLine: (pct) =>
			`Évaluation : ${num(pct, 'fr')} % des brouillons de Muse Spark citent la source mot pour mot`,
		heroA: 'Les règles de l’apostille,',
		heroEm: 'citées',
		heroB: 'directement de la source.',
		intro:
			'Chaque exigence provient des pages officielles du bureau concerné. Dans chaque guide, les citations sont masquées par défaut : affichez-les avec Alt+C ou le bouton Citations pour voir la citation, la source et la date.',
		searchLabel: 'Rechercher dans le manuel',
		searchPlaceholder: 'Pays, État ou autorité…',
		matchingLabel: 'Autorités correspondantes',
		noMatch: (query) => `Aucune autorité ne correspond à « ${query} ».`,
		featuredLabel: 'Autorités à la une',
		featuredDossier: (n) => `Dossier à la une ${num(n, 'fr')}`,
		flagOf: (country) => `Drapeau : ${country}`,
		howEyebrow: 'Comment ça marche',
		howTitle: 'Trois vérifications avant toute publication',
		steps: [
			{
				model: 'Muse Spark',
				title: 'Spark rédige à partir des pages officielles',
				text: 'Chaque fait part d’une citation copiée d’une page gouvernementale surveillée, jamais de mémoire.'
			},
			{
				model: 'Vérification de la citation',
				title: 'La citation est vérifiée à l’octet près',
				text: 'Une citation refusée ou manquante repart une fois vers le modèle ; un brouillon invérifiable n’est jamais publié.'
			},
			{
				model: 'Relecture humaine',
				title: 'Le personnel approuve, le dossier se met à jour en direct',
				text: 'Seuls les faits acceptés arrivent dans le dossier public, quelques secondes après approbation.'
			}
		],
		featuredEyebrow: 'À la une',
		featuredTitle: 'Commencez par une destination',
		featuredUnavailable: 'Les destinations à la une sont indisponibles.',
		lastVerified: (date) => `Dernière vérification : ${date}`,
		verifiedAgo: (ago) => `vérifié ${ago}`,
		openDossier: 'Ouvrir le dossier',
		indexEyebrow: 'Répertoire',
		indexTitle: 'Toutes les autorités',
		filterLabel: 'Filtrer les autorités',
		all: 'Toutes',
		lanes: {
			hague: 'Membres de La Haye',
			legalization: 'Légalisation',
			states: 'États américains',
			regions: 'États et provinces',
			other: 'Autres destinations'
		},
		kinds: { hague: 'Membre de La Haye', states: 'État américain', legalization: 'Légalisation' },
		searchUnavailable: 'La recherche est indisponible.',
		loadingIndex: 'Chargement du répertoire des autorités',
		noOthers: 'Aucune autre autorité pour l’instant.',
		noInGroup: 'Aucune autorité dans ce groupe pour l’instant.',
		newAppear: 'Les nouvelles destinations apparaissent ici à mesure que le personnel les vérifie.',
		tryAll: 'Essayez le filtre « Toutes » pour voir toutes les destinations.',
		startFeatured: 'Commencez par un dossier à la une :',
		matchCount: (n) => `${num(n, 'fr')} ${n < 2 ? 'résultat' : 'résultats'}`,
		matchCap: (cap, n) =>
			`${num(cap, 'fr')} résultats affichés sur ${num(n, 'fr')} : affinez la recherche.`,
		matchingQuery: (query) => `Autorités correspondant à « ${query} »`,
		indexSummary: (authorities, countries) =>
			`${num(authorities, 'fr')} ${authorities < 2 ? 'autorité' : 'autorités'} dans ${num(countries, 'fr')} pays. Ouvrez un pays pour voir ses autorités.`,
		byCountry: 'Autorités par pays',
		authorityCount: (n) => `${num(n, 'fr')} ${n < 2 ? 'autorité' : 'autorités'}`,
		whichOffice: (country) =>
			`Quel bureau traite mon document ? Tous les bureaux (${country})`,
		international: 'International'
	}
};

/**
 * @typedef {{
 *   parties: (n: number) => string,
 *   empty: string,
 *   mapLabel: (caption: string) => string
 * }} MapCopy
 */

/** The Hague member map's caption (brand/HagueMap.svelte). @type {Record<Locale, MapCopy>} */
export const MAP_COPY = {
	en: {
		parties: (n) => `${n} ${n === 1 ? 'party' : 'parties'} named in staff-approved HCCH facts`,
		empty: 'Member states appear here once staff approve HCCH status facts',
		mapLabel: (caption) => `World map: ${caption}`
	},
	es: {
		parties: (n) =>
			`${num(n, 'es')} ${n === 1 ? 'Parte citada' : 'Partes citadas'} en datos de la HCCH aprobados por el personal`,
		empty:
			'Los Estados miembros aparecen aquí cuando el personal aprueba los datos de la HCCH sobre su estatus',
		mapLabel: (caption) => `Mapa del mundo: ${caption}`
	},
	fr: {
		parties: (n) =>
			`${num(n, 'fr')} ${n < 2 ? 'Partie citée' : 'Parties citées'} dans des faits HCCH approuvés par le personnel`,
		empty:
			'Les États membres apparaissent ici une fois les faits HCCH sur leur statut approuvés par le personnel',
		mapLabel: (caption) => `Carte du monde : ${caption}`
	}
};
