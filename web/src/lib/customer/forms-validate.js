// Customer form validation: TS port of crates/forms/src/validate.rs (P2-F3).
// The SPA runs these checks for instant inline feedback; the Rust module is
// authoritative and the server rejects anything invalid. Error codes AND
// messages must stay identical — web/test/forms-validate-vectors.json pins
// both, executed by web/test/forms-validate.test.mjs (this file) and by the
// Rust parity test in crates/web/src/p2.rs.
//
// Two faithfully-ported quirks (deliberate, not bugs in this file):
// - email allows a second '@' in the domain ("a@b@c.co" passes both);
// - address_block ignores non-object values ("whatever" passes both).
// Fix in Rust first, then re-port and extend the vectors.

/** @typedef {{ key: string, code: string, message: string }} FieldError */
/**
 * @typedef {object} FieldDef
 * @property {string} key
 * @property {string} label
 * @property {string} type
 * @property {boolean} [required]
 * @property {number} [max_len]
 * @property {string} [pattern]
 * @property {string[]} [options]
 */

/**
 * Blank check mirroring Rust is_blank: missing/null/whitespace-only/empty
 * array/object-with-all-blank-values are blank; anything else is not.
 * @param {unknown} value
 * @returns {boolean}
 */
export function isBlank(value) {
	if (value === null || value === undefined) return true;
	if (typeof value === 'string') return value.trim() === '';
	if (Array.isArray(value)) return value.length === 0;
	if (typeof value === 'object') return Object.values(value).every(isBlank);
	return false;
}

/**
 * Parse `25`, `25.5`, or `25.00` into cents. Mirrors Rust parse_money_cents:
 * negatives, signs, empty, >2 decimals, and non-digits are rejected.
 * @param {string} input
 * @returns {number | null}
 */
export function parseMoneyCents(input) {
	const s = input.trim();
	if (s === '' || s.startsWith('-') || s.startsWith('+')) return null;
	let dollars;
	let cents;
	const dot = s.indexOf('.');
	if (dot === -1) {
		dollars = s;
		cents = '00';
	} else {
		dollars = s.slice(0, dot);
		cents = s.slice(dot + 1);
		if (cents.length === 0 || cents.length > 2 || !/^[0-9]+$/.test(cents)) return null;
	}
	if (dollars === '' || !/^[0-9]+$/.test(dollars)) return null;
	if (cents.length === 1) cents = cents + '0';
	const total = Number(dollars) * 100 + Number(cents);
	if (!Number.isSafeInteger(total)) return null;
	return total;
}

/**
 * @param {string} s
 * @returns {boolean}
 */
function isEmail(s) {
	// Rust u8::is_ascii_whitespace: space, \t, \n, \f, \r (no \v).
	if (/[ \t\n\f\r]/.test(s)) return false;
	const at = s.indexOf('@');
	if (at === -1) return false;
	const local = s.slice(0, at);
	const domain = s.slice(at + 1);
	return (
		local !== '' && !domain.startsWith('.') && domain.includes('.') && !domain.endsWith('.')
	);
}

/**
 * @param {string} s
 * @returns {boolean}
 */
function isPhone(s) {
	const digits = (s.match(/[0-9]/g) ?? []).length;
	return digits >= 7 && /^[0-9 .\-()+]*$/.test(s) && s !== '';
}

/**
 * Strict YYYY-MM-DD including leap years. Mirrors Rust is_date.
 * @param {string} s
 * @returns {boolean}
 */
function isDate(s) {
	const parts = s.split('-');
	if (parts.length !== 3) return false;
	const [ys, ms, ds] = parts;
	if (ys.length !== 4 || ms.length !== 2 || ds.length !== 2) return false;
	if (!/^[0-9]+$/.test(ys) || !/^[0-9]+$/.test(ms) || !/^[0-9]+$/.test(ds)) return false;
	const y = Number(ys);
	const m = Number(ms);
	const d = Number(ds);
	if (m === 0 || m > 12 || d === 0) return false;
	const leap = y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0);
	const max =
		m === 2 ? (leap ? 29 : 28) : [4, 6, 9, 11].includes(m) ? 30 : 31;
	return d <= max;
}

/**
 * Mirrors Rust `s.trim().parse::<f64>().is_ok()`: plain decimals and
 * exponents parse; hex, empty, and trailing junk do not. Rust also accepts
 * bare NaN/infinity spellings, so this port does too.
 * @param {string} s
 * @returns {boolean}
 */
function isNumber(s) {
	const t = s.trim();
	if (t === '') return false;
	if (!Number.isNaN(Number(t))) {
		// Number() accepts hex ("0x10") and "" (guarded above); Rust rejects hex.
		if (/^0[xX]/.test(t)) return false;
		return true;
	}
	return /^(nan|[+-]?(inf|infinity))$/i.test(t);
}

/**
 * @param {unknown} value
 * @returns {string | null} missing address part, or null when complete
 * (or when the value is not an object — Rust quirk, see header).
 */
function missingAddressPart(value) {
	if (typeof value !== 'object' || value === null || Array.isArray(value)) return null;
	for (const part of ['name', 'street', 'city', 'state', 'zip']) {
		if (isBlank(/** @type {Record<string, unknown>} */ (value)[part])) return part;
	}
	return null;
}

/**
 * Validate one field value. Returns the field's errors (empty = valid).
 * @param {FieldDef} field
 * @param {unknown} value
 * @returns {FieldError[]}
 */
export function validateField(field, value) {
	const errors = [];
	const label = field.label;
	if (isBlank(value)) {
		if (field.required) {
			errors.push({ key: field.key, code: 'required', message: `${label} is required` });
		}
		return errors;
	}
	if (
		field.max_len !== undefined &&
		field.max_len !== null &&
		typeof value === 'string' &&
		[...value].length > field.max_len
	) {
		errors.push({
			key: field.key,
			code: 'max_len',
			message: `${label} must be at most ${field.max_len} characters`
		});
	}
	if (field.pattern !== undefined && field.pattern !== null && typeof value === 'string') {
		let matches = false;
		try {
			matches = new RegExp(`^(?:${field.pattern})$`).test(value);
		} catch {
			matches = false;
		}
		if (!matches) {
			errors.push({
				key: field.key,
				code: 'pattern',
				message: `${label} has an invalid format`
			});
		}
	}
	switch (field.type) {
		case 'text':
		case 'textarea':
			if (typeof value !== 'string') {
				errors.push({ key: field.key, code: 'type', message: `${label} must be text` });
			}
			break;
		case 'email':
			if (typeof value !== 'string' || !isEmail(value)) {
				errors.push({
					key: field.key,
					code: 'email',
					message: `${label} must be a valid email address`
				});
			}
			break;
		case 'tel':
			if (typeof value !== 'string' || !isPhone(value)) {
				errors.push({
					key: field.key,
					code: 'tel',
					message: `${label} must be a valid phone number`
				});
			}
			break;
		case 'date':
			if (typeof value !== 'string' || !isDate(value)) {
				errors.push({
					key: field.key,
					code: 'date',
					message: `${label} must be a date like 2026-11-04`
				});
			}
			break;
		case 'number': {
			const ok =
				(typeof value === 'number' && !Number.isNaN(value)) ||
				(typeof value === 'string' && isNumber(value));
			if (!ok) {
				errors.push({ key: field.key, code: 'number', message: `${label} must be a number` });
			}
			break;
		}
		case 'money': {
			const ok =
				(typeof value === 'string' && parseMoneyCents(value) !== null) ||
				(typeof value === 'number' && Number.isInteger(value) && value >= 0);
			if (!ok) {
				errors.push({
					key: field.key,
					code: 'money',
					message: `${label} must be an amount like 25.00`
				});
			}
			break;
		}
		case 'select':
		case 'radio':
			if (
				typeof value !== 'string' ||
				!(field.options ?? []).some((o) => o === value)
			) {
				errors.push({
					key: field.key,
					code: 'option',
					message: `${label} must be one of the listed options`
				});
			}
			break;
		case 'checkbox':
			if (typeof value !== 'boolean') {
				errors.push({
					key: field.key,
					code: 'type',
					message: `${label} must be checked or unchecked`
				});
			}
			break;
		case 'address_block': {
			const missing = missingAddressPart(value);
			if (missing !== null) {
				errors.push({
					key: field.key,
					code: 'address',
					message: `${label} is missing ${missing}`
				});
			}
			break;
		}
		default:
			break;
	}
	return errors;
}

/**
 * Validate a full submission: one pass per schema field, in order.
 * Unknown keys are ignored (draft state may carry extras).
 * @param {FieldDef[]} fields
 * @param {Record<string, unknown>} values
 * @returns {FieldError[]}
 */
export function validateSubmission(fields, values) {
	return fields.flatMap((field) =>
		validateField(field, Object.hasOwn(values, field.key) ? values[field.key] : undefined)
	);
}
