// Accept-with-edit helpers for /review (plan §5.5, §9.3). Key chips
// re-key a draft's statement from the slot registry; the topic select
// moves the draft to another frozen topic (CCC closures filed as
// `service_notice` become `holiday_hours`). Admin-only. Pure: node
// tests drive it directly.

import { SECTIONS, TOPICS, TOPIC_SECTION, resolveSlot, splitKey } from '../public-ui/slots.js';
import { acceptBody } from './review-client.js';

/** Call scripts never change topic and nothing moves in: they are never public. */
export const LOCKED_TOPIC = 'phone_script';

/** Where the pseudo-sections' topics show, for the topic select. */
/** @type {Record<string, string>} */
const PSEUDO_WHERE = { status: 'Service status banner', hidden: 'Never shown publicly' };

/**
 * A statement's key prefix: the guide's grammar (`{field}: {value}`),
 * plus a key whose value is still empty (`fee:`).
 */
const KEY = /^([a-z][a-z0-9_]*):(?:\s+|$)/;

/**
 * Topic choices for the select, alphabetical, each with where its
 * facts show on the public page.
 * @returns {{ topic: string, where: string }[]}
 */
export function topicChoices() {
	/** @type {Map<string, string>} */
	const where = new Map(SECTIONS.map((s) => [s.id, s.heading ?? PSEUDO_WHERE[s.id] ?? '']));
	return [...TOPICS].sort().map((topic) => ({ topic, where: where.get(TOPIC_SECTION[topic]) ?? '' }));
}

/**
 * Whether a draft filed under `from` may be accepted under `to`: any
 * frozen topic, never into or out of `phone_script` (the server
 * refuses those moves too).
 * @param {string} from
 * @param {string} to
 */
export function canMoveTopic(from, to) {
	if (!TOPICS.includes(to)) return false;
	return from === to || (from !== LOCKED_TOPIC && to !== LOCKED_TOPIC);
}

/**
 * The statement's key, with any `__instance` suffix, or null when it
 * is unkeyed.
 * @param {string} statement
 * @returns {string|null}
 */
export function statementKey(statement) {
	return KEY.exec(String(statement ?? ''))?.[1] ?? null;
}

/**
 * Rewrite only the key prefix: `{old}: ` becomes `{key}: `, or
 * `{key}: ` goes in front of an unkeyed statement. The value text is
 * kept exactly as typed.
 * @param {string} statement
 * @param {string} key
 * @returns {string}
 */
export function rekey(statement, key) {
	const s = String(statement ?? '');
	const m = KEY.exec(s);
	return `${key}: ${m ? s.slice(m[0].length) : s}`;
}

/**
 * The statement after a key chip click. The chip's key replaces the
 * old one; an instance carries over when the chip's slot lists one
 * value per instance: `apostille_eastern_branch_street` becomes
 * `office__cheney`, `notice__x` becomes `restored__x`.
 * @param {string} statement
 * @param {string} chip a canonical key of `topic`
 * @param {string} topic the topic it will be accepted under
 * @param {string} draftTopic the topic the draft was filed under
 * @returns {string}
 */
export function chipStatement(statement, chip, topic, draftTopic) {
	const key = statementKey(statement);
	let instance = key === null ? null : splitKey(key).instance;
	if (key !== null && instance === null) {
		instance = resolveSlot(draftTopic, key)?.instance ?? resolveSlot(topic, key)?.instance ?? null;
	}
	const many = resolveSlot(topic, chip)?.slot.many === true;
	return rekey(statement, many && instance ? `${chip}__${instance}` : chip);
}

/**
 * One line on how the public page will label this statement.
 * @param {string} statement
 * @param {string} topic
 * @returns {string}
 */
export function keyNote(statement, topic) {
	if (TOPIC_SECTION[topic] === 'hidden') return 'Call scripts never show on the public page.';
	const key = statementKey(statement);
	if (key === null) return 'No key yet: pick one so the page can label it.';
	const hit = resolveSlot(topic, key);
	if (!hit) return `Unknown key for ${topic}: the page lists it under “Also on the office's pages”.`;
	return `The page labels it “${hit.slot.label}”${hit.instance ? ` (${hit.instance})` : ''}.`;
}

/**
 * The accept-with-edit body. The statement goes only when edited, and
 * `topic` only when the reviewer moved the draft, so a server that
 * predates the field never sees it. Null when invalid (see
 * `acceptBody`) or when the move is refused.
 * @param {{ topic: string, statement: string }} draft
 * @param {string} editText
 * @param {string} topic the select's value; empty means unchanged
 * @returns {{ reason?: string, statement?: string, topic?: string } | null}
 */
export function editAcceptBody(draft, editText, topic) {
	const to = topic || draft.topic;
	if (to !== draft.topic && !canMoveTopic(draft.topic, to)) return null;
	return acceptBody(
		undefined,
		editText === draft.statement ? undefined : editText,
		to === draft.topic ? undefined : to
	);
}
