// Form-resemblance framework: address input helpers.
//
// The address_block contract comes from forms-validate.js: an object
// with exactly the parts name/street/city/state/zip, where a
// non-blank value missing any part fails with code `address` and
// message `<label> is missing <part>`. This module mirrors that
// shape for inline per-part feedback; final validity still goes
// through validateField so codes never drift.

import { isBlank } from './validation.js';

/**
 * @typedef {{ code: string, name: string }} UsState
 * @typedef {{ name?: unknown, street?: unknown, city?: unknown, state?: unknown, zip?: unknown, line2?: unknown }} AddressValue
 */

/** Parts the address_block rule requires, in check order. */
export const ADDRESS_PARTS = ['name', 'street', 'city', 'state', 'zip'];

/** US states + DC + inhabited territories, alphabetical by name. */
export const US_STATES = [
	['AL', 'Alabama'],
	['AK', 'Alaska'],
	['AZ', 'Arizona'],
	['AR', 'Arkansas'],
	['CA', 'California'],
	['CO', 'Colorado'],
	['CT', 'Connecticut'],
	['DE', 'Delaware'],
	['DC', 'District of Columbia'],
	['FL', 'Florida'],
	['GA', 'Georgia'],
	['HI', 'Hawaii'],
	['ID', 'Idaho'],
	['IL', 'Illinois'],
	['IN', 'Indiana'],
	['IA', 'Iowa'],
	['KS', 'Kansas'],
	['KY', 'Kentucky'],
	['LA', 'Louisiana'],
	['ME', 'Maine'],
	['MD', 'Maryland'],
	['MA', 'Massachusetts'],
	['MI', 'Michigan'],
	['MN', 'Minnesota'],
	['MS', 'Mississippi'],
	['MO', 'Missouri'],
	['MT', 'Montana'],
	['NE', 'Nebraska'],
	['NV', 'Nevada'],
	['NH', 'New Hampshire'],
	['NJ', 'New Jersey'],
	['NM', 'New Mexico'],
	['NY', 'New York'],
	['NC', 'North Carolina'],
	['ND', 'North Dakota'],
	['OH', 'Ohio'],
	['OK', 'Oklahoma'],
	['OR', 'Oregon'],
	['PA', 'Pennsylvania'],
	['RI', 'Rhode Island'],
	['SC', 'South Carolina'],
	['SD', 'South Dakota'],
	['TN', 'Tennessee'],
	['TX', 'Texas'],
	['UT', 'Utah'],
	['VT', 'Vermont'],
	['VA', 'Virginia'],
	['WA', 'Washington'],
	['WV', 'West Virginia'],
	['WI', 'Wisconsin'],
	['WY', 'Wyoming'],
	['AS', 'American Samoa'],
	['GU', 'Guam'],
	['MP', 'Northern Mariana Islands'],
	['PR', 'Puerto Rico'],
	['VI', 'U.S. Virgin Islands']
].map(
	([code, name]) =>
		/** @type {UsState} */ ({ code, name })
);

/**
 * Two-letter code match (case-insensitive, trimmed).
 * @param {string} value
 * @returns {boolean}
 */
export function isUsStateCode(value) {
	const v = value.trim().toUpperCase();
	return US_STATES.some((s) => s.code === v);
}

/**
 * State/territory name for a code (case-insensitive), or null.
 * @param {string} code
 * @returns {string | null}
 */
export function stateNameForCode(code) {
	const s = US_STATES.find((s) => s.code === code.trim().toUpperCase());
	return s ? s.name : null;
}

/**
 * Required parts that are blank, in `address_block` check order.
 * A fully-blank value returns every part (the field-level `required`
 * message covers it); a non-object returns [] mirroring the
 * address_block quirk (non-objects pass the rule).
 * @param {unknown} value
 * @returns {string[]}
 */
export function missingAddressParts(value) {
	if (typeof value !== 'object' || value === null || Array.isArray(value)) return [];
	const record = /** @type {Record<string, unknown>} */ (value);
	return ADDRESS_PARTS.filter((part) => isBlank(record[part]));
}

/**
 * One-line display form, skipping blank parts: line2 joins the
 * street with a space, state/zip join with a space, segments join
 * with commas. Empty string when everything is blank.
 * @param {AddressValue | null | undefined} value
 * @returns {string}
 */
export function formatAddressOneLine(value) {
	if (value === null || value === undefined) return '';
	const text = (/** @type {unknown} */ v) =>
		typeof v === 'string' ? v.trim() : '';
	const street = [text(value.street), text(value.line2)]
		.filter((s) => s !== '')
		.join(' ');
	const tail = [text(value.city), [text(value.state), text(value.zip)]
		.filter((s) => s !== '')
		.join(' ')].filter((s) => s !== '');
	return [text(value.name), street, ...tail].filter((s) => s !== '').join(', ');
}
