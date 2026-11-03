// One staff compose run (MISSION WIN SA-26 W1e): Muse Spark builds a
// page plan (or structures offices) from accepted facts while staff
// watch. POST /admin/{plans|offices}/compose answers 202 {run_id,
// topic}; the run then reports on the staff-only `run:{id}` bus topic
// (`compose.started/done/failed` or `offices.started/done/failed`).
// No polling: the socket carries the result, and the bus hello (sent
// on every connect) triggers one reconcile read, so a run that ended
// before the socket opened still lands. A reopened link (`?run=`)
// reconstructs a finished run from the stored proposal.
import { ApiError, errorText, postJson } from './admin-api.js';
import { fetchOffices, fetchPlans } from './proposals.js';
import { fetchRunCalls, type ModelCall } from './model-calls.js';
import { subscribe, type BusMessage } from '../live.svelte.js';

export type ComposeKind = 'plans' | 'offices';
export type ComposePhase = 'idle' | 'starting' | 'running' | 'done' | 'failed';
export type StepState = 'pending' | 'active' | 'done' | 'failed' | 'skipped';
/** What the one next action on a failure should be. */
export type FailKind = 'cap' | 'unknown' | 'facts' | 'error';

export interface ComposeSummary {
	plan_id: number | null;
	version: number | null;
	passed: boolean | null;
	blocks: number | null;
	dropped: number;
	proposed: number[];
	model: string;
	latency_ms: number | null;
	input_tokens: number;
	output_tokens: number;
	cached_tokens: number;
	reasoning_tokens: number;
}

export interface StepDef {
	id: 'queued' | 'compose' | 'validate' | 'entail' | 'done';
	label: string;
	/** Label while the step runs (present tense). */
	active: string;
	/** What the step does, shown before it has a result. */
	hint: string;
}

export const SPARK = 'muse-spark-1.3';

const PLAN_STEPS: StepDef[] = [
	{ id: 'queued', label: 'Run started', active: 'Starting run', hint: 'Staff request accepted' },
	{ id: 'compose', label: 'Muse Spark composed', active: 'Muse Spark composing', hint: 'Plan from accepted facts' },
	{ id: 'validate', label: 'Validator', active: 'Validator checking', hint: 'Fact ids, numbers, hours, links' },
	{ id: 'entail', label: 'Muse Wonder entailment', active: 'Muse Wonder checking', hint: 'Each block against its facts' },
	{ id: 'done', label: 'Proposal ready', active: 'Storing proposal', hint: 'Waits for staff accept' }
];

const OFFICE_STEPS: StepDef[] = [
	{ id: 'queued', label: 'Run started', active: 'Starting run', hint: 'Staff request accepted' },
	{ id: 'compose', label: 'Muse Spark structured', active: 'Muse Spark structuring', hint: 'Offices from accepted facts' },
	{ id: 'validate', label: 'Guards', active: 'Guards checking', hint: 'Address, hours, PO box, timezone' },
	{ id: 'done', label: 'Proposals ready', active: 'Storing proposals', hint: 'Wait for staff accept' }
];

/** Server step names (a future `*.step {step}` event) → timeline index. */
const STEP_NAMES: Record<string, StepDef['id']> = {
	compose: 'compose',
	structure: 'compose',
	validate: 'validate',
	guards: 'validate',
	entail: 'entail',
	entailment: 'entail',
	store: 'done'
};

const SLOW_MS = 60_000;
// Model budget is 300 s per call; Spark + Wonder plus margin.
const STALL_MS = 330_000;
const STAGGER_MS = 180;

const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
const maybeNum = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);

function summaryFrom(msg: BusMessage): ComposeSummary {
	return {
		plan_id: maybeNum(msg.plan_id),
		version: maybeNum(msg.version),
		passed: typeof msg.passed === 'boolean' ? msg.passed : null,
		blocks: maybeNum(msg.blocks),
		dropped: num(msg.dropped),
		proposed: Array.isArray(msg.proposed) ? msg.proposed.filter((x): x is number => typeof x === 'number') : [],
		model: typeof msg.model === 'string' && msg.model ? msg.model : SPARK,
		latency_ms: maybeNum(msg.latency_ms),
		input_tokens: num(msg.input_tokens),
		output_tokens: num(msg.output_tokens),
		cached_tokens: num(msg.cached_tokens),
		reasoning_tokens: num(msg.reasoning_tokens)
	};
}

/** Precondition failures name accepted facts: send staff to the Facts tab. */
function failKindOf(error: string): FailKind {
	if (/accepted .*facts|needs at least/i.test(error)) return 'facts';
	if (/daily model-call cap|daily_call_cap/i.test(error)) return 'cap';
	if (/unknown authority/i.test(error)) return 'unknown';
	return 'error';
}

export class ComposeRun {
	readonly kind: ComposeKind;
	readonly steps: StepDef[];

	phase = $state<ComposePhase>('idle');
	slug = $state('');
	runId = $state<number | null>(null);
	model = $state(SPARK);
	/** Wall-clock start (ms); 0 when reopened from a link. */
	startedAt = $state(0);
	endedAt = $state(0);
	now = $state(0);
	stepAt = $state(0);
	error = $state<string | null>(null);
	failKind = $state<FailKind | null>(null);
	summary = $state<ComposeSummary | null>(null);
	calls = $state<ModelCall[]>([]);
	callsState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	slow = $state(false);
	stalled = $state(false);
	checking = $state(false);
	/** Reopened from `?run=`: the start time is unknown. */
	reopened = $state(false);

	#unsub: (() => void) | null = null;
	#tick: ReturnType<typeof setInterval> | null = null;
	#timers = new Set<ReturnType<typeof setTimeout>>();
	#onDone: ((s: ComposeSummary, run: ComposeRun) => void | Promise<void>) | undefined;
	#reduceMotion = false;
	/** Set once the run reached done/failed (also during the stagger). */
	#settled = false;

	constructor(
		kind: ComposeKind,
		onDone?: (s: ComposeSummary, run: ComposeRun) => void | Promise<void>
	) {
		this.kind = kind;
		this.steps = kind === 'plans' ? PLAN_STEPS : OFFICE_STEPS;
		this.#onDone = onDone;
		if (typeof window !== 'undefined') {
			this.#reduceMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
		}
	}

	get busy(): boolean {
		return this.phase === 'starting' || this.phase === 'running';
	}

	/** Elapsed ms for the live timer (null when the start is unknown). */
	get elapsedMs(): number | null {
		if (!this.startedAt) return null;
		return Math.max(0, (this.endedAt || this.now || this.startedAt) - this.startedAt);
	}

	stepState(i: number): StepState {
		if (this.phase === 'idle') return 'pending';
		if (this.phase === 'done') return 'done';
		if (this.phase === 'failed') {
			if (i < this.stepAt) return 'done';
			return i === this.stepAt ? 'failed' : 'skipped';
		}
		if (i < this.stepAt) return 'done';
		return i === this.stepAt ? 'active' : 'pending';
	}

	/** Model calls of this run whose model id starts with `prefix`. */
	callsBy(prefix: string): ModelCall[] {
		return this.calls.filter((c) => c.model.startsWith(prefix));
	}

	async start(slug: string): Promise<void> {
		if (this.busy || !slug) return;
		this.#reset();
		this.slug = slug;
		this.phase = 'starting';
		this.startedAt = Date.now();
		this.now = this.startedAt;
		try {
			const out = await postJson<{ run_id?: unknown }>(
				`/admin/${this.kind}/compose`,
				{ slug },
				this.kind === 'plans' ? 'Compose' : 'Structure offices'
			);
			const runId = typeof out?.run_id === 'number' ? out.run_id : null;
			if (runId === null) throw new ApiError('The server started no run.', 0);
			this.#watch(runId);
		} catch (err) {
			const status = err instanceof ApiError ? err.status : 0;
			const kind: FailKind = status === 429 ? 'cap' : status === 404 ? 'unknown' : 'error';
			this.#fail(kind, errorText(err, 'Compose failed to start.'));
		}
	}

	/** Reopen a run from a link: listen, and reconstruct if it finished. */
	attach(runId: number, slug = ''): void {
		if (this.busy) return;
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
			const found = await this.#reconcile();
			if (!found) void this.#loadCalls();
		} finally {
			this.checking = false;
		}
	}

	reset(): void {
		if (this.busy) return;
		this.#reset();
	}

	dispose(): void {
		this.#stop();
	}

	#reset() {
		this.#stop();
		this.#settled = false;
		this.phase = 'idle';
		this.runId = null;
		this.model = SPARK;
		this.startedAt = 0;
		this.endedAt = 0;
		this.now = 0;
		this.stepAt = 0;
		this.error = null;
		this.failKind = null;
		this.summary = null;
		this.calls = [];
		this.callsState = 'idle';
		this.slow = false;
		this.stalled = false;
		this.reopened = false;
	}

	#later(fn: () => void, ms: number) {
		const t = setTimeout(() => {
			this.#timers.delete(t);
			fn();
		}, ms);
		this.#timers.add(t);
	}

	#stop() {
		this.#unsub?.();
		this.#unsub = null;
		if (this.#tick !== null) clearInterval(this.#tick);
		this.#tick = null;
		for (const t of this.#timers) clearTimeout(t);
		this.#timers.clear();
	}

	#watch(runId: number) {
		this.runId = runId;
		this.phase = 'running';
		this.stepAt = 1;
		if (this.startedAt) {
			this.#tick = setInterval(() => (this.now = Date.now()), 100);
		}
		this.#unsub = subscribe(`run:${runId}`, (msg) => this.#onMessage(msg));
		this.#later(() => (this.slow = this.phase === 'running'), SLOW_MS);
		this.#later(() => (this.stalled = this.phase === 'running'), STALL_MS);
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
		const prefix = this.kind === 'plans' ? 'compose' : 'offices';
		const type = typeof msg?.type === 'string' ? msg.type : '';
		if (typeof msg.slug === 'string' && msg.slug && !this.slug) this.slug = msg.slug;
		if (type === `${prefix}.started`) {
			if (typeof msg.model === 'string' && msg.model) this.model = msg.model;
			return;
		}
		if (type === `${prefix}.step`) {
			const id = STEP_NAMES[typeof msg.step === 'string' ? msg.step : ''];
			const at = this.steps.findIndex((s) => s.id === id);
			if (at > this.stepAt) this.stepAt = at;
			return;
		}
		if (type === `${prefix}.done`) {
			this.#finish(summaryFrom(msg));
			return;
		}
		if (type === `${prefix}.failed`) {
			const error = typeof msg.error === 'string' && msg.error ? msg.error : 'The run failed.';
			this.#fail(failKindOf(error), error);
		}
	}

	/** Look for this run's stored proposal; true when it finished it. */
	async #reconcile(): Promise<boolean> {
		const runId = this.runId;
		if (runId === null || this.phase !== 'running' || this.#settled) return false;
		try {
			if (this.kind === 'plans') {
				const rows = await fetchPlans('all', this.slug || undefined);
				const row = rows.find((p) => p.run_id === runId);
				if (!row || this.runId !== runId) return false;
				if (!this.slug) this.slug = row.authority_slug;
				this.#finish({
					plan_id: row.id,
					version: row.version,
					passed: row.validator.passed,
					blocks: row.plan.blocks.length,
					dropped: row.validator.dropped_blocks.length,
					proposed: [],
					model: SPARK,
					latency_ms: row.spend?.latency_ms ?? null,
					input_tokens: row.spend?.input_tokens ?? 0,
					output_tokens: row.spend?.output_tokens ?? 0,
					cached_tokens: row.spend?.cached_tokens ?? 0,
					reasoning_tokens: row.spend?.reasoning_tokens ?? 0
				});
				return true;
			}
			const rows = await fetchOffices('all', this.slug || undefined);
			const mine = rows.filter((o) => o.run_id === runId);
			if (mine.length === 0 || this.runId !== runId) return false;
			if (!this.slug) this.slug = mine[0].authority_slug;
			// Offices store no spend row: the per-call rows carry it.
			const calls = await fetchRunCalls(runId).catch(() => [] as ModelCall[]);
			const sum = (k: 'input_tokens' | 'output_tokens' | 'cached_tokens' | 'reasoning_tokens') =>
				calls.reduce((n, c) => n + c[k], 0);
			this.#finish({
				plan_id: null,
				version: null,
				passed: null,
				blocks: null,
				dropped: 0,
				proposed: mine.map((o) => o.id),
				model: calls[0]?.model ?? SPARK,
				latency_ms: calls.length ? calls.reduce((n, c) => n + (c.latency_ms ?? 0), 0) : null,
				input_tokens: sum('input_tokens'),
				output_tokens: sum('output_tokens'),
				cached_tokens: sum('cached_tokens'),
				reasoning_tokens: sum('reasoning_tokens')
			});
			return true;
		} catch {
			// The socket still carries the result; a failed read changes nothing.
			return false;
		}
	}

	#finish(summary: ComposeSummary) {
		if (this.phase !== 'running' || this.#settled) return;
		this.#settled = true;
		this.#stop();
		this.summary = summary;
		this.model = summary.model || this.model;
		this.endedAt = this.startedAt ? Date.now() : 0;
		this.slow = false;
		this.stalled = false;
		const last = this.steps.length - 1;
		const advance = () => {
			if (this.phase !== 'running') return;
			if (this.stepAt < last) {
				this.stepAt += 1;
				this.#later(advance, this.#reduceMotion ? 0 : STAGGER_MS);
				return;
			}
			this.phase = 'done';
			void this.#loadCalls();
			void this.#onDone?.(summary, this);
		};
		advance();
	}

	#fail(kind: FailKind, message: string) {
		if (this.#settled) return;
		this.#settled = true;
		this.#stop();
		this.phase = 'failed';
		this.failKind = kind;
		this.error = message;
		this.endedAt = this.startedAt ? Date.now() : 0;
		this.slow = false;
		this.stalled = false;
		if (this.runId !== null) void this.#loadCalls();
	}

	async #loadCalls() {
		const runId = this.runId;
		if (runId === null) return;
		this.callsState = 'loading';
		try {
			const calls = await fetchRunCalls(runId);
			if (this.runId !== runId) return;
			this.calls = calls;
			this.callsState = 'ready';
		} catch {
			if (this.runId === runId) this.callsState = 'failed';
		}
	}
}
