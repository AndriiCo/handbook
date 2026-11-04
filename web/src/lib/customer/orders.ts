// Customer commerce client (MISSION WIN SA-28): order options, orders,
// quotes, checkout and the cabinet. Shapes follow the contract seeds
// in docs/contracts/ (order-options, order, orders-list,
// order-quote, order-paused, plan). Every value a visitor sees comes
// from these responses: no fee, carrier, address or form field is
// written here. The labels below name contract enumerations only.
import { resolve } from '$app/paths';
import { cookieValue } from './passkey.js';

export type OrderKind = 'label_fedex' | 'label_usps' | 'mail_kit' | 'address_label';
export type OrderStatus =
	| 'draft'
	| 'quoted'
	| 'awaiting_payment'
	| 'paid'
	| 'label_purchasing'
	| 'labels_ready'
	| 'packing'
	| 'shipped'
	| 'in_transit'
	| 'delivered'
	| 'needs_staff'
	| 'refunded'
	| 'canceled';
export type LegRole = 'outbound' | 'return' | 'kit_to_customer';

export type DeliveryOption = {
	kind: OrderKind;
	enabled: boolean;
	title: string;
	reason: string | null;
	fact_id: number | null;
	from_cents: number | null;
};

export type OrderOptions = {
	slug: string;
	mode: string;
	paid_enabled: boolean;
	currency: string;
	options: DeliveryOption[];
};

export type MoneyLine = { label: string; cents: number };

export type CarrierQuote = {
	leg: LegRole;
	carrier: string;
	service: string;
	cents: number;
	delivery_days: number | null;
};

export type Quote = {
	order_id: string;
	mode: string;
	currency: string;
	total_cents: number;
	expires_at: number | null;
	lines: MoneyLine[];
	carrier_quotes: CarrierQuote[];
};

export type Leg = {
	role: LegRole;
	carrier: string;
	service: string;
	shipment_id: number;
	status: string;
	tracking_number: string | null;
	tracking_url: string | null;
	label_url: string | null;
	topic: string;
};

export type OrderEvent = { at: number; kind: string; text: string; leg: LegRole | null };

export type ReturnAddress = {
	name: string;
	line1?: string;
	line2?: string | null;
	city: string;
	region: string;
	postal_code: string;
	country: string;
};

export type Order = {
	id: string;
	kind: OrderKind;
	status: OrderStatus;
	mode: string;
	authority: { slug: string; name: string };
	path_id: string | null;
	created_at: number;
	updated_at: number;
	amount: { currency: string; total_cents: number; lines: MoneyLine[] } | null;
	return_to: ReturnAddress | null;
	legs: Leg[];
	form_fill_id: string | null;
	events: OrderEvent[];
	/** `demo`: a W-3 fake Stripe payment (no card charged). */
	receipt: { paid_at: number; receipt_url: string | null; demo?: boolean } | null;
	actions: { id: string; label: string }[];
	topic: string;
};

export type OrderSummary = {
	id: string;
	kind: OrderKind;
	status: OrderStatus;
	mode: string;
	authority: { slug: string; name: string };
	created_at: number;
	updated_at: number;
	total_cents: number;
	currency: string;
	next: string | null;
};

export type Citation = { quote?: string; source_url?: string; captured_at?: string };

export type Fact = {
	id: number;
	topic: string;
	status?: string;
	statement: string;
	drafted_by?: string;
	created_by?: string;
	citations?: Citation[];
};

export type PlanPath = { id: string; label: string; fact_ids: number[] };

export type PlanBlock = {
	id: string;
	kind: string;
	title?: string;
	body?: string;
	tone?: string;
	fact_ids: number[];
	show_for: string[];
	form_ids?: number[];
	lines?: { label: string; amount_cents: number; per?: string; fact_id?: number }[];
};

export type FormField = {
	name: string;
	label: string;
	help: string;
	type: string;
	required: boolean;
	options: string[];
	prefill_topic: string | null;
	label_quote: string | null;
};

export type FormTemplate = {
	id: number;
	title: string;
	official_url: string | null;
	source_fact_id: number | null;
	fillable: boolean;
	fields: FormField[];
};

export type PlanBundle = {
	slug: string;
	plan: {
		id: number;
		version: number;
		headline?: string;
		paths: PlanPath[];
		blocks: PlanBlock[];
	} | null;
	facts: Fact[];
	forms: FormTemplate[];
};

export type ApiResult<T = any> = { ok: boolean; status: number; body: T };

/** `/apostille/api/v1` on the public build (base-relative on admin). */
export function apiUrl(path: string): string {
	const root = resolve('/(public)').replace(/\/$/, '');
	return `${root}/api/v1${path}`;
}

export async function getJson<T = any>(path: string): Promise<ApiResult<T>> {
	try {
		const res = await fetch(apiUrl(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	} catch {
		return { ok: false, status: 0, body: {} as T };
	}
}

/** CSRF double-submit POST, same as the login and security pages. */
export async function postJson<T = any>(path: string, payload: unknown): Promise<ApiResult<T>> {
	try {
		const res = await fetch(apiUrl(path), {
			method: 'POST',
			credentials: 'same-origin',
			headers: {
				'content-type': 'application/json',
				'x-csrf': cookieValue(document.cookie, 'hb_csrf')
			},
			body: JSON.stringify(payload ?? {})
		});
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	} catch {
		return { ok: false, status: 0, body: {} as T };
	}
}

export const isPaused = (r: ApiResult): boolean =>
	r.status === 503 && r.body?.error === 'paid_tiers_paused';

/** Server-authored topics only; anything malformed is never opened. */
export function isCabinetTopic(topic: unknown): topic is string {
	return typeof topic === 'string' && /^(billing|shipment):[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(topic);
}

// ------------------------------------------------------------ display

export function money(cents: number, currency: string, opts: { compact?: boolean } = {}): string {
	const whole = cents % 100 === 0;
	try {
		return new Intl.NumberFormat(undefined, {
			style: 'currency',
			currency: currency.toUpperCase(),
			minimumFractionDigits: opts.compact && whole ? 0 : 2,
			maximumFractionDigits: opts.compact && whole ? 0 : 2
		}).format(cents / 100);
	} catch {
		return `${(cents / 100).toFixed(2)} ${currency.toUpperCase()}`;
	}
}

export function fmtDate(unix: number): string {
	return new Date(unix * 1000).toLocaleDateString(undefined, {
		year: 'numeric',
		month: 'short',
		day: 'numeric'
	});
}

export function fmtDateTime(unix: number): string {
	return new Date(unix * 1000).toLocaleString(undefined, {
		month: 'short',
		day: 'numeric',
		hour: 'numeric',
		minute: '2-digit'
	});
}

export function fmtTime(unix: number): string {
	return new Date(unix * 1000).toLocaleTimeString(undefined, {
		hour: 'numeric',
		minute: '2-digit'
	});
}

/** Contract enumeration → display name (no carrier claims). */
export const KIND_LABEL: Record<OrderKind, string> = {
	label_fedex: 'FedEx labels',
	label_usps: 'USPS labels',
	mail_kit: 'Mailing kit',
	address_label: 'Address label'
};

export const STATUS_LABEL: Record<string, string> = {
	draft: 'Draft',
	quoted: 'Quoted',
	awaiting_payment: 'Awaiting payment',
	paid: 'Paid',
	label_purchasing: 'Buying labels',
	labels_ready: 'Labels ready',
	packing: 'Packing',
	shipped: 'Shipped',
	in_transit: 'In transit',
	delivered: 'Delivered',
	needs_staff: 'With our team',
	refunded: 'Refunded',
	canceled: 'Canceled'
};

export const ROLE_LABEL: Record<LegRole, string> = {
	kit_to_customer: 'Kit to you',
	outbound: 'To the authority',
	return: 'Back to you'
};

export function humanize(value: string): string {
	const text = value.replace(/[_-]+/g, ' ').trim().toLowerCase();
	return text.charAt(0).toUpperCase() + text.slice(1);
}

export function statusLabel(status: string): string {
	return STATUS_LABEL[status] ?? humanize(status);
}

const CARRIER_NAME: Record<string, string> = { usps: 'USPS', fedex: 'FedEx', fed_ex: 'FedEx' };

export function carrierName(carrier: string): string {
	return CARRIER_NAME[carrier.toLowerCase()] ?? carrier.toUpperCase();
}

/** `USPS_PRIORITY` → `USPS Priority` (the carrier token keeps its case). */
export function serviceName(service: string, carrier = ''): string {
	const words = service.split(/[_\s]+/).filter(Boolean);
	return words
		.map((w) =>
			w.toLowerCase() === carrier.toLowerCase() || CARRIER_NAME[w.toLowerCase()]
				? carrierName(w)
				: w.charAt(0).toUpperCase() + w.slice(1).toLowerCase()
		)
		.join(' ');
}

/** Tone of a status badge: verified green, changed amber, neutral. */
export function statusTone(status: string): 'done' | 'active' | 'attention' | 'muted' {
	if (status === 'delivered' || status === 'labels_ready' || status === 'paid') return 'done';
	if (status === 'needs_staff' || status === 'awaiting_payment') return 'attention';
	if (status === 'refunded' || status === 'canceled' || status === 'draft') return 'muted';
	return 'active';
}

export const STATUS_BADGE: Record<ReturnType<typeof statusTone>, string> = {
	done: 'border-verified bg-verified-bg text-verified',
	active: 'border-line bg-surface-2 text-accent-strong',
	attention: 'border-changed bg-changed-bg text-changed',
	muted: 'border-line bg-surface text-ink-muted'
};

/** Linear progress through the happy path, per kind. */
const FLOW: OrderStatus[] = [
	'draft',
	'quoted',
	'awaiting_payment',
	'paid',
	'label_purchasing',
	'labels_ready',
	'packing',
	'shipped',
	'in_transit',
	'delivered'
];

export function stagesFor(kind: OrderKind): OrderStatus[] {
	if (kind === 'address_label') return ['labels_ready', 'in_transit', 'delivered'];
	if (kind === 'mail_kit') return ['paid', 'labels_ready', 'packing', 'shipped', 'delivered'];
	return ['paid', 'labels_ready', 'in_transit', 'delivered'];
}

/** Index of the last stage reached (-1 before the first). */
export function stageReached(kind: OrderKind, status: string): number {
	const at = FLOW.indexOf(status as OrderStatus);
	if (at < 0) return -1;
	const stages = stagesFor(kind);
	let reached = -1;
	stages.forEach((s, i) => {
		if (FLOW.indexOf(s) <= at) reached = i;
	});
	return reached;
}

export function citationOf(fact: Fact | null | undefined): Citation | null {
	const c = fact?.citations?.[0];
	return c && typeof c.quote === 'string' && c.quote.trim() ? c : null;
}

export function hostOf(url: string | undefined): string | null {
	if (!url) return null;
	try {
		return new URL(url).hostname.replace(/^www\./, '');
	} catch {
		return null;
	}
}

/** Same-origin API paths or https links only; anything else is dropped. */
export function safeHref(url: string | null | undefined): string | null {
	if (typeof url !== 'string' || !url) return null;
	if (url.startsWith('/') && !url.startsWith('//')) return url;
	return /^https:\/\//.test(url) ? url : null;
}

export function formFillPdf(fillId: string): string {
	return apiUrl(`/p2/forms/fills/${encodeURIComponent(fillId)}.pdf`);
}

// ------------------------------------------------------------ cabinet

/** Orders not yet paid for (no receipt, no labels bought). */
export const UNPAID = ['draft', 'quoted', 'awaiting_payment', 'canceled'];

export type OrdersLoad =
	| { kind: 'ok'; orders: OrderSummary[]; topic: string | null }
	| { kind: 'login' }
	| { kind: 'error' };

/** The caller's orders, newest first (401 → sign in). */
export async function loadOrders(): Promise<OrdersLoad> {
	const r = await getJson('/p2/orders');
	if (r.status === 401) return { kind: 'login' };
	if (!r.ok || !Array.isArray(r.body?.orders)) return { kind: 'error' };
	const orders = [...(r.body.orders as OrderSummary[])].sort((a, b) => b.created_at - a.created_at);
	return { kind: 'ok', orders, topic: typeof r.body.topic === 'string' ? r.body.topic : null };
}

/** Order details for a short list (the cabinet caps it; ids-only list). */
export async function loadDetails(ids: string[], cap = 20): Promise<Order[]> {
	const rows = await Promise.all(
		ids.slice(0, cap).map((id) => getJson<Order>(`/p2/orders/${encodeURIComponent(id)}`))
	);
	return rows
		.filter((r) => r.ok && typeof r.body?.id === 'string')
		.map((r) => ({ ...r.body, legs: r.body.legs ?? [], events: r.body.events ?? [] }));
}

// ------------------------------------------------------------ session

/**
 * The existing session probes: `/me` (demo session → customer_ref)
 * and `/auth/me` (passkey/OAuth session → user_id). Both answer 200
 * `{"user": null}` to anonymous callers, so the probe never logs a
 * console error.
 */
export async function probeSignedIn(): Promise<boolean> {
	const [me, auth] = await Promise.all([getJson('/me'), getJson('/auth/me')]);
	const a = me.ok && typeof me.body?.customer_ref === 'string' && me.body.customer_ref !== '';
	const b = auth.ok && typeof auth.body?.user_id === 'string' && auth.body.user_id !== '';
	return a || b;
}

export function loginHref(next: string): string {
	return `${resolve('/(customer)/login')}?next=${encodeURIComponent(next)}`;
}
