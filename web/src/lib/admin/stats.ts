// Admin dashboard data: `GET /apostille/api/v1/admin/stats`
// (contract seed docs/lanes/contracts/admin-stats.json) plus the
// staff-only `ops` WS topic (ids only; the page re-reads the API).
import { modelName } from './format.js';

export interface ModelStat {
	model: string;
	calls: number;
	p50_ms: number;
	p95_ms: number;
	input_tokens: number;
	output_tokens: number;
	cached_tokens: number;
	reasoning_tokens: number;
	errors: number;
}

export interface Activity {
	at: number;
	kind: string;
	text: string;
	href: string | null;
	live?: boolean;
}

export interface Point {
	t: number;
	n: number;
	cents?: number;
}

export interface AdminStats {
	generated_at: number;
	kpis: {
		authorities: number;
		facts_accepted: number;
		proposals_open: number;
		plans_accepted: number;
		offices_accepted: number;
		forms_active: number;
		orders_today: number;
		paid_cents_7d: number;
		needs_staff: number;
	};
	meta: {
		calls_24h: number;
		errors_24h: number;
		cache_share: number;
		by_model: ModelStat[];
		by_pipeline: { pipeline: string; calls: number }[];
		a5: {
			quote_found_rate: number;
			value_in_quote_rate: number;
			topic_accuracy: number;
			cases: number;
			graded_at: number;
		} | null;
	};
	series: { calls_by_hour: Point[]; orders_by_day: Point[] };
	activity: Activity[];
}

const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
const str = (v: unknown): string => (typeof v === 'string' ? v : '');
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Loose = any;

function points(raw: Loose): Point[] {
	if (!Array.isArray(raw)) return [];
	return raw
		.filter((p: Loose) => p && typeof p.t === 'number')
		.map((p: Loose) => ({ t: p.t, n: num(p.n), cents: num(p.cents) }))
		.sort((a: Point, b: Point) => a.t - b.t);
}

/** Defensive read of the stats body: missing fields become zeros. */
export function normalizeStats(raw: Loose): AdminStats {
	const k = raw?.kpis ?? {};
	const m = raw?.meta ?? {};
	const a5 = m?.a5;
	return {
		generated_at: num(raw?.generated_at),
		kpis: {
			authorities: num(k.authorities),
			facts_accepted: num(k.facts_accepted),
			proposals_open: num(k.proposals_open),
			plans_accepted: num(k.plans_accepted),
			offices_accepted: num(k.offices_accepted),
			forms_active: num(k.forms_active),
			orders_today: num(k.orders_today),
			paid_cents_7d: num(k.paid_cents_7d),
			needs_staff: num(k.needs_staff)
		},
		meta: {
			calls_24h: num(m.calls_24h),
			errors_24h: num(m.errors_24h),
			cache_share: num(m.cache_share),
			by_model: Array.isArray(m.by_model)
				? m.by_model
						.filter((r: Loose) => r && typeof r.model === 'string')
						.map((r: Loose) => ({
							model: r.model,
							calls: num(r.calls),
							p50_ms: num(r.p50_ms),
							p95_ms: num(r.p95_ms),
							input_tokens: num(r.input_tokens),
							output_tokens: num(r.output_tokens),
							cached_tokens: num(r.cached_tokens),
							reasoning_tokens: num(r.reasoning_tokens),
							errors: num(r.errors)
						}))
				: [],
			by_pipeline: Array.isArray(m.by_pipeline)
				? m.by_pipeline
						.filter((r: Loose) => r && typeof r.pipeline === 'string')
						.map((r: Loose) => ({ pipeline: r.pipeline, calls: num(r.calls) }))
						.sort((a: { calls: number }, b: { calls: number }) => b.calls - a.calls)
				: [],
			a5:
				a5 && typeof a5 === 'object'
					? {
							quote_found_rate: num(a5.quote_found_rate),
							value_in_quote_rate: num(a5.value_in_quote_rate),
							topic_accuracy: num(a5.topic_accuracy),
							cases: num(a5.cases),
							graded_at: num(a5.graded_at)
						}
					: null
		},
		series: {
			calls_by_hour: points(raw?.series?.calls_by_hour),
			orders_by_day: points(raw?.series?.orders_by_day)
		},
		activity: Array.isArray(raw?.activity)
			? raw.activity
					.filter((a: Loose) => a && typeof a.at === 'number' && typeof a.text === 'string')
					.map((a: Loose) => ({
						at: a.at,
						kind: str(a.kind),
						text: a.text,
						href: typeof a.href === 'string' && a.href.startsWith('/') ? a.href : null
					}))
			: []
	};
}

/** True when the console has nothing to show yet (fresh install). */
export function isEmpty(s: AdminStats): boolean {
	const k = s.kpis;
	return (
		k.authorities === 0 &&
		k.facts_accepted === 0 &&
		k.proposals_open === 0 &&
		s.meta.calls_24h === 0 &&
		s.activity.length === 0
	);
}

const OUTCOME: Record<string, string> = {
	timeout: 'timed out',
	refusal: 'was refused',
	length: 'hit the length cap',
	http_4xx: 'got a 4xx',
	http_5xx: 'got a 5xx',
	parse_error: 'returned unparseable JSON'
};

/**
 * One feed row from an `ops` bus event (ids only). Unknown kinds and
 * the server hello return null.
 */
export function activityFromOps(msg: Loose): Activity | null {
	const type = str(msg?.type);
	const at = typeof msg?.at === 'number' ? msg.at : Math.floor(Date.now() / 1000);
	const order = str(msg?.order_id);
	switch (type) {
		case 'order.paid':
			return order ? { at, kind: type, text: `Order ${order} paid`, href: `/orders/${order}`, live: true } : null;
		case 'order.needs_staff':
			return order
				? { at, kind: type, text: `Order ${order} needs staff`, href: `/orders/${order}`, live: true }
				: null;
		case 'kit.packed':
			return order
				? { at, kind: type, text: `Kit packed for order ${order}`, href: `/orders/${order}`, live: true }
				: null;
		case 'compile.done': {
			const run = num(msg?.run_id);
			return run
				? { at, kind: type, text: `Compile run ${run} finished`, href: `/compile/${run}`, live: true }
				: null;
		}
		case 'plan_accepted': {
			const version = num(msg?.version);
			return {
				at,
				kind: type,
				text: version ? `Page plan v${version} accepted` : 'Page plan accepted',
				href: '/review?tab=plans',
				live: true
			};
		}
		case 'model_call.error': {
			const model = str(msg?.model) ? modelName(str(msg?.model)) : 'A model';
			const pipeline = str(msg?.pipeline);
			const outcome = OUTCOME[str(msg?.outcome)] ?? 'failed';
			return {
				at,
				kind: type,
				text: `${model} call ${outcome}${pipeline ? ` (${pipeline})` : ''}`,
				href: '/trace',
				live: true
			};
		}
		default:
			return null;
	}
}

/** Dedupe key: same kind + same link means the same event. */
export function activityKey(a: Activity): string {
	// Model errors share one link (/trace): key them by time instead.
	if (a.kind === 'model_call.error') return `${a.kind}|${a.at}`;
	return `${a.kind}|${a.href ?? a.text}`;
}

/** Server rows plus live rows the server has not reported yet, newest first. */
export function mergeActivity(server: Activity[], live: Activity[], cap = 12): Activity[] {
	const seen = new Set(server.map(activityKey));
	const extra = live.filter((a) => !seen.has(activityKey(a)));
	return [...extra, ...server].sort((a, b) => b.at - a.at).slice(0, cap);
}
