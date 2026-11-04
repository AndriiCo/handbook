import test from 'node:test';
import assert from 'node:assert/strict';
import {
	hasOpenModal,
	installCitationHotkey,
	isCitationHotkey,
	isEditableTarget
} from '../src/lib/public-ui/doc/hotkey.js';

const alt = (over = {}) => ({
	code: 'KeyC',
	key: 'c',
	altKey: true,
	ctrlKey: false,
	metaKey: false,
	shiftKey: false,
	...over
});

test('Alt/⌥+C matches by code or key, with no other modifier', () => {
	assert.equal(isCitationHotkey(alt()), true);
	// macOS Option+C types "ç": the physical key still says KeyC.
	assert.equal(isCitationHotkey(alt({ key: 'ç' })), true);
	// Cyrillic layout: "с" on the same physical key.
	assert.equal(isCitationHotkey(alt({ key: 'с' })), true);
	// Dvorak: the C letter sits on another physical key.
	assert.equal(isCitationHotkey(alt({ code: 'KeyI', key: 'c' })), true);
	// Caps Lock: an upper-case key without Shift.
	assert.equal(isCitationHotkey(alt({ key: 'C' })), true);

	assert.equal(isCitationHotkey(alt({ altKey: false })), false);
	assert.equal(isCitationHotkey(alt({ ctrlKey: true })), false);
	assert.equal(isCitationHotkey(alt({ metaKey: true })), false);
	assert.equal(isCitationHotkey(alt({ shiftKey: true })), false);
	assert.equal(isCitationHotkey(alt({ code: 'KeyX', key: 'x' })), false);
	assert.equal(isCitationHotkey({ key: 'c' }), false);
	assert.equal(isCitationHotkey(null), false);
	assert.equal(isCitationHotkey(undefined), false);
});

test('form fields and editable regions keep the key', () => {
	const field = (match) => ({ closest: (sel) => (match && sel.includes('input') ? {} : null) });
	assert.equal(isEditableTarget(field(true)), true);
	assert.equal(isEditableTarget(field(false)), false);
	assert.equal(isEditableTarget({ isContentEditable: true }), true);
	assert.equal(isEditableTarget({ isContentEditable: false, closest: () => null }), false);
	assert.equal(isEditableTarget(null), false);
	assert.equal(isEditableTarget('input'), false);
	assert.equal(
		isEditableTarget({
			closest: () => {
				throw new Error('detached');
			}
		}),
		false
	);
});

test('an open modal dialog blocks the shortcut; a hidden one does not', () => {
	const doc = (els, nativeModal = false) => ({
		querySelectorAll: (sel) => (sel === '[aria-modal="true"]' ? els : []),
		querySelector: (sel) => (sel === 'dialog:modal' && nativeModal ? {} : null)
	});
	const shown = { hidden: false, getClientRects: () => [{}] };
	const unrendered = { hidden: false, getClientRects: () => [] };
	const hidden = { hidden: true, getClientRects: () => [{}] };
	assert.equal(hasOpenModal(doc([shown])), true);
	assert.equal(hasOpenModal(doc([unrendered, hidden])), false);
	assert.equal(hasOpenModal(doc([], true)), true);
	assert.equal(hasOpenModal(doc([])), false);
	assert.equal(hasOpenModal(null), false);
	assert.equal(
		hasOpenModal({
			querySelectorAll: () => [],
			querySelector: () => {
				throw new SyntaxError(':modal');
			}
		}),
		false
	);
});

function target() {
	/** @type {Map<string, Set<Function>>} */
	const listeners = new Map();
	return {
		addEventListener(type, fn) {
			if (!listeners.has(type)) listeners.set(type, new Set());
			listeners.get(type).add(fn);
		},
		removeEventListener(type, fn) {
			listeners.get(type)?.delete(fn);
		},
		count: (type) => listeners.get(type)?.size ?? 0,
		fire(type, e) {
			for (const fn of listeners.get(type) ?? []) fn(e);
		}
	};
}

function keydown(over = {}) {
	return {
		...alt(),
		repeat: false,
		isComposing: false,
		defaultPrevented: false,
		target: { closest: () => null },
		prevented: 0,
		preventDefault() {
			this.prevented += 1;
			this.defaultPrevented = true;
		},
		...over
	};
}

const quietDoc = { querySelectorAll: () => [], querySelector: () => null };

test('the installer toggles on Alt+C and prevents the default only then', () => {
	const t = target();
	let toggles = 0;
	const remove = installCitationHotkey({ onToggle: () => (toggles += 1), target: t, doc: quietDoc });
	assert.equal(t.count('keydown'), 1);

	const hit = keydown();
	t.fire('keydown', hit);
	assert.equal(toggles, 1);
	assert.equal(hit.prevented, 1);

	const other = keydown({ code: 'KeyV', key: 'v' });
	t.fire('keydown', other);
	assert.equal(toggles, 1);
	assert.equal(other.prevented, 0);

	remove();
	assert.equal(t.count('keydown'), 0);
	t.fire('keydown', keydown());
	assert.equal(toggles, 1);
});

test('the installer skips repeats, composition, editable targets, modals and handled keys', () => {
	const t = target();
	let toggles = 0;
	let modal = false;
	const doc = {
		querySelectorAll: () => (modal ? [{ hidden: false, getClientRects: () => [{}] }] : []),
		querySelector: () => null
	};
	installCitationHotkey({ onToggle: () => (toggles += 1), target: t, doc });
	const skipped = [
		keydown({ repeat: true }),
		keydown({ isComposing: true }),
		keydown({ target: { closest: (sel) => (sel.includes('textarea') ? {} : null) } }),
		keydown({ target: { isContentEditable: true } }),
		keydown({ defaultPrevented: true })
	];
	for (const e of skipped) t.fire('keydown', e);
	modal = true;
	const underModal = keydown();
	t.fire('keydown', underModal);
	assert.equal(toggles, 0);
	for (const e of [...skipped.slice(0, 4), underModal]) assert.equal(e.prevented, 0);

	modal = false;
	t.fire('keydown', keydown());
	assert.equal(toggles, 1);
});

test('without a target the installer is a no-op', () => {
	const remove = installCitationHotkey({ onToggle: () => assert.fail('never'), target: null, doc: null });
	assert.equal(typeof remove, 'function');
	remove();
});
