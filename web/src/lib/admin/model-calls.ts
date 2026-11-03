// Per-attempt Meta model telemetry (SA-22 `model_call` rows):
// `GET /apostille/api/v1/admin/model-calls?pipeline=&since=`, shaped
// like docs/contracts/admin-model-calls.json.
import { getJson } from './admin-api.js';

export interface ModelCall {
	id: number;
	at: number;
	model: string;
	pipeline: string;
	run_id: number | null;
	latency_ms: number | null;
	ttft_ms: number | null;
	input_tokens: number;
	output_tokens: number;
	cached_tokens: number;
	reasoning_tokens: number;
	outcome: string;
	attempt: number;
}

export interface CallSummary {
	since: number;
	calls: number;
	p50_ms: number | null;
	p95_ms: number | null;
	input_tokens: number;
	output_tokens: number;
	cached_tokens: number;
	reasoning_tokens: number;
}

export const PIPELINES = ['draft', 'factcheck', 'offices', 'plan', 'forms', 'triage', 'precheck', 'voice'];

const n = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
const maybe = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Loose = any;

function callOf(c: Loose): ModelCall {
	return {
		id: c.id,
		at: n(c.at),
		model: c.model,
		pipeline: typeof c.pipeline === 'string' ? c.pipeline : '',
		run_id: maybe(c.run_id),
		latency_ms: maybe(c.latency_ms),
		ttft_ms: maybe(c.ttft_ms),
		input_tokens: n(c.input_tokens),
		output_tokens: n(c.output_tokens),
		cached_tokens: n(c.cached_tokens),
		reasoning_tokens: n(c.reasoning_tokens),
		outcome: typeof c.outcome === 'string' ? c.outcome : 'ok',
		attempt: n(c.attempt) || 1
	};
}

const rows = (body: Loose): ModelCall[] =>
	(Array.isArray(body?.calls) ? body.calls : [])
		.filter((c: Loose) => c && typeof c.id === 'number' && typeof c.model === 'string')
		.map(callOf);

/**
 * Every attempt one run made (compose: Spark, then Wonder), oldest
 * first. `since=0` so a run reopened from a link still finds its rows.
 */
export async function fetchRunCalls(runId: number): Promise<ModelCall[]> {
	const body = await getJson<Loose>(`/admin/model-calls?run_id=${runId}&since=0&limit=50`, 'Run calls');
	return rows(body)
		.filter((c) => c.run_id === runId)
		.sort((a, b) => a.at - b.at || a.id - b.id);
}

export async function fetchModelCalls(
	pipeline: string | null
): Promise<{ calls: ModelCall[]; summary: CallSummary | null }> {
	const q = pipeline ? `?pipeline=${encodeURIComponent(pipeline)}` : '';
	const body = await getJson<Loose>(`/admin/model-calls${q}`, 'Model calls');
	const calls: ModelCall[] = rows(body).sort((a: ModelCall, b: ModelCall) => b.at - a.at);
	const s = body?.summary;
	const summary: CallSummary | null =
		s && typeof s === 'object'
			? {
					since: n(s.since),
					calls: n(s.calls),
					p50_ms: maybe(s.p50_ms),
					p95_ms: maybe(s.p95_ms),
					input_tokens: n(s.input_tokens),
					output_tokens: n(s.output_tokens),
					cached_tokens: n(s.cached_tokens),
					reasoning_tokens: n(s.reasoning_tokens)
				}
			: null;
	return { calls, summary };
}
