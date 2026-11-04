// The walk-in report draft (SA-21) a visitor is writing, kept in
// localStorage until the report is sent, so a closed tab, a sign-in
// round trip or a crash loses nothing. One draft per office slug
// (`hb_report_draft:<slug>`, or `:any` on the bare /report page).
// Photos are not stored here: they upload the moment they are picked,
// and the draft keeps only the server's upload ids and names (a staged
// upload lives a day). Pure and injectable, so node tests drive it.

export const DRAFT_PREFIX = 'hb_report_draft:';
/** Autosave cadence while the form is open (ms). */
export const AUTOSAVE_MS = 60 * 1000;
/** Staged uploads expire server-side after a day; so does the draft's list. */
export const UPLOAD_TTL_MS = 24 * 3600 * 1000;
const MAX_BODY = 2000;

/** @param {string | null | undefined} slug */
export function draftKey(slug) {
	const s = typeof slug === 'string' ? slug.trim().toLowerCase() : '';
	return DRAFT_PREFIX + (s || 'any');
}

/**
 * @typedef {{ id: string, name: string, size: number, at: number }} StagedUpload
 * @typedef {{ authority: string, body: string, uploads: StagedUpload[], savedAt: number }} Draft
 */

/**
 * A stored draft, or null when absent or unreadable. Uploads older
 * than the server keeps them are dropped.
 * @param {string | null | undefined} raw
 * @param {number} [now] ms since the epoch
 * @returns {Draft | null}
 */
export function parseDraft(raw, now = Date.now()) {
	if (typeof raw !== 'string' || !raw) return null;
	let v;
	try {
		v = JSON.parse(raw);
	} catch {
		return null;
	}
	if (!v || typeof v !== 'object') return null;
	const uploads = Array.isArray(v.uploads)
		? v.uploads
				.filter(
					(/** @type {any} */ u) =>
						u &&
						typeof u.id === 'string' &&
						/^[0-9a-f]{32}$/.test(u.id) &&
						typeof u.at === 'number' &&
						now - u.at < UPLOAD_TTL_MS
				)
				.map((/** @type {any} */ u) => ({
					id: u.id,
					name: typeof u.name === 'string' ? u.name.slice(0, 128) : 'photo',
					size: typeof u.size === 'number' ? u.size : 0,
					at: u.at
				}))
				.slice(0, 3)
		: [];
	return {
		authority: typeof v.authority === 'string' ? v.authority.slice(0, 128) : '',
		body: typeof v.body === 'string' ? v.body.slice(0, MAX_BODY) : '',
		uploads,
		savedAt: typeof v.savedAt === 'number' ? v.savedAt : 0
	};
}

/** @param {{ body?: string, uploads?: unknown[] }} d */
export function isEmptyDraft(d) {
	return !d || (!(d.body ?? '').trim() && !(d.uploads ?? []).length);
}

/**
 * Write the draft (or clear it when empty). Storage may be missing or
 * throw (private mode, blocked site data): that is never an error.
 * @param {Pick<Storage, 'setItem' | 'removeItem'> | null | undefined} storage
 * @param {string} key
 * @param {Omit<Draft, 'savedAt'>} draft
 * @param {number} [now]
 * @returns {number | null} the save time, or null when nothing was kept
 */
export function saveDraft(storage, key, draft, now = Date.now()) {
	try {
		if (isEmptyDraft(draft)) {
			storage?.removeItem(key);
			return null;
		}
		storage?.setItem(key, JSON.stringify({ ...draft, savedAt: now }));
		return storage ? now : null;
	} catch {
		return null;
	}
}

/**
 * @param {Pick<Storage, 'getItem'> | null | undefined} storage
 * @param {string} key
 * @param {number} [now]
 */
export function loadDraft(storage, key, now = Date.now()) {
	try {
		return parseDraft(storage?.getItem(key), now);
	} catch {
		return null;
	}
}

/** @param {Pick<Storage, 'removeItem'> | null | undefined} storage @param {string} key */
export function clearDraft(storage, key) {
	try {
		storage?.removeItem(key);
	} catch {
		/* storage blocked: nothing to clear */
	}
}

/**
 * A page selection as a quote line for the note: whitespace folded,
 * capped, prefixed "> ".
 * @param {string | null | undefined} text
 */
export function quoteLine(text) {
	const t = (text ?? '').replace(/\s+/g, ' ').trim();
	if (!t) return '';
	return `> "${t.length > 300 ? t.slice(0, 299) + '…' : t}"\n`;
}

/**
 * The photo kind by magic bytes (the server checks again), or null.
 * @param {Uint8Array} b the file's first 12 bytes or more
 * @returns {'jpeg' | 'png' | 'webp' | 'gif' | null}
 */
export function photoKind(b) {
	const at = (/** @type {number[]} */ sig, off = 0) =>
		b.length >= off + sig.length && sig.every((v, i) => b[off + i] === v);
	if (at([0xff, 0xd8, 0xff])) return 'jpeg';
	if (at([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return 'png';
	if (at([0x52, 0x49, 0x46, 0x46]) && at([0x57, 0x45, 0x42, 0x50], 8)) return 'webp';
	if (at([0x47, 0x49, 0x46, 0x38]) && (b[4] === 0x37 || b[4] === 0x39) && b[5] === 0x61) return 'gif';
	return null;
}
