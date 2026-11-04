// A place page's words (/in/usa, /in/usa/wa) in the three site languages
// (lib/i18n.ts LOCALES), picked by the shared site locale so the masthead
// switcher re-renders the page in place. Place labels, office names,
// level labels and prerequisites come from the API and stay as sent.
// `kind` is location.js `placeWord`: what a country's sub-places are
// called ('state' or 'province').

/** @typedef {'en' | 'es' | 'fr'} Locale */
/** @typedef {'state' | 'province'} PlaceKind */

/**
 * @typedef {{
 *   headTitle: (label: string | null) => string,
 *   notFound: (path: string) => string,
 *   notInIndex: string,
 *   backToIndex: string,
 *   unavailable: string,
 *   loading: string,
 *   breadcrumb: string,
 *   home: string,
 *   eyebrow: string,
 *   flagOf: (place: string) => string,
 *   officeCount: (n: number) => string,
 *   placeCount: (n: number, kind: PlaceKind) => string,
 *   whatDoc: string,
 *   docType: string,
 *   docClasses: Record<string, string>,
 *   whereLabel: string,
 *   federal: string,
 *   first: (prereq: string) => string,
 *   verifiedByStaff: string,
 *   unverified: string,
 *   source: string,
 *   byPlaceAnswer: (doc: string, kind: PlaceKind) => string,
 *   noOffice: string,
 *   notSure: string,
 *   routeIt: string,
 *   routeRest: string,
 *   allOffices: string,
 *   byPlace: (kind: PlaceKind) => string,
 *   eachPlace: (kind: PlaceKind) => string
 * }} PlaceCopy
 */

/** @param {number} n @param {Locale} locale */
const num = (n, locale) => (locale === 'en' ? String(n) : n.toLocaleString(locale));

/** @type {Record<Locale, PlaceCopy>} */
export const PLACE_COPY = {
	en: {
		headTitle: (label) =>
			`${label ? `Apostille offices in ${label}` : 'Apostille offices'} — Apostille Handbook`,
		notFound: (path) => `No offices are listed for “${path}”.`,
		notInIndex: 'This place is not in the handbook index.',
		backToIndex: 'Back to the authority index',
		unavailable: 'This place is unavailable.',
		loading: 'Loading place',
		breadcrumb: 'Breadcrumb',
		home: 'Home',
		eyebrow: 'Apostille offices',
		flagOf: (place) => `${place} flag`,
		officeCount: (n) => `${n} ${n === 1 ? 'office' : 'offices'}`,
		placeCount: (n, kind) => `${n} ${kind}${n === 1 ? '' : 's'}`,
		whatDoc: 'What is your document?',
		docType: 'Document type',
		// The picker chips (location.js DOC_CLASSES ids), same wording.
		docClasses: {
			vital_record: 'Birth, marriage or death certificate',
			education: 'Diploma or transcript',
			notarized_private: 'Notarized document (power of attorney, affidavit)',
			state_court: 'State court document',
			company: 'Company document',
			state_agency: 'State agency document',
			federal_agency: 'Federal agency document',
			fbi_check: 'FBI background check',
			federal_court: 'Federal court document'
		},
		whereLabel: 'Where to apostille it',
		federal: 'federal',
		first: (prereq) => `First: ${prereq}`,
		verifiedByStaff: 'Verified by staff',
		unverified: 'Unverified',
		source: 'Source:',
		byPlaceAnswer: (doc, kind) =>
			`A ${doc.toLowerCase()} is apostilled in the ${kind} that issued it. Pick the ${kind} below.`,
		noOffice: 'No office here is listed for this document yet.',
		notSure: 'Not sure?',
		routeIt: 'We’ll route it',
		routeRest: ' — start an order and staff pick the office.',
		allOffices: 'All offices here',
		byPlace: (kind) => `By ${kind}`,
		eachPlace: (kind) => `Each ${kind} page shows its own offices next to the federal ones.`
	},
	es: {
		headTitle: (label) =>
			`${label ? `Oficinas de apostilla en ${label}` : 'Oficinas de apostilla'} — Apostille Handbook`,
		notFound: (path) => `No hay oficinas registradas para «${path}».`,
		notInIndex: 'Este lugar no figura en el índice del manual.',
		backToIndex: 'Volver al índice de autoridades',
		unavailable: 'Este lugar no está disponible.',
		loading: 'Cargando el lugar',
		breadcrumb: 'Ruta de navegación',
		home: 'Inicio',
		eyebrow: 'Oficinas de apostilla',
		flagOf: (place) => `Bandera de ${place}`,
		officeCount: (n) => `${num(n, 'es')} ${n === 1 ? 'oficina' : 'oficinas'}`,
		placeCount: (n, kind) =>
			`${num(n, 'es')} ${kind === 'province' ? (n === 1 ? 'provincia' : 'provincias') : n === 1 ? 'estado' : 'estados'}`,
		whatDoc: '¿Qué documento tiene?',
		docType: 'Tipo de documento',
		docClasses: {
			vital_record: 'Acta de nacimiento, matrimonio o defunción',
			education: 'Diploma o certificado de notas',
			notarized_private: 'Documento notarizado (poder notarial, declaración jurada)',
			state_court: 'Documento de un tribunal estatal',
			company: 'Documento de empresa',
			state_agency: 'Documento de una agencia estatal',
			federal_agency: 'Documento de una agencia federal',
			fbi_check: 'Certificado de antecedentes del FBI',
			federal_court: 'Documento de un tribunal federal'
		},
		whereLabel: 'Dónde apostillarlo',
		federal: 'federal',
		first: (prereq) => `Primero: ${prereq}`,
		verifiedByStaff: 'Verificado por el personal',
		unverified: 'Sin verificar',
		source: 'Fuente:',
		byPlaceAnswer: (doc, kind) =>
			kind === 'province'
				? `${doc}: se apostilla en la provincia que lo expidió. Elija la provincia a continuación.`
				: `${doc}: se apostilla en el estado que lo expidió. Elija el estado a continuación.`,
		noOffice: 'Todavía no hay ninguna oficina registrada aquí para este documento.',
		notSure: '¿No está seguro?',
		routeIt: 'Nosotros lo encaminamos',
		routeRest: ': inicie un pedido y el personal elegirá la oficina.',
		allOffices: 'Todas las oficinas de aquí',
		byPlace: (kind) => (kind === 'province' ? 'Por provincia' : 'Por estado'),
		eachPlace: (kind) =>
			kind === 'province'
				? 'La página de cada provincia muestra sus propias oficinas junto a las federales.'
				: 'La página de cada estado muestra sus propias oficinas junto a las federales.'
	},
	fr: {
		headTitle: (label) =>
			`${label ? `Bureaux d’apostille : ${label}` : 'Bureaux d’apostille'} — Apostille Handbook`,
		notFound: (path) => `Aucun bureau n’est répertorié pour « ${path} ».`,
		notInIndex: 'Ce lieu ne figure pas dans le répertoire du manuel.',
		backToIndex: 'Retour au répertoire des autorités',
		unavailable: 'Ce lieu est indisponible.',
		loading: 'Chargement du lieu',
		breadcrumb: 'Fil d’Ariane',
		home: 'Accueil',
		eyebrow: 'Bureaux d’apostille',
		flagOf: (place) => `Drapeau : ${place}`,
		officeCount: (n) => `${num(n, 'fr')} ${n < 2 ? 'bureau' : 'bureaux'}`,
		placeCount: (n, kind) =>
			`${num(n, 'fr')} ${kind === 'province' ? (n < 2 ? 'province' : 'provinces') : n < 2 ? 'État' : 'États'}`,
		whatDoc: 'Quel est votre document ?',
		docType: 'Type de document',
		docClasses: {
			vital_record: 'Acte de naissance, de mariage ou de décès',
			education: 'Diplôme ou relevé de notes',
			notarized_private: 'Document notarié (procuration, déclaration sous serment)',
			state_court: 'Document d’un tribunal d’État',
			company: 'Document de société',
			state_agency: 'Document d’une agence d’État',
			federal_agency: 'Document d’une agence fédérale',
			fbi_check: 'Vérification des antécédents du FBI',
			federal_court: 'Document d’un tribunal fédéral'
		},
		whereLabel: 'Où le faire apostiller',
		federal: 'fédéral',
		first: (prereq) => `D’abord : ${prereq}`,
		verifiedByStaff: 'Vérifié par le personnel',
		unverified: 'Non vérifié',
		source: 'Source :',
		byPlaceAnswer: (doc, kind) =>
			kind === 'province'
				? `${doc} : ce document est apostillé dans la province qui l’a délivré. Choisissez la province ci-dessous.`
				: `${doc} : ce document est apostillé dans l’État qui l’a délivré. Choisissez l’État ci-dessous.`,
		noOffice: 'Aucun bureau n’est encore répertorié ici pour ce document.',
		notSure: 'Vous hésitez ?',
		routeIt: 'Nous l’orientons',
		routeRest: ' : passez commande et le personnel choisira le bureau.',
		allOffices: 'Tous les bureaux ici',
		byPlace: (kind) => (kind === 'province' ? 'Par province' : 'Par État'),
		eachPlace: (kind) =>
			kind === 'province'
				? 'La page de chaque province présente ses propres bureaux à côté des bureaux fédéraux.'
				: 'La page de chaque État présente ses propres bureaux à côté des bureaux fédéraux.'
	}
};
