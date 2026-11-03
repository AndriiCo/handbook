// Staff Orders console (W1c): shapes of `GET /admin/orders` and
// `GET /admin/orders/{id}` (crates/web/src/admin_orders.rs; the
// detail is orders.rs `order_view` plus `customer_ref`), the filter
// tabs and the staff wording. Staff see the customer_ref, never an
// email. Admin-only: never imported from (public) routes.
import { STATUS_LABEL, humanize, type Order, type OrderEvent } from '#lib/customer/orders.js';

export type AdminOrderRow = {
	id: string;
	kind: string;
	status: string;
	mode: string;
	authority_slug: string;
	customer: string;
	total_cents: number;
	currency: string;
	created_at: number;
	needs_staff: boolean;
	/** Contract field (admin-orders.json): why the order needs staff. */
	reason?: string | null;
};

export type AdminReceipt = { paid_at: number; receipt_url: string | null; refunded?: boolean };

/** A timeline row; `pending` marks an optimistic one not yet saved. */
export type AdminEvent = OrderEvent & { pending?: boolean };

export type AdminOrder = Omit<Order, 'receipt' | 'events' | 'kind' | 'status'> & {
	kind: string;
	status: string;
	customer_ref: string;
	receipt: AdminReceipt | null;
	events: AdminEvent[];
};

export type OrdersTab = 'all' | 'needs_staff' | 'to_pack' | 'shipped';

export const TABS: { id: OrdersTab; label: string; status: string | null }[] = [
	{ id: 'all', label: 'All', status: null },
	{ id: 'needs_staff', label: 'Needs staff', status: 'needs_staff' },
	{ id: 'to_pack', label: 'To pack', status: 'packing' },
	{ id: 'shipped', label: 'Shipped', status: 'shipped' }
];

/** `?status=` → tab (the dashboard links `?status=needs_staff`). */
export function tabFromParam(value: string | null): OrdersTab {
	if (value === 'packing') return 'to_pack';
	return TABS.some((t) => t.id === value) ? (value as OrdersTab) : 'all';
}

export function inTab(row: AdminOrderRow, tab: OrdersTab): boolean {
	const status = TABS.find((t) => t.id === tab)?.status ?? null;
	return status === null || row.status === status;
}

/** Same counting rule as the server's `counts`. */
export function countTabs(rows: AdminOrderRow[]): Record<OrdersTab, number> {
	return {
		all: rows.length,
		needs_staff: rows.filter((r) => r.status === 'needs_staff').length,
		to_pack: rows.filter((r) => r.status === 'packing').length,
		shipped: rows.filter((r) => r.status === 'shipped').length
	};
}

/** Staff wording: the customer reads "With our team", staff read the queue name. */
export function staffStatusLabel(status: string): string {
	if (status === 'needs_staff') return 'Needs staff';
	return STATUS_LABEL[status] ?? humanize(status);
}

const str = (v: unknown, fallback = ''): string => (typeof v === 'string' ? v : fallback);
const num = (v: unknown, fallback = 0): number => (typeof v === 'number' && Number.isFinite(v) ? v : fallback);

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Loose = any;

export function normalizeRows(body: Loose): AdminOrderRow[] {
	const rows: Loose[] = Array.isArray(body?.orders) ? body.orders : [];
	return rows
		.filter((r) => r && typeof r.id === 'string' && r.id)
		.map((r) => ({
			id: r.id,
			kind: str(r.kind, 'mail_kit'),
			status: str(r.status, 'draft'),
			mode: str(r.mode, 'test'),
			authority_slug: str(r.authority_slug),
			customer: str(r.customer),
			total_cents: num(r.total_cents),
			currency: str(r.currency, 'usd') || 'usd',
			created_at: num(r.created_at),
			needs_staff: r.status === 'needs_staff',
			reason: typeof r.reason === 'string' && r.reason ? r.reason : null
		}));
}

export function normalizeOrder(body: Loose): AdminOrder | null {
	if (!body || typeof body.id !== 'string') return null;
	return {
		...body,
		kind: str(body.kind, 'mail_kit'),
		status: str(body.status, 'draft'),
		customer_ref: str(body.customer_ref),
		legs: Array.isArray(body.legs) ? body.legs : [],
		events: Array.isArray(body.events) ? body.events : [],
		receipt: body.receipt && typeof body.receipt === 'object' ? body.receipt : null,
		amount: body.amount && typeof body.amount === 'object' ? body.amount : null,
		authority: body.authority ?? { slug: '', name: '' }
	} as AdminOrder;
}

/** An `ops` bus event about one order (`order.created`, `order.paid`, `order.advanced`, …). */
export function orderEventId(msg: { type?: unknown; order_id?: unknown }): string | null {
	if (typeof msg?.type !== 'string' || !msg.type.startsWith('order.')) return null;
	return typeof msg.order_id === 'string' && msg.order_id ? msg.order_id : null;
}

/** Why this order is with staff: the newest needs_staff event, else the row's reason. */
export function staffReason(order: AdminOrder | null, row: AdminOrderRow | null): string | null {
	const ev = [...(order?.events ?? [])].reverse().find((e) => e.kind === 'order.needs_staff');
	return ev?.text ?? row?.reason ?? null;
}
