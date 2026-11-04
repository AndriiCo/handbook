// Slot registry (plan §5.3): the rows the apostille guide can show.
//
// A slot is one thing a customer needs ("Fee per document", "Mailing
// address"): a frozen topic (crates/contracts/src/topics.rs) plus the
// statement keys that fill it. Statements keep the analyser's grammar
// `{field}[__{instance}]: {value}`. A slot's first key is canonical
// (the /review key chips offer it); the rest are aliases for keys
// already in production or in the judge replay corpus. Sections place
// slots in step order; two pseudo-sections never render as headings:
// `status` (service notices, a banner under the h1) and `hidden`
// (`phone_script`, never public). `slots.json` is the single source of
// truth; this module only indexes it. Pure: no window/fetch, so node
// tests drive it directly.

import registry from './slots.json' with { type: 'json' };
import { NOTICE_TOPIC, parseStatement } from './guide.js';

/**
 * @typedef {'money'|'money_tier'|'postal'|'payee'|'phone'|'email'|'hours'|'time_rule'|'duration'|'closure'|'text'|'link_from_citation'|'notice'|'notice_update'} SlotKind
 */

/**
 * @typedef {object} Slot
 * @property {string} id `{topic}.{name}`: the canonical key, except `mailing.usps` (key `apostille_mailing_address`)
 * @property {string} topic the frozen topic it reads
 * @property {readonly string[]} keys canonical key first, then aliases
 * @property {readonly { topic: string, keys: readonly string[] }[]} also keys of other topics that fill it (`po_box: po_box` is the USPS address)
 * @property {string} label plain-language row label: no digits, `$`, addresses or dates
 * @property {SlotKind} kind how the value reads
 * @property {boolean} many several values list side by side (one per office, closure or notice) instead of merging
 * @property {Readonly<Record<string, string>>} instances alias key → the instance it implies
 * @property {'cost'|'time'|'pay'|'send'|'form'|null} glance its "At a glance" row, if any
 * @property {'mail'|'courier'|'counter'|null} route the one way of sending it belongs to, if any
 * @property {boolean} copy the value gets a Copy button (payee, addresses)
 * @property {string} section the section it renders in: its topic's, unless the slot names another
 */

/**
 * @typedef {object} Section
 * @property {string} id
 * @property {number|null} step the step number, or null (contact, pseudo-sections)
 * @property {string|null} heading null for pseudo-sections
 * @property {readonly string[]} topics
 * @property {boolean} pseudo never a heading: `status` (notices) and `hidden` (call scripts)
 */

/**
 * @typedef {object} Profile
 * @property {string} id
 * @property {readonly string[]} country country names it matches (any case)
 * @property {readonly string[]} kind authority kinds it matches
 * @property {RegExp|null} slug matches rows that carry no kind
 * @property {readonly string[]} critical slot ids the page must fill or mark "Not confirmed yet"
 */

/** @type {any} */
const data = registry;

/**
 * Page sections in reading order, then the `status` and `hidden`
 * pseudo-sections.
 * @type {readonly Section[]}
 */
export const SECTIONS = Object.freeze(
	data.sections.map((/** @type {any} */ s) =>
		Object.freeze({
			id: s.id,
			step: s.step ?? null,
			heading: s.heading ?? null,
			topics: Object.freeze([...s.topics]),
			pseudo: s.pseudo === true
		})
	)
);

/**
 * Topic → id of the section it renders in, for all 26 frozen topics.
 * @type {Readonly<Record<string, string>>}
 */
export const TOPIC_SECTION = Object.freeze(
	Object.fromEntries(SECTIONS.flatMap((s) => s.topics.map((t) => [t, s.id])))
);

/** Every frozen topic, in page order. */
export const TOPICS = Object.freeze(SECTIONS.flatMap((s) => s.topics));

/**
 * Every slot in page order (sections, then rows within them).
 * @type {readonly Slot[]}
 */
export const SLOTS = Object.freeze(
	data.slots.map((/** @type {any} */ s) =>
		Object.freeze({
			id: s.id,
			topic: s.topic,
			keys: Object.freeze([...s.keys]),
			also: Object.freeze(
				(s.also ?? []).map((/** @type {any} */ a) =>
					Object.freeze({ topic: a.topic, keys: Object.freeze([...a.keys]) })
				)
			),
			label: s.label,
			kind: s.kind,
			many: s.many === true,
			instances: Object.freeze({ ...s.instances }),
			glance: s.glance ?? null,
			route: s.route ?? null,
			copy: s.copy === true,
			section: s.section ?? TOPIC_SECTION[s.topic]
		})
	)
);

/**
 * Page profiles: which slots a kind of office must fill.
 * @type {readonly Profile[]}
 */
export const PROFILES = Object.freeze(
	Object.entries(data.profiles).map(([id, /** @type {any} */ p]) =>
		Object.freeze({
			id,
			country: Object.freeze([...p.country]),
			kind: Object.freeze([...p.kind]),
			slug: typeof p.slug === 'string' ? new RegExp(p.slug) : null,
			critical: Object.freeze([...p.critical])
		})
	)
);

/** `{topic}\n{key}` → its slot, and the instance an alias implies. */
/** @type {Map<string, { slot: Slot, instance: string|null }>} */
const INDEX = new Map();
for (const slot of SLOTS) {
	for (const read of [{ topic: slot.topic, keys: slot.keys }, ...slot.also]) {
		for (const key of read.keys) {
			const at = `${read.topic}\n${key}`;
			// First claim wins; slots.test refuses any duplicate.
			if (!INDEX.has(at)) INDEX.set(at, { slot, instance: slot.instances[key] ?? null });
		}
	}
}

const BY_ID = new Map(SLOTS.map((s) => [s.id, s]));

/** An instance after `__`: words or digits joined by `_` (`cheney`, `2026_11_02`). */
const INSTANCE = /^[a-z0-9]+(?:_[a-z0-9]+)*$/;

/**
 * Split a statement key at its first `__`: `weekday__cheney` →
 * `{ base: 'weekday', instance: 'cheney' }`. A key without a valid
 * suffix is its own base.
 * @param {string} key
 * @returns {{ base: string, instance: string|null }}
 */
export function splitKey(key) {
	const k = String(key ?? '');
	const at = k.indexOf('__');
	if (at > 0 && INSTANCE.test(k.slice(at + 2))) {
		return { base: k.slice(0, at), instance: k.slice(at + 2) };
	}
	return { base: k, instance: null };
}

/**
 * The slot a statement key fills in a topic, or null when no slot
 * claims it (the page lists those under "Also on the office's
 * pages"). Reads each slot's own topic and its `also` topics. A
 * `__instance` suffix names the instance (`weekday__cheney`); without
 * one, an alias may imply it (`apostille_main_branch_street` →
 * `tumwater`). An explicit suffix wins over the alias.
 * @param {string} topic
 * @param {string|null} key the statement's field key (`parseStatement`)
 * @returns {{ slot: Slot, instance: string|null } | null}
 */
export function resolveSlot(topic, key) {
	if (typeof topic !== 'string' || typeof key !== 'string' || key === '') return null;
	const exact = INDEX.get(`${topic}\n${key}`);
	if (exact) return { slot: exact.slot, instance: exact.instance };
	const { base, instance } = splitKey(key);
	if (instance === null) return null;
	const hit = INDEX.get(`${topic}\n${base}`);
	return hit ? { slot: hit.slot, instance } : null;
}

/**
 * A topic's canonical keys, in registry order: what the /review key
 * chips offer. Aliases and `also` keys are never offered.
 * @param {string} topic
 * @returns {{ key: string, label: string }[]}
 */
export function canonicalKeys(topic) {
	return SLOTS.filter((s) => s.topic === topic).map((s) => ({ key: s.keys[0], label: s.label }));
}

/**
 * @param {string} id e.g. `fees.per_document`
 * @returns {Slot|null}
 */
export function slotById(id) {
	return BY_ID.get(id) ?? null;
}

/**
 * The page profile for an authority row (`{ country, kind, slug }`),
 * or null for a thin page. A row with a kind matches on kind and
 * country. A row without one (the e2e fixtures carry none) matches on
 * the slug pattern, and on the country too when it names one.
 * @param {{ country?: unknown, kind?: unknown, slug?: unknown } | null | undefined} authority
 * @returns {Profile|null}
 */
export function profileFor(authority) {
	if (!authority || typeof authority !== 'object') return null;
	const country = typeof authority.country === 'string' ? authority.country.trim().toLowerCase() : '';
	const kind = typeof authority.kind === 'string' ? authority.kind.trim() : '';
	const slug = typeof authority.slug === 'string' ? authority.slug : '';
	for (const p of PROFILES) {
		const countryOk = p.country.some((c) => c.toLowerCase() === country);
		if (kind) {
			if (countryOk && p.kind.includes(kind)) return p;
		} else if (p.slug?.test(slug) && (country === '' || countryOk)) {
			return p;
		}
	}
	return null;
}

/**
 * Pair service-notice facts by the id after `__`: `notice__<id>` is
 * an outage line, `restored__<id>` the office's line that resolves
 * it. One row per notice, with its restore or null; then each restore
 * whose notice is not among `items`. A bare `notice`, an unkeyed or
 * otherwise keyed notice has id null and never pairs; a bare
 * `restored` names no notice and stays unpaired. Facts of other
 * topics are skipped. The newest restore of an id (the larger fact id)
 * wins in any input order; without ids, the later item does.
 * @template {{ id?: unknown, topic?: unknown, statement?: unknown }} T
 * @param {readonly T[]} items
 * @returns {{ id: string|null, notice: T|null, restored: T|null }[]}
 */
export function pairNotices(items) {
	/** @type {{ id: string|null, notice: T|null, restored: T|null }[]} */
	const pairs = [];
	/** @type {Map<string, T>} */
	const restores = new Map();
	/** @type {T[]} */
	const unnamed = [];
	for (const item of items ?? []) {
		if (item?.topic !== NOTICE_TOPIC) continue;
		const hit = resolveSlot(NOTICE_TOPIC, parseStatement(String(item.statement ?? '')).fieldKey);
		if (hit?.slot.kind === 'notice_update') {
			if (hit.instance) {
				const prior = restores.get(hit.instance);
				if (!prior || !(Number(prior.id) > Number(item.id))) restores.set(hit.instance, item);
			} else {
				unnamed.push(item);
			}
		} else {
			pairs.push({ id: hit?.slot.kind === 'notice' ? hit.instance : null, notice: item, restored: null });
		}
	}
	/** @type {Set<string>} */
	const paired = new Set();
	for (const p of pairs) {
		const restored = p.id === null ? undefined : restores.get(p.id);
		if (p.id !== null && restored) {
			p.restored = restored;
			paired.add(p.id);
		}
	}
	for (const [id, item] of restores) if (!paired.has(id)) pairs.push({ id, notice: null, restored: item });
	for (const item of unnamed) pairs.push({ id: null, notice: null, restored: item });
	return pairs;
}
