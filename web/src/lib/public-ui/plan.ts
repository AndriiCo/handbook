// "Your apostille plan" (MISSION WIN W2a): the staff-accepted render
// plan from `GET /api/v1/authorities/{slug}/plan` (always 200; shape
// docs/contracts/plan.json). The page renders allowlisted block
// kinds only, each resting on accepted facts it cites; anything else in
// the payload is ignored. Pure helpers: no window/fetch.

export type Citation = { quote?: string; source_url?: string; captured_at?: string };

export type PlanFact = {
	id: number;
	topic?: string;
	status?: string;
	statement?: string;
	citations?: Citation[];
	drafted_by?: string;
	created_by?: string;
	conflict?: boolean;
};

export type PlanPath = { id: string; label: string; fact_ids: number[] };

type BlockBase = { id: string; fact_ids: number[]; show_for: string[] };
export type FeeLine = {
	label: string;
	amount_cents: number;
	per?: string;
	fact_id?: number;
	currency?: string;
};
export type PlanBlock =
	| (BlockBase & { kind: 'step' | 'timing' | 'mail'; title: string; body: string })
	| (BlockBase & { kind: 'callout'; tone: 'info' | 'warning'; title: string; body: string })
	| (BlockBase & { kind: 'office'; office_ids: number[] })
	| (BlockBase & { kind: 'fees'; lines: FeeLine[] })
	| (BlockBase & { kind: 'forms'; form_ids: number[] });

export type Plan = {
	id: number;
	version: number;
	accepted_at: number | null;
	headline: string;
	paths: PlanPath[];
	blocks: PlanBlock[];
};

export type Hours = { days: string[]; open: string; close: string; fact_id?: number };
export type Office = {
	id: number;
	name: string;
	visit?: { kind?: string; appointment_required?: boolean; notes?: string; fact_ids?: number[] };
	address?: {
		line1?: string | null;
		line2?: string | null;
		city?: string | null;
		region?: string | null;
		postal_code?: string | null;
		country?: string | null;
		fact_id?: number;
	} | null;
	mail?: {
		po_box?: string | null;
		line1?: string | null;
		city?: string | null;
		region?: string | null;
		postal_code?: string | null;
		country?: string | null;
		usps_only?: boolean;
		fact_id?: number;
	} | null;
	hours: Hours[];
	timezone?: string | null;
	timezone_source?: string | null;
	phone?: string | null;
	directions_url?: string | null;
	fact_ids: number[];
};

export type PlanForm = {
	id: number;
	title: string;
	official_url?: string | null;
	source_fact_id?: number;
	fillable?: boolean;
	fields?: unknown[];
};

export type Provenance = {
	models: { model: string; role: string }[];
	validator?: { passed?: boolean; checks?: number; dropped_blocks?: number } | null;
	cross_check?: {
		supported?: number;
		contradicted?: number;
		not_found?: number;
		checked_before_accept?: boolean;
	} | null;
	run?: {
		run_id?: number;
		latency_ms?: number;
		p95_ms?: number;
		input_tokens?: number;
		output_tokens?: number;
		cached_tokens?: number;
		reasoning_tokens?: number;
	} | null;
	approved?: { at?: number } | null;
	sources: { url: string; captured_at?: string }[];
};

export type PlanView = {
	plan: Plan;
	facts: Map<number, PlanFact>;
	offices: Map<number, Office>;
	forms: Map<number, PlanForm>;
	provenance: Provenance | null;
};

const TEXT_KINDS = new Set(['step', 'timing', 'mail', 'callout']);
export const BLOCK_KINDS = new Set([...TEXT_KINDS, 'office', 'fees', 'forms']);

const str = (v: unknown): string => (typeof v === 'string' ? v : '');
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const ids = (v: unknown): number[] =>
	Array.isArray(v) ? v.filter((x): x is number => typeof x === 'number') : [];
const strs = (v: unknown): string[] =>
	Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
const rows = (v: unknown): any[] => (Array.isArray(v) ? v.filter((x) => x && typeof x === 'object') : []);

/**
 * Parse a `/plan` body into a renderable view, or null (no accepted
 * plan, a non-JSON body, an unknown shape) so the page falls back to
 * the fact guide. Blocks outside the allowlist, or citing no accepted
 * fact, or pointing at offices/forms the payload lacks, are dropped.
 */
export function parsePlan(body: unknown): PlanView | null {
	const b = body as any;
	const p = b?.plan;
	if (!p || typeof p !== 'object') return null;
	const facts = new Map<number, PlanFact>();
	for (const f of rows(b.facts)) {
		if (typeof f.id === 'number' && f.status === 'accepted' && f.topic !== 'phone_script')
			facts.set(f.id, f as PlanFact);
	}
	const offices = new Map<number, Office>();
	for (const o of rows(b.offices)) {
		if (typeof o.id !== 'number' || (o.status && o.status !== 'accepted')) continue;
		offices.set(o.id, {
			...o,
			name: str(o.name),
			hours: rows(o.hours).filter((h) => /^\d{2}:\d{2}$/.test(h.open) && /^\d{2}:\d{2}$/.test(h.close)),
			fact_ids: ids(o.fact_ids)
		} as Office);
	}
	const forms = new Map<number, PlanForm>();
	for (const f of rows(b.forms)) {
		if (typeof f.id === 'number' && (!f.status || f.status === 'active'))
			forms.set(f.id, { ...f, title: str(f.title) } as PlanForm);
	}
	const cited = (list: number[]) => list.some((id) => facts.has(id));
	const blocks: PlanBlock[] = [];
	for (const raw of rows(p.blocks)) {
		const kind = str(raw.kind);
		if (!BLOCK_KINDS.has(kind)) continue;
		const base = { id: str(raw.id) || `b${blocks.length}`, fact_ids: ids(raw.fact_ids), show_for: strs(raw.show_for) };
		if (!cited(base.fact_ids)) continue;
		if (TEXT_KINDS.has(kind)) {
			const title = str(raw.title);
			const bodyText = str(raw.body);
			if (!title && !bodyText) continue;
			if (kind === 'callout')
				blocks.push({ ...base, kind, tone: raw.tone === 'warning' ? 'warning' : 'info', title, body: bodyText });
			else blocks.push({ ...base, kind: kind as 'step' | 'timing' | 'mail', title, body: bodyText });
		} else if (kind === 'office') {
			const office_ids = ids(raw.office_ids).filter((id) => offices.has(id));
			if (office_ids.length) blocks.push({ ...base, kind, office_ids });
		} else if (kind === 'fees') {
			const lines = rows(raw.lines).filter(
				(l) => typeof l.amount_cents === 'number' && l.amount_cents >= 0 && str(l.label)
			) as FeeLine[];
			if (lines.length) blocks.push({ ...base, kind, lines });
		} else if (kind === 'forms') {
			const form_ids = ids(raw.form_ids).filter((id) => forms.has(id));
			if (form_ids.length) blocks.push({ ...base, kind, form_ids });
		}
	}
	if (blocks.length === 0) return null;
	const pv = b.provenance && typeof b.provenance === 'object' ? b.provenance : null;
	return {
		plan: {
			id: num(p.id) ?? 0,
			version: num(p.version) ?? 0,
			accepted_at: num(p.accepted_at),
			headline: str(p.headline),
			paths: rows(p.paths)
				.filter((x) => str(x.id) && str(x.label))
				.map((x) => ({ id: x.id, label: x.label, fact_ids: ids(x.fact_ids) })),
			blocks
		},
		facts,
		offices,
		forms,
		provenance: pv
			? {
					...pv,
					models: rows(pv.models).filter((m) => str(m.model)),
					sources: rows(pv.sources).filter((s) => str(s.url))
				}
			: null
	};
}

/** The selected path id from `?path=`, or null when unknown/absent. */
export function selectedPath(plan: Plan, raw: string | null): string | null {
	return raw && plan.paths.some((p) => p.id === raw) ? raw : null;
}

/** Blocks for a path: `show_for: []` is every path; no path → universal only. */
export function blocksFor(plan: Plan, path: string | null): PlanBlock[] {
	return plan.blocks.filter((b) => b.show_for.length === 0 || (path !== null && b.show_for.includes(path)));
}

// ---------- offices: hours and "Open now" ----------

export const DAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'] as const;
const DAY_SHORT: Record<string, string> = {
	mon: 'Mon',
	tue: 'Tue',
	wed: 'Wed',
	thu: 'Thu',
	fri: 'Fri',
	sat: 'Sat',
	sun: 'Sun'
};

const minutes = (hhmm: string) => Number(hhmm.slice(0, 2)) * 60 + Number(hhmm.slice(3, 5));

/** "16:30" → "4:30 PM" (Intl, fixed UTC instant so no zone shifts it). */
export function clock(hhmm: string): string {
	const m = minutes(hhmm);
	return new Intl.DateTimeFormat('en-US', { hour: 'numeric', minute: '2-digit', timeZone: 'UTC' }).format(
		Date.UTC(2000, 0, 3, Math.floor(m / 60), m % 60)
	);
}

/** Hours rows with consecutive days of equal times merged ("Mon–Fri"). */
export function hoursRows(office: Office): { days: string; time: string; fact_id?: number }[] {
	const byDay = new Map<string, Hours>();
	for (const h of office.hours) for (const d of h.days) if (DAY_SHORT[d] && !byDay.has(d)) byDay.set(d, h);
	const out: { days: string; time: string; fact_id?: number }[] = [];
	let run: string[] = [];
	const flush = () => {
		if (!run.length) return;
		const h = byDay.get(run[0])!;
		const days = run.length > 1 ? `${DAY_SHORT[run[0]]}–${DAY_SHORT[run[run.length - 1]]}` : DAY_SHORT[run[0]];
		out.push({ days, time: `${clock(h.open)} – ${clock(h.close)}`, fact_id: h.fact_id });
		run = [];
	};
	for (const d of DAYS) {
		const h = byDay.get(d);
		const prev = run.length ? byDay.get(run[run.length - 1]) : null;
		if (!h) {
			flush();
			continue;
		}
		if (prev && (prev.open !== h.open || prev.close !== h.close)) flush();
		run.push(d);
	}
	flush();
	return out;
}

/** True when the office's timezone is grounded (quoted or staff-confirmed). */
export function hasGroundedZone(office: Office): boolean {
	return (
		typeof office.timezone === 'string' &&
		!!office.timezone &&
		(office.timezone_source === 'quoted' || office.timezone_source === 'staff_confirmed')
	);
}

export type OpenStatus = { open: boolean; text: string };

/**
 * "Open now · closes 4:30 PM" / "Closed · opens Mon 8:00 AM", computed
 * with Intl in the office's own zone. Null without a grounded zone or
 * hours (the card then shows the hours table only).
 */
export function openStatus(office: Office, now: Date): OpenStatus | null {
	if (!hasGroundedZone(office) || office.hours.length === 0) return null;
	let parts: Intl.DateTimeFormatPart[];
	try {
		parts = new Intl.DateTimeFormat('en-US', {
			timeZone: office.timezone!,
			weekday: 'short',
			hour: '2-digit',
			minute: '2-digit',
			hourCycle: 'h23'
		}).formatToParts(now);
	} catch {
		return null;
	}
	const get = (t: string) => parts.find((p) => p.type === t)?.value ?? '';
	const today = DAYS.indexOf(get('weekday').toLowerCase().slice(0, 3) as (typeof DAYS)[number]);
	if (today < 0) return null;
	const nowMin = (Number(get('hour')) % 24) * 60 + Number(get('minute'));
	const spans = (d: number) =>
		office.hours
			.filter((h) => h.days.includes(DAYS[d]))
			.map((h) => ({ open: minutes(h.open), close: minutes(h.close), h }))
			.sort((a, b) => a.open - b.open);
	for (const s of spans(today)) {
		if (nowMin >= s.open && nowMin < s.close) return { open: true, text: `Open now · closes ${clock(s.h.close)}` };
	}
	for (let k = 0; k < 7; k++) {
		const d = (today + k) % 7;
		const next = spans(d).find((s) => k > 0 || s.open > nowMin);
		if (!next) continue;
		const when = k === 0 ? 'today' : k === 1 ? 'tomorrow' : DAY_SHORT[DAYS[d]];
		return { open: false, text: `Closed · opens ${when} ${clock(next.h.open)}` };
	}
	return { open: false, text: 'Closed' };
}

/** Long zone name for the hours caption ("Pacific Standard Time"). */
export function zoneName(tz: string, now: Date): string {
	try {
		const p = new Intl.DateTimeFormat('en-US', { timeZone: tz, timeZoneName: 'long' }).formatToParts(now);
		return p.find((x) => x.type === 'timeZoneName')?.value ?? tz;
	} catch {
		return tz;
	}
}

// ---------- fees ----------

/**
 * Money for a fee line: its own currency when the payload names one,
 * else USD only when the cited source itself quotes a `$` amount; with
 * neither, a plain number (never a guessed currency).
 */
export function money(cents: number, currency: string | null): string {
	const value = cents / 100;
	if (currency) {
		try {
			return new Intl.NumberFormat('en-US', {
				style: 'currency',
				currency,
				minimumFractionDigits: cents % 100 === 0 ? 0 : 2
			}).format(value);
		} catch {
			// fall through to a plain number
		}
	}
	return new Intl.NumberFormat('en-US', { minimumFractionDigits: cents % 100 === 0 ? 0 : 2 }).format(value);
}

export function lineCurrency(line: FeeLine, fact: PlanFact | undefined): string | null {
	if (typeof line.currency === 'string' && /^[A-Z]{3}$/.test(line.currency)) return line.currency;
	const quoted = `${fact?.citations?.[0]?.quote ?? ''} ${fact?.statement ?? ''}`;
	return /\$\s?\d/.test(quoted) ? 'USD' : null;
}

/** Total for `count` documents: per-document lines scale, others are flat. */
export function feeTotal(lines: FeeLine[], count: number): number {
	return lines.reduce((sum, l) => sum + (l.per === 'document' ? l.amount_cents * count : l.amount_cents), 0);
}

// ---------- provenance ----------

/** `muse-spark-1.3` → `Muse Spark 1.3`; `sam-3.1` → `SAM 3.1`. */
export function modelLabel(model: string): string {
	return model
		.split('-')
		.map((w) => (/^\d/.test(w) ? w : w === 'sam' ? 'SAM' : w.charAt(0).toUpperCase() + w.slice(1)))
		.join(' ');
}

export function roleLabel(role: string): string {
	if (role === 'extract') return 'Extracted each fact from official pages';
	if (role === 'cross_check') return 'Cross-checked each fact against its quote';
	if (role === 'compose') return 'Composed this plan from accepted facts';
	const s = role.replaceAll('_', ' ');
	return s.charAt(0).toUpperCase() + s.slice(1);
}

export function seconds(ms: number | undefined): string {
	return typeof ms === 'number' && Number.isFinite(ms) ? `${(ms / 1000).toFixed(1)} s` : '—';
}

export function count(n: number | undefined): string {
	return typeof n === 'number' && Number.isFinite(n) ? new Intl.NumberFormat('en-US').format(n) : '—';
}

const TOPIC_LABEL: Record<string, string> = {
	walkin: 'Walk-in',
	po_box: 'PO Box',
	id_requirements: 'ID requirements'
};

/** Chip label for a fact's topic (frozen vocabulary, readable form). */
export function topicName(topic: string): string {
	if (TOPIC_LABEL[topic]) return TOPIC_LABEL[topic];
	const s = topic.replaceAll('_', ' ');
	return s.charAt(0).toUpperCase() + s.slice(1);
}

/** Epoch seconds → `YYYY-MM-DD` (UTC), or ''. */
export function day(epoch: number | null | undefined): string {
	return typeof epoch === 'number' && Number.isFinite(epoch) ? new Date(epoch * 1000).toISOString().slice(0, 10) : '';
}
