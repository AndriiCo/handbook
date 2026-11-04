// The customer guide's own words in French (key for key with en.js;
// read the notes there). Machine-drafted: awaiting a native speaker's
// review. Fact values, quotes and office names stay as the office's
// pages write them; only these template words are translated.
//
// Terms: apostille, bureau (the issuing office), frais, envoi par la
// poste / par coursier / en personne, délai de traitement, source,
// citation. Register: vous.

/** @param {number} n @param {string} one @param {string} many */
const counted = (n, one, many) => `${n} ${n <= 1 ? one : many}`;

/** @param {'email'|'visit'|'call'} how */
const byHow = (how) => (how === 'email' ? 'par e-mail' : how === 'visit' ? 'au bureau' : 'par téléphone');

/** @type {import('./index.js').GuideCopy} */
export default {
	intl: 'fr',

	present: {
		eyebrow: 'Guide de l’apostille',
		h1: (region) => `Obtenir une apostille : ${region}`,
		lede: (issuer) => `Délivrée par : ${issuer}.`,
		staleWarning: 'Peut-être obsolète : vérifiez auprès du bureau avant d’envoyer de l’argent',
		staleMonths: (months) => `Capturé il y a ${months} mois`,
		absent: 'Absent du site web du bureau',
		pointerUsps: 'Les pages du bureau indiquent des boîtes postales différentes : lisez d’abord l’étape de l’envoi par la poste',
		pointer: 'Les pages du bureau donnent des valeurs différentes : lisez d’abord l’étape ci-dessous',
		pointerContested: 'Le bureau nous a dit autre chose : lisez d’abord l’étape ci-dessous',
		homePage: 'Page d’accueil',
		notConfirmed: 'Non confirmé : demandez au bureau',
		remoteLabel: 'Notarisation à distance',
		remoteText:
			'Nous n’avons pas confirmé si le bureau accepte les documents notariés à distance. Renseignez-vous avant d’en payer une.',
		detail: 'Détail',
		routes: {
			mail: 'Par la poste',
			courier: 'Par coursier',
			visit: 'En personne',
			visitSameDay: 'En personne, le jour même'
		}
	},

	sections: {
		before: 'Vérifiez qu’il s’agit du bon bureau',
		prepare: 'Préparez votre document',
		form: 'Remplissez le formulaire de demande',
		fees: 'Réglez les frais',
		send: 'Envoyez ou déposez vos documents',
		after: 'Récupérez-le et vérifiez-le',
		contact: 'Contactez le bureau'
	},

	slots: {
		'jurisdiction.scope': 'Documents traités par ce bureau',
		'jurisdiction.federal_route': 'Documents fédéraux',
		'jurisdiction.refusals': 'Ce que le bureau n’apostille pas',
		'authentication.product': 'Type de certificat',
		'authentication.domestic_validity': 'Usage dans le pays émetteur',
		'notarization.wa_notary': 'Exigences notariales',
		'copies.vital_records': 'Actes de naissance, de décès et de mariage',
		'forms.form': 'Formulaire de demande',
		'forms.revision': 'Version du formulaire',
		'mailing.enclose': 'Pièces à joindre',
		'fees.per_document': 'Frais par document',
		'fees.expedite': 'Frais du service accéléré',
		'fees.same_day': 'Frais du service le jour même',
		'fees.refund': 'Remboursements',
		'payment.payee': 'À l’ordre de',
		'payment.methods_counter': 'Paiement sur place',
		'mailing.usps': 'Adresse postale',
		'mailing.courier': 'Adresse pour coursier',
		'mailing.courier_rule': 'Envoi par coursier',
		'address.office': 'Adresse du bureau',
		'hours.weekday': 'Heures d’ouverture',
		'walkin.cutoff': 'Heure d’arrivée limite',
		'walkin.same_day_only': 'Service le jour même uniquement',
		'walkin.no_mail': 'Pas de demandes par la poste',
		'walkin.public_record': 'Registres publics',
		'holiday_hours.closure': 'Fermetures du bureau',
		'processing_time.standard': 'Délai de traitement standard',
		'processing_time.expedited': 'Délai de traitement accéléré',
		'processing_time.same_day': 'Traitement le jour même',
		'mailing.return': 'Mode de retour',
		'website.verify': 'Vérifier une apostille en ligne',
		'contacts.phone': 'Téléphone',
		'contacts.phone_option': 'Option du menu téléphonique',
		'contacts.phone_hours': 'Horaires téléphoniques',
		'contacts.email': 'E-mail',
		'contacts.chat': 'Chat en ligne',
		'contacts.tty': 'Téléphone texte (ATS)',
		'contacts.no_texts': 'SMS',
		'service_notice.notice': 'Avis de service',
		'service_notice.restored': 'Service rétabli'
	},

	fields: {
		per_document: 'Frais par document',
		apostille_main_branch_street: 'Adresse du bureau',
		standard: 'Traitement standard',
		weekday: 'Horaires en semaine',
		po_box: 'Boîte postale',
		methods: 'Moyens de paiement acceptés'
	},

	topics: {
		hours: 'Horaires',
		fees: 'Frais',
		forms: 'Formulaires',
		address: 'Adresse',
		parking: 'Stationnement',
		route_photos: 'Photos de l’itinéraire',
		rejections: 'Refus',
		contacts: 'Contacts',
		po_box: 'Boîte postale',
		walkin: 'Accueil sans rendez-vous',
		mailing: 'Envoi par la poste',
		appointment: 'Rendez-vous',
		payment: 'Paiement',
		processing_time: 'Délai de traitement',
		jurisdiction: 'Compétence',
		legalization_chain: 'Chaîne de légalisation',
		website: 'Site web',
		holiday_hours: 'Horaires des jours fériés',
		id_requirements: 'Pièces d’identité exigées',
		copies: 'Copies',
		translation: 'Traduction',
		notarization: 'Notarisation',
		authentication: 'Authentification',
		dropbox: 'Boîte de dépôt',
		service_notice: 'Avis de service'
	},

	glance: {
		heading: 'En bref',
		rows: { cost: 'Coût', time: 'Délai', pay: 'Paiement', send: 'Envoi', form: 'Formulaire' },
		parts: {
			'processing_time.standard': 'Standard',
			'processing_time.expedited': 'Accéléré',
			'processing_time.same_day': 'Le jour même',
			'payment.payee': 'À l’ordre de',
			'payment.methods_counter': 'En personne',
			'mailing.usps': 'Par la poste',
			'mailing.courier': 'Par coursier',
			'address.office': 'En personne'
		},
		or: 'ou',
		worded: '(formulé différemment sur les pages du bureau)',
		downloadPdf: 'Télécharger (PDF)',
		getOnPage: 'Le trouver sur la page du bureau',
		fasterStep: (step) => `Options plus rapides à l’étape ${step}`,
		fasterFees: 'Options plus rapides sous « Réglez les frais »'
	},

	guide: {
		citeFailed: 'Les sources ne se sont pas chargées : les numéros ne peuvent pas encore les ouvrir.',
		retry: 'Réessayer',
		updates: 'Mises à jour',
		afterNote: 'Prévoyez un délai supplémentaire pour la poste, à l’aller comme au retour.',
		issuerSos: 'le Secrétariat d’État',
		issuerOffice: 'le bureau émetteur',
		nav: {
			before: 'Bon bureau',
			prepare: 'Préparer',
			form: 'Formulaire',
			fees: 'Frais',
			send: 'Envoyer',
			after: 'Récupérer',
			contact: 'Contact',
			updates: 'Mises à jour'
		},
		originalNote:
			'Les informations citées des pages du bureau s’affichent dans leur langue d’origine, telles que le bureau les publie.'
	},

	common: {
		newTab: '(s’ouvre dans un nouvel onglet)',
		sourceNotOnFile: 'Source non archivée',
		alsoOnPages: 'Également sur les pages du bureau',
		notConfirmed: 'Non confirmé',
		copyNames: {
			payee: 'le nom du bénéficiaire',
			mailing: 'l’adresse postale',
			courier: 'l’adresse pour coursier',
			recommended: 'l’adresse postale recommandée',
			office: 'l’adresse du bureau',
			cityOffice: (city) => `l’adresse du bureau de ${city}`
		}
	},

	header: {
		breadcrumb: 'Fil d’Ariane',
		home: 'Accueil',
		flag: (country) => `Drapeau : ${country}`,
		unofficial: 'Guide non officiel, pas un site gouvernemental',
		officialSite: 'Site officiel :'
	},

	status: {
		serviceNotice: 'Avis de service',
		capturedFrom: (day) => `Capturé sur la page du bureau le ${day}`,
		noOpen: (day) => `Aucun avis de service ouvert dans le registre des avis du bureau (dernière lecture : ${day}).`,
		resolved: 'Résolu :'
	},

	section: {
		step: (n) => `Étape ${n} :`,
		call: 'Appeler ce numéro',
		email: 'Écrire un e-mail',
		openPage: 'Ouvrir la page du bureau',
		beforeApply: 'Avant de faire la demande',
		chain: 'Chaîne de légalisation',
		chainNote: 'Chaque étape ci-dessous provient des pages mêmes du bureau, dans l’ordre.',
		notConfirmedBody:
			'Les pages du bureau ne le précisent pas. Renseignez-vous auprès du bureau avant d’envoyer des documents ou de l’argent :'
	},

	row: {
		conflict: 'Les pages du bureau donnent des valeurs différentes',
		worded: 'Formulé différemment sur les pages du bureau'
	},

	copyButton: {
		label: 'Copier',
		copied: (text) => `Copié : ${text}`,
		selected: (text) => `Sélectionné : ${text}. Copiez-le avec votre raccourci clavier.`,
		selectYourself: (text) => `Sélectionnez-le et copiez-le vous-même : ${text}`
	},

	contents: { label: 'Sur cette page' },

	routes: {
		label: 'Façons d’envoyer vos documents',
		hints: {
			mail: 'Envoyez les documents au bureau par la poste.',
			courier: 'Envoyez-les en express par un service de coursier.',
			visit: 'Apportez-les vous-même au guichet.'
		}
	},

	empty: {
		heading: 'Pas encore vérifié',
		body: 'Nous ne publions une information qu’après qu’une personne l’a rapprochée des pages mêmes du bureau. Personne ne l’a encore fait pour ce bureau : cette page n’affiche donc ni frais, ni adresses, ni délais de traitement.',
		listed: 'Site officiel répertorié :',
		notChecked: '— pas encore vérifié par nous',
		browse: 'Parcourir les bureaux que nous avons vérifiés'
	},

	about: {
		heading: 'À propos de ce guide',
		unofficial: 'Guide non officiel, pas un site gouvernemental.',
		applyDirect: ['Vous pouvez faire la demande directement auprès de : ', ''],
		perDocument: [', pour ', ' par document'],
		rule: 'Nous ne publions une information qu’après qu’un relecteur de notre équipe l’a rapprochée des mots mêmes du bureau. Information générale, pas un conseil juridique : vérifiez auprès du bureau avant d’envoyer des documents ou de l’argent.',
		report: 'Signaler un changement sur les pages du bureau',
		reportContinue: 'Reprendre votre signalement de changement (brouillon enregistré)'
	},

	follow: {
		title: (name) => `Recevoir les mises à jour : ${name}`,
		description:
			'Un e-mail lorsque notre équipe accepte une information modifiée pour ce bureau. Rien d’autre, jamais.',
		checkInbox: 'Consultez votre boîte de réception pour confirmer.',
		sent: (email) =>
			`Nous avons envoyé un lien de confirmation à ${email} : il expire dans 24 heures, et rien n’est envoyé tant que vous n’avez pas cliqué dessus.`,
		emailLabel: 'Adresse e-mail',
		sending: 'Envoi…',
		follow: 'Suivre',
		invalid: 'Saisissez une adresse e-mail valide.',
		tooMany: 'Trop de tentatives : attendez quelques minutes puis réessayez.',
		off: 'Le suivi des mises à jour est désactivé pour le moment : réessayez plus tard.',
		failed: 'Une erreur s’est produite : réessayez plus tard.',
		privacy:
			'Confidentialité : nous ne conservons que votre adresse (scellée : notre équipe ne peut pas la lire) et le bureau que vous suivez. Vous confirmez avant tout envoi ; au plus un résumé par bureau et par heure ; chaque e-mail contient un lien de désinscription en un clic et notre adresse postale. La désinscription fonctionne toujours, immédiatement.'
	},

	fees: {
		calculator: 'Calculez votre total',
		documents: 'Nombre de documents',
		fewer: 'Un document de moins',
		more: 'Un document de plus',
		speed: 'Rapidité',
		speeds: { standard: 'Standard', expedite: 'Accéléré', same_day: 'Le jour même, en personne' },
		callOffice: 'Appelez le bureau :',
		yourTotal: 'Votre total',
		totalCounter: 'Total au guichet',
		totalEnclose: 'Total à joindre',
		sameDayFee: 'frais du jour même',
		expediteFee: 'frais d’accélération',
		range: (from, to) => (from === to ? counted(from, 'document', 'documents') : `${from}–${to} documents`),
		askOffice: 'Renseignez-vous auprès du bureau avant de payer.',
		ask: {
			conflict: 'Les pages du bureau indiquent des montants différents pour ces frais.',
			contested: 'Le bureau nous a dit autre chose que ses pages.',
			'over-max': 'Ce calculateur ne calcule pas de total pour autant de documents.',
			'no-tier': 'Les pages du bureau n’indiquent pas ces frais pour autant de documents.',
			count: 'Indiquez combien de documents vous envoyez.',
			other: 'Les pages mêmes du bureau ne donnent pas assez d’éléments pour calculer ce total.'
		}
	},

	form: {
		download: 'Télécharger le formulaire officiel (PDF)',
		getOnPage: 'Obtenir le formulaire sur la page du bureau',
		fill: 'Le remplir en ligne, gratuitement',
		fillNote:
			'Nous reportons vos réponses dans le formulaire même du bureau ; vous l’imprimez, le signez et l’envoyez par la poste. Nécessite un compte gratuit.'
	},

	conflict: {
		worded: 'Formulé différemment sur les pages du bureau',
		heading: 'Les pages mêmes du bureau indiquent des adresses postales différentes',
		printedOn: 'Publié sur :',
		recommendation: 'Notre recommandation',
		recommendationStaff: (day) => `Notre recommandation (équipe${day ? `, ${day}` : ''}) :`,
		use: 'utilisez',
		confirmed: (how, channel) => ` — confirmé ${byHow(how)} avec le bureau${channel ? ` (${channel})` : ''}`,
		toldUs: (how, day) => `Le bureau nous l’a dit ${byHow(how)}${day ? ` le ${day}` : ''} :`,
		whatYouCanDo: 'Ce que vous pouvez faire :',
		call: ['Appelez le ', ' avant d’envoyer par la poste.'],
		sendCourier: (first) => `${first ? 'Envoyez' : 'Ou envoyez'} par coursier à l’`,
		overnight: 'adresse pour envois express',
		askWhich:
			'Demandez au bureau quelle adresse est valable avant d’envoyer par la poste. Ses coordonnées figurent sous',
		contactLink: 'Contactez le bureau'
	},

	send: {
		mail: 'Par la poste',
		mailUs: 'Par la poste (USPS)',
		courier: 'Par coursier',
		courierUs: 'Par coursier (FedEx, UPS)',
		visit: 'En personne',
		visitSameDay: 'En personne, le jour même',
		carriers: 'Transporteurs cités par les pages du bureau',
		notStated: (carrier) => `${carrier} : non précisé`,
		otherCarriers: 'Autres transporteurs',
		mailingMissing: 'Adresse postale',
		courierMissing: 'Adresse pour coursier',
		courierPhone: [
			'Les étiquettes de coursier demandent un téléphone du destinataire ; le numéro publié du bureau est le ',
			'.'
		]
	},

	label: {
		fedex: ['Imprimez une étiquette FedEx prépayée vers l’adresse pour coursier ', '— à partir de '],
		unavailable:
			'Les étiquettes de coursier prépayées ne sont pas disponibles pour ce bureau pour le moment. Vous pouvez envoyer vous-même par FedEx ou UPS : copiez l’adresse pour coursier ci-dessus.',
		paidBy: (issuer) => `Service payant d’Apostille Handbook, et non de ${issuer}.`,
		free: 'Imprimez une étiquette d’adresse gratuite pour l’adresse postale',
		freeBy: (issuer) => `Service gratuit d’Apostille Handbook, et non de ${issuer}.`,
		courierCity: 'Ville pour coursier'
	},

	offices: {
		google: (title) => `Ouvrir dans Google Maps : ${title} (s’ouvre dans un nouvel onglet)`,
		apple: (title) => `Ouvrir dans Plans d’Apple : ${title} (s’ouvre dans un nouvel onglet)`,
		cutoff: (time) => `· Heure limite aujourd’hui : ${time}`,
		timesIn: (zone) => `Heures : ${zone}`,
		office: 'Bureau',
		officeMissing: 'Adresse du bureau',
		every: 'Pour tous les bureaux',
		upcoming: 'Fermetures à venir',
		more: 'Autres fermetures',
		closedToday: 'Fermé aujourd’hui',
		openNow: (time) => `Ouvert · ferme à ${time}`,
		opensToday: (time) => `Fermé · ouvre aujourd’hui à ${time}`,
		opensOn: (when, time) => `Fermé · ouvre ${when} à ${time}`,
		tomorrow: 'demain',
		closedNow: 'Fermé',
		noon: 'midi'
	},

	officeCard: {
		appointment: 'Sur rendez-vous',
		walkIn: 'Sans rendez-vous',
		mailOnly: 'Par la poste uniquement',
		address: 'Adresse',
		directions: 'Itinéraire',
		directionsTo: (name) => ` vers ${name} (s’ouvre dans un nouvel onglet)`,
		hours: 'Horaires',
		timesIn: (zone) => `Heures : ${zone}`,
		openNow: (time) => `Ouvert · ferme à ${time}`,
		opens: (when, time) => `Fermé · ouvre ${when} à ${time}`,
		today: 'aujourd’hui',
		tomorrow: 'demain',
		closed: 'Fermé'
	},

	updates: {
		recaptured: (day) => `Page capturée à nouveau le ${day} ; modification trop importante pour être listée`,
		changesOn: (host) => `Modifications sur ${host}`,
		added: 'Ajouté : ',
		removed: 'Supprimé : ',
		more: (n) => `${n === 1 ? 'Une autre ligne modifiée' : `${n} autres lignes modifiées`} sur la page.`,
		older: 'Mises à jour plus anciennes',
		none: 'Aucune modification à signaler pour l’instant.'
	},

	made: {
		eyebrow: 'Rédigé par un modèle, approuvé par une personne',
		heading: 'Comment cette page a été faite',
		summary: (n, models, pages) =>
			`${counted(n, 'enregistrement', 'enregistrements')}${models.length > 0 ? `, rédigés par ${models.join(' et ')}` : ''}${pages ? ` à partir de ${pages}` : ''}, chacun approuvé par un relecteur de notre équipe avant de figurer sur cette page.`,
		records: (n) => `Les enregistrements derrière cette page (${n})`,
		whatChanged: (day) => `Ce qui a changé le ${day}`,
		since: (day) => `· depuis le ${day}`
	},

	provenance: {
		pages: (n) => counted(n, 'page officielle', 'pages officielles'),
		captured: (day) => `Capturées le ${day}`,
		capturedAuthority: 'Capturées auprès de l’autorité',
		roles: {
			extract: 'A extrait chaque information des pages officielles',
			cross_check: 'A recoupé chaque information avec sa citation',
			compose: 'A composé ce plan à partir d’informations acceptées'
		},
		crossCheck: (a, b, c) => `${a} confirmées · ${b} contredites · ${c} introuvables`,
		checksPassed: (n) => `${n} contrôles réussis`,
		checksRun: (n) => `${n} contrôles effectués`,
		validator: 'Validateur déterministe : chaque bloc cite des informations acceptées de cette autorité',
		dropped: (n, k) => `${n} ${k <= 1 ? 'bloc écarté' : 'blocs écartés'}`,
		approved: 'Approuvé par un relecteur de notre équipe',
		accepted: (day, version) => `Accepté le ${day}${version ? ` · plan v${version}` : ''}`,
		acceptedBefore: 'Accepté avant publication',
		step: (n) => `Étape ${n}`,
		runTime: 'Durée d’exécution',
		tokens: (kind) => `Jetons ${kind}`,
		tokenKinds: { Input: 'd’entrée', Output: 'de sortie', Cached: 'en cache', Reasoning: 'de raisonnement' }
	},

	endnotes: {
		heading: 'Sources',
		intro:
			'Chaque numéro de la page renvoie à une source ci-dessous. Les citations sont reprises mot pour mot de la page nommée ; tout le reste est indiqué comme note de l’équipe ou comme calcul.',
		backTo: (label) => `Revenir à : ${label}`,
		backToCitation: 'Revenir à la citation',
		basedOn: 'D’après'
	},

	card: {
		close: 'Fermer'
	},

	toggle: {
		citations: 'Citations',
		print: 'Imprimer',
		hintOff: 'Activez les citations pour imprimer les sources.',
		hintOn: 'Les sources s’impriment à la fin.'
	},

	print: {
		brand: 'Apostille Handbook',
		dataAsOf: (at) => `Données au ${at}`,
		printed: (day) => `Imprimé le ${day}`,
		brackets: 'Les numéros entre crochets renvoient aux sources listées à la fin.',
		checkLive: 'Consultez la page en ligne avant d’envoyer des documents ou de l’argent.'
	},

	cite: {
		marker: 'Citation ',
		captured: (range) => `Capturé ${range}`,
		approved: (range, first) => `${first ? 'Approuvé' : 'approuvé'} ${range}`,
		sourcesCaptured: (range) => `Sources capturées ${range}`,
		approvedByStaff: (day) => `approuvées par l’équipe ${day}`,
		officialPages: (n) => counted(n, 'page officielle', 'pages officielles'),
		otherPages: (n) => counted(n, 'autre page web', 'autres pages web'),
		webPages: (n) => counted(n, 'page web', 'pages web'),
		calls: (n) => counted(n, 'appel téléphonique', 'appels téléphoniques'),
		emails: (n) => counted(n, 'e-mail', 'e-mails'),
		contradicts: 'contredit la valeur affichée',
		webPage: 'Page web officielle',
		openPage: 'Ouvrir la page',
		officialPdf: 'PDF officiel',
		pdfHead: (title, host) =>
			title ? `${title} — PDF officiel${host ? `, ${host}` : ''}.` : `PDF officiel${host ? ` — ${host}` : ''}.`,
		openPdf: 'Ouvrir le PDF',
		officialUpload: (pdf) => (pdf ? 'PDF officiel' : 'Document officiel'),
		uploadedBy: (host) => (host ? `téléversé par l’équipe depuis ${host}` : 'téléversé par l’équipe'),
		uploadHead: (title, pdf, from) =>
			title ? `${title} — ${pdf ? 'PDF' : 'document'} officiel ${from}.` : `${pdf ? 'PDF' : 'Document'} officiel ${from}.`,
		openDocument: 'Ouvrir le document',
		call: 'Appel téléphonique',
		staffNotes: 'notes de l’équipe',
		notRecorded: 'notes de l’équipe, non enregistré',
		checkNumber: 'Vérifiez vous-même au même numéro.',
		email: 'E-mail',
		emailFrom: (from) => `E-mail de ${from}`,
		emailNote: 'note de l’équipe ; l’e-mail n’est pas publié',
		visit: 'Visite au bureau',
		notFoundOn: (pages) => `Introuvable sur ces pages du bureau : ${pages}`,
		notFound: 'Introuvable sur les pages du bureau',
		checkedByStaff: 'vérifié par l’équipe',
		calculated: 'Calculé sur cette page',
		staffNote: 'Note de l’équipe',
		hidden: 'Citations masquées',
		shown: (n) => `Citations affichées : ${counted(n, 'source numérotée', 'sources numérotées')}`
	},

	contest: {
		lead: (how, day) => `Note de l’équipe : le bureau nous a dit autre chose ${byHow(how)}${day ? ` le ${day}` : ''}`
	}
};
