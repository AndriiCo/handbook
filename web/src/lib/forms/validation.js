// Form-resemblance framework: validation entry point.
//
// Thin wrapper over the customer validator
// (../customer/forms-validate.js), which is a faithful TS port of
// crates/forms/src/validate.rs. Error codes AND messages stay
// identical end to end — the vectors file
// (web/test/forms-validate-vectors.json) pins both, and framework
// code must never invent a code. Per-form routes import from here,
// never from ../customer/ directly, so the framework stays the one
// place that knows the layering.

export {
	isBlank,
	parseMoneyCents,
	validateField,
	validateSubmission
} from '../customer/forms-validate.js';

/**
 * @typedef {import('../customer/forms-validate.js').FieldError} FieldError
 */

/**
 * Errors for one field key, in validation order.
 * @param {FieldError[]} errors
 * @param {string} key
 * @returns {FieldError[]}
 */
export function errorsForKey(errors, key) {
	return errors.filter((e) => e.key === key);
}

/**
 * True when a validateSubmission result holds no errors.
 * @param {FieldError[]} errors
 * @returns {boolean}
 */
export function isValid(errors) {
	return errors.length === 0;
}
