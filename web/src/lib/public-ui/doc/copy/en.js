// The customer guide's own words in English: the source table that
// es.js and fr.js translate key for key (doc/copy/index.js picks one by
// the site language). Only template copy lives here. Fact values,
// quotes, office names and page titles come from the office's pages
// and stay in their own language (docs/site-theme.md §6).
//
// Template copy carries no digits, `$`, addresses or dates of its own:
// every such token is an argument.

/** @param {number} n @param {string} one @param {string} many */
const counted = (n, one, many) => `${n} ${n === 1 ? one : many}`;

export default {
	/** The `Intl` locale tag dates and numbers format in. */
	intl: 'en-US',

	present: {
		eyebrow: 'Apostille guide',
		/** @param {string} region */
		h1: (region) => `Get an apostille in ${region}`,
		/** @param {string} issuer the issuing office's name */
		lede: (issuer) => `Issued by ${issuer}.`,
		staleWarning: 'May be out of date — check with the office before you send money',
		/** @param {number} months */
		staleMonths: (months) => `Captured ${months} months ago`,
		absent: "Not on the office's website",
		pointerUsps: "The office's pages list different PO boxes — read the mailing step first",
		pointer: "The office's pages give different values — read the step below first",
		pointerContested: 'The office told us something different — read the step below first',
		homePage: 'Home page',
		notConfirmed: 'Not confirmed yet — ask the office',
		remoteLabel: 'Remote notarization',
		remoteText:
			"We haven't confirmed whether the office accepts remotely notarized documents. Ask before you pay for one.",
		/** A label for an unkeyed field with no name. */
		detail: 'Detail',
		routes: {
			mail: 'By mail',
			courier: 'By courier',
			visit: 'In person',
			visitSameDay: 'In person, same day'
		}
	},

	/** Section headings by section id (slots.json). */
	sections: {
		before: 'Check this is the right office',
		prepare: 'Prepare your document',
		form: 'Fill in the request form',
		fees: 'Pay the fees',
		send: 'Send or bring your documents',
		after: 'Get it back and check it',
		contact: 'Contact the office'
	},

	/** Row labels by slot id (slots.json). */
	slots: {
		'jurisdiction.scope': 'Documents this office handles',
		'jurisdiction.federal_route': 'Federal documents',
		'jurisdiction.refusals': 'What the office will not apostille',
		'authentication.product': 'Type of certificate',
		'authentication.domestic_validity': 'Use in the issuing country',
		'notarization.wa_notary': 'Notary requirements',
		'copies.vital_records': 'Birth, death and marriage records',
		'forms.form': 'Request form',
		'forms.revision': 'Form version',
		'mailing.enclose': 'What to include',
		'fees.per_document': 'Fee per document',
		'fees.expedite': 'Expedited service fee',
		'fees.same_day': 'Same-day service fee',
		'fees.refund': 'Refunds',
		'payment.payee': 'Payable to',
		'payment.methods_counter': 'Paying in person',
		'mailing.usps': 'Mailing address',
		'mailing.courier': 'Courier address',
		'mailing.courier_rule': 'Sending by courier',
		'address.office': 'Office address',
		'hours.weekday': 'Opening hours',
		'walkin.cutoff': 'Latest arrival time',
		'walkin.same_day_only': 'Same-day service only',
		'walkin.no_mail': 'No mail-in requests',
		'walkin.public_record': 'Public records',
		'holiday_hours.closure': 'Office closures',
		'processing_time.standard': 'Standard processing time',
		'processing_time.expedited': 'Expedited processing time',
		'processing_time.same_day': 'Same-day processing',
		'mailing.return': 'How it comes back',
		'website.verify': 'Check an apostille online',
		'contacts.phone': 'Phone',
		'contacts.phone_option': 'Phone menu option',
		'contacts.phone_hours': 'Phone hours',
		'contacts.email': 'Email',
		'contacts.chat': 'Online chat',
		'contacts.tty': 'Text telephone (TTY)',
		'contacts.no_texts': 'Text messages',
		'service_notice.notice': 'Service notice',
		'service_notice.restored': 'Service restored'
	},

	/** Labels of known analyser keys no slot claims (guide.js fieldLabel). */
	fields: {
		per_document: 'Fee per document',
		apostille_main_branch_street: 'Office address',
		standard: 'Standard processing',
		weekday: 'Weekday hours',
		po_box: 'PO Box',
		methods: 'Accepted payment'
	},

	/**
	 * Record group headings by topic (the frozen vocabulary,
	 * crates/contracts/src/topics.rs); a topic not listed reads as its
	 * key, humanised (handbook.js topicLabel), as every topic does here.
	 */
	topics: /** @type {Record<string, string>} */ ({}),

	/** "At a glance": row labels by row id, sub-labels by slot id. */
	glance: {
		heading: 'At a glance',
		rows: { cost: 'Cost', time: 'Time', pay: 'Pay', send: 'Send', form: 'Form' },
		parts: {
			'processing_time.standard': 'Standard',
			'processing_time.expedited': 'Expedited',
			'processing_time.same_day': 'Same day',
			'payment.payee': 'Payable to',
			'payment.methods_counter': 'In person',
			'mailing.usps': 'By mail',
			'mailing.courier': 'By courier',
			'address.office': 'In person'
		},
		or: 'or',
		worded: "(worded differently on the office's pages)",
		downloadPdf: 'Download (PDF)',
		getOnPage: "Get it on the office's page",
		/** @param {number} step */
		fasterStep: (step) => `Faster options in step ${step}`,
		fasterFees: 'Faster options under Pay the fees'
	},

	guide: {
		citeFailed: "Sources didn't load, so the numbers can't open them yet.",
		retry: 'Retry',
		updates: 'Updates',
		afterNote: 'Allow extra time for mail both ways.',
		issuerSos: 'the Secretary of State',
		issuerOffice: 'the issuing office',
		/** Short names in "On this page", by section id. */
		nav: {
			before: 'Right office',
			prepare: 'Prepare',
			form: 'Request form',
			fees: 'Fees',
			send: 'Send',
			after: 'Get it back',
			contact: 'Contact',
			updates: 'Updates'
		},
		/** Under the header on a translated page ('' in English): values keep their language. */
		originalNote: ''
	},

	common: {
		newTab: '(opens in a new tab)',
		sourceNotOnFile: 'Source not on file',
		alsoOnPages: "Also on the office's pages",
		notConfirmed: 'Not confirmed yet',
		/** Copy-button names ("Copy the payee name"). */
		copyNames: {
			payee: 'the payee name',
			mailing: 'the mailing address',
			courier: 'the courier address',
			recommended: 'the recommended mailing address',
			office: 'the office address',
			/** @param {string} city */
			cityOffice: (city) => `the ${city} office address`
		}
	},

	header: {
		breadcrumb: 'Breadcrumb',
		home: 'Home',
		/** @param {string} country */
		flag: (country) => `${country} flag`,
		unofficial: 'Unofficial guide, not a government website',
		officialSite: 'Official site:'
	},

	status: {
		serviceNotice: 'Service notice',
		/** @param {string} day */
		capturedFrom: (day) => `Captured from the office's page ${day}`,
		/** @param {string} day */
		noOpen: (day) => `No open service notice in the office's notice log (last read ${day}).`,
		resolved: 'Resolved:'
	},

	section: {
		/** @param {number} n */
		step: (n) => `Step ${n}:`,
		call: 'Call this number',
		email: 'Write an email',
		openPage: "Open the office's page",
		beforeApply: 'Before you apply',
		chain: 'Legalization chain',
		chainNote: "Each hop below comes from the office's own pages, in order.",
		notConfirmedBody: "The office's pages don't say. Ask the office before you send documents or money:"
	},

	row: {
		conflict: "The office's pages give different values",
		worded: "Worded differently on the office's pages"
	},

	copyButton: {
		label: 'Copy',
		/** @param {string} text */
		copied: (text) => `Copied: ${text}`,
		/** @param {string} text */
		selected: (text) => `Selected: ${text}. Copy it with your keyboard shortcut.`,
		/** @param {string} text */
		selectYourself: (text) => `Select and copy it yourself: ${text}`
	},

	contents: { label: 'On this page' },

	routes: {
		label: 'Ways to send your documents',
		hints: {
			mail: 'Post the documents to the office.',
			courier: 'Send them overnight with a courier service.',
			visit: 'Bring them to the counter yourself.'
		}
	},

	empty: {
		heading: 'Not checked yet',
		body: "We publish a detail only after a person matches it to the office's own pages. Nobody has done that for this office yet, so this page shows no fees, addresses or processing times.",
		listed: 'Listed official site:',
		notChecked: '— not yet checked by us',
		browse: "Browse offices we've checked"
	},

	about: {
		heading: 'About this guide',
		unofficial: 'Unofficial guide, not a government website.',
		/** Around the office's name: "You can apply directly to {office}". */
		applyDirect: ['You can apply directly to ', ''],
		/** Around the fee: " for {amount} per document". */
		perDocument: [' for ', ' per document'],
		rule: "We publish a detail only after a staff reviewer matches it to the office's own words. General information, not legal advice — confirm with the office before you send documents or money.",
		report: "Report a change on the office's pages",
		reportContinue: 'Continue your report of a change (draft saved)'
	},

	follow: {
		/** @param {string} name */
		title: (name) => `Get updates for ${name}`,
		description: 'One email when staff accept a changed fact for this office. Nothing else, ever.',
		checkInbox: 'Check your inbox to confirm.',
		/** @param {string} email */
		sent: (email) =>
			`We sent a confirm link to ${email} — it dies in 24 hours, and nothing sends until you click it.`,
		emailLabel: 'Email address',
		sending: 'Sending…',
		follow: 'Follow',
		invalid: 'Enter a valid email address.',
		tooMany: 'Too many tries — wait a few minutes and try again.',
		off: 'Follow updates are off right now — try again later.',
		failed: 'Something went wrong — try again later.',
		privacy:
			'Privacy: we store only your address (sealed — staff cannot read it) and which office you follow. Confirm before anything sends; at most one digest per office per hour; every email carries a one-click unsubscribe link and our postal address. Unsubscribing always works, instantly.'
	},

	fees: {
		calculator: 'Work out your total',
		documents: 'Number of documents',
		fewer: 'One document fewer',
		more: 'One document more',
		speed: 'Speed',
		speeds: { standard: 'Standard', expedite: 'Expedited', same_day: 'Same day, in person' },
		callOffice: 'Call the office:',
		yourTotal: 'Your total',
		totalCounter: 'Total at the counter',
		totalEnclose: 'Total to enclose',
		sameDayFee: 'same-day fee',
		expediteFee: 'expedite fee',
		/** @param {number} from @param {number} to */
		range: (from, to) => (from === to ? counted(from, 'document', 'documents') : `${from}–${to} documents`),
		askOffice: 'Ask the office before you pay.',
		ask: {
			conflict: "The office's pages list different amounts for this fee.",
			contested: 'The office told us something different from its pages.',
			'over-max': "This calculator doesn't work out totals for that many documents.",
			'no-tier': "The office's pages don't give this fee for that many documents.",
			count: 'Enter how many documents you are sending.',
			other: "The office's own pages don't give enough to work out this total."
		}
	},

	form: {
		download: 'Download the official form (PDF)',
		getOnPage: "Get the form on the office's page",
		fill: 'Fill it in online — free',
		fillNote:
			"We put your answers into the office's own form; you print, sign and mail it. Needs a free account."
	},

	conflict: {
		worded: "Worded differently on the office's pages",
		heading: "The office's own pages list different mailing addresses",
		printedOn: 'Printed on:',
		recommendation: 'Our recommendation',
		/** @param {string} day '' when the decision has no day */
		recommendationStaff: (day) => `Our recommendation (staff${day ? `, ${day}` : ''}):`,
		/** Before the recommended value. */
		use: 'use',
		/** @param {'email'|'visit'|'call'} how @param {string} channel */
		confirmed: (how, channel) =>
			` — confirmed ${how === 'email' ? 'by email' : how === 'visit' ? 'at the office' : 'by phone'} with the office${channel ? ` (${channel})` : ''}`,
		/** @param {'email'|'visit'|'call'} how @param {string} day */
		toldUs: (how, day) =>
			`The office told us ${how === 'email' ? 'by email' : how === 'visit' ? 'at the office' : 'by phone'}${day ? ` on ${day}` : ''}:`,
		whatYouCanDo: 'What you can do:',
		/** Around the phone number (and its menu option). */
		call: ['Call ', ' before you mail.'],
		/** @param {boolean} first */
		sendCourier: (first) => `${first ? 'Send' : 'Or send'} by courier to the `,
		overnight: 'overnight address',
		askWhich: 'Ask the office which address is current before you mail. Its contact details are under',
		contactLink: 'Contact the office'
	},

	send: {
		mail: 'By mail',
		mailUs: 'By mail (USPS)',
		courier: 'By courier',
		courierUs: 'By courier (FedEx, UPS)',
		visit: 'In person',
		visitSameDay: 'In person, same day',
		carriers: "Carriers the office's pages name",
		/** @param {string} carrier */
		notStated: (carrier) => `${carrier}: not stated`,
		otherCarriers: 'Other carriers',
		mailingMissing: 'Mailing address',
		courierMissing: 'Courier address',
		/** Around the office's phone number. */
		courierPhone: ["Courier labels ask for a recipient phone; the office's published number is ", '.']
	},

	label: {
		/** Around the city and the price of the paid FedEx label. */
		fedex: ['Print a prepaid FedEx label to the ', 'courier address\u00a0— from '],
		unavailable:
			"Prepaid courier labels aren't available for this office right now. You can send by FedEx or UPS yourself — copy the courier address above.",
		/** @param {string} issuer */
		paidBy: (issuer) => `Paid service by the Apostille Handbook, not ${issuer}.`,
		free: 'Print a free address label for the mailing address',
		/** @param {string} issuer */
		freeBy: (issuer) => `Free service by the Apostille Handbook, not ${issuer}.`,
		courierCity: 'Courier city'
	},

	offices: {
		/** @param {string} title */
		google: (title) => `Open in Google Maps: ${title} (opens in a new tab)`,
		/** @param {string} title */
		apple: (title) => `Open in Apple Maps: ${title} (opens in a new tab)`,
		/** @param {string} time */
		cutoff: (time) => `· Today's cut-off: ${time}`,
		/** @param {string} zone */
		timesIn: (zone) => `Times in ${zone}`,
		office: 'Office',
		officeMissing: 'Office address',
		every: 'For every office',
		upcoming: 'Upcoming closures',
		more: 'More closures',
		closedToday: 'Closed today',
		/** @param {string} time */
		openNow: (time) => `Open now · closes ${time}`,
		/** @param {string} time */
		opensToday: (time) => `Closed now · opens today at ${time}`,
		/** @param {string} when @param {string} time */
		opensOn: (when, time) => `Closed now · opens ${when} at ${time}`,
		tomorrow: 'tomorrow',
		closedNow: 'Closed now',
		noon: 'noon'
	},

	/** The office record card (plan/OfficeCard). */
	officeCard: {
		appointment: 'Appointment required',
		walkIn: 'Walk-in',
		mailOnly: 'Mail only',
		address: 'Address',
		directions: 'Directions',
		/** @param {string} name */
		directionsTo: (name) => ` to ${name} (opens in a new tab)`,
		hours: 'Hours',
		/** @param {string} zone */
		timesIn: (zone) => `Times in ${zone}`,
		/** @param {string} time */
		openNow: (time) => `Open now · closes ${time}`,
		/** @param {string} when @param {string} time */
		opens: (when, time) => `Closed · opens ${when} ${time}`,
		today: 'today',
		tomorrow: 'tomorrow',
		closed: 'Closed'
	},

	updates: {
		/** @param {string} day */
		recaptured: (day) => `Page re-captured ${day}; change too large to list`,
		/** @param {string} host */
		changesOn: (host) => `Changes on ${host}`,
		added: 'Added: ',
		removed: 'Removed: ',
		/** @param {number} n */
		more: (n) => `${n === 1 ? 'One more changed line' : `${n} more changed lines`} on the page.`,
		older: 'Older updates',
		none: 'No changes to list yet.'
	},

	made: {
		eyebrow: 'Drafted by a model or quoted by staff, approved by a person',
		heading: 'How this page was made',
		/**
		 * @param {number} n records
		 * @param {string[]} models
		 * @param {string} pages "8 official pages", or ''
		 */
		summary: (n, models, pages) =>
			`${counted(n, 'record', 'records')}${models.length > 0 ? `, drafted by ${models.join(' and ')}` : ''}${pages ? ` from ${pages}` : ''}, each approved by a staff reviewer before it reached this page.`,
		/** In `summary`'s list, for records staff quoted from a page ("drafted by Muse Spark 1.3 and staff"). */
		staff: 'staff',
		/** @param {number} n */
		records: (n) => `The records behind this page (${n})`,
		/** @param {string} day */
		whatChanged: (day) => `What changed on ${day}`,
		/** @param {string} day */
		since: (day) => `· since ${day}`
	},

	/** "How this page was made": the pipeline panel (plan/ProvenancePanel). */
	provenance: {
		/** @param {number} n */
		pages: (n) => counted(n, 'official page', 'official pages'),
		/** @param {string} day */
		captured: (day) => `Captured ${day}`,
		capturedAuthority: 'Captured from the authority',
		roles: {
			extract: 'Extracted facts from official pages',
			cross_check: 'Cross-checked each fact against its quote',
			compose: 'Composed this plan from accepted facts'
		},
		/** @param {string} a @param {string} b @param {string} c */
		crossCheck: (a, b, c) => `${a} supported · ${b} contradicted · ${c} not found`,
		/** @param {string} n */
		checksPassed: (n) => `${n} checks passed`,
		/** @param {string} n */
		checksRun: (n) => `${n} checks run`,
		validator: 'Deterministic validator: every block cites accepted facts of this authority',
		/** @param {string} n @param {number} k */
		dropped: (n, k) => `${n} block${k === 1 ? '' : 's'} dropped`,
		approved: 'Approved by staff reviewer',
		/** @param {string} day @param {number} version */
		accepted: (day, version) => `Accepted ${day}${version ? ` · plan v${version}` : ''}`,
		acceptedBefore: 'Accepted before publishing',
		/** @param {number} n */
		step: (n) => `Step ${n}`,
		runTime: 'Run time',
		/** @param {string} kind */
		tokens: (kind) => `${kind} tokens`,
		tokenKinds: { Input: 'Input', Output: 'Output', Cached: 'Cached', Reasoning: 'Reasoning' }
	},

	endnotes: {
		heading: 'Sources',
		intro:
			'Each number on the page points to a source below. Quotes are copied word for word from the page named; anything else is labelled as a staff note or a calculation.',
		/** @param {string} label */
		backTo: (label) => `Back to ${label}`,
		backToCitation: 'Back to the citation',
		basedOn: 'Based on'
	},

	card: {
		close: 'Close'
	},

	toggle: {
		citations: 'Citations',
		print: 'Print',
		hintOff: 'Turn on citations to print sources.',
		hintOn: 'Sources print at the end.'
	},

	print: {
		brand: 'Apostille Handbook',
		/** @param {string} at */
		dataAsOf: (at) => `Data as of ${at}`,
		/** @param {string} day */
		printed: (day) => `Printed ${day}`,
		brackets: 'Numbers in brackets point to the sources listed at the end.',
		checkLive: 'Check the live page before you send documents or money.'
	},

	/** Source descriptions, dates and counts (cite.js). */
	cite: {
		marker: 'Citation ',
		/** @param {string} range */
		captured: (range) => `Captured ${range}`,
		/** @param {string} range @param {boolean} first */
		approved: (range, first) => `${first ? 'Approved' : 'approved'} ${range}`,
		/** @param {string} range */
		sourcesCaptured: (range) => `Sources captured ${range}`,
		/** @param {string} day */
		approvedByStaff: (day) => `approved by staff ${day}`,
		/** @param {number} n */
		officialPages: (n) => counted(n, 'official page', 'official pages'),
		/** @param {number} n */
		otherPages: (n) => counted(n, 'other web page', 'other web pages'),
		/** @param {number} n */
		webPages: (n) => counted(n, 'web page', 'web pages'),
		/** @param {number} n */
		calls: (n) => counted(n, 'phone call', 'phone calls'),
		/** @param {number} n */
		emails: (n) => counted(n, 'email', 'emails'),
		contradicts: 'contradicts the value shown',
		webPage: 'Official web page',
		openPage: 'Open the page',
		officialPdf: 'Official PDF',
		/** "{title} — official PDF, {host}." @param {string} title @param {string} host */
		pdfHead: (title, host) =>
			title ? `${title} — official PDF${host ? `, ${host}` : ''}.` : `Official PDF${host ? ` — ${host}` : ''}.`,
		openPdf: 'Open PDF',
		/** @param {boolean} pdf @returns {string} */
		officialUpload: (pdf) => (pdf ? 'Official PDF' : 'Official document'),
		/** @param {string} host */
		uploadedBy: (host) => (host ? `uploaded by staff from ${host}` : 'uploaded by staff'),
		/** @param {string} title @param {boolean} pdf @param {string} from */
		uploadHead: (title, pdf, from) =>
			title ? `${title} — official ${pdf ? 'PDF' : 'document'} ${from}.` : `Official ${pdf ? 'PDF' : 'document'} ${from}.`,
		openDocument: 'Open the document',
		call: 'Phone call',
		staffNotes: 'staff notes',
		notRecorded: 'staff notes, not recorded',
		checkNumber: 'Check it yourself on the same number.',
		email: 'Email',
		/** @param {string} from */
		emailFrom: (from) => `Email from ${from}`,
		emailNote: "staff note; the email isn't published",
		visit: 'Visit to the office',
		/** @param {string} pages */
		notFoundOn: (pages) => `Not found on these office pages: ${pages}`,
		notFound: "Not found on the office's pages",
		checkedByStaff: 'checked by staff',
		calculated: 'Calculated on this page',
		staffNote: 'Staff note',
		hidden: 'Citations hidden',
		/** @param {number} n */
		shown: (n) => `Citations shown: ${counted(n, 'source', 'sources')} numbered`
	},

	/** A staff note that contradicts a value (doc/postal.js contestLead). */
	contest: {
		/** @param {'email'|'visit'|'call'} how @param {string} day */
		lead: (how, day) =>
			`Staff note: the office told us otherwise ${how === 'email' ? 'by email' : how === 'visit' ? 'at the office' : 'by phone'}${day ? ` on ${day}` : ''}`
	}
};
