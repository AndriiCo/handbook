import test from 'node:test';
import assert from 'node:assert/strict';
import { canRefund, canTransition, isStaffMove, staffMoves } from '../src/lib/admin/order-moves.js';

const moves = (status, kind = 'mail_kit', paid = true) => staffMoves({ status, kind, paid }).map((m) => `${m.role}:${m.to}`);

test('can_transition mirrors orders.rs', () => {
	assert.equal(canTransition('packing', 'shipped'), true);
	assert.equal(canTransition('shipped', 'delivered'), false);
	assert.equal(canTransition('delivered', 'needs_staff'), false);
	assert.equal(canTransition('draft', 'refunded'), false);
	assert.equal(canTransition('paid', 'refunded'), true);
	assert.equal(canTransition('needs_staff', 'packing'), true);
});

test('staff moves follow the advance allowlist', () => {
	// Mid-checkout orders never move.
	assert.equal(isStaffMove({ status: 'awaiting_payment', kind: 'mail_kit', paid: false }, 'canceled'), false);
	// Money states are not staff moves.
	assert.equal(isStaffMove({ status: 'awaiting_payment', kind: 'mail_kit', paid: true }, 'paid'), false);
	assert.equal(isStaffMove({ status: 'shipped', kind: 'mail_kit', paid: true }, 'refunded'), false);
	// Cancel only from draft or quoted.
	assert.equal(isStaffMove({ status: 'quoted', kind: 'label_usps', paid: false }, 'canceled'), true);
	assert.equal(isStaffMove({ status: 'paid', kind: 'label_usps', paid: true }, 'canceled'), false);
	// Unpaid orders get no fulfilment moves, except the free label.
	assert.equal(isStaffMove({ status: 'draft', kind: 'label_usps', paid: false }, 'labels_ready'), false);
	assert.equal(isStaffMove({ status: 'draft', kind: 'address_label', paid: false }, 'labels_ready'), true);
});

test('the sheet promotes one forward step per state', () => {
	assert.deepEqual(moves('paid'), ['primary:label_purchasing', 'flag:needs_staff']);
	assert.deepEqual(moves('labels_ready'), ['primary:packing', 'flag:needs_staff']);
	assert.deepEqual(moves('labels_ready', 'label_usps'), ['primary:in_transit', 'flag:needs_staff']);
	assert.deepEqual(moves('packing'), ['primary:shipped', 'flag:needs_staff']);
	assert.deepEqual(moves('in_transit'), ['primary:delivered', 'flag:needs_staff']);
	assert.deepEqual(moves('needs_staff', 'label_fedex'), ['primary:labels_ready', 'secondary:label_purchasing']);
	assert.deepEqual(moves('needs_staff'), ['primary:labels_ready', 'secondary:packing', 'secondary:label_purchasing']);
	assert.deepEqual(moves('quoted', 'label_usps', false), ['cancel:canceled']);
	assert.deepEqual(moves('delivered'), []);
	assert.deepEqual(moves('refunded'), []);
	assert.deepEqual(moves('awaiting_payment', 'label_usps', false), []);
});

test('refund needs a paid, unrefunded order', () => {
	assert.equal(canRefund({ status: 'packing', receipt: { refunded: false } }), true);
	assert.equal(canRefund({ status: 'packing', receipt: null }), false);
	assert.equal(canRefund({ status: 'refunded', receipt: { refunded: true } }), false);
	assert.equal(canRefund({ status: 'canceled', receipt: { refunded: true } }), false);
});
