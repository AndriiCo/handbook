// W-3 fake modes in the staff console. Admin-only: never imported from
// (public) or (customer) routes.
//
// GET {apiBase}/admin/version → { …, fakes: [{ slug, label }] }
//   names every integration running its local fake (`<area>.mode = fake`;
//   the server refuses fakes off loopback unless the host is staging).

/**
 * @typedef {{ slug: string, label: string }} FakeIntegration
 */

/**
 * The FAKE ribbon's text, or null when nothing runs fake. Pure (tested).
 * @param {unknown} fakes the `fakes` array from /admin/version
 * @returns {string | null}
 */
export function fakeRibbon(fakes) {
	if (!Array.isArray(fakes)) return null;
	const names = fakes
		.map((f) => (f && typeof f === 'object' ? /** @type {FakeIntegration} */ (f).label : null))
		.filter((label) => typeof label === 'string' && label.trim() !== '');
	if (names.length === 0) return null;
	return `FAKE — ${names.join(', ')}. Demo stand-ins: nothing here sends mail, ships a parcel or charges a card.`;
}
