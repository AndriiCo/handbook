// Alt/⌥+C turns citations on and off (R2 citations mode).
//
// Why this key: WCAG 2.1.4 covers single-character shortcuts only, so a
// modifier chord needs no off switch; NVDA and JAWS browse modes pass
// Alt chords through but swallow a bare "c"; Firefox's Alt letters are
// F/E/V/S/B/T/H; Cmd/Ctrl+K is search. `code` survives macOS's "ç" and
// Cyrillic layouts, `key` covers Dvorak. The button stays the primary
// control.
//
// Pure: the matcher and the installer take plain objects, so node
// tests drive them without a browser.

/**
 * Alt/⌥+C with no other modifier.
 * @param {{ code?: string, key?: string, altKey?: boolean, ctrlKey?: boolean, metaKey?: boolean, shiftKey?: boolean }|null|undefined} e
 * @returns {boolean}
 */
export function isCitationHotkey(e) {
	if (!e) return false;
	const key = typeof e.key === 'string' ? e.key.toLowerCase() : '';
	return (e.code === 'KeyC' || key === 'c') && !!e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey;
}

const EDITABLE = 'input, textarea, select, [contenteditable]:not([contenteditable="false"])';

/**
 * True when typing there must win over the shortcut: form fields and
 * editable regions (inherited contenteditable included).
 * @param {unknown} target
 * @returns {boolean}
 */
export function isEditableTarget(target) {
	const el = /** @type {{ isContentEditable?: boolean, closest?: (s: string) => unknown }|null} */ (
		target && typeof target === 'object' ? target : null
	);
	if (!el) return false;
	if (el.isContentEditable === true) return true;
	if (typeof el.closest !== 'function') return false;
	try {
		return !!el.closest(EDITABLE);
	} catch {
		return false;
	}
}

/**
 * True while a modal dialog is open: an `[aria-modal="true"]` element
 * that is rendered, or a native `dialog:modal`.
 * @param {{ querySelectorAll?: (s: string) => ArrayLike<unknown>, querySelector?: (s: string) => unknown }|null|undefined} doc
 * @returns {boolean}
 */
export function hasOpenModal(doc) {
	if (!doc) return false;
	if (typeof doc.querySelectorAll === 'function') {
		try {
			for (const raw of Array.from(doc.querySelectorAll('[aria-modal="true"]'))) {
				const el = /** @type {{ hidden?: boolean, getClientRects?: () => ArrayLike<unknown> }} */ (raw);
				if (el.hidden === true) continue;
				if (typeof el.getClientRects === 'function' && el.getClientRects().length === 0) continue;
				return true;
			}
		} catch {
			// an unreadable document never blocks the shortcut
		}
	}
	if (typeof doc.querySelector === 'function') {
		try {
			if (doc.querySelector('dialog:modal')) return true;
		} catch {
			// `:modal` unsupported: no native modal can be open either
		}
	}
	return false;
}

/**
 * Listen for Alt/⌥+C on `target` (default `window`). Skips auto-repeat,
 * IME composition, editable targets and open modal dialogs, and calls
 * `preventDefault` only when it handles the key. Returns the remover.
 * @param {{
 *   onToggle: () => void,
 *   target?: { addEventListener: Function, removeEventListener: Function }|null,
 *   doc?: Parameters<typeof hasOpenModal>[0]
 * }} opts
 * @returns {() => void}
 */
export function installCitationHotkey(opts) {
	const target =
		opts.target ?? (typeof window !== 'undefined' ? /** @type {any} */ (window) : null);
	const doc = opts.doc ?? (typeof document !== 'undefined' ? document : null);
	if (!target || typeof target.addEventListener !== 'function') return () => {};
	/** @param {KeyboardEvent} e */
	const onKeydown = (e) => {
		if (!e || e.defaultPrevented || e.repeat || e.isComposing) return;
		if (!isCitationHotkey(e)) return;
		if (isEditableTarget(e.target)) return;
		if (hasOpenModal(doc)) return;
		e.preventDefault();
		opts.onToggle();
	};
	target.addEventListener('keydown', onKeydown);
	return () => target.removeEventListener('keydown', onKeydown);
}
