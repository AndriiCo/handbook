// Form-resemblance framework: phone formatting.
//
// Display formatting only — validity stays with the `tel` rule in
// forms-validate.js (7+ digits, phone charset). The formatter never
// rejects input: NANP numbers get canonical display form, anything
// else passes through trimmed and unmangled.

/**
 * Digits in the input, preserving a leading '+' marker separately.
 * @param {string} input
 * @returns {{ plus: boolean, digits: string }}
 */
function splitPlus(input) {
	const trimmed = input.trim();
	const plus = trimmed.startsWith('+');
	return { plus, digits: trimmed.replace(/[^0-9]/g, '') };
}

/**
 * All digit characters in the input, no separators.
 * @param {string} input
 * @returns {string}
 */
export function normalizePhoneDigits(input) {
	return splitPlus(input).digits;
}

/**
 * Format NANP numbers for display: 10 digits as `(555) 123-4567`,
 * 11 digits starting with 1 as `+1 (555) 123-4567`. Every other
 * input returns trimmed and unchanged (international numbers,
 * extensions, partial drafts).
 * @param {string} input
 * @returns {string}
 */
export function formatPhone(input) {
	const { digits } = splitPlus(input);
	if (digits.length === 10) {
		return `(${digits.slice(0, 3)}) ${digits.slice(3, 6)}-${digits.slice(6)}`;
	}
	if (digits.length === 11 && digits.startsWith('1')) {
		const ten = digits.slice(1);
		return `+1 (${ten.slice(0, 3)}) ${ten.slice(3, 6)}-${ten.slice(6)}`;
	}
	return input.trim();
}
