// The /order wizard's words in the three site languages (lib/i18n.ts
// LOCALES), with the components only the wizard renders (OrderStepper,
// DeliveryCard, FactChip, JourneyGraphic). The page picks one set by the
// shared site locale (lib/locale.svelte.ts), so the masthead switcher
// re-renders the wizard in place. What the API sends stays as sent:
// authority and country names, plan headlines and path labels, option
// titles and disabled reasons, price line labels, form fields, fact
// statements and server `error` text.

import { ENUM_LABELS, INTL_LOCALE } from './orders-copy.js';

/** @typedef {'en' | 'es' | 'fr'} CopyLocale */

/**
 * A capture date (ISO text or unix seconds) as a calendar day in the
 * site language. Read in UTC: the stamp names the day it was captured.
 * Anything unreadable comes back as given.
 * @param {string | number | null | undefined} value
 * @param {CopyLocale} [locale]
 * @returns {string}
 */
export function fmtDay(value, locale = 'en') {
	if (value === null || value === undefined || value === '') return '';
	const date = typeof value === 'number' ? new Date(value * 1000) : new Date(String(value));
	if (Number.isNaN(date.getTime())) return String(value).split('T')[0];
	return date.toLocaleDateString(INTL_LOCALE[locale], {
		year: 'numeric',
		month: 'short',
		day: 'numeric',
		timeZone: 'UTC'
	});
}


/**
 * @typedef {{
 *   eyebrow: string,
 *   intro: string,
 *   unavailableHeading: string,
 *   loadFailed: string,
 *   loadingForm: string,
 *   noDestHeading: string,
 *   noDestBody: string,
 *   backToIndex: string,
 *   steps: { destination: string, document: string, form: string, delivery: string, pay: string },
 *   stepperLabel: string,
 *   whereUsed: string,
 *   destAuthority: string,
 *   chooseAuthority: string,
 *   searchDest: string,
 *   noDestMatch: string,
 *   otherCountry: string,
 *   fromPlan: string,
 *   continue: string,
 *   back: string,
 *   chooseDestHint: string,
 *   whichDoc: string,
 *   loadingDocs: string,
 *   docTypeFor: (authority: string) => string,
 *   publicDocument: string,
 *   planCites: (n: number) => string,
 *   noDocPlan: string,
 *   noPagePlan: (authority: string) => string,
 *   chooseDocHint: string,
 *   fillForm: string,
 *   loadingTheForm: string,
 *   officialForm: string,
 *   filledFile: (title: string) => string,
 *   required: string,
 *   onOfficialForm: string,
 *   freeFillHeading: string,
 *   freeFillIntro: (title: string) => string,
 *   freeFillNeedsAccount: string,
 *   signInToFill: string,
 *   fillAgree: string,
 *   downloadFilled: string,
 *   fillKeeps: string,
 *   checkFirst: string,
 *   fillTick: string,
 *   fieldProblem: (code: string, label: string, message: string, maxLen?: number) => string,
 *   downloadYours: string,
 *   printSign: string,
 *   openGuide: (authority: string) => string,
 *   paperInstead: string,
 *   stillNeeded: (labels: string[]) => string,
 *   noOnlineFormHeading: string,
 *   noOnlineFormBody: (authority: string) => string,
 *   fillDailyCap: (cap: number) => string,
 *   rateLimited: string,
 *   fillRejected: (why: string) => string,
 *   fillGone: string,
 *   fillFailed: string,
 *   formFillGeneric: string,
 *   howTravel: string,
 *   testModeNoCharges: string,
 *   signInToChoose: string,
 *   accountKeeps: string,
 *   signIn: string,
 *   loadingDelivery: string,
 *   deliveryUnavailable: string,
 *   deliveryRetry: string,
 *   tryAgain: string,
 *   paidPaused: string,
 *   freeStillWorks: string,
 *   deliveryOption: string,
 *   returnAddress: string,
 *   returnAddressHint: string,
 *   fullName: string,
 *   street: string,
 *   apartment: string,
 *   city: string,
 *   region: string,
 *   postalCode: string,
 *   countryCode: string,
 *   countryHelp: string,
 *   signInToContinue: string,
 *   createFreeLabel: string,
 *   gettingQuote: string,
 *   getQuote: string,
 *   chooseDeliveryHint: string,
 *   addAddressHint: string,
 *   orderFailed: string,
 *   quoteUnavailable: string,
 *   pricesUpdated: string,
 *   checkoutFailed: string,
 *   checkoutClosed: string,
 *   freeLabelCreated: string,
 *   pausedTitle: string,
 *   reviewPay: string,
 *   signInToPay: string,
 *   orderInAccount: string,
 *   comeBackToDelivery: string,
 *   checkoutPaused: string,
 *   freeAddressLabel: string,
 *   printAtHome: string,
 *   useFreeLabel: string,
 *   addAddressIn: string,
 *   addAddressFirst: string,
 *   colon: string,
 *   yourOrder: string,
 *   total: string,
 *   roles: { kit_to_customer: string, outbound: string, return: string },
 *   aboutDays: (n: number) => string,
 *   testMode: string,
 *   quoteHeld: (time: string) => string,
 *   stripeNote: string,
 *   openingCheckout: string,
 *   payAmount: (amount: string) => string,
 *   noQuote: string,
 *   freshQuote: string,
 *   chooseDeliveryFirst: string,
 *   pickTravel: string,
 *   backToDelivery: string,
 *   yourApostille: string,
 *   waiting: string,
 *   waitingHint: string,
 *   notChosen: string,
 *   noneOnFile: string,
 *   paperForm: string,
 *   fieldsFilled: (n: number, total: number) => string,
 *   free: string,
 *   fromPrice: (price: string) => string,
 *   fromAuthority: string,
 *   per: (unit: string) => string,
 *   needsJs: string,
 *   unavailableHere: string,
 *   showFact: (id: number) => string,
 *   factNo: (id: number) => string,
 *   captured: (day: string) => string,
 *   draftedBy: (model: string) => string,
 *   draftedByStaff: string,
 *   source: string,
 *   factFallback: (id: number) => string,
 *   you: string,
 *   chooseDestination: string,
 *   journeyLegs: (legs: string) => string,
 *   journeyBetween: (authority: string) => string,
 *   journeyChoose: string
 * }} OrderCopy
 */

/** @type {Record<CopyLocale, OrderCopy>} */
export const ORDER_COPY = {
	en: {
		eyebrow: 'Apostille order',
		intro:
			"Pick where the document is going and how it travels. Every option and price below comes from the authority's accepted facts.",
		unavailableHeading: 'The order form is unavailable.',
		loadFailed: 'The authority index is unavailable right now. Try again in a minute.',
		loadingForm: 'Loading order form',
		noDestHeading: 'No destinations yet.',
		noDestBody: 'The handbook has no published authorities, so there is nothing to order yet.',
		backToIndex: 'Back to the authority index',
		steps: {
			destination: 'Destination',
			document: 'Document',
			form: 'Form',
			delivery: 'Delivery',
			pay: 'Pay'
		},
		stepperLabel: 'Order progress',
		whereUsed: 'Where will the document be used?',
		destAuthority: 'Destination authority',
		chooseAuthority: 'Choose an authority…',
		searchDest: 'Search destinations…',
		noDestMatch: 'No destination matches.',
		otherCountry: 'Other',
		fromPlan: 'From the accepted plan:',
		continue: 'Continue',
		back: 'Back',
		chooseDestHint: 'Choose a destination to continue.',
		whichDoc: 'Which document are you sending?',
		loadingDocs: 'Loading document types',
		docTypeFor: (authority) => `Document type for ${authority}`,
		publicDocument: 'Public document',
		planCites: (n) => `Plan cites ${n} ${n === 1 ? 'accepted fact' : 'accepted facts'}`,
		noDocPlan: 'No document-specific plan yet',
		noPagePlan: (authority) => `${authority} has no page plan yet. Its accepted facts cover:`,
		chooseDocHint: 'Choose a document type to continue.',
		fillForm: 'Fill in the request form',
		loadingTheForm: 'Loading the form',
		officialForm: 'Official form',
		filledFile: (title) => `${title} (filled).pdf`,
		required: '(required)',
		onOfficialForm: 'On the official form:',
		freeFillHeading: 'Fill it in online — free',
		freeFillIntro: (title) =>
			`No order and no payment: we put your answers into ${title}, the office's own form. You print it, sign it and send it yourself.`,
		freeFillNeedsAccount:
			'The free fill needs a free account. Only your account can open the filled PDF.',
		signInToFill: 'Sign in to fill it in',
		fillAgree:
			"I understand this fills the office's own form with my answers. It is general information, not legal advice.",
		downloadFilled: 'Download the filled form (free)',
		fillKeeps:
			'We keep your answers and the filled PDF with your account until you ask us to delete them. Only your account can open the download link. Your account does not list past fills, so save the PDF to your device.',
		checkFirst: 'Check these first:',
		fillTick: 'Tick the box to confirm you understand what the free fill does.',
		// English keeps the shared validator's own wording (forms-validate.js).
		fieldProblem: (_code, _label, message) => message,
		downloadYours: 'Download your filled form (PDF)',
		printSign:
			'Print it, sign it, and send it with your documents and payment — the guide lists the address and fee.',
		openGuide: (authority) => `Open the guide for ${authority}`,
		paperInstead: 'Fill the paper form instead',
		stillNeeded: (labels) => `Still needed: ${labels.join(', ')}.`,
		noOnlineFormHeading: 'No online form for this authority.',
		noOnlineFormBody: (authority) =>
			`There is no fillable form on file for ${authority}. Continue to delivery; the authority's dossier lists what to send.`,
		fillDailyCap: (cap) =>
			`Your account has reached the limit of ${cap} filled forms in 24 hours. Try again later.`,
		rateLimited: 'Too many attempts — wait a minute and try again.',
		fillRejected: (why) => `The official form could not be filled: ${why}`,
		fillGone: 'This form is no longer on file. Reload the page to get the current one.',
		fillFailed: 'The form could not be filled right now. Check your connection and try again.',
		formFillGeneric: 'The official form could not be filled. Check your answers and try again.',
		howTravel: 'How should it travel?',
		testModeNoCharges: 'Test mode · no real charges',
		signInToChoose: 'Sign in to choose delivery.',
		accountKeeps: 'Orders, labels and tracking live in your account. Your choices so far are kept.',
		signIn: 'Sign in',
		loadingDelivery: 'Loading delivery options',
		deliveryUnavailable: 'Delivery options are unavailable right now.',
		deliveryRetry: 'Try again in a moment; your choices so far are kept.',
		tryAgain: 'Try again',
		paidPaused: 'Paid options are paused right now.',
		freeStillWorks: 'The free address label still works.',
		deliveryOption: 'Delivery option',
		returnAddress: 'Your return address',
		returnAddressHint: 'Printed on your labels, and where the apostilled document comes back to.',
		fullName: 'Full name',
		street: 'Street address',
		apartment: 'Apartment, suite (optional)',
		city: 'City',
		region: 'State or region',
		postalCode: 'Postal code',
		countryCode: 'Country code',
		countryHelp: 'Two letters, for example GB.',
		signInToContinue: 'Sign in to continue',
		createFreeLabel: 'Create free label',
		gettingQuote: 'Getting your quote',
		getQuote: 'Get quote',
		chooseDeliveryHint: 'Choose a delivery option.',
		addAddressHint: 'Add your return address to continue.',
		orderFailed: 'We could not create the order. Check your connection and try again.',
		quoteUnavailable: 'The quote is unavailable right now. Try again in a moment.',
		pricesUpdated: 'Prices were updated. Review the new total, then pay.',
		checkoutFailed: 'Checkout did not open. Your order is saved; try again in a moment.',
		checkoutClosed: 'Checkout closed. Your order is saved; pay when you are ready.',
		freeLabelCreated: 'Free address label created.',
		pausedTitle: 'Paid options are paused',
		reviewPay: 'Review and pay',
		signInToPay: 'Sign in to pay.',
		orderInAccount: 'Your order lives in your account.',
		comeBackToDelivery: 'and you will come back to delivery.',
		checkoutPaused: 'Checkout is paused.',
		freeAddressLabel: 'Free address label',
		printAtHome: '· print it at home and add your own postage.',
		useFreeLabel: 'Use the free label',
		addAddressIn: 'Add your return address in',
		addAddressFirst: ' first.',
		colon: ':',
		yourOrder: 'Your order',
		total: 'Total',
		roles: ENUM_LABELS.en.role,
		aboutDays: (n) => `, about ${n} ${n === 1 ? 'day' : 'days'}`,
		testMode: 'Test mode',
		quoteHeld: (time) => `Quote held until ${time}.`,
		stripeNote: "Payment opens on Stripe's secure checkout.",
		openingCheckout: 'Opening checkout',
		payAmount: (amount) => `Pay ${amount}`,
		noQuote: 'No quote yet.',
		freshQuote: 'Get a fresh quote for this order.',
		chooseDeliveryFirst: 'Choose delivery first.',
		pickTravel: 'Pick how the document travels, then the quote appears here.',
		backToDelivery: 'Back to delivery',
		yourApostille: 'Your apostille',
		waiting: 'Waiting for your choices',
		waitingHint: 'Choose a destination to see your order summary',
		notChosen: 'Not chosen yet',
		noneOnFile: 'None on file',
		paperForm: 'Paper form',
		fieldsFilled: (n, total) => `${n} of ${total} fields filled`,
		free: 'Free',
		fromPrice: (price) => `from ${price}`,
		fromAuthority: 'From the authority',
		per: (unit) => `per ${unit}`,
		needsJs: 'Placing an order needs JavaScript.',
		unavailableHere: 'Unavailable for this authority',
		showFact: (id) => `Show the cited fact ${id}`,
		factNo: (id) => `Fact #${id}`,
		captured: (day) => `captured ${day}`,
		draftedBy: (model) => `drafted by ${model}, approved by staff`,
		draftedByStaff: 'quoted and approved by staff',
		source: 'Source',
		factFallback: (id) => `Accepted fact #${id}. Its quote is on the authority's dossier.`,
		you: 'You',
		chooseDestination: 'Choose a destination',
		journeyLegs: (legs) => `Journey: ${legs}`,
		journeyBetween: (authority) => `Journey between you and ${authority}`,
		journeyChoose: 'Journey: choose a destination'
	},
	es: {
		eyebrow: 'Pedido de apostilla',
		intro:
			'Elija adónde va el documento y cómo viaja. Cada opción y cada precio de abajo provienen de los hechos aceptados de la autoridad.',
		unavailableHeading: 'El formulario de pedido no está disponible.',
		loadFailed:
			'El índice de autoridades no está disponible en este momento. Inténtelo de nuevo en un minuto.',
		loadingForm: 'Cargando el formulario de pedido',
		noDestHeading: 'Todavía no hay destinos.',
		noDestBody:
			'El manual no tiene autoridades publicadas, así que todavía no se puede hacer ningún pedido.',
		backToIndex: 'Volver al índice de autoridades',
		steps: {
			destination: 'Destino',
			document: 'Documento',
			form: 'Formulario',
			delivery: 'Envío',
			pay: 'Pago'
		},
		stepperLabel: 'Progreso del pedido',
		whereUsed: '¿Dónde se usará el documento?',
		destAuthority: 'Autoridad de destino',
		chooseAuthority: 'Elija una autoridad…',
		searchDest: 'Buscar destinos…',
		noDestMatch: 'Ningún destino coincide.',
		otherCountry: 'Otros',
		fromPlan: 'Según el plan aceptado:',
		continue: 'Continuar',
		back: 'Atrás',
		chooseDestHint: 'Elija un destino para continuar.',
		whichDoc: '¿Qué documento va a enviar?',
		loadingDocs: 'Cargando los tipos de documento',
		docTypeFor: (authority) => `Tipo de documento para ${authority}`,
		publicDocument: 'Documento público',
		planCites: (n) => `El plan cita ${n} ${n === 1 ? 'hecho aceptado' : 'hechos aceptados'}`,
		noDocPlan: 'Todavía no hay un plan específico para este documento',
		noPagePlan: (authority) =>
			`${authority} todavía no tiene un plan de página. Sus hechos aceptados cubren:`,
		chooseDocHint: 'Elija un tipo de documento para continuar.',
		fillForm: 'Rellene el formulario de solicitud',
		loadingTheForm: 'Cargando el formulario',
		officialForm: 'Formulario oficial',
		filledFile: (title) => `${title} (rellenado).pdf`,
		required: '(obligatorio)',
		onOfficialForm: 'En el formulario oficial:',
		freeFillHeading: 'Rellénelo en línea, gratis',
		freeFillIntro: (title) =>
			`Sin pedido y sin pago: pasamos sus respuestas a ${title}, el formulario propio de la oficina. Usted lo imprime, lo firma y lo envía.`,
		freeFillNeedsAccount:
			'El rellenado gratuito requiere una cuenta gratuita. Solo su cuenta puede abrir el PDF rellenado.',
		signInToFill: 'Inicie sesión para rellenarlo',
		fillAgree:
			'Entiendo que esto rellena el formulario propio de la oficina con mis respuestas. Es información general, no asesoramiento jurídico.',
		downloadFilled: 'Descargar el formulario rellenado (gratis)',
		fillKeeps:
			'Guardamos sus respuestas y el PDF rellenado con su cuenta hasta que nos pida eliminarlos. Solo su cuenta puede abrir el enlace de descarga. Su cuenta no muestra los formularios rellenados anteriormente, así que guarde el PDF en su dispositivo.',
		checkFirst: 'Revise esto primero:',
		fillTick: 'Marque la casilla para confirmar que entiende lo que hace el rellenado gratuito.',
		fieldProblem: (code, label, message, maxLen) => {
			switch (code) {
				case 'required':
					return `${label}: campo obligatorio`;
				case 'max_len':
					return maxLen === undefined
						? `${label}: el texto es demasiado largo`
						: `${label}: ${maxLen} caracteres como máximo`;
				case 'pattern':
					return `${label}: el formato no es válido`;
				case 'type':
					return `${label}: el valor no es válido`;
				case 'email':
					return `${label}: debe ser un correo electrónico válido`;
				case 'tel':
					return `${label}: debe ser un número de teléfono válido`;
				case 'date':
					return `${label}: debe ser una fecha como 2026-11-04`;
				case 'number':
					return `${label}: debe ser un número`;
				case 'money':
					return `${label}: debe ser un importe como 25.00`;
				case 'option':
					return `${label}: elija una de las opciones de la lista`;
				case 'address':
					return `${label}: la dirección está incompleta`;
				default:
					return message;
			}
		},
		downloadYours: 'Descargar su formulario rellenado (PDF)',
		printSign:
			'Imprímalo, fírmelo y envíelo junto con sus documentos y el pago; la guía indica la dirección y la tasa.',
		openGuide: (authority) => `Abrir la guía de ${authority}`,
		paperInstead: 'Prefiero rellenar el formulario en papel',
		stillNeeded: (labels) => `Falta: ${labels.join(', ')}.`,
		noOnlineFormHeading: 'No hay formulario en línea para esta autoridad.',
		noOnlineFormBody: (authority) =>
			`No tenemos ningún formulario rellenable de ${authority}. Continúe con el envío; el expediente de la autoridad indica qué enviar.`,
		fillDailyCap: (cap) =>
			`Su cuenta ha alcanzado el límite de ${cap} formularios rellenados en 24 horas. Inténtelo de nuevo más tarde.`,
		rateLimited: 'Demasiados intentos: espere un minuto e inténtelo de nuevo.',
		fillRejected: (why) => `No se pudo rellenar el formulario oficial: ${why}`,
		fillGone:
			'Este formulario ya no está disponible. Vuelva a cargar la página para obtener el actual.',
		fillFailed:
			'No se pudo rellenar el formulario en este momento. Compruebe su conexión e inténtelo de nuevo.',
		formFillGeneric:
			'No se pudo rellenar el formulario oficial. Revise sus respuestas e inténtelo de nuevo.',
		howTravel: '¿Cómo debe viajar?',
		testModeNoCharges: 'Modo de prueba · sin cargos reales',
		signInToChoose: 'Inicie sesión para elegir el envío.',
		accountKeeps:
			'Los pedidos, las etiquetas de envío y el seguimiento están en su cuenta. Se conservan las opciones elegidas hasta ahora.',
		signIn: 'Iniciar sesión',
		loadingDelivery: 'Cargando las opciones de envío',
		deliveryUnavailable: 'Las opciones de envío no están disponibles en este momento.',
		deliveryRetry: 'Inténtelo de nuevo en un momento; se conservan las opciones elegidas hasta ahora.',
		tryAgain: 'Reintentar',
		paidPaused: 'Las opciones de pago están en pausa en este momento.',
		freeStillWorks: 'La etiqueta de dirección gratuita sigue disponible.',
		deliveryOption: 'Opción de envío',
		returnAddress: 'Su dirección de devolución',
		returnAddressHint:
			'Se imprime en sus etiquetas de envío y es adonde vuelve el documento apostillado.',
		fullName: 'Nombre completo',
		street: 'Dirección',
		apartment: 'Apartamento, oficina (opcional)',
		city: 'Ciudad',
		region: 'Estado o región',
		postalCode: 'Código postal',
		countryCode: 'Código de país',
		countryHelp: 'Dos letras, por ejemplo ES.',
		signInToContinue: 'Inicie sesión para continuar',
		createFreeLabel: 'Crear etiqueta gratuita',
		gettingQuote: 'Obteniendo su presupuesto',
		getQuote: 'Obtener presupuesto',
		chooseDeliveryHint: 'Elija una opción de envío.',
		addAddressHint: 'Añada su dirección de devolución para continuar.',
		orderFailed: 'No pudimos crear el pedido. Compruebe su conexión e inténtelo de nuevo.',
		quoteUnavailable:
			'El presupuesto no está disponible en este momento. Inténtelo de nuevo en un momento.',
		pricesUpdated: 'Los precios se han actualizado. Revise el nuevo total y después pague.',
		checkoutFailed:
			'No se abrió el pago. Su pedido está guardado; inténtelo de nuevo en un momento.',
		checkoutClosed: 'Pago cerrado. Su pedido está guardado; pague cuando quiera.',
		freeLabelCreated: 'Etiqueta de dirección gratuita creada.',
		pausedTitle: 'Las opciones de pago están en pausa',
		reviewPay: 'Revisar y pagar',
		signInToPay: 'Inicie sesión para pagar.',
		orderInAccount: 'Su pedido está en su cuenta.',
		comeBackToDelivery: 'y volverá al paso de envío.',
		checkoutPaused: 'El pago está en pausa.',
		freeAddressLabel: 'Etiqueta de dirección gratuita',
		printAtHome: '· imprímala en casa y ponga usted el franqueo.',
		useFreeLabel: 'Usar la etiqueta gratuita',
		addAddressIn: 'Primero añada su dirección de devolución en',
		addAddressFirst: '.',
		colon: ':',
		yourOrder: 'Su pedido',
		total: 'Total',
		roles: ENUM_LABELS.es.role,
		aboutDays: (n) => `, unos ${n} ${n === 1 ? 'día' : 'días'}`,
		testMode: 'Modo de prueba',
		quoteHeld: (time) => `Presupuesto válido hasta las ${time}.`,
		stripeNote: 'El pago se abre en la página de pago segura de Stripe.',
		openingCheckout: 'Abriendo el pago',
		payAmount: (amount) => `Pagar ${amount}`,
		noQuote: 'Todavía no hay presupuesto.',
		freshQuote: 'Obtenga un presupuesto actualizado para este pedido.',
		chooseDeliveryFirst: 'Elija primero el envío.',
		pickTravel: 'Elija cómo viaja el documento y el presupuesto aparecerá aquí.',
		backToDelivery: 'Volver al envío',
		yourApostille: 'Su apostilla',
		waiting: 'Esperando sus elecciones',
		waitingHint: 'Elija un destino para ver el resumen de su pedido',
		notChosen: 'Sin elegir todavía',
		noneOnFile: 'Ninguno disponible',
		paperForm: 'Formulario en papel',
		fieldsFilled: (n, total) =>
			`${n} de ${total} ${total === 1 ? 'campo rellenado' : 'campos rellenados'}`,
		free: 'Gratis',
		fromPrice: (price) => `desde ${price}`,
		fromAuthority: 'Según la autoridad',
		per: (unit) => `por ${unit}`,
		needsJs: 'Para hacer un pedido se necesita JavaScript.',
		unavailableHere: 'No disponible para esta autoridad',
		showFact: (id) => `Mostrar el hecho citado ${id}`,
		factNo: (id) => `Hecho n.º ${id}`,
		captured: (day) => `capturado el ${day}`,
		draftedBy: (model) => `redactado por ${model}, aprobado por el personal`,
		draftedByStaff: 'citado y aprobado por el personal',
		source: 'Fuente',
		factFallback: (id) =>
			`Hecho aceptado n.º ${id}. Su cita figura en el expediente de la autoridad.`,
		you: 'Usted',
		chooseDestination: 'Elija un destino',
		journeyLegs: (legs) => `Trayecto: ${legs}`,
		journeyBetween: (authority) => `Trayecto entre usted y ${authority}`,
		journeyChoose: 'Trayecto: elija un destino'
	},
	fr: {
		eyebrow: 'Commande d’apostille',
		intro:
			'Choisissez la destination du document et son mode d’acheminement. Chaque option et chaque prix ci-dessous proviennent des faits acceptés de l’autorité.',
		unavailableHeading: 'Le formulaire de commande est indisponible.',
		loadFailed:
			'L’index des autorités est indisponible pour le moment. Réessayez dans une minute.',
		loadingForm: 'Chargement du formulaire de commande',
		noDestHeading: 'Aucune destination pour l’instant.',
		noDestBody:
			'Le manuel n’a aucune autorité publiée : il n’y a donc encore rien à commander.',
		backToIndex: 'Retour à l’index des autorités',
		steps: {
			destination: 'Destination',
			document: 'Document',
			form: 'Formulaire',
			delivery: 'Livraison',
			pay: 'Paiement'
		},
		stepperLabel: 'Progression de la commande',
		whereUsed: 'Où le document sera-t-il utilisé ?',
		destAuthority: 'Autorité de destination',
		chooseAuthority: 'Choisissez une autorité…',
		searchDest: 'Rechercher une destination…',
		noDestMatch: 'Aucune destination ne correspond.',
		otherCountry: 'Autres',
		fromPlan: 'D’après le plan accepté :',
		continue: 'Continuer',
		back: 'Retour',
		chooseDestHint: 'Choisissez une destination pour continuer.',
		whichDoc: 'Quel document envoyez-vous ?',
		loadingDocs: 'Chargement des types de document',
		docTypeFor: (authority) => `Type de document pour ${authority}`,
		publicDocument: 'Document public',
		planCites: (n) => `Le plan cite ${n} ${n === 1 ? 'fait accepté' : 'faits acceptés'}`,
		noDocPlan: 'Pas encore de plan propre à ce document',
		noPagePlan: (authority) =>
			`${authority} n’a pas encore de plan de page. Ses faits acceptés portent sur :`,
		chooseDocHint: 'Choisissez un type de document pour continuer.',
		fillForm: 'Remplissez le formulaire de demande',
		loadingTheForm: 'Chargement du formulaire',
		officialForm: 'Formulaire officiel',
		filledFile: (title) => `${title} (rempli).pdf`,
		required: '(obligatoire)',
		onOfficialForm: 'Sur le formulaire officiel :',
		freeFillHeading: 'Remplissez-le en ligne, gratuitement',
		freeFillIntro: (title) =>
			`Ni commande ni paiement : nous reportons vos réponses dans ${title}, le formulaire du bureau lui-même. Vous l’imprimez, le signez et l’envoyez vous-même.`,
		freeFillNeedsAccount:
			'Le remplissage gratuit nécessite un compte gratuit. Seul votre compte peut ouvrir le PDF rempli.',
		signInToFill: 'Connectez-vous pour le remplir',
		fillAgree:
			'Je comprends que ceci remplit le formulaire du bureau avec mes réponses. Il s’agit d’une information générale, et non d’un conseil juridique.',
		downloadFilled: 'Télécharger le formulaire rempli (gratuit)',
		fillKeeps:
			'Nous conservons vos réponses et le PDF rempli avec votre compte jusqu’à ce que vous nous demandiez de les supprimer. Seul votre compte peut ouvrir le lien de téléchargement. Votre compte ne liste pas les formulaires déjà remplis : enregistrez donc le PDF sur votre appareil.',
		checkFirst: 'Vérifiez d’abord ceci :',
		fillTick: 'Cochez la case pour confirmer que vous comprenez ce que fait le remplissage gratuit.',
		fieldProblem: (code, label, message, maxLen) => {
			switch (code) {
				case 'required':
					return `${label} : champ obligatoire`;
				case 'max_len':
					return maxLen === undefined
						? `${label} : texte trop long`
						: `${label} : ${maxLen} caractères au maximum`;
				case 'pattern':
					return `${label} : format non valide`;
				case 'type':
					return `${label} : valeur non valide`;
				case 'email':
					return `${label} : doit être une adresse e-mail valide`;
				case 'tel':
					return `${label} : doit être un numéro de téléphone valide`;
				case 'date':
					return `${label} : doit être une date au format 2026-11-04`;
				case 'number':
					return `${label} : doit être un nombre`;
				case 'money':
					return `${label} : doit être un montant comme 25.00`;
				case 'option':
					return `${label} : choisissez l’une des options proposées`;
				case 'address':
					return `${label} : adresse incomplète`;
				default:
					return message;
			}
		},
		downloadYours: 'Télécharger votre formulaire rempli (PDF)',
		printSign:
			'Imprimez-le, signez-le et envoyez-le avec vos documents et votre paiement : le guide indique l’adresse et les frais.',
		openGuide: (authority) => `Ouvrir le guide de ${authority}`,
		paperInstead: 'Remplir plutôt le formulaire papier',
		stillNeeded: (labels) => `Encore requis : ${labels.join(', ')}.`,
		noOnlineFormHeading: 'Aucun formulaire en ligne pour cette autorité.',
		noOnlineFormBody: (authority) =>
			`Nous n’avons aucun formulaire à remplir pour ${authority}. Passez à la livraison ; le dossier de l’autorité indique quoi envoyer.`,
		fillDailyCap: (cap) =>
			`Votre compte a atteint la limite de ${cap} formulaires remplis en 24 heures. Réessayez plus tard.`,
		rateLimited: 'Trop de tentatives : patientez une minute puis réessayez.',
		fillRejected: (why) => `Le formulaire officiel n’a pas pu être rempli : ${why}`,
		fillGone:
			'Ce formulaire n’est plus disponible. Rechargez la page pour obtenir la version actuelle.',
		fillFailed:
			'Le formulaire n’a pas pu être rempli pour le moment. Vérifiez votre connexion et réessayez.',
		formFillGeneric:
			'Le formulaire officiel n’a pas pu être rempli. Vérifiez vos réponses et réessayez.',
		howTravel: 'Comment doit-il voyager ?',
		testModeNoCharges: 'Mode test · aucun débit réel',
		signInToChoose: 'Connectez-vous pour choisir la livraison.',
		accountKeeps:
			'Les commandes, les étiquettes d’expédition et le suivi se trouvent dans votre compte. Vos choix sont conservés.',
		signIn: 'Se connecter',
		loadingDelivery: 'Chargement des options de livraison',
		deliveryUnavailable: 'Les options de livraison sont indisponibles pour le moment.',
		deliveryRetry: 'Réessayez dans un instant ; vos choix sont conservés.',
		tryAgain: 'Réessayer',
		paidPaused: 'Les options payantes sont suspendues pour le moment.',
		freeStillWorks: 'L’étiquette d’adresse gratuite reste disponible.',
		deliveryOption: 'Option de livraison',
		returnAddress: 'Votre adresse de retour',
		returnAddressHint:
			'Imprimée sur vos étiquettes d’expédition ; c’est là que le document apostillé vous revient.',
		fullName: 'Nom complet',
		street: 'Adresse',
		apartment: 'Appartement, bureau (facultatif)',
		city: 'Ville',
		region: 'État ou région',
		postalCode: 'Code postal',
		countryCode: 'Code pays',
		countryHelp: 'Deux lettres, par exemple FR.',
		signInToContinue: 'Connectez-vous pour continuer',
		createFreeLabel: 'Créer l’étiquette gratuite',
		gettingQuote: 'Calcul de votre devis',
		getQuote: 'Obtenir un devis',
		chooseDeliveryHint: 'Choisissez une option de livraison.',
		addAddressHint: 'Ajoutez votre adresse de retour pour continuer.',
		orderFailed:
			'Nous n’avons pas pu créer la commande. Vérifiez votre connexion et réessayez.',
		quoteUnavailable: 'Le devis est indisponible pour le moment. Réessayez dans un instant.',
		pricesUpdated: 'Les prix ont été mis à jour. Vérifiez le nouveau total, puis payez.',
		checkoutFailed:
			'Le paiement ne s’est pas ouvert. Votre commande est enregistrée ; réessayez dans un instant.',
		checkoutClosed:
			'Paiement fermé. Votre commande est enregistrée ; payez quand vous serez prêt.',
		freeLabelCreated: 'Étiquette d’adresse gratuite créée.',
		pausedTitle: 'Les options payantes sont suspendues',
		reviewPay: 'Vérifier et payer',
		signInToPay: 'Connectez-vous pour payer.',
		orderInAccount: 'Votre commande se trouve dans votre compte.',
		comeBackToDelivery: 'et vous reviendrez à l’étape de livraison.',
		checkoutPaused: 'Le paiement est suspendu.',
		freeAddressLabel: 'Étiquette d’adresse gratuite',
		printAtHome: '· imprimez-la chez vous et ajoutez votre propre affranchissement.',
		useFreeLabel: 'Utiliser l’étiquette gratuite',
		addAddressIn: 'Ajoutez d’abord votre adresse de retour à l’étape',
		addAddressFirst: '.',
		colon: ' :',
		yourOrder: 'Votre commande',
		total: 'Total',
		roles: ENUM_LABELS.fr.role,
		aboutDays: (n) => `, environ ${n} ${n === 1 ? 'jour' : 'jours'}`,
		testMode: 'Mode test',
		quoteHeld: (time) => `Devis garanti jusqu’à ${time}.`,
		stripeNote: 'Le paiement s’ouvre sur la page de paiement sécurisée de Stripe.',
		openingCheckout: 'Ouverture du paiement',
		payAmount: (amount) => `Payer ${amount}`,
		noQuote: 'Pas encore de devis.',
		freshQuote: 'Obtenez un devis à jour pour cette commande.',
		chooseDeliveryFirst: 'Choisissez d’abord la livraison.',
		pickTravel: 'Choisissez comment le document voyage ; le devis apparaîtra ici.',
		backToDelivery: 'Retour à la livraison',
		yourApostille: 'Votre apostille',
		waiting: 'En attente de vos choix',
		waitingHint: 'Choisissez une destination pour voir le récapitulatif de votre commande',
		notChosen: 'Pas encore choisi',
		noneOnFile: 'Aucun disponible',
		paperForm: 'Formulaire papier',
		fieldsFilled: (n, total) =>
			`${n} ${n <= 1 ? 'champ rempli' : 'champs remplis'} sur ${total}`,
		free: 'Gratuit',
		fromPrice: (price) => `à partir de ${price}`,
		fromAuthority: 'D’après l’autorité',
		per: (unit) => `par ${unit}`,
		needsJs: 'Passer une commande nécessite JavaScript.',
		unavailableHere: 'Indisponible pour cette autorité',
		showFact: (id) => `Afficher le fait cité ${id}`,
		factNo: (id) => `Fait n° ${id}`,
		captured: (day) => `capturé le ${day}`,
		draftedBy: (model) => `rédigé par ${model}, approuvé par le personnel`,
		draftedByStaff: 'cité et approuvé par le personnel',
		source: 'Source',
		factFallback: (id) =>
			`Fait accepté n° ${id}. Sa citation figure dans le dossier de l’autorité.`,
		you: 'Vous',
		chooseDestination: 'Choisissez une destination',
		journeyLegs: (legs) => `Trajet : ${legs}`,
		journeyBetween: (authority) => `Trajet entre vous et ${authority}`,
		journeyChoose: 'Trajet : choisissez une destination'
	}
};
