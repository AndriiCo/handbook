// Item helpers shared by the money and send blocks (FeeBlock,
// SendBlock, OfficeCards, FormBlock, ConflictBox, LabelCard): slot
// lookups, the DOM trace attributes every value node carries, and the
// few honest rows the page writes itself ("Not confirmed yet", the
// remote-notarization row). Pure: no window/fetch.

/** @typedef {import('./contract.js').Item} Item */

/**
 * Slot ids these blocks read (slots.json; R1 registry).
 */
export const SLOT = Object.freeze({
	perDocument: 'fees.per_document',
	expedite: 'fees.expedite',
	sameDay: 'fees.same_day',
	refund: 'fees.refund',
	payee: 'payment.payee',
	methodsCounter: 'payment.methods_counter',
	usps: 'mailing.usps',
	courier: 'mailing.courier',
	courierRule: 'mailing.courier_rule',
	enclose: 'mailing.enclose',
	return: 'mailing.return',
	office: 'address.office',
	hours: 'hours.weekday',
	cutoff: 'walkin.cutoff',
	sameDayOnly: 'walkin.same_day_only',
	noMail: 'walkin.no_mail',
	publicRecord: 'walkin.public_record',
	closure: 'holiday_hours.closure',
	sameDayTime: 'processing_time.same_day',
	phone: 'contacts.phone',
	phoneOption: 'contacts.phone_option',
	form: 'forms.form',
	revision: 'forms.revision',
	notarization: 'notarization.wa_notary'
});

/**
 * Slots an office card holds (one card per office instance).
 * @type {readonly string[]}
 */
export const OFFICE_SLOTS = Object.freeze([
	SLOT.office,
	SLOT.hours,
	SLOT.cutoff,
	SLOT.sameDayOnly,
	SLOT.noMail,
	SLOT.publicRecord
]);

/** "Not confirmed yet — ask the office": the honest value of an unfilled row. */
export const NOT_CONFIRMED = 'Not confirmed yet — ask the office';

/**
 * The first item of a slot (any state), or null.
 * @param {readonly (Item|null|undefined)[]|null|undefined} items
 * @param {string} slot
 * @returns {Item|null}
 */
export function bySlot(items, slot) {
	return (items ?? []).find((i) => i?.slot === slot) ?? null;
}

/**
 * Every item of a slot, in order.
 * @param {readonly (Item|null|undefined)[]|null|undefined} items
 * @param {string} slot
 * @returns {Item[]}
 */
export function allOfSlot(items, slot) {
	return /** @type {Item[]} */ ((items ?? []).filter((i) => i?.slot === slot));
}

/**
 * True for a value a customer can act on: accepted and undisputed
 * (fresh or stale; an uncited value is shown but never acted on).
 * @param {Item|null|undefined} item
 */
export function usable(item) {
	return !!item && (item.state === 'ok' || item.state === 'stale') && item.factIds.length > 0;
}

/**
 * The trace attributes of a value node (contract DOM conventions):
 * `data-fact-ids` for every value resting on facts, `data-state` for
 * computed, missing and absent values, and `data-inputs` naming the
 * items a computed value is worked out from.
 * @param {Item} item
 * @returns {Record<string, string>}
 */
export function traceAttrs(item) {
	/** @type {Record<string, string>} */
	const attrs = {};
	const ids = (item?.factIds ?? []).filter((id) => typeof id === 'number');
	if (ids.length > 0) attrs['data-fact-ids'] = ids.join(' ');
	if (item?.state === 'computed') {
		attrs['data-state'] = 'computed';
		attrs['data-inputs'] = (item.inputs ?? []).map((i) => i.id).join(' ');
	} else if (item?.state === 'absent') {
		attrs['data-state'] = 'absent';
	} else if (item?.state === 'missing' || ids.length === 0) {
		attrs['data-state'] = 'missing';
	}
	return attrs;
}

/**
 * A value the page writes itself, as an Item: an honest gap
 * ("missing"), or a token read out of a cited value (a city from an
 * address keeps that value's facts and cites).
 * @param {Partial<Item> & { id: string, label: string, display: string }} over
 * @returns {Item}
 */
export function localItem(over) {
	return {
		slot: null,
		kind: 'text',
		factIds: [],
		cites: [],
		state: 'missing',
		stale: null,
		alternatives: null,
		inputs: null,
		copy: false,
		route: null,
		instance: null,
		...over
	};
}

/**
 * A "Not confirmed yet — ask the office" row for an unfilled slot.
 * @param {string} id
 * @param {string} label
 * @param {string|null} [slot]
 */
export function notConfirmed(id, label, slot = null) {
	return localItem({ id, label, display: NOT_CONFIRMED, slot });
}

/**
 * A token read out of a cited value (a city, a phone), keeping the
 * value's facts and cites so the token stays traceable.
 * @param {Item} from
 * @param {string} suffix id suffix, e.g. "city"
 * @param {string} display the token, verbatim from `from.display`
 * @returns {Item}
 */
export function partOf(from, suffix, display) {
	return {
		...from,
		id: `${from.id}-${suffix}`,
		display,
		alternatives: null,
		copy: false
	};
}

/**
 * True when `country` names the United States (the authority row
 * spells it several ways).
 * @param {unknown} country
 */
export function isUS(country) {
	return /^(?:usa?|u\.s\.a?\.?|united states(?: of america)?)$/i.test(String(country ?? '').trim());
}

/**
 * An office's name as the object of a sentence ("Issued by …", "apply
 * directly to …"): "the" in front, unless the name already starts
 * with it or with its country's name ("Viet Nam — receiving
 * authorities" takes none).
 * @param {unknown} name
 * @param {unknown} [country]
 */
export function issuerName(name, country = null) {
	const n = String(name ?? '').trim();
	if (!n || /^the\s/i.test(n)) return n;
	const c = String(country ?? '').trim();
	if (c && !isUS(c) && n.toLowerCase().startsWith(c.toLowerCase())) return n;
	return `the ${n}`;
}

/** Citation kinds that point at a page or a file a reader can open. */
const PAGE_KINDS = ['web', 'pdf', 'upload'];

/** @param {string} url */
function isPdfPath(url) {
	try {
		return /\.pdf$/i.test(new URL(url).pathname);
	} catch {
		return false;
	}
}

/**
 * Where to get the request form (D6): the https address of the form
 * value's web, PDF or upload citations, a PDF first; while the value is
 * disputed (worded differently, or in conflict), its alternatives'
 * citations. Null when no usable value cites an https page.
 * @param {Item|null|undefined} item the `forms.form` Item
 * @returns {{ url: string, pdf: boolean }|null}
 */
export function formLink(item) {
	if (!item) return null;
	const alternatives = item.alternatives ?? [];
	const disputed = (item.state === 'conflict' || item.state === 'worded') && alternatives.length > 0;
	const holders = disputed ? alternatives.filter((a) => usable(a)) : usable(item) ? [item] : [];
	/** @type {string|null} */
	let page = null;
	for (const holder of holders) {
		for (const c of holder.cites ?? []) {
			const s = c?.source;
			if (!s || (s.kind && !PAGE_KINDS.includes(s.kind))) continue;
			if (typeof s.url !== 'string' || !s.url.startsWith('https://')) continue;
			const url = s.url.split('#')[0];
			if (isPdfPath(url)) return { url, pdf: true };
			page ??= url;
		}
	}
	return page ? { url: page, pdf: false } : null;
}

// Letters only Vietnamese writes (the Latin Extended Additional block,
// ơ and ư), and other scripts that name one language.
const VI = /[Ạ-ỹƠơƯư]/;
const SCRIPTS = /** @type {[RegExp, string][]} */ ([
	[/[Ͱ-Ͽ]/g, 'el'],
	[/[֐-׿]/g, 'he'],
	[/[฀-๿]/g, 'th'],
	[/[가-힯ᄀ-ᇿ]/g, 'ko'],
	[/[぀-ヿ]/g, 'ja'],
	[/[Ⴀ-ჿ]/g, 'ka'],
	[/[԰-֏]/g, 'hy']
]);

/**
 * The language of a value written in another language than the page
 * (WCAG 3.1.2), from its own letters, or null for English and for text
 * whose language its letters don't tell (most Latin and Cyrillic text).
 * Mixed text counts when at least a third of it is in that language.
 * @param {unknown} text
 * @returns {string|null}
 */
export function valueLang(text) {
	const s = String(text ?? '');
	if (!/[^\u0000-ɏ]/.test(s) && !VI.test(s)) return null;
	const words = s.split(/\s+/).filter((w) => /\p{L}/u.test(w));
	if (VI.test(s)) {
		const marked = words.filter((w) => /[^\u0000-\u007F]/.test(w)).length;
		return words.length > 0 && marked * 3 >= words.length ? 'vi' : null;
	}
	const letters = (s.match(/\p{L}/gu) ?? []).length;
	if (letters === 0) return null;
	/** @param {RegExp} re */
	const share = (re) => (s.match(re) ?? []).length * 3 >= letters;
	if (/[一-鿿]/.test(s) && !/[぀-ヿ]/.test(s) && share(/[一-鿿]/g)) return 'zh';
	for (const [re, lang] of SCRIPTS) if (share(re)) return lang;
	if (share(/[؀-ۿ]/g)) return /[پچژگ]/.test(s) ? 'fa' : 'ar';
	if (share(/[Ѐ-ӿ]/g)) {
		if (/[єіїґЄІЇҐ]/.test(s)) return 'uk';
		if (/[ыэЫЭ]/.test(s)) return 'ru';
	}
	return null;
}

/**
 * A value's text in runs, with each ZIP+4 code ("98504-0234") a run of
 * its own that the page keeps on one line (display only: the runs join
 * back to the same text).
 * @param {unknown} text
 * @returns {{ text: string, keep: boolean }[]}
 */
export function keepTogether(text) {
	const s = String(text ?? '');
	/** @type {{ text: string, keep: boolean }[]} */
	const out = [];
	let at = 0;
	for (const m of s.matchAll(/\b\d{5}-\d{4}\b/g)) {
		const start = m.index ?? 0;
		if (start > at) out.push({ text: s.slice(at, start), keep: false });
		out.push({ text: m[0], keep: true });
		at = start + m[0].length;
	}
	if (at < s.length || out.length === 0) out.push({ text: s.slice(at), keep: false });
	return out;
}

/**
 * Words that show a notarization row already speaks to remote
 * notarization. "RON" counts only in capitals (a name such as "Ron"
 * is not one).
 */
const REMOTE_RE =
	/\bremote(?:ly)?\b|\bonline\s+notari[sz]|\belectronic(?:ally)?\s+notari[sz]|\bnotari[sz]ed\s+online\b|\bvideo\s+(?:call|conference)/i;
const RON_RE = /\bRON\b/;
const NOTARY_RE = /\bnotar/i;

export const REMOTE_NOTARIZATION_TEXT =
	"We haven't confirmed whether the office accepts remotely notarized documents. Ask before you pay for one.";

/**
 * True when a Prepare row speaks to remote notarization: a notarization
 * row (`notarization.wa_notary`) that mentions it, or an unmapped also
 * row (no slot) that mentions it in a sentence about notarization. Rows
 * of other slots (copies, translations) never count.
 * @param {Item} item
 */
function speaksToRemote(item) {
	if (item.slot !== SLOT.notarization && item.slot !== null && item.slot !== undefined) return false;
	const texts = [item.display ?? '', ...(item.alternatives ?? []).map((a) => a.display ?? '')];
	return texts.some(
		(t) => (REMOTE_RE.test(t) || RON_RE.test(t)) && (item.slot === SLOT.notarization || NOTARY_RE.test(t))
	);
}

/**
 * The Prepare step's honest row: when no notarization row there
 * mentions remote notarization, say we haven't confirmed it. Null when
 * one does. Pass the Prepare section's items and its `also` rows; only
 * notarization rows and unmapped rows about notarization are read.
 * @param {readonly (Item|null|undefined)[]|null|undefined} items
 * @returns {Item|null}
 */
export function remoteNotarizationRow(items) {
	const mentions = (items ?? []).some((i) => !!i && speaksToRemote(i));
	if (mentions) return null;
	return localItem({
		id: 'prepare-remote-notarization',
		label: 'Remote notarization',
		display: REMOTE_NOTARIZATION_TEXT
	});
}
