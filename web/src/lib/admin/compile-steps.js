// Compile run steps for the dashboard card. The server
// (crates/web/src/compile.rs) walks one authority through five steps
// and reports each on the staff-only `run:{id}` topic as
// `compile.step {step, status, counts?, latency_ms?, reason?, summary?,
// detail?}`, then `compile.done {summary}` or `compile.failed
// {summary: {error}}`. These pure helpers fold those events into one
// step list, rebuild it from a stored run summary (a reopened or
// reconnected run), and format counts, seeds and the runs list.
// No DOM, no fetch. Admin-only.

/**
 * @typedef {'pending' | 'running' | 'done' | 'skipped' | 'stopped' | 'failed'} CompileStepStatus
 * @typedef {'capture' | 'extract' | 'cross_check' | 'offices' | 'compose'} CompileStepId
 * @typedef {{ id: CompileStepId, label: string, hint: string }} CompileStepDef
 * @typedef {{
 *   status: CompileStepStatus,
 *   counts: Record<string, number> | null,
 *   latency_ms: number | null,
 *   reason: string | null,
 *   detail: string | null,
 *   summary: Record<string, unknown> | null
 * }} CompileStepState
 * @typedef {Record<CompileStepId, CompileStepState>} CompileSteps
 * @typedef {{ key: string, text: string, warn: boolean }} CountChip
 * @typedef {{
 *   run_id: number,
 *   slug: string,
 *   status: string,
 *   started_at: number,
 *   finished_at: number | null,
 *   summary: Record<string, unknown>
 * }} CompileRunRow
 * @typedef {{ runs: CompileRunRow[], today: number, daily_runs: number }} CompileRuns
 * @typedef {'unknown' | 'conflict' | 'seeds' | 'cap' | 'error'} StartFailKind
 */

/** Server order (compile.rs `run_blocking`). @type {CompileStepDef[]} */
export const COMPILE_STEPS = [
	{ id: 'capture', label: 'Capture official pages', hint: 'Watched official pages plus extra URLs, up to 8' },
	{ id: 'extract', label: 'Draft facts (Muse Spark)', hint: 'New and never-drafted captures, up to 6' },
	{ id: 'cross_check', label: 'Cross-check (Muse Wonder)', hint: 'Every capture that drafted a proposal' },
	{ id: 'offices', label: 'Structure offices (Muse Spark)', hint: 'Address, hours and mail facts into office cards' },
	{
		id: 'compose',
		label: 'Compose page plan (Muse Spark + Wonder)',
		hint: 'Visitor plan from accepted facts, checked block by block'
	}
];

/** At most this many extra URLs per run (compile.rs MAX_SEEDS). */
export const MAX_SEEDS = 5;
const MAX_URL_LEN = 2048;

const STATUSES = new Set(['running', 'done', 'skipped', 'stopped', 'failed']);
const IDS = new Set(COMPILE_STEPS.map((s) => s.id));

/** @param {unknown} v */
const isNum = (v) => typeof v === 'number' && Number.isFinite(v);
/** @param {unknown} v @returns {v is Record<string, unknown>} */
const isObj = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
/** @param {unknown} v */
const str = (v) => (typeof v === 'string' && v ? v : null);

/** @param {unknown} v @returns {Record<string, number> | null} */
function numbers(v) {
	if (!isObj(v)) return null;
	/** @type {Record<string, number>} */
	const out = {};
	for (const [k, n] of Object.entries(v)) if (isNum(n)) out[k] = /** @type {number} */ (n);
	return Object.keys(out).length ? out : null;
}

/** @returns {CompileStepState} */
function blank() {
	return { status: 'pending', counts: null, latency_ms: null, reason: null, detail: null, summary: null };
}

/** Every step pending. @returns {CompileSteps} */
export function initialSteps() {
	return /** @type {CompileSteps} */ (Object.fromEntries(COMPILE_STEPS.map((s) => [s.id, blank()])));
}

/**
 * Fold one `compile.step` event into the list (returns a new object;
 * unknown steps or statuses change nothing). A later report of the
 * same step keeps what an earlier one said unless it says more.
 * @param {CompileSteps} steps
 * @param {Record<string, unknown>} msg
 * @returns {CompileSteps}
 */
export function applyStep(steps, msg) {
	const id = /** @type {CompileStepId} */ (msg?.step);
	const status = msg?.status;
	if (typeof id !== 'string' || !IDS.has(id) || typeof status !== 'string' || !STATUSES.has(status)) {
		return steps;
	}
	const prev = steps[id];
	/** @type {CompileStepState} */
	const next = {
		status: /** @type {CompileStepStatus} */ (status),
		counts: numbers(msg.counts) ?? prev.counts,
		latency_ms: isNum(msg.latency_ms) ? /** @type {number} */ (msg.latency_ms) : prev.latency_ms,
		reason: str(msg.reason) ?? (status === 'skipped' || status === 'stopped' ? prev.reason : null),
		// A sub-step string only means something while the step runs.
		detail: status === 'running' ? (str(msg.detail) ?? prev.detail) : null,
		summary: isObj(msg.summary) ? msg.summary : prev.summary
	};
	return { ...steps, [id]: next };
}

/**
 * The step list a stored run summary implies (compile.rs returns
 * `{capture, extract, cross_check, offices: {status, summary|reason},
 * plan: {status, summary|reason}, stopped_by_cap}`).
 * @param {unknown} summary
 * @returns {CompileSteps}
 */
export function stepsFromSummary(summary) {
	const steps = initialSteps();
	if (!isObj(summary)) return steps;
	const capped = summary.stopped_by_cap === true;
	const capture = numbers(summary.capture);
	if (capture) steps.capture = { ...blank(), status: 'done', counts: capture };
	const extract = numbers(summary.extract);
	if (extract) {
		steps.extract =
			(extract.captures ?? 0) > 0
				? { ...blank(), status: 'done', counts: extract }
				: { ...blank(), status: 'skipped', reason: 'no new or undrafted capture' };
	}
	const checks = numbers(summary.cross_check);
	if (checks) {
		steps.cross_check =
			'checked' in checks
				? { ...blank(), status: 'done', counts: checks }
				: {
						...blank(),
						status: 'skipped',
						reason: capped ? 'daily model-call cap reached' : 'no new proposals'
					};
	}
	for (const [id, key] of /** @type {const} */ ([
		['offices', 'offices'],
		['compose', 'plan']
	])) {
		const part = summary[key];
		if (!isObj(part)) continue;
		const status = part.status === 'done' ? 'done' : part.status === 'skipped' ? 'skipped' : null;
		if (!status) continue;
		const inner = isObj(part.summary) ? part.summary : null;
		steps[id] = {
			...blank(),
			status,
			summary: inner,
			// Model time of the composer (the run stores no step clock).
			latency_ms: inner && isNum(inner.latency_ms) ? /** @type {number} */ (inner.latency_ms) : null,
			reason: str(part.reason) ?? (status === 'skipped' && capped ? 'daily model-call cap reached' : null)
		};
	}
	return steps;
}

/**
 * Close a finished run: live reports win, the stored summary fills
 * what the socket missed, and a step nobody reported did not run.
 * @param {CompileSteps} steps
 * @param {unknown} summary
 * @returns {CompileSteps}
 */
export function settleDone(steps, summary) {
	const stored = stepsFromSummary(summary);
	/** @type {Partial<CompileSteps>} */
	const out = {};
	for (const { id } of COMPILE_STEPS) {
		const live = steps[id];
		const s = stored[id];
		/** @type {CompileStepState} */
		let next;
		if (live.status === 'pending' || live.status === 'running') {
			next = { ...s, latency_ms: s.latency_ms ?? live.latency_ms, counts: s.counts ?? live.counts };
		} else {
			next = { ...live, counts: live.counts ?? s.counts, summary: live.summary ?? s.summary };
		}
		if (next.status === 'pending' || next.status === 'running') next = { ...next, status: 'skipped', detail: null };
		out[id] = next;
	}
	return /** @type {CompileSteps} */ (out);
}

/**
 * A failed run: the step that was running failed, later ones never ran.
 * @param {CompileSteps} steps
 * @returns {CompileSteps}
 */
export function settleFailed(steps) {
	let failedOne = false;
	/** @type {Partial<CompileSteps>} */
	const out = {};
	for (const { id } of COMPILE_STEPS) {
		const s = steps[id];
		if (s.status === 'running') {
			out[id] = { ...s, status: 'failed', detail: null };
			failedOne = true;
		} else if (s.status === 'pending') {
			// Nothing was running (a failure before the first step):
			// the first unreported step is where it stopped.
			out[id] = failedOne ? { ...s, status: 'skipped' } : { ...s, status: 'failed' };
			failedOne = true;
		} else {
			out[id] = s;
		}
	}
	return /** @type {CompileSteps} */ (out);
}

/** Index of the step to point `aria-current` at, or -1. @param {CompileSteps} steps */
export function activeIndex(steps) {
	return COMPILE_STEPS.findIndex((s) => steps[s.id].status === 'running');
}

/** @param {number} n @param {string} one @param {string} many */
const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;

/**
 * Small count chips for one step, most telling first; zero counts are
 * left out except the step's headline number.
 * @param {CompileStepId} id
 * @param {CompileStepState} state
 * @returns {CountChip[]}
 */
export function countChips(id, state) {
	const c = state.counts ?? {};
	/** @type {CountChip[]} */
	const out = [];
	/** @param {string} key @param {string} text @param {boolean} [warn] */
	const add = (key, text, warn = false) => out.push({ key, text, warn });
	/** @param {string} key */
	const has = (key) => isNum(c[key]) && c[key] > 0;
	switch (id) {
		case 'capture':
			if (isNum(c.pages)) add('pages', plural(c.pages, 'page', 'pages'));
			if (has('created')) add('created', `${c.created} new`);
			if (has('duplicate')) add('duplicate', `${c.duplicate} unchanged`);
			if (has('skipped')) add('skipped', `${c.skipped} skipped`);
			if (has('failed')) add('failed', `${c.failed} failed`, true);
			if (has('dropped')) add('dropped', `${c.dropped} over cap`, true);
			break;
		case 'extract':
			if (isNum(c.captures)) add('captures', plural(c.captures, 'capture', 'captures'));
			if (isNum(c.proposed)) add('proposed', `${c.proposed} proposed`);
			if (has('rejected')) add('rejected', `${c.rejected} rejected`);
			if (has('no_text')) add('no_text', `${c.no_text} no text`);
			if (has('failed')) add('failed', `${c.failed} failed`, true);
			if (has('waiting')) add('waiting', `${c.waiting} waiting`);
			break;
		case 'cross_check':
			if (isNum(c.captures)) add('captures', plural(c.captures, 'capture', 'captures'));
			if (isNum(c.checked)) add('checked', `${c.checked} checked`);
			if (has('failed')) add('failed', `${c.failed} failed`, true);
			break;
		case 'offices': {
			const s = state.summary;
			if (!s) break;
			const proposed = Array.isArray(s.proposed) ? s.proposed.length : null;
			if (proposed !== null) add('proposed', `${plural(proposed, 'office', 'offices')} proposed`);
			if (isNum(s.dropped) && /** @type {number} */ (s.dropped) > 0) add('dropped', `${s.dropped} dropped`);
			break;
		}
		case 'compose': {
			const s = state.summary;
			if (!s) break;
			if (isNum(s.version)) add('version', `plan v${s.version}`);
			if (isNum(s.blocks)) add('blocks', plural(/** @type {number} */ (s.blocks), 'block', 'blocks'));
			if (isNum(s.dropped) && /** @type {number} */ (s.dropped) > 0) add('dropped', `${s.dropped} dropped`);
			if (s.passed === false) add('passed', 'needs staff', true);
			break;
		}
	}
	return out;
}

/** @type {Record<string, string>} */
const DETAILS = {
	guards: 'Guards checking address, hours and mail',
	validate: 'Validator checking fact ids and numbers',
	entail: 'Muse Wonder checking each block'
};

/** Words for a running step's sub-step string. @param {string | null} detail */
export function detailText(detail) {
	if (!detail) return null;
	return DETAILS[detail] ?? detail;
}

/** Server reason → sentence case for display. @param {string | null | undefined} text */
export function sentence(text) {
	if (typeof text !== 'string' || !text.trim()) return null;
	const t = text.trim();
	return t.charAt(0).toUpperCase() + t.slice(1);
}

/**
 * One line for a finished run: "4 pages · 7 facts proposed · 1 office · plan v5".
 * @param {unknown} summary
 * @returns {string}
 */
export function summaryLine(summary) {
	if (!isObj(summary)) return '';
	if (str(summary.error)) return /** @type {string} */ (sentence(/** @type {string} */ (summary.error)));
	/** @type {string[]} */
	const parts = [];
	const capture = numbers(summary.capture);
	if (capture && isNum(capture.pages)) parts.push(plural(capture.pages, 'page', 'pages'));
	const extract = numbers(summary.extract);
	if (extract && isNum(extract.proposed)) {
		parts.push(extract.proposed > 0 ? `${plural(extract.proposed, 'fact', 'facts')} proposed` : 'no new facts');
	}
	const offices = isObj(summary.offices) ? summary.offices : null;
	if (offices?.status === 'done' && isObj(offices.summary) && Array.isArray(offices.summary.proposed)) {
		parts.push(plural(offices.summary.proposed.length, 'office', 'offices'));
	}
	const plan = isObj(summary.plan) ? summary.plan : null;
	if (plan?.status === 'done' && isObj(plan.summary) && isNum(plan.summary.version)) {
		parts.push(`plan v${plan.summary.version}`);
	} else if (plan?.status === 'skipped') {
		parts.push('no plan');
	}
	if (summary.stopped_by_cap === true) parts.push('stopped at the model-call cap');
	return parts.join(' · ');
}

/**
 * Extra official URLs, one per line: https only, at most MAX_SEEDS,
 * duplicates folded. `errors` is empty when the list can be sent.
 * @param {string} text
 * @returns {{ urls: string[], errors: string[] }}
 */
export function parseSeeds(text) {
	/** @type {string[]} */
	const urls = [];
	/** @type {string[]} */
	const errors = [];
	const lines = String(text ?? '').split(/\r?\n/);
	lines.forEach((raw, i) => {
		const line = raw.trim();
		if (!line) return;
		let ok = false;
		try {
			const u = new URL(line);
			ok = u.protocol === 'https:' && u.hostname.length > 0 && line.length <= MAX_URL_LEN;
		} catch {
			ok = false;
		}
		if (!ok) {
			errors.push(`Line ${i + 1} is not an https:// URL.`);
			return;
		}
		if (!urls.includes(line)) urls.push(line);
	});
	if (urls.length > MAX_SEEDS) errors.push(`At most ${MAX_SEEDS} URLs per run (this list has ${urls.length}).`);
	return { urls, errors };
}

/**
 * `GET /admin/compile` → typed rows (bad rows dropped).
 * @param {unknown} body
 * @returns {CompileRuns}
 */
export function normalizeRuns(body) {
	const b = isObj(body) ? body : {};
	const rows = Array.isArray(b.runs) ? b.runs : [];
	/** @type {CompileRunRow[]} */
	const runs = [];
	for (const r of rows) {
		if (!isObj(r) || !isNum(r.run_id) || !str(r.slug)) continue;
		runs.push({
			run_id: /** @type {number} */ (r.run_id),
			slug: /** @type {string} */ (r.slug),
			status: str(r.status) ?? 'running',
			started_at: isNum(r.started_at) ? /** @type {number} */ (r.started_at) : 0,
			finished_at: isNum(r.finished_at) ? /** @type {number} */ (r.finished_at) : null,
			summary: isObj(r.summary) ? r.summary : {}
		});
	}
	return {
		runs,
		today: isNum(b.today) ? /** @type {number} */ (b.today) : 0,
		daily_runs: isNum(b.daily_runs) ? /** @type {number} */ (b.daily_runs) : 0
	};
}

/** What a failed start offers next, by HTTP status. @param {number} status @returns {StartFailKind} */
export function startFailKind(status) {
	if (status === 404) return 'unknown';
	if (status === 409) return 'conflict';
	if (status === 422) return 'seeds';
	if (status === 429) return 'cap';
	return 'error';
}

/** 75_000 → "1:15", 3_725_000 → "1:02:05", null → "". @param {number | null | undefined} ms */
export function fmtElapsed(ms) {
	if (!isNum(ms) || /** @type {number} */ (ms) < 0) return '';
	const total = Math.floor(/** @type {number} */ (ms) / 1000);
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = String(total % 60).padStart(2, '0');
	return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}
