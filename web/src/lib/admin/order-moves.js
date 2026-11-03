// Staff order moves for the Orders console (W1c). Mirrors the server
// rules so the sheet only offers buttons the server will accept:
// `can_transition` in crates/web/src/orders.rs and the staff
// allowlist in `advance` (crates/web/src/admin_orders.rs). The server
// stays the authority: a stale page still gets its 409 and reloads.
// Pure functions, no DOM. Admin-only.

const ENDED = ['delivered', 'refunded', 'canceled'];

/** @type {Record<string, string[]>} */
const PAIRS = {
	draft: ['quoted', 'labels_ready'],
	quoted: ['awaiting_payment', 'quoted'],
	awaiting_payment: ['quoted', 'paid'],
	paid: ['label_purchasing'],
	label_purchasing: ['labels_ready'],
	labels_ready: ['packing', 'in_transit'],
	packing: ['shipped'],
	shipped: ['in_transit'],
	in_transit: ['delivered'],
	needs_staff: ['label_purchasing', 'labels_ready', 'packing']
};

/** Fulfilment targets staff may set (admin_orders.rs `advance`). */
export const FULFILMENT = [
	'label_purchasing',
	'labels_ready',
	'packing',
	'shipped',
	'in_transit',
	'delivered',
	'needs_staff'
];

/**
 * orders.rs `can_transition`.
 * @param {string} from
 * @param {string} to
 */
export function canTransition(from, to) {
	const live = !ENDED.includes(from);
	if (to === 'needs_staff' || to === 'canceled') return live;
	if (to === 'refunded') return live && from !== 'draft';
	return (PAIRS[from] ?? []).includes(to);
}

/**
 * Would `POST /admin/orders/{id}/advance {to}` accept this move?
 * @param {{ status: string, kind: string, paid: boolean }} order
 * @param {string} to
 */
export function isStaffMove(order, to) {
	if (order.status === 'awaiting_payment') return false;
	const fulfilment = FULFILMENT.includes(to);
	const cancelUnpaid = to === 'canceled' && (order.status === 'draft' || order.status === 'quoted');
	if (!(fulfilment || cancelUnpaid)) return false;
	if (!canTransition(order.status, to)) return false;
	if (fulfilment && order.kind !== 'address_label' && !order.paid) return false;
	return true;
}

/** Button copy per target status. */
export const MOVE_LABEL = /** @type {Record<string, string>} */ ({
	label_purchasing: 'Start label purchase',
	labels_ready: 'Mark labels ready',
	packing: 'Start packing',
	shipped: 'Mark kit shipped',
	in_transit: 'Mark in transit',
	delivered: 'Mark delivered',
	needs_staff: 'Flag for staff',
	canceled: 'Cancel order'
});

/** The timeline text the server writes for each move (advance). */
export function moveText(/** @type {string} */ to) {
	switch (to) {
		case 'shipped':
			return 'Kit shipped to you';
		case 'in_transit':
			return 'On its way';
		case 'delivered':
			return 'Delivered';
		case 'labels_ready':
			return 'Labels ready';
		case 'packing':
			return 'Kit is being packed';
		case 'canceled':
			return 'Order canceled';
		default:
			return 'Order updated';
	}
}

// Kit stages (packing, shipped) only make sense for a mailing kit;
// the server allows them for any kind, the console does not offer them.
const KIT_ONLY = ['packing', 'shipped'];

/**
 * The moves to offer, best first. `primary` is the one forward step
 * the sheet promotes; `flag` (needs_staff) and `cancel` render apart.
 * @param {{ status: string, kind: string, paid: boolean }} order
 * @returns {{ to: string, label: string, role: 'primary' | 'secondary' | 'flag' | 'cancel' }[]}
 */
export function staffMoves(order) {
	const forward =
		order.status === 'needs_staff'
			? ['labels_ready', 'packing', 'label_purchasing']
			: ['label_purchasing', 'labels_ready', 'packing', 'shipped', 'in_transit', 'delivered'];
	const allowed = forward
		.filter((to) => order.kind === 'mail_kit' || !KIT_ONLY.includes(to))
		// A mailing kit leaves labels_ready by packing, not in transit.
		.filter((to) => !(order.kind === 'mail_kit' && order.status === 'labels_ready' && to === 'in_transit'))
		.filter((to) => isStaffMove(order, to));
	/** @type {{ to: string, label: string, role: 'primary' | 'secondary' | 'flag' | 'cancel' }[]} */
	const out = allowed.map((to, i) => ({ to, label: MOVE_LABEL[to], role: i === 0 ? 'primary' : 'secondary' }));
	if (order.status !== 'needs_staff' && isStaffMove(order, 'needs_staff')) {
		out.push({ to: 'needs_staff', label: MOVE_LABEL.needs_staff, role: 'flag' });
	}
	if (isStaffMove(order, 'canceled')) {
		out.push({ to: 'canceled', label: MOVE_LABEL.canceled, role: 'cancel' });
	}
	return out;
}

/**
 * Refund is offered for a paid, not yet refunded order (admin_orders.rs
 * `refund`: 409 otherwise, 503 while payments are not configured).
 * @param {{ status: string, receipt: { refunded?: boolean } | null }} order
 */
export function canRefund(order) {
	return !!order.receipt && order.receipt.refunded !== true && order.status !== 'refunded';
}
