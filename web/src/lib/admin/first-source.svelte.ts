// Open questions for one authority, shared by /calls and /email: one
// read of `GET /admin/first-source/{slug}/questions` per pick, then
// `authority:{slug}` (fact_accepted) re-reads it, debounced — an
// approval in /review drops the answered question while staff watch.
// No polling. Admin-only.
import { subscribe } from '#lib/live.svelte.js';
import { fetchQuestions, type QuestionSet } from './outreach-client.js';
import { errorText } from './admin-api.js';

export type LoadPhase = 'idle' | 'loading' | 'ready' | 'failed';

const DEBOUNCE_MS = 600;

export class OpenQuestionsState {
	slug = $state('');
	set = $state<QuestionSet | null>(null);
	phase = $state<LoadPhase>('idle');
	error = $state<string | null>(null);
	refreshing = $state(false);
	/** The authority topic delivered at least one message. */
	live = $state(false);
	/** Bumped when a live event re-read the list (row flash). */
	updatedAt = $state(0);

	#seq = 0;
	#unsub: (() => void) | null = null;
	#timer: ReturnType<typeof setTimeout> | null = null;
	#onLoaded: ((set: QuestionSet) => void) | null;

	constructor(onLoaded?: (set: QuestionSet) => void) {
		this.#onLoaded = onLoaded ?? null;
	}

	get questions() {
		return this.set?.questions ?? [];
	}

	get contacts() {
		return this.set?.contacts ?? [];
	}

	/** Switch authority ('' clears). Re-picking the same slug is a no-op. */
	select(slug: string) {
		if (slug === this.slug && this.phase !== 'failed') return;
		this.#stop();
		this.slug = slug;
		this.set = null;
		this.error = null;
		this.live = false;
		if (!slug) {
			this.phase = 'idle';
			return;
		}
		void this.reload();
		this.#unsub = subscribe(`authority:${slug}`, (msg) => {
			if (this.slug !== slug) return;
			this.live = true;
			if (msg.type !== 'fact_accepted') return;
			if (this.#timer !== null) clearTimeout(this.#timer);
			this.#timer = setTimeout(() => {
				this.#timer = null;
				void this.reload(true).then(() => (this.updatedAt = Date.now()));
			}, DEBOUNCE_MS);
		});
	}

	async reload(quiet = false) {
		const slug = this.slug;
		if (!slug) return;
		const seq = ++this.#seq;
		if (quiet && this.set) this.refreshing = true;
		else {
			this.phase = 'loading';
			this.error = null;
		}
		try {
			const set = await fetchQuestions(slug);
			if (seq !== this.#seq) return;
			this.set = set;
			this.phase = 'ready';
			this.error = null;
			this.#onLoaded?.(set);
		} catch (err) {
			if (seq !== this.#seq) return;
			this.error = errorText(err, 'Open questions failed to load.');
			if (!quiet || !this.set) this.phase = 'failed';
		} finally {
			if (seq === this.#seq) this.refreshing = false;
		}
	}

	#stop() {
		this.#seq += 1;
		this.#unsub?.();
		this.#unsub = null;
		if (this.#timer !== null) clearTimeout(this.#timer);
		this.#timer = null;
		this.refreshing = false;
	}

	dispose() {
		this.#stop();
	}
}
