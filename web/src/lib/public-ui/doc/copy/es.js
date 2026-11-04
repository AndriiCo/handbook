// The customer guide's own words in Spanish (key for key with en.js;
// read the notes there). Machine-drafted: awaiting a native speaker's
// review. Fact values, quotes and office names stay as the office's
// pages write them; only these template words are translated.
//
// Terms: apostilla, oficina (the issuing office), tarifa, envío por
// correo / por mensajería / en persona, plazo de tramitación, fuente,
// cita. Register: usted.

/** @param {number} n @param {string} one @param {string} many */
const counted = (n, one, many) => `${n} ${n === 1 ? one : many}`;

/** @param {'email'|'visit'|'call'} how */
const byHow = (how) => (how === 'email' ? 'por correo electrónico' : how === 'visit' ? 'en la oficina' : 'por teléfono');

/** @type {import('./index.js').GuideCopy} */
export default {
	intl: 'es',

	present: {
		eyebrow: 'Guía de apostilla',
		h1: (region) => `Obtener una apostilla en ${region}`,
		lede: (issuer) => `Expedida por: ${issuer}.`,
		staleWarning: 'Puede estar desactualizado: confírmelo con la oficina antes de enviar dinero',
		staleMonths: (months) => `Capturado hace ${months} meses`,
		absent: 'No figura en el sitio web de la oficina',
		pointerUsps: 'Las páginas de la oficina indican apartados postales distintos: lea primero el paso del envío por correo',
		pointer: 'Las páginas de la oficina dan valores distintos: lea primero el paso de abajo',
		pointerContested: 'La oficina nos dijo algo distinto: lea primero el paso de abajo',
		homePage: 'Página de inicio',
		notConfirmed: 'Sin confirmar: pregunte a la oficina',
		remoteLabel: 'Notarización remota',
		remoteText:
			'No hemos confirmado si la oficina acepta documentos notarizados a distancia. Pregunte antes de pagar por una.',
		detail: 'Dato',
		routes: {
			mail: 'Por correo',
			courier: 'Por mensajería',
			visit: 'En persona',
			visitSameDay: 'En persona, el mismo día'
		}
	},

	sections: {
		before: 'Compruebe que es la oficina correcta',
		prepare: 'Prepare su documento',
		form: 'Rellene el formulario de solicitud',
		fees: 'Pague las tarifas',
		send: 'Envíe o lleve sus documentos',
		after: 'Recíbalo y revíselo',
		contact: 'Contacte con la oficina'
	},

	slots: {
		'jurisdiction.scope': 'Documentos que tramita esta oficina',
		'jurisdiction.federal_route': 'Documentos federales',
		'jurisdiction.refusals': 'Lo que la oficina no apostilla',
		'authentication.product': 'Tipo de certificado',
		'authentication.domestic_validity': 'Uso en el país emisor',
		'notarization.wa_notary': 'Requisitos notariales',
		'copies.vital_records': 'Actas de nacimiento, defunción y matrimonio',
		'forms.form': 'Formulario de solicitud',
		'forms.revision': 'Versión del formulario',
		'mailing.enclose': 'Qué incluir',
		'fees.per_document': 'Tarifa por documento',
		'fees.expedite': 'Tarifa del servicio urgente',
		'fees.same_day': 'Tarifa del servicio en el mismo día',
		'fees.refund': 'Reembolsos',
		'payment.payee': 'A nombre de',
		'payment.methods_counter': 'Pago en persona',
		'mailing.usps': 'Dirección postal',
		'mailing.courier': 'Dirección para mensajería',
		'mailing.courier_rule': 'Envío por mensajería',
		'address.office': 'Dirección de la oficina',
		'hours.weekday': 'Horario de atención',
		'walkin.cutoff': 'Última hora de llegada',
		'walkin.same_day_only': 'Solo servicio en el mismo día',
		'walkin.no_mail': 'No se aceptan solicitudes por correo',
		'walkin.public_record': 'Registros públicos',
		'holiday_hours.closure': 'Cierres de la oficina',
		'processing_time.standard': 'Plazo de tramitación normal',
		'processing_time.expedited': 'Plazo de tramitación urgente',
		'processing_time.same_day': 'Tramitación en el mismo día',
		'mailing.return': 'Cómo se devuelve',
		'website.verify': 'Verificar una apostilla en línea',
		'contacts.phone': 'Teléfono',
		'contacts.phone_option': 'Opción del menú telefónico',
		'contacts.phone_hours': 'Horario telefónico',
		'contacts.email': 'Correo electrónico',
		'contacts.chat': 'Chat en línea',
		'contacts.tty': 'Teléfono de texto (TTY)',
		'contacts.no_texts': 'Mensajes de texto',
		'service_notice.notice': 'Aviso de servicio',
		'service_notice.restored': 'Servicio restablecido'
	},

	fields: {
		per_document: 'Tarifa por documento',
		apostille_main_branch_street: 'Dirección de la oficina',
		standard: 'Tramitación normal',
		weekday: 'Horario entre semana',
		po_box: 'Apartado postal',
		methods: 'Formas de pago aceptadas'
	},

	topics: {
		hours: 'Horario',
		fees: 'Tarifas',
		forms: 'Formularios',
		address: 'Dirección',
		parking: 'Aparcamiento',
		route_photos: 'Fotos del recorrido',
		rejections: 'Rechazos',
		contacts: 'Contacto',
		po_box: 'Apartado postal',
		walkin: 'Atención presencial',
		mailing: 'Envío por correo',
		appointment: 'Cita previa',
		payment: 'Pago',
		processing_time: 'Plazo de tramitación',
		jurisdiction: 'Competencia',
		legalization_chain: 'Cadena de legalización',
		website: 'Sitio web',
		holiday_hours: 'Horario en días festivos',
		id_requirements: 'Requisitos de identificación',
		copies: 'Copias',
		translation: 'Traducción',
		notarization: 'Notarización',
		authentication: 'Autenticación',
		dropbox: 'Buzón',
		service_notice: 'Avisos de servicio'
	},

	glance: {
		heading: 'De un vistazo',
		rows: { cost: 'Coste', time: 'Plazo', pay: 'Pago', send: 'Envío', form: 'Formulario' },
		parts: {
			'processing_time.standard': 'Normal',
			'processing_time.expedited': 'Urgente',
			'processing_time.same_day': 'Mismo día',
			'payment.payee': 'A nombre de',
			'payment.methods_counter': 'En persona',
			'mailing.usps': 'Por correo',
			'mailing.courier': 'Por mensajería',
			'address.office': 'En persona'
		},
		or: 'o',
		worded: '(redactado de forma distinta en las páginas de la oficina)',
		downloadPdf: 'Descargar (PDF)',
		getOnPage: 'Obténgalo en la página de la oficina',
		fasterStep: (step) => `Opciones más rápidas en el paso ${step}`,
		fasterFees: 'Opciones más rápidas en «Pague las tarifas»'
	},

	guide: {
		citeFailed: 'Las fuentes no se cargaron, así que los números aún no pueden abrirlas.',
		retry: 'Reintentar',
		updates: 'Novedades',
		afterNote: 'Cuente con tiempo adicional para el correo de ida y vuelta.',
		issuerSos: 'la Secretaría de Estado',
		issuerOffice: 'la oficina emisora',
		nav: {
			before: 'Oficina correcta',
			prepare: 'Preparar',
			form: 'Formulario',
			fees: 'Tarifas',
			send: 'Enviar',
			after: 'Recibirlo',
			contact: 'Contacto',
			updates: 'Novedades'
		},
		originalNote:
			'Los datos citados de las páginas de la oficina se muestran en su idioma original, tal como la oficina los publica.'
	},

	common: {
		newTab: '(se abre en una pestaña nueva)',
		sourceNotOnFile: 'Fuente no archivada',
		alsoOnPages: 'También en las páginas de la oficina',
		notConfirmed: 'Sin confirmar',
		copyNames: {
			payee: 'el nombre del beneficiario',
			mailing: 'la dirección postal',
			courier: 'la dirección para mensajería',
			recommended: 'la dirección postal recomendada',
			office: 'la dirección de la oficina',
			cityOffice: (city) => `la dirección de la oficina de ${city}`
		}
	},

	header: {
		breadcrumb: 'Ruta de navegación',
		home: 'Inicio',
		flag: (country) => `Bandera: ${country}`,
		unofficial: 'Guía no oficial, no es un sitio web del gobierno',
		officialSite: 'Sitio oficial:'
	},

	status: {
		serviceNotice: 'Aviso de servicio',
		capturedFrom: (day) => `Capturado de la página de la oficina el ${day}`,
		noOpen: (day) => `No hay avisos de servicio abiertos en el registro de avisos de la oficina (última lectura: ${day}).`,
		resolved: 'Resuelto:'
	},

	section: {
		step: (n) => `Paso ${n}:`,
		call: 'Llamar a este número',
		email: 'Escribir un correo',
		openPage: 'Abrir la página de la oficina',
		beforeApply: 'Antes de presentar la solicitud',
		chain: 'Cadena de legalización',
		chainNote: 'Cada etapa de abajo procede de las propias páginas de la oficina, en orden.',
		notConfirmedBody:
			'Las páginas de la oficina no lo indican. Pregunte a la oficina antes de enviar documentos o dinero:'
	},

	row: {
		conflict: 'Las páginas de la oficina dan valores distintos',
		worded: 'Redactado de forma distinta en las páginas de la oficina'
	},

	copyButton: {
		label: 'Copiar',
		copied: (text) => `Copiado: ${text}`,
		selected: (text) => `Seleccionado: ${text}. Cópielo con el atajo de teclado.`,
		selectYourself: (text) => `Selecciónelo y cópielo usted: ${text}`
	},

	contents: { label: 'En esta página' },

	routes: {
		label: 'Formas de enviar sus documentos',
		hints: {
			mail: 'Envíe los documentos a la oficina por correo.',
			courier: 'Envíelos con una empresa de mensajería urgente.',
			visit: 'Llévelos usted mismo a la ventanilla.'
		}
	},

	empty: {
		heading: 'Aún sin comprobar',
		body: 'Solo publicamos un dato después de que una persona lo coteja con las propias páginas de la oficina. Nadie lo ha hecho todavía para esta oficina, así que esta página no muestra tarifas, direcciones ni plazos de tramitación.',
		listed: 'Sitio oficial registrado:',
		notChecked: '— aún no comprobado por nosotros',
		browse: 'Ver las oficinas que hemos comprobado'
	},

	about: {
		heading: 'Acerca de esta guía',
		unofficial: 'Guía no oficial, no es un sitio web del gobierno.',
		applyDirect: ['Puede presentar la solicitud directamente ante ', ''],
		perDocument: [' por ', ' por documento'],
		rule: 'Solo publicamos un dato después de que un revisor del personal lo coteja con las propias palabras de la oficina. Información general, no asesoramiento jurídico: confírmelo con la oficina antes de enviar documentos o dinero.',
		report: 'Informar de un cambio en las páginas de la oficina',
		reportContinue: 'Continuar su informe de un cambio (borrador guardado)'
	},

	follow: {
		title: (name) => `Recibir novedades de ${name}`,
		description:
			'Un correo cuando el personal acepte un dato modificado de esta oficina. Nada más, nunca.',
		checkInbox: 'Revise su bandeja de entrada para confirmar.',
		sent: (email) =>
			`Hemos enviado un enlace de confirmación a ${email}: caduca en 24 horas y no se envía nada hasta que haga clic en él.`,
		emailLabel: 'Dirección de correo electrónico',
		sending: 'Enviando…',
		follow: 'Seguir',
		invalid: 'Introduzca una dirección de correo electrónico válida.',
		tooMany: 'Demasiados intentos: espere unos minutos y vuelva a intentarlo.',
		off: 'Las novedades por correo están desactivadas ahora mismo: inténtelo más tarde.',
		failed: 'Algo salió mal: inténtelo más tarde.',
		privacy:
			'Privacidad: solo guardamos su dirección (sellada: el personal no puede leerla) y la oficina que sigue. Confirme antes de que se envíe nada; como máximo un resumen por oficina y por hora; cada correo incluye un enlace para darse de baja con un clic y nuestra dirección postal. Darse de baja siempre funciona, al instante.'
	},

	fees: {
		calculator: 'Calcule su total',
		documents: 'Número de documentos',
		fewer: 'Un documento menos',
		more: 'Un documento más',
		speed: 'Rapidez',
		speeds: { standard: 'Normal', expedite: 'Urgente', same_day: 'En el mismo día, en persona' },
		callOffice: 'Llame a la oficina:',
		yourTotal: 'Su total',
		totalCounter: 'Total en ventanilla',
		totalEnclose: 'Total a adjuntar',
		sameDayFee: 'tarifa del mismo día',
		expediteFee: 'tarifa urgente',
		range: (from, to) => (from === to ? counted(from, 'documento', 'documentos') : `${from}–${to} documentos`),
		askOffice: 'Pregunte a la oficina antes de pagar.',
		ask: {
			conflict: 'Las páginas de la oficina indican importes distintos para esta tarifa.',
			contested: 'La oficina nos dijo algo distinto de lo que dicen sus páginas.',
			'over-max': 'Esta calculadora no calcula totales para tantos documentos.',
			'no-tier': 'Las páginas de la oficina no indican esta tarifa para tantos documentos.',
			count: 'Indique cuántos documentos va a enviar.',
			other: 'Las propias páginas de la oficina no dan datos suficientes para calcular este total.'
		}
	},

	form: {
		download: 'Descargar el formulario oficial (PDF)',
		getOnPage: 'Obtener el formulario en la página de la oficina',
		fill: 'Rellénelo en línea, gratis',
		fillNote:
			'Pasamos sus respuestas al propio formulario de la oficina; usted lo imprime, lo firma y lo envía por correo. Requiere una cuenta gratuita.'
	},

	conflict: {
		worded: 'Redactado de forma distinta en las páginas de la oficina',
		heading: 'Las propias páginas de la oficina indican direcciones postales distintas',
		printedOn: 'Publicado en:',
		recommendation: 'Nuestra recomendación',
		recommendationStaff: (day) => `Nuestra recomendación (personal${day ? `, ${day}` : ''}):`,
		use: 'use',
		confirmed: (how, channel) => ` — confirmado ${byHow(how)} con la oficina${channel ? ` (${channel})` : ''}`,
		toldUs: (how, day) => `La oficina nos lo dijo ${byHow(how)}${day ? ` el ${day}` : ''}:`,
		whatYouCanDo: 'Qué puede hacer:',
		call: ['Llame al ', ' antes de enviarlo por correo.'],
		sendCourier: (first) => `${first ? 'Envíelo' : 'O envíelo'} por mensajería a la `,
		overnight: 'dirección para envíos urgentes',
		askWhich:
			'Pregunte a la oficina qué dirección está vigente antes de enviarlo por correo. Sus datos de contacto están en',
		contactLink: 'Contacte con la oficina'
	},

	send: {
		mail: 'Por correo',
		mailUs: 'Por correo (USPS)',
		courier: 'Por mensajería',
		courierUs: 'Por mensajería (FedEx, UPS)',
		visit: 'En persona',
		visitSameDay: 'En persona, el mismo día',
		carriers: 'Transportistas que nombran las páginas de la oficina',
		notStated: (carrier) => `${carrier}: no se indica`,
		otherCarriers: 'Otros transportistas',
		mailingMissing: 'Dirección postal',
		courierMissing: 'Dirección para mensajería',
		courierPhone: [
			'Las etiquetas de mensajería piden un teléfono del destinatario; el número publicado de la oficina es el ',
			'.'
		]
	},

	label: {
		fedex: ['Imprima una etiqueta prepagada de FedEx para la dirección de mensajería ', '— desde '],
		unavailable:
			'Las etiquetas de mensajería prepagadas no están disponibles para esta oficina ahora mismo. Puede enviarlo usted por FedEx o UPS: copie la dirección para mensajería de arriba.',
		paidBy: (issuer) => `Servicio de pago de Apostille Handbook, no de ${issuer}.`,
		free: 'Imprima una etiqueta gratuita con la dirección postal',
		freeBy: (issuer) => `Servicio gratuito de Apostille Handbook, no de ${issuer}.`,
		courierCity: 'Ciudad de mensajería'
	},

	offices: {
		google: (title) => `Abrir en Google Maps: ${title} (se abre en una pestaña nueva)`,
		apple: (title) => `Abrir en Apple Maps: ${title} (se abre en una pestaña nueva)`,
		cutoff: (time) => `· Hora límite de hoy: ${time}`,
		timesIn: (zone) => `Horas en ${zone}`,
		office: 'Oficina',
		officeMissing: 'Dirección de la oficina',
		every: 'Para todas las oficinas',
		upcoming: 'Próximos cierres',
		more: 'Más cierres',
		closedToday: 'Cerrado hoy',
		openNow: (time) => `Abierto ahora · cierra a las ${time}`,
		opensToday: (time) => `Cerrado ahora · abre hoy a las ${time}`,
		opensOn: (when, time) => `Cerrado ahora · abre ${when} a las ${time}`,
		tomorrow: 'mañana',
		closedNow: 'Cerrado ahora',
		noon: 'mediodía'
	},

	officeCard: {
		appointment: 'Con cita previa',
		walkIn: 'Sin cita',
		mailOnly: 'Solo por correo',
		address: 'Dirección',
		directions: 'Cómo llegar',
		directionsTo: (name) => ` a ${name} (se abre en una pestaña nueva)`,
		hours: 'Horario',
		timesIn: (zone) => `Horas en ${zone}`,
		openNow: (time) => `Abierto ahora · cierra a las ${time}`,
		opens: (when, time) => `Cerrado · abre ${when} a las ${time}`,
		today: 'hoy',
		tomorrow: 'mañana',
		closed: 'Cerrado'
	},

	updates: {
		recaptured: (day) => `Página capturada de nuevo el ${day}; el cambio es demasiado grande para listarlo`,
		changesOn: (host) => `Cambios en ${host}`,
		added: 'Añadido: ',
		removed: 'Eliminado: ',
		more: (n) => `${n === 1 ? 'Una línea más cambió' : `${n} líneas más cambiaron`} en la página.`,
		older: 'Novedades anteriores',
		none: 'Aún no hay cambios que mostrar.'
	},

	made: {
		eyebrow: 'Redactado por un modelo o citado por el personal, aprobado por una persona',
		heading: 'Cómo se hizo esta página',
		summary: (n, models, pages) =>
			`${counted(n, 'registro', 'registros')}${models.length > 0 ? `, redactados por ${models.join(' y ')}` : ''}${pages ? ` a partir de ${pages}` : ''}; cada uno lo aprobó un revisor del personal antes de llegar a esta página.`,
		staff: 'el personal',
		records: (n) => `Los registros de esta página (${n})`,
		whatChanged: (day) => `Qué cambió el ${day}`,
		since: (day) => `· desde el ${day}`
	},

	provenance: {
		pages: (n) => counted(n, 'página oficial', 'páginas oficiales'),
		captured: (day) => `Capturadas el ${day}`,
		capturedAuthority: 'Capturadas de la autoridad',
		roles: {
			extract: 'Extrajo datos de las páginas oficiales',
			cross_check: 'Contrastó cada dato con su cita',
			compose: 'Redactó este plan a partir de datos aceptados'
		},
		crossCheck: (a, b, c) => `${a} respaldados · ${b} contradichos · ${c} no encontrados`,
		checksPassed: (n) => `${n} comprobaciones superadas`,
		checksRun: (n) => `${n} comprobaciones realizadas`,
		validator: 'Validador determinista: cada bloque cita datos aceptados de esta autoridad',
		dropped: (n, k) => `${n} ${k === 1 ? 'bloque descartado' : 'bloques descartados'}`,
		approved: 'Aprobado por un revisor del personal',
		accepted: (day, version) => `Aceptado el ${day}${version ? ` · plan v${version}` : ''}`,
		acceptedBefore: 'Aceptado antes de publicarse',
		step: (n) => `Paso ${n}`,
		runTime: 'Tiempo de ejecución',
		tokens: (kind) => `Tokens de ${kind}`,
		tokenKinds: { Input: 'entrada', Output: 'salida', Cached: 'caché', Reasoning: 'razonamiento' }
	},

	endnotes: {
		heading: 'Fuentes',
		intro:
			'Cada número de la página remite a una fuente de abajo. Las citas se copian palabra por palabra de la página indicada; todo lo demás se marca como nota del personal o como cálculo.',
		backTo: (label) => `Volver a ${label}`,
		backToCitation: 'Volver a la cita',
		basedOn: 'Basado en'
	},

	card: {
		close: 'Cerrar'
	},

	toggle: {
		citations: 'Citas',
		print: 'Imprimir',
		hintOff: 'Active las citas para imprimir las fuentes.',
		hintOn: 'Las fuentes se imprimen al final.'
	},

	print: {
		brand: 'Apostille Handbook',
		dataAsOf: (at) => `Datos a fecha de ${at}`,
		printed: (day) => `Impreso el ${day}`,
		brackets: 'Los números entre corchetes remiten a las fuentes enumeradas al final.',
		checkLive: 'Consulte la página en línea antes de enviar documentos o dinero.'
	},

	cite: {
		marker: 'Cita ',
		captured: (range) => `Capturado ${range}`,
		approved: (range, first) => `${first ? 'Aprobado' : 'aprobado'} ${range}`,
		sourcesCaptured: (range) => `Fuentes capturadas ${range}`,
		approvedByStaff: (day) => `aprobadas por el personal ${day}`,
		officialPages: (n) => counted(n, 'página oficial', 'páginas oficiales'),
		otherPages: (n) => counted(n, 'otra página web', 'otras páginas web'),
		webPages: (n) => counted(n, 'página web', 'páginas web'),
		calls: (n) => counted(n, 'llamada telefónica', 'llamadas telefónicas'),
		emails: (n) => counted(n, 'correo electrónico', 'correos electrónicos'),
		contradicts: 'contradice el valor mostrado',
		webPage: 'Página web oficial',
		openPage: 'Abrir la página',
		officialPdf: 'PDF oficial',
		pdfHead: (title, host) =>
			title ? `${title} — PDF oficial${host ? `, ${host}` : ''}.` : `PDF oficial${host ? ` — ${host}` : ''}.`,
		openPdf: 'Abrir el PDF',
		officialUpload: (pdf) => (pdf ? 'PDF oficial' : 'Documento oficial'),
		uploadedBy: (host) => (host ? `subido por el personal desde ${host}` : 'subido por el personal'),
		uploadHead: (title, pdf, from) =>
			title ? `${title} — ${pdf ? 'PDF' : 'documento'} oficial ${from}.` : `${pdf ? 'PDF' : 'Documento'} oficial ${from}.`,
		openDocument: 'Abrir el documento',
		call: 'Llamada telefónica',
		staffNotes: 'notas del personal',
		notRecorded: 'notas del personal, no grabada',
		checkNumber: 'Compruébelo usted mismo en el mismo número.',
		email: 'Correo electrónico',
		emailFrom: (from) => `Correo electrónico de ${from}`,
		emailNote: 'nota del personal; el correo no se publica',
		visit: 'Visita a la oficina',
		notFoundOn: (pages) => `No figura en estas páginas de la oficina: ${pages}`,
		notFound: 'No figura en las páginas de la oficina',
		checkedByStaff: 'comprobado por el personal',
		calculated: 'Calculado en esta página',
		staffNote: 'Nota del personal',
		hidden: 'Citas ocultas',
		shown: (n) => `Citas visibles: ${counted(n, 'fuente numerada', 'fuentes numeradas')}`
	},

	contest: {
		lead: (how, day) => `Nota del personal: la oficina nos dijo otra cosa ${byHow(how)}${day ? ` el ${day}` : ''}`
	}
};
