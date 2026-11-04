// Postal helpers for "Send or bring your documents" (plan §3 step 5,
// D3): PO boxes, carriers named in cited text, the city of an address,
// map searches, phone links, and the staff answer that settles a
// disputed mailing address. Pure: no window/fetch.

/** @typedef {import('./contract.js').Item} Item */

/**
 * One published evidence note (`/plan` `evidence[]`). Public notes
 * carry the day only (`date`, Pacific), never a clock time.
 * @typedef {{
 *   id: number,
 *   kind: 'call'|'email'|'visit'|'decision'|'absence',
 *   stance?: string,
 *   fact_id?: number|null,
 *   preferred_fact_id?: number|null,
 *   slot?: string,
 *   date?: string,
 *   channel?: string,
 *   summary?: string,
 *   supports?: number[],
 *   status?: string
 * }} EvidenceNote
 */

// "PO Box", "P.O. Box", "Post Office Box"; caller boxes and drawers
// are postal-only too.
const PO_BOX_RE = /\bP\.?\s*O\.?\s*(?:Box|Drawer)\b|\bPost\s+Office\s+Box\b|\bCaller\s+Box\b/i;

/**
 * True when an address names a post-office box.
 * @param {unknown} text
 */
export function isPoBox(text) {
	return PO_BOX_RE.test(String(text ?? ''));
}

// A US street line: a house number, up to four words, then a street
// type ("243 Israel Rd SE", "801 Capitol Way S", "1500 11th Street").
const STREET_RE =
	/\b\d{1,6}(?:-\d{1,4})?\s+(?:[A-Za-z0-9.'’-]+\s+){0,4}?(?:St|Street|Rd|Road|Ave|Avenue|Blvd|Boulevard|Way|Dr|Drive|Ln|Lane|Ct|Court|Pl|Place|Pkwy|Parkway|Hwy|Highway|Sq|Square|Ter|Terrace|Cir|Circle|Loop|Trl|Trail|Plaza|Plz|Pike|Row|Walk|Center|Ctr)\b\.?/i;

/**
 * True when a value names a street address and no post-office box
 * (D3: the USPS branch never shows one).
 * @param {unknown} text
 */
export function isStreetOnly(text) {
	const s = String(text ?? '');
	return STREET_RE.test(s) && !isPoBox(s);
}

/** Carriers a statement can name, in chip order. */
export const CARRIERS = Object.freeze([
	{ key: 'usps', label: 'USPS', re: /\bUSPS\b|\bU\.\s?S\.\s+Postal\s+Service\b|\bUnited\s+States\s+Postal\s+Service\b/i },
	{ key: 'fedex', label: 'FedEx', re: /\bFed\s?Ex\b/i },
	{ key: 'ups', label: 'UPS', re: /\bUPS\b/ },
	{ key: 'dhl', label: 'DHL', re: /\bDHL\b/ }
]);

/**
 * Carrier keys a text names, in chip order.
 * @param {unknown} text
 * @returns {string[]}
 */
export function carriersNamed(text) {
	const s = String(text ?? '');
	return CARRIERS.filter((c) => c.re.test(s)).map((c) => c.key);
}

/**
 * True when a statement limits mail to USPS ("USPS only", "only by
 * USPS"), the same phrases the order options read.
 * @param {unknown} text
 */
export function saysUspsOnly(text) {
	return /\busps\s+(?:mail\s+)?only\b|\bu\.\s?s\.\s+postal\s+service\s+only\b|\bonly\s+(?:by|via|through)\s+(?:the\s+)?(?:usps|u\.\s?s\.\s+postal\s+service)\b/i.test(
		String(text ?? '')
	);
}

/**
 * @typedef {{ key: string, label: string, stated: boolean, factIds: number[] }} Chip
 */

/**
 * Carrier chips for the two posting branches, from cited statements
 * only. Mail: "✓ USPS" when a statement limits mail to USPS. Courier:
 * a ✓ for each carrier the courier statements name. Every other
 * carrier is "not stated" (one chip), never ✗. A branch with nothing
 * stated gets no chips.
 * @param {{ mail: readonly (Item|null)[], courier: readonly (Item|null)[] }} branches
 * @returns {{ mail: Chip[], courier: Chip[] }}
 */
export function carrierChips(branches) {
	/** @param {readonly (Item|null)[]} items */
	const cited = (items) =>
		items.filter(
			/** @returns {i is Item} */ (i) => !!i && (i.state === 'ok' || i.state === 'stale') && i.factIds.length > 0
		);
	/** @type {Chip[]} */
	const mail = [];
	const uspsOnly = cited(branches.mail).filter((i) => saysUspsOnly(i.display));
	if (uspsOnly.length > 0) {
		mail.push({ key: 'usps', label: 'USPS', stated: true, factIds: uspsOnly.flatMap((i) => i.factIds) });
	}
	/** @type {Chip[]} */
	const courier = [];
	for (const c of CARRIERS) {
		if (c.key === 'usps') continue;
		const naming = cited(branches.courier).filter((i) => c.re.test(i.display));
		if (naming.length > 0) {
			courier.push({ key: c.key, label: c.label, stated: true, factIds: naming.flatMap((i) => i.factIds) });
		}
	}
	const other = { key: 'other', label: 'Other carriers', stated: false, factIds: [] };
	return {
		mail: mail.length > 0 ? [...mail, other] : [],
		courier: courier.length > 0 ? [...courier, other] : []
	};
}

const STATE_ZIP = /^(?:[A-Z]{2}|[A-Z][a-z]+(?:\s[A-Z][a-z]+)?)\.?\s+\d{5}(?:-\d{4})?$/;
const CITY_STATE_ZIP = /^(.+?)\s+(?:[A-Z]{2})\.?\s+\d{5}(?:-\d{4})?$/;

/**
 * @param {string} s
 */
function cityOk(s) {
	const city = s.replace(/[.\s]+$/, '').trim();
	return /^[A-Za-zÀ-ÿ][A-Za-zÀ-ÿ .'-]{1,39}$/.test(city) ? city : null;
}

/**
 * The city of a US-style address ("243 Israel Rd SE, Tumwater, WA
 * 98501" → "Tumwater"), verbatim from the text, or null.
 * @param {unknown} text
 * @returns {string|null}
 */
export function postalCity(text) {
	const parts = String(text ?? '')
		.split(/[\n,]/)
		.map((p) => p.trim().replace(/\.$/, ''))
		.filter(Boolean);
	for (let i = parts.length - 1; i >= 0; i--) {
		const part = parts[i];
		if (STATE_ZIP.test(part) && i > 0) {
			const city = cityOk(parts[i - 1]);
			if (city) return city;
		}
		const inline = CITY_STATE_ZIP.exec(part);
		if (inline) {
			const city = cityOk(inline[1]);
			if (city) return city;
		}
		if (/^\d{5}(?:-\d{4})?$/.test(part) && i > 1 && /^[A-Z]{2}$/.test(parts[i - 1])) {
			const city = cityOk(parts[i - 2]);
			if (city) return city;
		}
	}
	return null;
}

// Country words a map search appends (the prototype's
// HANDBOOK_MAP_COUNTRY, keyed by the spellings authority rows use).
const MAP_COUNTRY = /** @type {Record<string, string>} */ ({
	us: 'USA',
	usa: 'USA',
	'united states': 'USA',
	'united states of america': 'USA',
	ca: 'Canada',
	canada: 'Canada',
	mx: 'México',
	mexico: 'México',
	méxico: 'México'
});

/**
 * The map search for an office, built from its own address text: lines
 * joined, parenthetical asides dropped (they defeat geocoding), the city
 * and the country appended when missing. Derived, never shown (port of
 * the prototype's `handbookMapQuery`).
 * @param {unknown} address
 * @param {unknown} [city]
 * @param {unknown} [country]
 */
export function mapQuery(address, city, country) {
	/** @param {unknown} v */
	const strip = (v) => String(v ?? '').replace(/\([^)]*\)/g, ' ');
	let q = strip(address).replace(/\s*\n\s*/g, ', ');
	q = q.replace(/\s+/g, ' ').replace(/\s*,\s*/g, ', ').replace(/(?:,\s*){2,}/g, ', ');
	q = q.replace(/^[\s,]+|[\s,.]+$/g, '');
	const town = strip(city).replace(/\s+/g, ' ').trim();
	if (town && !q.toLowerCase().includes(town.toLowerCase())) q += `, ${town}`;
	const raw = String(country ?? '').trim();
	const cc = MAP_COUNTRY[raw.toLowerCase()] ?? raw;
	// Segment equality, not substring: "México" sits inside "Ciudad de México".
	const segs = q.split(',').map((x) => x.trim().toLowerCase());
	if (q && cc && !segs.includes(cc.toLowerCase())) q += `, ${cc}`;
	return q;
}

/**
 * Google and Apple map searches for a query (plain-text search URLs;
 * links, never fetched).
 * @param {string} query
 * @returns {{ google: string, apple: string }|null}
 */
export function mapLinks(query) {
	if (!query) return null;
	const q = encodeURIComponent(query);
	return {
		google: `https://www.google.com/maps/search/?api=1&query=${q}`,
		apple: `https://maps.apple.com/?q=${q}`
	};
}

/**
 * The first phone number in a value, as printed, or null.
 * @param {unknown} text
 */
export function phoneIn(text) {
	const m = /\+?\(?\d[\d\s().-]{5,}\d/.exec(String(text ?? ''));
	return m ? m[0].trim() : null;
}

/**
 * A `tel:` link for the first phone number in a value, or null.
 * @param {unknown} text
 */
export function telHref(text) {
	const phone = phoneIn(text);
	if (!phone) return null;
	const digits = phone.replace(/[^\d+]/g, '');
	return digits.replace(/\D/g, '').length >= 7 ? `tel:${digits}` : null;
}

/** How the office answered, in the order a recommendation quotes them. */
const ANSWER_KINDS = /** @type {const} */ (['email', 'call', 'visit']);

/**
 * @typedef {{
 *   decision: EvidenceNote,
 *   note: EvidenceNote|null,
 *   channel: 'email'|'call'|'visit'|null,
 *   preferred: Item,
 *   unlocks: boolean
 * }} OfficeAnswer
 */

/**
 * The staff recommendation that settles a disputed value (plan §3
 * ConflictBox, §5.4): a published staff decision preferring one of the
 * item's alternatives, and the office's answer it rests on, an email,
 * a call or a visit note (in that order). A decision backed by the
 * office's answer unlocks Copy on the value it prefers (the other
 * values stay listed); one without it is shown, but unlocks nothing.
 * Decisions backed by an answer win over those without. Null when no
 * published decision names one of the values.
 * @param {Item} item the disputed item (with `alternatives`)
 * @param {readonly EvidenceNote[]|null|undefined} evidence
 * @returns {OfficeAnswer|null}
 */
export function officeAnswer(item, evidence) {
	/** @type {EvidenceNote[]} */
	const rows = Array.isArray(evidence) ? [...evidence] : [];
	const notes = rows.filter(
		(n) => !!n && typeof n === 'object' && typeof n.id === 'number' && (!n.status || n.status === 'published')
	);
	const alternatives = item?.alternatives ?? [];
	/** @type {Map<number, EvidenceNote>} */
	const byId = new Map(notes.map((n) => [n.id, n]));
	/** @type {OfficeAnswer|null} */
	let bare = null;
	/** @type {OfficeAnswer|null} */
	let best = null;
	for (const decision of notes) {
		const preferredId = decision.preferred_fact_id;
		if (decision.kind !== 'decision' || typeof preferredId !== 'number') continue;
		const preferred = alternatives.find((a) => a.factIds.includes(preferredId)) ?? null;
		if (!preferred) continue;
		/** @type {number[]} */
		const ids = Array.isArray(decision.supports) ? decision.supports : [];
		/** @type {EvidenceNote[]} */
		const supports = [];
		for (const id of ids) {
			const note = byId.get(id);
			if (note && typeof note.summary === 'string' && note.summary.trim()) supports.push(note);
		}
		const kind = ANSWER_KINDS.find((k) => supports.some((n) => n.kind === k));
		if (!kind) {
			bare ??= { decision, note: null, channel: null, preferred, unlocks: false };
			continue;
		}
		const note = /** @type {EvidenceNote} */ (supports.find((n) => n.kind === kind));
		if (!best || ANSWER_KINDS.indexOf(kind) < ANSWER_KINDS.indexOf(/** @type {'email'|'call'|'visit'} */ (best.channel))) {
			best = { decision, note, channel: kind, preferred, unlocks: true };
		}
	}
	return best ?? bare;
}

/**
 * The visible line of a staff note that contradicts a value
 * (`Item.contest`): "Staff note: the office told us otherwise by phone
 * on November 4, 2026".
 * @param {{ kind: string, date?: string|null }} contest
 */
export function contestLead(contest) {
	const how = contest.kind === 'email' ? 'by email' : contest.kind === 'visit' ? 'at the office' : 'by phone';
	const day = noteDay(contest.date);
	return `Staff note: the office told us otherwise ${how}${day ? ` on ${day}` : ''}`;
}

/**
 * A note's day (`YYYY-MM-DD`) as "November 4, 2026"; '' when unreadable.
 * @param {unknown} day
 */
export function noteDay(day) {
	const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(day ?? ''));
	if (!m) return '';
	const at = Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]), 12);
	return new Intl.DateTimeFormat('en-US', { month: 'long', day: 'numeric', year: 'numeric', timeZone: 'UTC' }).format(at);
}
