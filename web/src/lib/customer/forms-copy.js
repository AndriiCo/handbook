// The /forms fill page and the /forms/[id] submission page in the three
// site languages (lib/i18n.ts LOCALES). Only the page chrome is here:
// the form schema (titles, field labels, help, options, withheld
// reasons, replica notices) and server `error` text come from the API
// and stay as written.
//
// Validation messages: forms-validate.js keeps the English wording the
// Rust validator pins (forms-validate-vectors.json), so `fieldError`
// returns that message as-is in English and re-words it by error
// `code` in Spanish and French.

/** @typedef {{ code?: string, message: string }} FieldErrorLike */
/** @typedef {{ label: string, type?: string, max_len?: number | null }} FieldLike */
/** @typedef {'name' | 'street' | 'city' | 'state' | 'zip'} AddrPart */

/**
 * @typedef {{
 *   title: string,
 *   heading: string,
 *   pickIntro: string,
 *   authority: string,
 *   formCode: string,
 *   loading: string,
 *   loadForm: string,
 *   cannotLoad: string,
 *   noActiveForm: string,
 *   underReverification: string,
 *   unreachable: string,
 *   edition: (code: string, revision: string, version: number) => string,
 *   aboutPrintout: string,
 *   formUpdated: string,
 *   superseded: string,
 *   added: (list: string) => string,
 *   removed: (list: string) => string,
 *   changed: (list: string) => string,
 *   carryOver: (version: number) => string,
 *   prefilledFrom: (id: number) => string,
 *   typeManually: string,
 *   reportIt: string,
 *   whatLooksWrong: string,
 *   sendReport: string,
 *   sending: string,
 *   choose: string,
 *   yes: string,
 *   addressLegend: (label: string) => string,
 *   addrPart: Record<AddrPart, string>,
 *   notYet: string,
 *   draftSaved: string,
 *   draftStored: (id: number) => string,
 *   reportSent: string,
 *   saveDraft: string,
 *   review: string,
 *   reviewHeading: string,
 *   yesValue: string,
 *   noValue: string,
 *   overrideHeading: string,
 *   overrideBody: string,
 *   submitFailed: string,
 *   backToEdit: string,
 *   submitting: string,
 *   submitRequest: string,
 *   needsAttention: string,
 *   draftFailed: string,
 *   serverRejected: string,
 *   submitFailedBody: string,
 *   correctionThanks: string,
 *   correctionFailed: string,
 *   fieldError: (err: FieldErrorLike, field: FieldLike) => string,
 *   submission: (id: string | number) => string,
 *   submissionNotFound: string,
 *   submissionUnreachable: string,
 *   cannotLoadSubmission: string,
 *   submittedAt: (date: string) => string,
 *   updates: (state: string) => string,
 *   liveState: Record<'connecting' | 'live' | 'reconnecting' | 'offline', string>,
 *   overrideOnFile: string,
 *   overrideOnFileBody: string,
 *   downloadPdf: string,
 *   printSign: string,
 *   noPdf: string,
 *   noPdfBody: string,
 *   somethingWrong: string,
 *   reportPrompt: string,
 *   submissionThanks: string
 * }} FormsCopy
 */

/** The address part named in `… is missing {part}` (forms-validate.js). */
const missingPart = (/** @type {string} */ message) =>
	/** @type {AddrPart | null} */ (/ is missing (name|street|city|state|zip)$/.exec(message)?.[1] ?? null);

/**
 * Spanish and French validation wording by error code; unknown codes
 * fall back to the English message.
 * @param {Partial<Record<string, (label: string, field: FieldLike, err: FieldErrorLike) => string>>} rules
 * @returns {(err: FieldErrorLike, field: FieldLike) => string}
 */
const byCode = (rules) => (err, field) => {
	const rule = err.code ? rules[err.code] : undefined;
	return rule ? rule(field.label, field, err) : err.message;
};

/** @type {Record<'en' | 'es' | 'fr', FormsCopy>} */
export const FORMS_COPY = {
	en: {
		title: 'Forms',
		heading: 'Apostille request form',
		pickIntro:
			'Pick the issuing authority and the form. Verified addresses and fees come prefilled; the server re-checks everything on submit.',
		authority: 'Authority',
		formCode: 'Form code',
		loading: 'Loading…',
		loadForm: 'Load form',
		cannotLoad: 'Cannot load this form.',
		noActiveForm: 'No active form for that authority and code.',
		underReverification: 'This form is under re-verification. Please try again later.',
		unreachable: 'Form service unreachable — is the API server running?',
		edition: (code, revision, version) =>
			`${code} · edition ${revision} (v${version}) · wet-ink signature: print and sign after submitting`,
		aboutPrintout: 'About your printout.',
		formUpdated: 'This form was updated.',
		superseded: 'This form edition was superseded.',
		added: (list) => `Added: ${list}`,
		removed: (list) => `Removed: ${list}`,
		changed: (list) => `Changed: ${list}`,
		carryOver: (version) => `Load v${version} and carry my entries over`,
		prefilledFrom: (id) => `Prefilled from verified record #${id} — fix it if it is wrong.`,
		typeManually: 'Type it manually, or',
		reportIt: 'report it',
		whatLooksWrong: 'What looks wrong?',
		sendReport: 'Send report',
		sending: 'Sending…',
		choose: 'Choose…',
		yes: 'Yes',
		addressLegend: (label) => `${label} address`,
		addrPart: { name: 'name', street: 'street', city: 'city', state: 'state', zip: 'zip' },
		notYet: 'Not yet.',
		draftSaved: 'Draft saved.',
		draftStored: (id) => `Draft #${id} is stored; submit it from the review step.`,
		reportSent: 'Report sent.',
		saveDraft: 'Save draft',
		review: 'Review',
		reviewHeading: 'Review',
		yesValue: 'yes',
		noValue: 'no',
		overrideHeading: 'You edited a prefilled value.',
		overrideBody: 'Staff will see the override flag on this submission.',
		submitFailed: 'Submit failed.',
		backToEdit: 'Back to edit',
		submitting: 'Submitting…',
		submitRequest: 'Submit request',
		needsAttention: 'Some fields need attention before review.',
		draftFailed: 'Could not save the draft. Nothing was lost — your entries are still above.',
		serverRejected: 'The server rejected some fields. Your entries are preserved.',
		submitFailedBody: 'Submission failed. Nothing was stored — please try again.',
		correctionThanks: 'Thanks — staff will re-check this value. The form stays usable.',
		correctionFailed: 'Could not send the report. Please try again.',
		fieldError: (err) => err.message,
		submission: (id) => `Submission #${id}`,
		submissionNotFound: 'Submission not found. It may belong to another account.',
		submissionUnreachable: 'Submission service unreachable — is the API server running?',
		cannotLoadSubmission: 'Cannot load this submission.',
		submittedAt: (date) => `submitted ${date}`,
		updates: (state) => `updates: ${state}`,
		liveState: {
			connecting: 'connecting',
			live: 'live',
			reconnecting: 'reconnecting',
			offline: 'offline'
		},
		overrideOnFile: 'Prefill override on file.',
		overrideOnFileBody: 'You typed over a prefilled value; staff can see the flag.',
		downloadPdf: 'Download filled PDF',
		printSign: 'Print it, sign in ink on the signature line, and mail it with your own postage.',
		noPdf: 'No PDF yet.',
		noPdfBody:
			'This submission is still a draft, or the PDF is rendering — this page refreshes itself.',
		somethingWrong: 'Something looks wrong?',
		reportPrompt: 'Report it and staff will re-check the prefilled values.',
		submissionThanks: 'Thanks — staff will re-check this submission.'
	},
	es: {
		title: 'Formularios',
		heading: 'Formulario de solicitud de apostilla',
		pickIntro:
			'Elija la autoridad emisora y el formulario. Las direcciones y tarifas verificadas vienen ya completadas; el servidor vuelve a comprobarlo todo al enviar.',
		authority: 'Autoridad',
		formCode: 'Código del formulario',
		loading: 'Cargando…',
		loadForm: 'Cargar formulario',
		cannotLoad: 'No se puede cargar este formulario.',
		noActiveForm: 'No hay ningún formulario activo para esa autoridad y ese código.',
		underReverification:
			'Este formulario se está volviendo a verificar. Inténtelo de nuevo más tarde.',
		unreachable:
			'No se puede contactar con el servicio de formularios: ¿está en marcha el servidor de la API?',
		edition: (code, revision, version) =>
			`${code} · edición ${revision} (v${version}) · firma manuscrita: imprímalo y fírmelo después de enviarlo`,
		aboutPrintout: 'Sobre su copia impresa.',
		formUpdated: 'Este formulario se actualizó.',
		superseded: 'Esta edición del formulario fue reemplazada.',
		added: (list) => `Añadidos: ${list}`,
		removed: (list) => `Eliminados: ${list}`,
		changed: (list) => `Modificados: ${list}`,
		carryOver: (version) => `Cargar la v${version} y conservar mis respuestas`,
		prefilledFrom: (id) =>
			`Completado a partir del registro verificado n.º ${id}: corríjalo si es incorrecto.`,
		typeManually: 'Escríbalo manualmente o',
		reportIt: 'informe del error',
		whatLooksWrong: '¿Qué parece incorrecto?',
		sendReport: 'Enviar aviso',
		sending: 'Enviando…',
		choose: 'Elija…',
		yes: 'Sí',
		addressLegend: (label) => `Dirección: ${label}`,
		addrPart: {
			name: 'nombre',
			street: 'calle',
			city: 'ciudad',
			state: 'estado',
			zip: 'código postal'
		},
		notYet: 'Todavía no.',
		draftSaved: 'Borrador guardado.',
		draftStored: (id) =>
			`El borrador n.º ${id} está guardado; envíelo desde el paso de revisión.`,
		reportSent: 'Aviso enviado.',
		saveDraft: 'Guardar borrador',
		review: 'Revisar',
		reviewHeading: 'Revisión',
		yesValue: 'sí',
		noValue: 'no',
		overrideHeading: 'Ha modificado un valor completado automáticamente.',
		overrideBody: 'El personal verá la marca de modificación en esta solicitud.',
		submitFailed: 'El envío falló.',
		backToEdit: 'Volver a editar',
		submitting: 'Enviando…',
		submitRequest: 'Enviar solicitud',
		needsAttention: 'Algunos campos requieren su atención antes de la revisión.',
		draftFailed:
			'No se pudo guardar el borrador. No se ha perdido nada: sus respuestas siguen arriba.',
		serverRejected: 'El servidor rechazó algunos campos. Sus respuestas se conservan.',
		submitFailedBody: 'El envío falló. No se guardó nada: inténtelo de nuevo.',
		correctionThanks:
			'Gracias: el personal volverá a comprobar este valor. Puede seguir usando el formulario.',
		correctionFailed: 'No se pudo enviar el aviso. Inténtelo de nuevo.',
		fieldError: byCode({
			required: (label) => `«${label}» es obligatorio`,
			max_len: (label, field) =>
				`«${label}» admite como máximo ${field.max_len} caracteres`,
			pattern: (label) => `«${label}» tiene un formato no válido`,
			type: (label, field) =>
				field.type === 'checkbox'
					? `«${label}» debe estar marcado o desmarcado`
					: `«${label}» debe ser texto`,
			email: (label) => `«${label}» debe ser una dirección de correo electrónico válida`,
			tel: (label) => `«${label}» debe ser un número de teléfono válido`,
			date: (label) => `«${label}» debe ser una fecha como 2026-11-04`,
			number: (label) => `«${label}» debe ser un número`,
			money: (label) => `«${label}» debe ser un importe como 25.00`,
			option: (label) => `«${label}» debe ser una de las opciones de la lista`,
			address: (label, _field, err) => {
				const part = missingPart(err.message);
				return part
					? `A «${label}» le falta: ${FORMS_COPY.es.addrPart[part]}`
					: `«${label}» está incompleto`;
			}
		}),
		submission: (id) => `Solicitud n.º ${id}`,
		submissionNotFound: 'Solicitud no encontrada. Puede pertenecer a otra cuenta.',
		submissionUnreachable:
			'No se puede contactar con el servicio de solicitudes: ¿está en marcha el servidor de la API?',
		cannotLoadSubmission: 'No se puede cargar esta solicitud.',
		submittedAt: (date) => `enviada el ${date}`,
		updates: (state) => `novedades: ${state}`,
		liveState: {
			connecting: 'conectando',
			live: 'en vivo',
			reconnecting: 'reconectando',
			offline: 'sin conexión'
		},
		overrideOnFile: 'Modificación de un valor completado registrada.',
		overrideOnFileBody:
			'Ha sobrescrito un valor completado automáticamente; el personal puede ver la marca.',
		downloadPdf: 'Descargar el PDF completado',
		printSign:
			'Imprímalo, fírmelo con tinta en la línea de firma y envíelo por correo con su propio franqueo.',
		noPdf: 'Todavía no hay PDF.',
		noPdfBody:
			'Esta solicitud sigue siendo un borrador o el PDF se está generando; esta página se actualiza sola.',
		somethingWrong: '¿Algo parece incorrecto?',
		reportPrompt: 'Avísenos y el personal volverá a comprobar los valores completados.',
		submissionThanks: 'Gracias: el personal volverá a comprobar esta solicitud.'
	},
	fr: {
		title: 'Formulaires',
		heading: 'Formulaire de demande d’apostille',
		pickIntro:
			'Choisissez l’autorité émettrice et le formulaire. Les adresses et frais vérifiés sont préremplis ; le serveur revérifie tout à l’envoi.',
		authority: 'Autorité',
		formCode: 'Code du formulaire',
		loading: 'Chargement…',
		loadForm: 'Charger le formulaire',
		cannotLoad: 'Impossible de charger ce formulaire.',
		noActiveForm: 'Aucun formulaire actif pour cette autorité et ce code.',
		underReverification:
			'Ce formulaire est en cours de revérification. Veuillez réessayer plus tard.',
		unreachable: 'Service de formulaires injoignable : le serveur d’API est-il démarré ?',
		edition: (code, revision, version) =>
			`${code} · édition ${revision} (v${version}) · signature manuscrite : imprimez et signez après l’envoi`,
		aboutPrintout: 'À propos de votre impression.',
		formUpdated: 'Ce formulaire a été mis à jour.',
		superseded: 'Cette édition du formulaire a été remplacée.',
		added: (list) => `Ajoutés : ${list}`,
		removed: (list) => `Supprimés : ${list}`,
		changed: (list) => `Modifiés : ${list}`,
		carryOver: (version) => `Charger la v${version} et reprendre mes saisies`,
		prefilledFrom: (id) =>
			`Prérempli à partir de l’enregistrement vérifié n° ${id} : corrigez-le s’il est erroné.`,
		typeManually: 'Saisissez-le vous-même ou',
		reportIt: 'signalez-le',
		whatLooksWrong: 'Qu’est-ce qui semble incorrect ?',
		sendReport: 'Envoyer le signalement',
		sending: 'Envoi en cours…',
		choose: 'Choisissez…',
		yes: 'Oui',
		addressLegend: (label) => `Adresse : ${label}`,
		addrPart: { name: 'nom', street: 'rue', city: 'ville', state: 'État', zip: 'code postal' },
		notYet: 'Pas encore.',
		draftSaved: 'Brouillon enregistré.',
		draftStored: (id) =>
			`Le brouillon n° ${id} est enregistré ; envoyez-le depuis l’étape de vérification.`,
		reportSent: 'Signalement envoyé.',
		saveDraft: 'Enregistrer le brouillon',
		review: 'Vérifier',
		reviewHeading: 'Vérification',
		yesValue: 'oui',
		noValue: 'non',
		overrideHeading: 'Vous avez modifié une valeur préremplie.',
		overrideBody: 'Le personnel verra l’indicateur de modification sur cette demande.',
		submitFailed: 'L’envoi a échoué.',
		backToEdit: 'Revenir à la saisie',
		submitting: 'Envoi en cours…',
		submitRequest: 'Envoyer la demande',
		needsAttention: 'Certains champs demandent votre attention avant la vérification.',
		draftFailed:
			'Impossible d’enregistrer le brouillon. Rien n’est perdu : vos saisies sont toujours ci-dessus.',
		serverRejected: 'Le serveur a refusé certains champs. Vos saisies sont conservées.',
		submitFailedBody: 'L’envoi a échoué. Rien n’a été enregistré : veuillez réessayer.',
		correctionThanks:
			'Merci : le personnel revérifiera cette valeur. Le formulaire reste utilisable.',
		correctionFailed: 'Impossible d’envoyer le signalement. Veuillez réessayer.',
		fieldError: byCode({
			required: (label) => `« ${label} » est obligatoire`,
			max_len: (label, field) =>
				`« ${label} » doit comporter au plus ${field.max_len} caractères`,
			pattern: (label) => `« ${label} » a un format non valide`,
			type: (label, field) =>
				field.type === 'checkbox'
					? `« ${label} » doit être coché ou décoché`
					: `« ${label} » doit être du texte`,
			email: (label) => `« ${label} » doit être une adresse e-mail valide`,
			tel: (label) => `« ${label} » doit être un numéro de téléphone valide`,
			date: (label) => `« ${label} » doit être une date au format 2026-11-04`,
			number: (label) => `« ${label} » doit être un nombre`,
			money: (label) => `« ${label} » doit être un montant comme 25.00`,
			option: (label) => `« ${label} » doit être l’une des options proposées`,
			address: (label, _field, err) => {
				const part = missingPart(err.message);
				return part
					? `« ${label} » : il manque ${FORMS_COPY.fr.addrPart[part]}`
					: `« ${label} » est incomplet`;
			}
		}),
		submission: (id) => `Demande n° ${id}`,
		submissionNotFound: 'Demande introuvable. Elle appartient peut-être à un autre compte.',
		submissionUnreachable: 'Service des demandes injoignable : le serveur d’API est-il démarré ?',
		cannotLoadSubmission: 'Impossible de charger cette demande.',
		submittedAt: (date) => `envoyée le ${date}`,
		updates: (state) => `mises à jour : ${state}`,
		liveState: {
			connecting: 'connexion…',
			live: 'en direct',
			reconnecting: 'reconnexion…',
			offline: 'hors ligne'
		},
		overrideOnFile: 'Modification d’une valeur préremplie enregistrée.',
		overrideOnFileBody:
			'Vous avez remplacé une valeur préremplie ; le personnel voit l’indicateur.',
		downloadPdf: 'Télécharger le PDF rempli',
		printSign:
			'Imprimez-le, signez à l’encre sur la ligne de signature et envoyez-le par la poste à vos frais.',
		noPdf: 'Pas encore de PDF.',
		noPdfBody:
			'Cette demande est encore un brouillon, ou le PDF est en cours de génération ; cette page se met à jour toute seule.',
		somethingWrong: 'Quelque chose semble incorrect ?',
		reportPrompt: 'Signalez-le et le personnel revérifiera les valeurs préremplies.',
		submissionThanks: 'Merci : le personnel revérifiera cette demande.'
	}
};
