// One staff Compile run (dashboard card): POST /admin/compile answers
// 202 {run_id, topic}; the server then walks the authority through
// capture → extract → cross-check → offices → plan and reports every
// step on the staff-only `run:{id}` topic (compile.rs). No polling:
// the bus hello (sent on every connect) triggers one read of
// GET /admin/compile/{id}, so a run that ended before the socket
// opened still lands, and a reopened run (`?compile=`) rebuilds its
// steps from the stored summary.
import { ApiError, errorText, getJson, postJson } from './admin-api.js';
import { subscribe, type BusMessage } from '../live.svelte.js';
import {
	applyStep,
	initialSteps,
	normalizeRuns,
	sentence,
	settleDone,
	settleFailed,
	startFailKind,
	type CompileRunRow,
	type CompileRuns,
	type CompileSteps,
	type StartFailKind
} from './compile-steps.js';

export type CompilePhase = 'idle' | 'starting' | 'running' | 'done' | 'failed';
/** A start refusal (by HTTP status) or a run that failed midway. */
export type CompileFailKind = StartFailKind | 'run';

// Each model call has a 300 s budget; a quiet socket past that plus
// margin earns a "Check the run" button.
const STALL_MS = 330_000;

export function fetchCompileRuns(): Promise<CompileRuns> {
	return getJson<unknown>('/admin/compile', 'Compile runs').then(normalizeRuns);
}

async function fetchCompileRun(runId: number): Promise<CompileRunRow | null> {
	const body = await getJson<unknown>(`/admin/compile/${runId}`, 'Compile run');
	return normalizeRuns({ runs: [body] }).runs[0] ?? null;
}

export class CompileRun {
	phase = $state<CompilePhase>('idle');
	slug = $state('');
	runId = $state<number | null>(null);
	steps = $state<CompileSteps>(initialSteps());
	summary = $state<Record<string, unknown> | null>(null);
	error = $state<string | null>(null);
	failKind = $state<CompileFailKind | null>(null);
	/** 409: the run already going for this authority. */
	conflictRunId = $state<number | null>(null);
	stalled = $state(false);
	checking = $state(false);
	/** Opened from a link or the runs list, not started here. */
	reopened = $state(false);
	now = $state(0);

	/** Client-clock start (ms) of a run started here; 0 otherwise. */
	#localStart = $state(0);
	/** Server start (unix s) of a reopened run. */
	#serverStart = $state(0);
	/** Server clock minus this browser's, from event `at` stamps. */
	#skewMs = $state(0);
	/** Fixed run length once it ended (ms). */
	#durationMs = $state<number | null>(null);

	#unsub: (() => void) | null = null;
	#tick: ReturnType<typeof setInterval> | null = null;
	#lastEvent = 0;
	#settled = false;
	/** Bumped on every reset: a start answered after one is dropped. */
	#gen = 0;
	#onEnd: ((run: CompileRun) => void) | undefined;

	constructor(onEnd?: (run: CompileRun) => void) {
		this.#onEnd = onEnd;
	}

	get busy(): boolean {
		return this.phase === 'starting' || this.phase === 'running';
	}

	/** Elapsed run time for the timer (null when the start is unknown). */
	get elapsedMs(): number | null {
		if (this.#durationMs !== null) return this.#durationMs;
		const start = this.#localStart || (this.#serverStart ? this.#serverStart * 1000 - this.#skewMs : 0);
		if (!start || !this.now) return null;
		return Math.max(0, this.now - start);
	}

	async start(slug: string, seeds: string[] = []): Promise<void> {
		// A run started here holds the card; a watched one gives way.
		if (!slug || this.phase === 'starting' || (this.busy && !this.reopened)) return;
		this.#reset();
		const gen = this.#gen;
		this.slug = slug;
		this.phase = 'starting';
		this.#localStart = Date.now();
		this.now = this.#localStart;
		try {
			const body = seeds.length ? { slug, seed_urls: seeds } : { slug };
			const out = await postJson<{ run_id?: unknown }>('/admin/compile', body, 'Compile');
			if (gen !== this.#gen) return;
			const runId = typeof out?.run_id === 'number' ? out.run_id : null;
			if (runId === null) throw new ApiError('The server started no run.', 0);
			this.#watch(runId);
			// The server's first act is the capture step; show it now
			// rather than wait for the socket to open.
			this.steps = applyStep(this.steps, { step: 'capture', status: 'running' });
		} catch (err) {
			if (gen !== this.#gen) return;
			const status = err instanceof ApiError ? err.status : 0;
			const body = err instanceof ApiError ? err.body : null;
			const kind = startFailKind(status);
			this.conflictRunId = kind === 'conflict' && typeof body?.run_id === 'number' ? body.run_id : null;
			const server = status === 401 || status === 403 ? null : sentence(body?.error as string | undefined);
			this.#settled = true;
			this.phase = 'failed';
			this.failKind = kind;
			this.error = server ? `${server.replace(/\.$/, '')}.` : errorText(err, 'Compile did not start.');
			this.#localStart = 0;
		}
	}

	/** Watch a run started elsewhere (a 409, a link, the runs list). */
	attach(runId: number, slug = ''): void {
		if (this.busy && this.runId === runId) return;
		this.#reset();
		this.slug = slug;
		this.reopened = true;
		this.#watch(runId);
		void this.#reconcile();
	}

	/** Staff-initiated re-read for a run that has gone quiet. */
	async check(): Promise<void> {
		if (this.phase !== 'running' || this.#settled || this.checking) return;
		this.checking = true;
		try {
			await this.#reconcile();
			this.#lastEvent = Date.now();
			this.stalled = false;
		} finally {
			this.checking = false;
		}
	}

	reset(): void {
		if (this.phase === 'starting' || (this.busy && !this.reopened)) return;
		this.#reset();
	}

	dispose(): void {
		this.#stop();
	}

	#reset() {
		this.#stop();
		this.#gen += 1;
		this.#settled = false;
		this.phase = 'idle';
		this.slug = '';
		this.runId = null;
		this.steps = initialSteps();
		this.summary = null;
		this.error = null;
		this.failKind = null;
		this.conflictRunId = null;
		this.stalled = false;
		this.checking = false;
		this.reopened = false;
		this.now = 0;
		this.#localStart = 0;
		this.#serverStart = 0;
		this.#skewMs = 0;
		this.#durationMs = null;
	}

	#stop() {
		this.#unsub?.();
		this.#unsub = null;
		if (this.#tick !== null) clearInterval(this.#tick);
		this.#tick = null;
	}

	#watch(runId: number) {
		this.runId = runId;
		this.phase = 'running';
		this.now = Date.now();
		this.#lastEvent = this.now;
		this.#tick = setInterval(() => {
			this.now = Date.now();
			if (this.phase === 'running' && this.now - this.#lastEvent > STALL_MS) this.stalled = true;
		}, 1000);
		this.#unsub = subscribe(`run:${runId}`, (msg) => this.#onMessage(msg));
	}

	#onMessage(msg: BusMessage) {
		if (this.phase !== 'running' || this.#settled) return;
		if (msg?.type === 'subscribed') {
			// Every (re)connect: one read catches a result published
			// before this socket existed.
			void this.#reconcile();
			return;
		}
		if (typeof msg?.run_id === 'number' && msg.run_id !== this.runId) return;
		if (typeof msg.at === 'number' && Number.isFinite(msg.at)) this.#skewMs = msg.at * 1000 - Date.now();
		this.#lastEvent = Date.now();
		this.now = this.#lastEvent;
		this.stalled = false;
		if (typeof msg.slug === 'string' && msg.slug && !this.slug) this.slug = msg.slug;
		switch (msg.type) {
			case 'compile.step':
				this.steps = applyStep(this.steps, msg);
				return;
			case 'compile.done':
				this.#finish(msg.summary);
				return;
			case 'compile.failed': {
				const s = msg.summary as { error?: unknown } | undefined;
				this.#failRun(typeof s?.error === 'string' && s.error ? s.error : 'The run failed.');
			}
		}
	}

	/** Read the stored row; true when it showed the run finished. */
	async #reconcile(): Promise<boolean> {
		const runId = this.runId;
		if (runId === null || this.phase !== 'running' || this.#settled) return false;
		try {
			const row = await fetchCompileRun(runId);
			if (!row || this.runId !== runId || this.#settled) return false;
			if (!this.slug) this.slug = row.slug;
			if (row.started_at) this.#serverStart = row.started_at;
			if (row.status === 'done' || row.status === 'failed') {
				if (!this.#localStart && row.finished_at && row.started_at) {
					this.#durationMs = Math.max(0, (row.finished_at - row.started_at) * 1000);
				}
				if (row.status === 'done') this.#finish(row.summary);
				else this.#failRun(typeof row.summary.error === 'string' ? row.summary.error : 'The run failed.');
				return true;
			}
			return false;
		} catch (err) {
			// A stale link: no such run will ever report.
			if (err instanceof ApiError && err.status === 404 && this.runId === runId) {
				this.#failRun(`run ${runId} was not found`);
				return true;
			}
			// Otherwise the socket still carries the result; a failed
			// read changes nothing.
			return false;
		}
	}

	#end() {
		this.now = Date.now();
		if (this.#durationMs === null) this.#durationMs = this.elapsedMs;
		this.#stop();
		this.stalled = false;
	}

	#finish(summary: unknown) {
		if (this.phase !== 'running' || this.#settled) return;
		this.#settled = true;
		this.summary = summary && typeof summary === 'object' ? (summary as Record<string, unknown>) : {};
		this.steps = settleDone(this.steps, this.summary);
		this.#end();
		this.phase = 'done';
		this.#onEnd?.(this);
	}

	#failRun(message: string) {
		if (this.#settled) return;
		this.#settled = true;
		this.steps = settleFailed(this.steps);
		this.#end();
		this.phase = 'failed';
		this.failKind = 'run';
		this.error = `${sentence(message)?.replace(/\.$/, '') ?? 'The run failed'}.`;
		this.#onEnd?.(this);
	}
}
