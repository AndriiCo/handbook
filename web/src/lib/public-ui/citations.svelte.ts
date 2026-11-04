// Citations mode, the shared switch (R2): one page-wide `on` that the
// markers (doc/Cite), the card (doc/CitationLayer), the endnotes
// (doc/Endnotes), the toggle (doc/CitationsToggle) and print read.
//
// - Precedence on load: `?cite=1|0`, then localStorage `hb_cite`, then
//   off (cite.js `citePreference`). Storage reads and writes sit in
//   try/catch: private windows and blocked storage fall back to off.
// - A toggle writes both: storage, and the URL with the authority
//   page's shallow-replace pattern, `goto(…, { shallow: true, replace:
//   true, state })`, so a copied link opens the same way. A shallow goto
//   runs navigation hooks; the root layout's onNavigate skips shallow
//   navigations, so a toggle never starts a page view transition.
// - `message` feeds a polite live region: "Citations shown: N sources
//   numbered" / "Citations hidden".
// - While attached, `html[data-cite="on|off"]` mirrors the state for
//   the print sheet (app.css @media print).
// Client-only module (the public SPA renders with `ssr = false`).

import { goto } from '$app/navigation';
import { page } from '$app/state';
import { CITE_STORAGE_KEY, citeAnnouncement, citeHref, citePreference } from './cite.js';
import { site } from '../locale.svelte.js';

function readStored(): string | null {
	try {
		return localStorage.getItem(CITE_STORAGE_KEY);
	} catch {
		return null;
	}
}

function writeStored(on: boolean): void {
	try {
		localStorage.setItem(CITE_STORAGE_KEY, on ? '1' : '0');
	} catch {
		// Storage blocked: the URL still carries the choice.
	}
}

function writeUrl(on: boolean): void {
	let href = '';
	try {
		href = citeHref(location.href, on);
	} catch {
		return;
	}
	const fallback = () => {
		try {
			history.replaceState(history.state, '', href);
		} catch {
			// The address bar keeps the old parameter; storage still holds the choice.
		}
	};
	try {
		goto(href, { shallow: true, replace: true, state: page.state ?? {} }).catch(fallback);
	} catch {
		fallback();
	}
}

class CitationsState {
	/** Citations shown: markers, the card and the endnotes. */
	on = $state(false);
	/** Numbered sources on the page (set by Endnotes and CitationLayer). */
	count = $state(0);
	/** Polite live-region text after a toggle. */
	message = $state('');

	#attached = 0;
	#returnTo = new Map<number, string>();

	/** Read the starting state from the address and storage. */
	init(search: string = typeof location === 'undefined' ? '' : location.search): void {
		this.on = citePreference(search, readStored());
		this.#mirror();
	}

	/**
	 * Mirror the state onto `<html data-cite>` while a toggle is on the
	 * page; the returned function removes it again.
	 */
	attach(): () => void {
		this.#attached += 1;
		this.#mirror();
		return () => {
			this.#attached = Math.max(0, this.#attached - 1);
			if (this.#attached === 0 && typeof document !== 'undefined') {
				document.documentElement.removeAttribute('data-cite');
			}
		};
	}

	/** Show or hide citations; writes storage and the URL, then announces. */
	set(next: boolean): void {
		if (next === this.on) return;
		this.on = next;
		writeStored(next);
		writeUrl(next);
		this.#mirror();
		this.message = citeAnnouncement(next, this.count, site.locale);
	}

	toggle(): void {
		this.set(!this.on);
	}

	/** The marker a reader followed to endnote `n`, for its backlink. */
	rememberReturn(n: number, markerId: string): void {
		if (markerId) this.#returnTo.set(n, markerId);
	}

	returnFor(n: number): string | null {
		return this.#returnTo.get(n) ?? null;
	}

	#mirror(): void {
		if (this.#attached === 0 || typeof document === 'undefined') return;
		document.documentElement.setAttribute('data-cite', this.on ? 'on' : 'off');
	}
}

/** The page-wide citations switch. */
export const citations = new CitationsState();

// Read the preference as soon as the page code loads, so markers and
// endnotes render on the first paint of a `?cite=1` link.
if (typeof window !== 'undefined') citations.init();
