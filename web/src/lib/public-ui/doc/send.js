// The values "Send or bring your documents" repeats from other steps
// (plan §4: repeat uses reuse the number). What to include (step 3) and
// how it comes back (step 6) sit in the mail branch; the office phone
// sits in the courier branch's recipient line and, with its menu
// option, in the disputed-address box's "What you can do". Each repeat
// is a copy of the step's own Item under an id of its own, so its
// marker shows the same [n] with a marker id of its own; the page
// numbers the copies after every other item (cite.js), which never
// moves a number. SendBlock renders exactly the copies this returns,
// under the same conditions, so no numbered use is missing from the
// page. Pure: no window/fetch.

import { SLOT, bySlot, usable } from './items.js';
import { isPoBox } from './postal.js';

/** @typedef {import('./contract.js').Item} Item */
/** @typedef {import('./contract.js').Section} Section */

/**
 * @typedef {{
 *   enclose: Item|null,
 *   returns: Item|null,
 *   courierPhone: Item|null,
 *   conflictPhone: Item|null,
 *   conflictOption: Item|null
 * }} SendRepeats
 */

/**
 * A repeat of `item` under `${item.id}-${use}` (its alternatives too),
 * without Copy; null when there is no item or it belongs to the send
 * step itself (then the step shows the item, not a repeat).
 * @param {Item|null} item
 * @param {string} use
 * @param {readonly Item[]} own the send step's items
 * @returns {Item|null}
 */
function repeatOf(item, use, own) {
	if (!item || own.includes(item)) return null;
	return {
		...item,
		id: `${item.id}-${use}`,
		copy: false,
		alternatives: item.alternatives ? item.alternatives.map((a) => ({ ...a, id: `${a.id}-${use}`, copy: false })) : null
	};
}

/**
 * The send step's repeats of other steps' values, or null when the page
 * has no send step.
 * @param {readonly Section[]} sections `page.sections`
 * @returns {SendRepeats|null}
 */
export function sendRepeats(sections) {
	const send = (sections ?? []).find((s) => s?.id === 'send');
	if (!send) return null;
	const own = send.items ?? [];
	const all = (sections ?? []).flatMap((s) => [...(s?.items ?? []), ...(s?.also ?? [])]);
	const find = (/** @type {string} */ slot) => bySlot(own, slot) ?? bySlot(all, slot);
	const usps = find(SLOT.usps);
	const box = !!usps && usps.state === 'conflict' && (usps.alternatives?.length ?? 0) > 0;
	const courierRaw = find(SLOT.courier);
	const courier = courierRaw && !isPoBox(courierRaw.display) ? courierRaw : null;
	const phone = bySlot(all, SLOT.phone);
	const option = bySlot(all, SLOT.phoneOption);
	const callable = !!phone && usable(phone);
	return {
		enclose: repeatOf(bySlot(all, SLOT.enclose), 'send', own),
		returns: repeatOf(bySlot(all, SLOT.return), 'send', own),
		courierPhone: courier && usable(courier) && callable ? repeatOf(phone, 'courier', own) : null,
		conflictPhone: box && callable ? repeatOf(phone, 'conflict', own) : null,
		conflictOption: box && callable && !!option && usable(option) ? repeatOf(option, 'conflict', own) : null
	};
}

/**
 * The repeats as a list, for numbering.
 * @param {SendRepeats|null} repeats
 * @returns {Item[]}
 */
export function repeatItems(repeats) {
	if (!repeats) return [];
	return [repeats.enclose, repeats.returns, repeats.conflictPhone, repeats.conflictOption, repeats.courierPhone].filter(
		/** @returns {i is Item} */ (i) => !!i
	);
}
