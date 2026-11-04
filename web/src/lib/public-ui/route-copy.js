// The /route answer page's words in the three site languages (lib/i18n.ts
// LOCALES), picked by the shared site locale. Country names come from the
// browser's CLDR region names in the same language; office names,
// prerequisites and fact values come from the API and stay as sent.
// Document chips, "verified", "source" and "first:" reuse PLACE_COPY.

/** @typedef {'en' | 'es' | 'fr'} Locale */

/**
 * @typedef {{ title: string, text: (from: string, to: string) => string }} MethodCopy
 * @typedef {{
 *   headTitle: (from: string, to: string) => string,
 *   headBlank: string,
 *   eyebrow: string,
 *   title: (from: string, to: string) => string,
 *   pickTitle: string,
 *   pickText: string,
 *   backHome: string,
 *   change: string,
 *   copyLink: string,
 *   copied: string,
 *   loading: string,
 *   methods: { apostille: MethodCopy, legalization: MethodCopy, same: MethodCopy, unknown: MethodCopy },
 *   methodSource: string,
 *   step1: (from: string) => string,
 *   whatDoc: string,
 *   officeLabel: string,
 *   pickPlace: string,
 *   noOffices: (from: string) => string,
 *   allOffices: (from: string) => string,
 *   feesTitle: string,
 *   per: (unit: string) => string,
 *   step2: (to: string) => string,
 *   step2Apostille: (to: string) => string,
 *   step2Chain: string,
 *   step2NoChain: (to: string) => string,
 *   step2Same: string,
 *   guideFor: (to: string) => string,
 *   notCovered: (to: string) => string,
 *   doneTitle: string,
 *   doneText: string,
 *   doneCta: string,
 *   abroadTitle: (from: string) => string,
 *   abroadText: string
 * }} RouteCopy
 */

/** @type {Record<Locale, RouteCopy>} */
export const ROUTE_COPY = {
	en: {
		headTitle: (from, to) => `From ${from} to ${to}: what to do — Apostille Handbook`,
		headBlank: 'What to do with your document — Apostille Handbook',
		eyebrow: 'Your answer',
		title: (from, to) => `A document from ${from}, used in ${to}`,
		pickTitle: 'Choose two countries',
		pickText: 'This page answers for a pair of countries: where the document was issued and where it will be used.',
		backHome: 'Start again',
		change: 'Change countries',
		copyLink: 'Copy link to this answer',
		copied: 'Link copied',
		loading: 'Loading your answer',
		methods: {
			apostille: {
				title: 'You need an apostille',
				text: (from) =>
					`Both countries accept the apostille. The document gets one apostille certificate in ${from}; no embassy or consulate visit is needed.`
			},
			legalization: {
				title: 'You need consular legalization',
				text: (from, to) =>
					`An apostille is not enough between these two countries. The document is certified in ${from}, then legalized by the embassy or consulate of ${to}.`
			},
			same: {
				title: 'No apostille needed',
				text: () =>
					'A document used in the country that issued it usually needs no apostille. Check with whoever asked for it.'
			},
			unknown: {
				title: 'We can’t confirm the method yet',
				text: () =>
					'The list of countries that accept the apostille is not verified here yet. Follow each country’s guide below.'
			}
		},
		methodSource: 'Based on the Hague Conference’s list of member countries',
		step1: (from) => `Step 1 — in ${from}`,
		whatDoc: 'What kind of document is it?',
		officeLabel: 'The office that handles it',
		pickPlace: 'Where was it issued?',
		noOffices: (from) => `We haven’t verified the offices in ${from} yet.`,
		allOffices: (from) => `All offices in ${from}`,
		feesTitle: 'Fees and timing, from the office',
		per: (unit) => `per ${unit}`,
		step2: (to) => `Step 2 — for ${to}`,
		step2Apostille: (to) =>
			`With the apostille attached, the document is ready to use in ${to}. Some offices also ask for a certified translation, so check the guide.`,
		step2Chain: 'Then, the embassy or consulate:',
		step2NoChain: (to) => `The embassy steps for ${to} aren’t verified here yet.`,
		step2Same: 'Nothing more to do here.',
		guideFor: (to) => `Full guide for ${to}`,
		notCovered: (to) => `We haven’t verified the rules of ${to} yet.`,
		doneTitle: 'Want us to handle it?',
		doneText: 'We prepare the shipping to the right office and track it for you.',
		doneCta: 'Start an order',
		abroadTitle: (from) => `This happens in ${from}`,
		abroadText:
			'We can only file documents with offices in the United States. Follow the steps above, or ask a notary or lawyer where the document was issued.'
	},
	es: {
		headTitle: (from, to) => `De ${from} a ${to}: qué hacer — Apostille Handbook`,
		headBlank: 'Qué hacer con su documento — Apostille Handbook',
		eyebrow: 'Su respuesta',
		title: (from, to) => `Un documento de ${from} para usar en ${to}`,
		pickTitle: 'Elija dos países',
		pickText: 'Esta página responde para un par de países: dónde se emitió el documento y dónde se va a usar.',
		backHome: 'Volver a empezar',
		change: 'Cambiar los países',
		copyLink: 'Copiar el enlace a esta respuesta',
		copied: 'Enlace copiado',
		loading: 'Cargando su respuesta',
		methods: {
			apostille: {
				title: 'Necesita una apostilla',
				text: (from) =>
					`Los dos países aceptan la apostilla. El documento recibe un certificado de apostilla en ${from}; no hace falta ir a la embajada ni al consulado.`
			},
			legalization: {
				title: 'Necesita legalización consular',
				text: (from, to) =>
					`Entre estos dos países la apostilla no basta. El documento se certifica en ${from} y luego lo legaliza la embajada o el consulado de ${to}.`
			},
			same: {
				title: 'No necesita apostilla',
				text: () =>
					'Un documento que se usa en el mismo país que lo emitió normalmente no necesita apostilla. Consulte con quien se lo pidió.'
			},
			unknown: {
				title: 'Todavía no podemos confirmar el trámite',
				text: () =>
					'La lista de países que aceptan la apostilla aún no está verificada aquí. Siga la guía de cada país más abajo.'
			}
		},
		methodSource: 'Según la lista de países miembros de la Conferencia de La Haya',
		step1: (from) => `Paso 1: en ${from}`,
		whatDoc: '¿Qué tipo de documento es?',
		officeLabel: 'La oficina que se encarga',
		pickPlace: '¿Dónde se emitió?',
		noOffices: (from) => `Todavía no hemos verificado las oficinas de ${from}.`,
		allOffices: (from) => `Todas las oficinas de ${from}`,
		feesTitle: 'Tasas y plazos, según la oficina',
		per: (unit) => `por ${unit}`,
		step2: (to) => `Paso 2: para ${to}`,
		step2Apostille: (to) =>
			`Con la apostilla, el documento ya se puede usar en ${to}. Algunas oficinas piden además una traducción certificada: consulte la guía.`,
		step2Chain: 'Después, la embajada o el consulado:',
		step2NoChain: (to) => `Los pasos ante la embajada de ${to} aún no están verificados aquí.`,
		step2Same: 'No hay que hacer nada más.',
		guideFor: (to) => `Guía completa de ${to}`,
		notCovered: (to) => `Todavía no hemos verificado las reglas de ${to}.`,
		doneTitle: '¿Quiere que nos encarguemos?',
		doneText: 'Preparamos el envío a la oficina correcta y le hacemos el seguimiento.',
		doneCta: 'Empezar un pedido',
		abroadTitle: (from) => `Este trámite se hace en ${from}`,
		abroadText:
			'Solo podemos presentar documentos ante oficinas de Estados Unidos. Siga los pasos de arriba o consulte a un notario o abogado del país que emitió el documento.'
	},
	fr: {
		headTitle: (from, to) => `De ${from} vers ${to} : que faire — Apostille Handbook`,
		headBlank: 'Que faire de votre document — Apostille Handbook',
		eyebrow: 'Votre réponse',
		title: (from, to) => `Un document de ${from}, utilisé dans ce pays : ${to}`,
		pickTitle: 'Choisissez deux pays',
		pickText: 'Cette page répond pour une paire de pays : où le document a été délivré et où il sera utilisé.',
		backHome: 'Recommencer',
		change: 'Changer de pays',
		copyLink: 'Copier le lien vers cette réponse',
		copied: 'Lien copié',
		loading: 'Chargement de votre réponse',
		methods: {
			apostille: {
				title: 'Il vous faut une apostille',
				text: (from) =>
					`Les deux pays acceptent l’apostille. Le document reçoit un certificat d’apostille (${from}) ; inutile de passer par l’ambassade ou le consulat.`
			},
			legalization: {
				title: 'Il vous faut une légalisation consulaire',
				text: (from, to) =>
					`Entre ces deux pays, l’apostille ne suffit pas. Le document est certifié dans le pays d’origine (${from}), puis légalisé par l’ambassade ou le consulat (${to}).`
			},
			same: {
				title: 'Pas besoin d’apostille',
				text: () =>
					'Un document utilisé dans le pays qui l’a délivré n’a en général pas besoin d’apostille. Renseignez-vous auprès de la personne qui le demande.'
			},
			unknown: {
				title: 'Nous ne pouvons pas encore confirmer la démarche',
				text: () =>
					'La liste des pays qui acceptent l’apostille n’est pas encore vérifiée ici. Suivez le guide de chaque pays ci-dessous.'
			}
		},
		methodSource: 'D’après la liste des pays membres de la Conférence de La Haye',
		step1: (from) => `Étape 1 — pays d’origine : ${from}`,
		whatDoc: 'De quel type de document s’agit-il ?',
		officeLabel: 'Le bureau compétent',
		pickPlace: 'Où a-t-il été délivré ?',
		noOffices: (from) => `Nous n’avons pas encore vérifié les bureaux (${from}).`,
		allOffices: (from) => `Tous les bureaux (${from})`,
		feesTitle: 'Frais et délais, selon le bureau',
		per: (unit) => `par ${unit}`,
		step2: (to) => `Étape 2 — pays de destination : ${to}`,
		step2Apostille: (to) =>
			`Avec l’apostille, le document peut être utilisé (${to}). Certains bureaux demandent aussi une traduction certifiée : consultez le guide.`,
		step2Chain: 'Ensuite, l’ambassade ou le consulat :',
		step2NoChain: (to) => `Les démarches consulaires (${to}) ne sont pas encore vérifiées ici.`,
		step2Same: 'Rien d’autre à faire.',
		guideFor: (to) => `Guide complet : ${to}`,
		notCovered: (to) => `Nous n’avons pas encore vérifié les règles (${to}).`,
		doneTitle: 'Vous voulez que nous nous en occupions ?',
		doneText: 'Nous préparons l’envoi vers le bon bureau et en assurons le suivi.',
		doneCta: 'Commencer une commande',
		abroadTitle: (from) => `Cette étape se fait sur place (${from})`,
		abroadText:
			'Nous ne pouvons déposer des documents qu’auprès de bureaux aux États-Unis. Suivez les étapes ci-dessus, ou adressez-vous à un notaire ou un avocat du pays qui a délivré le document.'
	}
};
