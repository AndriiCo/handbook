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
 *   heroTitle: string,
 *   intro: string,
 *   fromLabel: string,
 *   toLabel: string,
 *   countryPlaceholder: string,
 *   countriesLabel: (field: string) => string,
 *   noCountry: (query: string) => string,
 *   clear: (field: string) => string,
 *   swap: string,
 *   guessed: string,
 *   docLabel: string,
 *   submit: string,
 *   needBoth: string,
 *   pathsLabel: string,
 *   checkTitle: string,
 *   checkText: string,
 *   checkCta: string,
 *   doneTitle: string,
 *   doneText: string,
 *   doneCta: string,
 *   journeysTitle: string,
 *   journeys: Record<string, { title: string, note: string }>,
 *   methodTitle: string,
 *   apostilleTitle: string,
 *   apostilleText: string,
 *   legalTitle: string,
 *   legalText: string,
 *   mapLead: string,
 *   examplesLabel: string,
 *   examples: Record<string, string>,
 *   flagOf: (country: string) => string,
 *   indexTitle: string,
 *   indexIntro: string,
 *   indexFilterLabel: string,
 *   indexFilterPlaceholder: string,
 *   noMatch: (query: string) => string,
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
		headTitle: 'Apostille Handbook — Get your document accepted in another country',
		metaDescription:
			'Moving a document between countries? Find out if it needs an apostille or consular legalization, which office handles it, and what to do next.',
		heroTitle: 'Where does your document need to be accepted?',
		intro:
			'Tell us where the document comes from and where it will be used. We tell you whether it needs an apostille or consular legalization, which office handles it, and what to do next.',
		fromLabel: 'Document issued in',
		toLabel: 'To be used in',
		countryPlaceholder: 'Choose a country…',
		countriesLabel: (field) => `Countries for “${field}”`,
		noCountry: (query) => `No country matches “${query}”.`,
		clear: (field) => `Clear “${field}”`,
		swap: 'Swap the two countries',
		guessed: 'Picked from your time zone. Change it if that’s not right.',
		docLabel: 'What is the document? (optional)',
		submit: 'Show me what to do',
		needBoth: 'Choose both countries to see the answer.',
		pathsLabel: 'Two ways to get it done',
		checkTitle: 'Do it yourself',
		checkText:
			'Free, step by step: which office, what to send, and the official page each rule comes from.',
		checkCta: 'Check the requirements',
		doneTitle: 'Have it done for you',
		doneText:
			'For documents issued in the United States: we prepare the shipping to the right office and track it for you.',
		doneCta: 'Start an order',
		journeysTitle: 'Common requests',
		journeys: {
			usMxBirth: { title: 'US birth certificate for Mexico', note: 'Dual nationality, marriage, school' },
			usVnDiploma: { title: 'US diploma for work in Viet Nam', note: 'Work permit, university' },
			usEgFbi: { title: 'FBI background check for Egypt', note: 'Residency, work visa' },
			mxUsBirth: { title: 'Mexican birth certificate for the US', note: 'Immigration, school, marriage' },
			usUaPoa: { title: 'Power of attorney for Ukraine', note: 'Property, inheritance, family' },
			usEsMarriage: { title: 'US marriage certificate for Spain', note: 'Residency, civil registry' }
		},
		methodTitle: 'Apostille or legalization?',
		apostilleTitle: 'Apostille',
		apostilleText:
			'When both countries belong to the Hague Apostille Convention, one certificate from the country that issued the document is enough. No embassy visit.',
		legalTitle: 'Consular legalization',
		legalText:
			'When either country is not a member, the document goes through a chain: certified in the issuing country, then legalized by the destination’s embassy or consulate.',
		mapLead: 'Shaded countries accept an apostille.',
		examplesLabel: 'See an example:',
		examples: {
			wa: 'Washington State apostille office',
			vietnam: 'Viet Nam',
			egypt: 'Egypt (embassy route)',
			hcch: 'Who accepts an apostille'
		},
		flagOf: (country) => `${country} flag`,
		indexTitle: 'Every country and office',
		indexIntro: 'Already know the office you need? Find its guide here.',
		indexFilterLabel: 'Filter countries and offices',
		indexFilterPlaceholder: 'Country, state, or office…',
		noMatch: (query) => `No authority matches “${query}”.`,
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
		noOthers: 'No authorities yet.',
		noInGroup: 'No authorities in this group yet.',
		newAppear: 'New destinations appear here as staff verify them.',
		tryAll: 'Try the “All” filter to see every destination.',
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
		headTitle: 'Apostille Handbook — Que su documento sea válido en otro país',
		metaDescription:
			'¿Lleva un documento de un país a otro? Sepa si necesita apostilla o legalización consular, qué oficina se encarga y qué hacer después.',
		heroTitle: '¿Dónde tiene que ser aceptado su documento?',
		intro:
			'Díganos de dónde es el documento y dónde lo va a usar. Le decimos si necesita apostilla o legalización consular, qué oficina se encarga y qué hacer después.',
		fromLabel: 'Documento emitido en',
		toLabel: 'Para usar en',
		countryPlaceholder: 'Elija un país…',
		countriesLabel: (field) => `Países para «${field}»`,
		noCountry: (query) => `Ningún país coincide con «${query}».`,
		clear: (field) => `Borrar «${field}»`,
		swap: 'Intercambiar los dos países',
		guessed: 'Elegido según su zona horaria. Cámbielo si no es correcto.',
		docLabel: '¿Qué documento es? (opcional)',
		submit: 'Mostrarme qué hacer',
		needBoth: 'Elija los dos países para ver la respuesta.',
		pathsLabel: 'Dos maneras de hacerlo',
		checkTitle: 'Hágalo usted mismo',
		checkText:
			'Gratis, paso a paso: qué oficina, qué enviar y la página oficial de la que sale cada regla.',
		checkCta: 'Consultar los requisitos',
		doneTitle: 'Que lo hagamos por usted',
		doneText:
			'Para documentos emitidos en Estados Unidos: preparamos el envío a la oficina correcta y le hacemos el seguimiento.',
		doneCta: 'Empezar un pedido',
		journeysTitle: 'Solicitudes frecuentes',
		journeys: {
			usMxBirth: { title: 'Acta de nacimiento de EE. UU. para México', note: 'Doble nacionalidad, matrimonio, escuela' },
			usVnDiploma: { title: 'Título de EE. UU. para trabajar en Vietnam', note: 'Permiso de trabajo, universidad' },
			usEgFbi: { title: 'Antecedentes del FBI para Egipto', note: 'Residencia, visado de trabajo' },
			mxUsBirth: { title: 'Acta de nacimiento mexicana para EE. UU.', note: 'Inmigración, escuela, matrimonio' },
			usUaPoa: { title: 'Poder notarial para Ucrania', note: 'Propiedad, herencia, familia' },
			usEsMarriage: { title: 'Acta de matrimonio de EE. UU. para España', note: 'Residencia, registro civil' }
		},
		methodTitle: '¿Apostilla o legalización?',
		apostilleTitle: 'Apostilla',
		apostilleText:
			'Si los dos países forman parte del Convenio de la Apostilla de La Haya, basta un certificado del país que emitió el documento. Sin pasar por la embajada.',
		legalTitle: 'Legalización consular',
		legalText:
			'Si alguno de los dos no es miembro, el documento sigue una cadena: se certifica en el país que lo emitió y luego lo legaliza la embajada o el consulado del país de destino.',
		mapLead: 'Los países sombreados aceptan la apostilla.',
		examplesLabel: 'Vea un ejemplo:',
		examples: {
			wa: 'Oficina de apostillas del estado de Washington',
			vietnam: 'Vietnam',
			egypt: 'Egipto (vía embajada)',
			hcch: 'Quién acepta la apostilla'
		},
		flagOf: (country) => `Bandera de ${country}`,
		indexTitle: 'Todos los países y oficinas',
		indexIntro: '¿Ya sabe qué oficina necesita? Encuentre aquí su guía.',
		indexFilterLabel: 'Filtrar países y oficinas',
		indexFilterPlaceholder: 'País, estado u oficina…',
		noMatch: (query) => `Ninguna autoridad coincide con «${query}».`,
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
		noOthers: 'Todavía no hay autoridades.',
		noInGroup: 'Todavía no hay autoridades en este grupo.',
		newAppear: 'Los nuevos destinos aparecen aquí a medida que el personal los verifica.',
		tryAll: 'Pruebe el filtro «Todas» para ver todos los destinos.',
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
		headTitle: 'Apostille Handbook — Faire accepter votre document dans un autre pays',
		metaDescription:
			'Vous faites passer un document d’un pays à un autre ? Sachez s’il faut une apostille ou une légalisation consulaire, quel bureau s’en charge et quoi faire ensuite.',
		heroTitle: 'Où votre document doit-il être accepté ?',
		intro:
			'Dites-nous d’où vient le document et où il sera utilisé. Nous vous disons s’il faut une apostille ou une légalisation consulaire, quel bureau s’en charge et quoi faire ensuite.',
		fromLabel: 'Pays qui a délivré le document',
		toLabel: 'Pays où il sera utilisé',
		countryPlaceholder: 'Choisissez un pays…',
		countriesLabel: (field) => `Pays pour « ${field} »`,
		noCountry: (query) => `Aucun pays ne correspond à « ${query} ».`,
		clear: (field) => `Effacer « ${field} »`,
		swap: 'Inverser les deux pays',
		guessed: 'Choisi d’après votre fuseau horaire. Modifiez-le si ce n’est pas le bon.',
		docLabel: 'De quel document s’agit-il ? (facultatif)',
		submit: 'Voir quoi faire',
		needBoth: 'Choisissez les deux pays pour voir la réponse.',
		pathsLabel: 'Deux façons de procéder',
		checkTitle: 'Le faire vous-même',
		checkText:
			'Gratuit, étape par étape : quel bureau, quoi envoyer, et la page officielle d’où vient chaque règle.',
		checkCta: 'Consulter les exigences',
		doneTitle: 'Nous en occuper pour vous',
		doneText:
			'Pour les documents délivrés aux États-Unis : nous préparons l’envoi vers le bon bureau et en assurons le suivi.',
		doneCta: 'Commencer une commande',
		journeysTitle: 'Demandes fréquentes',
		journeys: {
			usMxBirth: { title: 'Acte de naissance américain pour le Mexique', note: 'Double nationalité, mariage, école' },
			usVnDiploma: { title: 'Diplôme américain pour travailler au Viêt Nam', note: 'Permis de travail, université' },
			usEgFbi: { title: 'Casier judiciaire du FBI pour l’Égypte', note: 'Résidence, visa de travail' },
			mxUsBirth: { title: 'Acte de naissance mexicain pour les États-Unis', note: 'Immigration, école, mariage' },
			usUaPoa: { title: 'Procuration pour l’Ukraine', note: 'Biens, succession, famille' },
			usEsMarriage: { title: 'Acte de mariage américain pour l’Espagne', note: 'Résidence, état civil' }
		},
		methodTitle: 'Apostille ou légalisation ?',
		apostilleTitle: 'Apostille',
		apostilleText:
			'Quand les deux pays sont parties à la Convention Apostille de La Haye, un seul certificat du pays qui a délivré le document suffit. Pas de passage à l’ambassade.',
		legalTitle: 'Légalisation consulaire',
		legalText:
			'Quand l’un des deux pays n’est pas membre, le document suit une chaîne : certifié dans le pays qui l’a délivré, puis légalisé par l’ambassade ou le consulat du pays de destination.',
		mapLead: 'Les pays en couleur acceptent l’apostille.',
		examplesLabel: 'Voir un exemple :',
		examples: {
			wa: 'Bureau des apostilles de l’État de Washington',
			vietnam: 'Viêt Nam',
			egypt: 'Égypte (voie consulaire)',
			hcch: 'Qui accepte l’apostille'
		},
		flagOf: (country) => `Drapeau : ${country}`,
		indexTitle: 'Tous les pays et bureaux',
		indexIntro: 'Vous savez déjà quel bureau il vous faut ? Trouvez son guide ici.',
		indexFilterLabel: 'Filtrer les pays et les bureaux',
		indexFilterPlaceholder: 'Pays, État ou bureau…',
		noMatch: (query) => `Aucune autorité ne correspond à « ${query} ».`,
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
		noOthers: 'Aucune autorité pour l’instant.',
		noInGroup: 'Aucune autorité dans ce groupe pour l’instant.',
		newAppear: 'Les nouvelles destinations apparaissent ici à mesure que le personnel les vérifie.',
		tryAll: 'Essayez le filtre « Toutes » pour voir toutes les destinations.',
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
