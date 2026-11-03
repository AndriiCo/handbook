// Staff review of model proposals beyond single facts (MISSION WIN
// W1b): page plans (SA-22), offices and form maps (SA-23). Shapes are
// the contract seeds docs/lanes/contracts/admin-{plan-proposals,
// office-proposals,form-maps}.json. Every decision is a CSRF POST;
// the server publishes the ids-only `authority:{slug}` event.
import { getJson, postJson } from './admin-api.js';

export interface Guard {
	id: string;
	ok: boolean;
	detail: string;
}

export interface PlanBlock {
	id: string;
	kind: 'step' | 'callout' | 'office' | 'fees' | 'mail' | 'forms' | 'timing' | string;
	title?: string;
	body?: string;
	tone?: 'info' | 'warning';
	office_ids?: number[];
	form_ids?: number[];
	lines?: { label: string; amount_cents: number; per?: string; fact_id?: number }[];
	fact_ids: number[];
	show_for: string[];
}

export interface PlanProposal {
	id: number;
	authority_slug: string;
	version: number;
	status: string;
	created_at: number;
	run_id: number | null;
	plan: {
		headline: string;
		paths: { id: string; label: string; fact_ids: number[] }[];
		blocks: PlanBlock[];
		hide_topics: string[];
	};
	validator: { passed: boolean; checks: Guard[]; dropped_blocks: string[] };
	diff: { added: string[]; removed: string[]; changed: string[] };
	current_version: number | null;
}

export interface OfficeProposal {
	id: number;
	authority_slug: string;
	name: string;
	status: string;
	visit: { kind: string; appointment_required: boolean; notes: string; fact_ids: number[] } | null;
	address: {
		line1: string;
		line2: string | null;
		city: string;
		region: string;
		postal_code: string;
		country: string;
		fact_id: number | null;
	} | null;
	mail: {
		po_box: string | null;
		line1: string;
		city: string;
		region: string;
		postal_code: string;
		country: string;
		usps_only: boolean;
		fact_id: number | null;
	} | null;
	hours: { days: string[]; open: string; close: string; fact_id: number | null }[];
	timezone: string | null;
	timezone_source: string | null;
	phone: string | null;
	directions_url: string | null;
	fact_ids: number[];
	guards: Guard[];
}

export interface FormField {
	name: string;
	label: string;
	help: string;
	type: string;
	required: boolean;
	options: string[];
	prefill_topic: string | null;
	label_quote: string;
}

export interface FormMap {
	id: number;
	authority_slug: string;
	status: string;
	source_url: string;
	capture_id: number | null;
	created_at: number;
	acroform_fields: number;
	mapped: number;
	fields: FormField[];
	guards: Guard[];
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Loose = any;

const arr = <T>(v: unknown): T[] => (Array.isArray(v) ? (v as T[]) : []);
const guards = (v: unknown): Guard[] =>
	arr<Loose>(v)
		.filter((g) => g && typeof g.id === 'string')
		.map((g) => ({ id: g.id, ok: g.ok === true, detail: typeof g.detail === 'string' ? g.detail : '' }));

export async function fetchPlans(status = 'proposed'): Promise<PlanProposal[]> {
	const body = await getJson<Loose>(`/admin/plans?status=${encodeURIComponent(status)}`, 'Page plans');
	return arr<Loose>(body?.plans)
		.filter((p) => p && typeof p.id === 'number' && p.plan && typeof p.plan === 'object')
		.map((p) => ({
			id: p.id,
			authority_slug: String(p.authority_slug ?? ''),
			version: Number(p.version ?? 0),
			status: String(p.status ?? 'proposed'),
			created_at: Number(p.created_at ?? 0),
			run_id: typeof p.run_id === 'number' ? p.run_id : null,
			plan: {
				headline: String(p.plan.headline ?? ''),
				paths: arr<Loose>(p.plan.paths).filter((x) => x && typeof x.id === 'string'),
				blocks: arr<Loose>(p.plan.blocks)
					.filter((b) => b && typeof b.id === 'string' && typeof b.kind === 'string')
					.map((b) => ({ ...b, fact_ids: arr<number>(b.fact_ids), show_for: arr<string>(b.show_for) })),
				hide_topics: arr<string>(p.plan.hide_topics)
			},
			validator: {
				passed: p.validator?.passed === true,
				checks: guards(p.validator?.checks),
				dropped_blocks: arr<string>(p.validator?.dropped_blocks)
			},
			diff: {
				added: arr<string>(p.diff?.added),
				removed: arr<string>(p.diff?.removed),
				changed: arr<string>(p.diff?.changed)
			},
			current_version: typeof p.current_version === 'number' ? p.current_version : null
		}));
}

export async function fetchOffices(status = 'proposed'): Promise<OfficeProposal[]> {
	const body = await getJson<Loose>(`/admin/offices?status=${encodeURIComponent(status)}`, 'Offices');
	return arr<Loose>(body?.offices)
		.filter((o) => o && typeof o.id === 'number' && typeof o.name === 'string')
		.map((o) => ({
			...o,
			authority_slug: String(o.authority_slug ?? ''),
			status: String(o.status ?? 'proposed'),
			visit: o.visit ?? null,
			address: o.address ?? null,
			mail: o.mail ?? null,
			hours: arr<Loose>(o.hours).filter((h) => h && Array.isArray(h.days)),
			timezone: typeof o.timezone === 'string' ? o.timezone : null,
			timezone_source: typeof o.timezone_source === 'string' ? o.timezone_source : null,
			phone: typeof o.phone === 'string' ? o.phone : null,
			directions_url:
				typeof o.directions_url === 'string' && o.directions_url.startsWith('https://')
					? o.directions_url
					: null,
			fact_ids: arr<number>(o.fact_ids),
			guards: guards(o.guards)
		}));
}

export async function fetchFormMaps(status = 'proposed'): Promise<FormMap[]> {
	const body = await getJson<Loose>(`/admin/form-maps?status=${encodeURIComponent(status)}`, 'Form maps');
	return arr<Loose>(body?.form_maps)
		.filter((f) => f && typeof f.id === 'number')
		.map((f) => ({
			id: f.id,
			authority_slug: String(f.authority_slug ?? ''),
			status: String(f.status ?? 'proposed'),
			source_url: typeof f.source_url === 'string' ? f.source_url : '',
			capture_id: typeof f.capture_id === 'number' ? f.capture_id : null,
			created_at: Number(f.created_at ?? 0),
			acroform_fields: Number(f.acroform_fields ?? 0),
			mapped: Number(f.mapped ?? 0),
			fields: arr<Loose>(f.fields)
				.filter((x) => x && typeof x.name === 'string')
				.map((x) => ({
					name: x.name,
					label: String(x.label ?? ''),
					help: String(x.help ?? ''),
					type: String(x.type ?? 'text'),
					required: x.required === true,
					options: arr<string>(x.options),
					prefill_topic: typeof x.prefill_topic === 'string' ? x.prefill_topic : null,
					label_quote: String(x.label_quote ?? '')
				})),
			guards: guards(f.guards)
		}));
}

export type ProposalKind = 'plans' | 'offices' | 'form-maps';

/** Accept or reject one proposal; resolves to the server's status. */
export async function decide(
	kind: ProposalKind,
	id: number,
	action: 'accept' | 'reject',
	reason?: string
): Promise<string> {
	const body = action === 'reject' ? { reason: reason ?? '' } : {};
	const out = await postJson<Loose>(`/admin/${kind}/${id}/${action}`, body, action === 'accept' ? 'Accept' : 'Reject');
	return typeof out?.status === 'string' ? out.status : action === 'accept' ? 'accepted' : 'rejected';
}

export interface FactCheck {
	fact_id: number;
	verdict: 'supported' | 'contradicted' | 'not_found' | string;
	quote: string;
	model: string;
	at: number;
}

/** Muse Wonder cross-check verdicts for one authority (SA-23). */
export async function fetchFactChecks(slug: string): Promise<FactCheck[]> {
	const body = await getJson<Loose>(`/admin/fact-checks?slug=${encodeURIComponent(slug)}`, 'Fact checks');
	return arr<Loose>(body?.checks).filter((c) => c && typeof c.fact_id === 'number' && typeof c.verdict === 'string');
}

/** "08:00" → "8:00 AM" without inventing a timezone. */
export function clock24(hhmm: string): string {
	const m = /^(\d{1,2}):(\d{2})$/.exec(hhmm);
	if (!m) return hhmm;
	const h = Number(m[1]);
	const suffix = h >= 12 ? 'PM' : 'AM';
	return `${((h + 11) % 12) + 1}:${m[2]} ${suffix}`;
}

const DAY = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];
const DAY_LABEL: Record<string, string> = {
	mon: 'Mon',
	tue: 'Tue',
	wed: 'Wed',
	thu: 'Thu',
	fri: 'Fri',
	sat: 'Sat',
	sun: 'Sun'
};

/** ["mon","tue","wed"] → "Mon–Wed"; non-contiguous → "Mon, Wed". */
export function dayRange(days: string[]): string {
	const idx = days.map((d) => DAY.indexOf(d)).filter((i) => i >= 0).sort((a, b) => a - b);
	if (idx.length === 0) return days.join(', ');
	const contiguous = idx.every((v, i) => i === 0 || v === idx[i - 1] + 1);
	if (contiguous && idx.length > 2) return `${DAY_LABEL[DAY[idx[0]]]}–${DAY_LABEL[DAY[idx[idx.length - 1]]]}`;
	return idx.map((i) => DAY_LABEL[DAY[i]]).join(', ');
}
