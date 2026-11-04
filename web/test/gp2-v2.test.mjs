// GP2-page v2 (xtask/ops/gp2-page.mjs): the in-page predicate against a
// small fake DOM, the raw-key rule, and the row's problem lines.
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {
	RAW_KEY,
	RAW_KEY_ANY,
	gp2PageProblems,
	hasRawKey,
	probeValueNodes,
	publicFacts,
	rawKeyLines
} from '../../xtask/ops/gp2-page.mjs';

const SHOWN = { display: 'inline', visibility: 'visible', opacity: '1', overflow: 'visible' };

/**
 * One fake element. `rect` is its client rect, `style` its computed
 * style, `seen` what checkVisibility() answers (omitted: no such
 * method, as in an older engine).
 */
function el({ ids, text = 'value', rect = { left: 10, top: 10, width: 120, height: 20 }, style = {}, seen, parent = null }) {
	const r = { ...rect, right: rect.left + rect.width, bottom: rect.top + rect.height };
	return {
		getAttribute: (name) => (name === 'data-fact-ids' ? ids : null),
		getBoundingClientRect: () => r,
		...(seen === undefined ? {} : { checkVisibility: () => seen }),
		innerText: text,
		parentElement: parent,
		style: { ...SHOWN, ...style }
	};
}

/**
 * A fake window holding `nodes` as its `[data-fact-ids]` elements;
 * `all` answers `querySelectorAll('*')` and `text()` is body.innerText.
 */
function windowOf(nodes, scroll = { x: 0, y: 0 }, { all = nodes, text = () => '' } = {}) {
	const body = {
		style: SHOWN,
		getBoundingClientRect: () => ({ width: 390, height: 900 }),
		get innerText() {
			return text();
		}
	};
	return {
		document: {
			body,
			documentElement: {},
			querySelectorAll: (selector) => (selector === '[data-fact-ids]' ? nodes : selector === '*' ? all : [])
		},
		getComputedStyle: (node) => node.style,
		scrollX: scroll.x,
		scrollY: scroll.y
	};
}

const facts = (n, over = {}) =>
	Array.from({ length: n }, (_, i) => ({ id: i + 1, topic: 'fees', status: 'accepted', ...over }));

test('publicFacts keeps accepted and status-less facts, drops call scripts', () => {
	const kept = publicFacts([
		{ id: 1, topic: 'fees', status: 'accepted' },
		{ id: 2, topic: 'fees' },
		{ id: 3, topic: 'phone_script', status: 'accepted' },
		{ id: 4, topic: 'fees', status: 'proposed' },
		null
	]);
	assert.deepEqual(kept.map((f) => f.id), [1, 2]);
	assert.deepEqual(publicFacts(undefined), []);
});

test('probe: visible value nodes give their ids, merged rows give every id', () => {
	const probe = probeValueNodes(undefined, windowOf([el({ ids: '1' }), el({ ids: '2  3' })], undefined, { text: () => 'Fee per document' }));
	assert.deepEqual(probe, { nodes: 2, visible: 2, ids: ['1', '2', '3'], empty: [], revealed: 0, text: 'Fee per document' });
});

test('probe: content-visibility:auto subtrees render before text is read; hidden ones stay hidden', () => {
	// Chromium: an off-screen auto subtree still measures, but its text is
	// missing from innerText (the node reads empty, body text omits it).
	const box = (contentVisibility, onSet) => ({
		style: {
			...SHOWN,
			display: 'block',
			contentVisibility,
			setProperty(name, value, priority) {
				onSet?.();
				if (name === 'content-visibility' && priority === 'important') this.contentVisibility = value;
			}
		},
		parentElement: null,
		getBoundingClientRect: () => ({ width: 390, height: 500 })
	});
	const auto = box('auto');
	const hidden = box('hidden', () => assert.fail('content-visibility:hidden must stay hidden'));
	const skipped = () => auto.style.contentVisibility === 'auto';
	const far = { ...el({ ids: '10', parent: auto }), get innerText() { return skipped() ? '' : 'value far below'; } };
	const closed = el({ ids: '12', parent: hidden, seen: false, text: 'records' });
	const env = windowOf([far, closed], undefined, {
		all: [auto, far, hidden, closed],
		text: () => (skipped() ? 'top' : 'top\nstandard_fee: $15.00\nvalue far below')
	});
	const probe = probeValueNodes(undefined, env);
	assert.equal(probe.revealed, 1);
	assert.equal(hidden.style.contentVisibility, 'hidden');
	assert.deepEqual(probe.ids, ['10']);
	assert.deepEqual(probe.empty, [], 'the rendered value has its text');
	assert.equal(probe.text, 'top\nstandard_fee: $15.00\nvalue far below');
	const out = gp2PageProblems({ label: '390', probe, text: probe.text, facts: facts(1, { id: 10 }) });
	assert.deepEqual(out.problems, ['390: raw key text visible (1 line(s)): "standard_fee: $15.00"']);
});

test('probe: hidden value nodes do not count', () => {
	const collapsed = { style: { ...SHOWN, display: 'block', overflow: 'hidden' }, parentElement: null, getBoundingClientRect: () => ({ width: 300, height: 0 }) };
	const faded = { style: { ...SHOWN, opacity: '0' }, parentElement: null, getBoundingClientRect: () => ({ width: 300, height: 40 }) };
	const nodes = [
		el({ ids: '1', rect: { left: 0, top: 0, width: 0, height: 0 } }), // display:none, closed <details>
		el({ ids: '2', rect: { left: 0, top: 0, width: 1, height: 1 } }), // sr-only clip
		el({ ids: '3', style: { visibility: 'hidden' } }),
		el({ ids: '4', style: { visibility: 'collapse' } }),
		el({ ids: '5', style: { opacity: '0' } }),
		el({ ids: '6', seen: false }), // checkVisibility: closed <details>, ancestor opacity
		el({ ids: '7', rect: { left: -10000, top: 10, width: 120, height: 20 } }), // pushed off the page
		el({ ids: '8', parent: collapsed }),
		el({ ids: '9', parent: faded }),
		el({ ids: '10', style: { display: 'none' } }),
		el({ ids: '11', seen: true }) // the one that shows
	];
	const probe = probeValueNodes(undefined, windowOf(nodes));
	assert.equal(probe.nodes, 11);
	assert.equal(probe.visible, 1);
	assert.deepEqual(probe.ids, ['11']);
});

test('probe: a node below the fold is visible; scrolled-past nodes are on the page', () => {
	const below = el({ ids: '1', rect: { left: 10, top: 4000, width: 120, height: 20 } });
	const above = el({ ids: '2', rect: { left: 10, top: -900, width: 120, height: 20 } });
	const probe = probeValueNodes(undefined, windowOf([below, above], { x: 0, y: 1200 }));
	assert.deepEqual(probe.ids, ['1', '2']);
});

test('probe: a visible node without text is reported', () => {
	const probe = probeValueNodes(undefined, windowOf([el({ ids: '4 5', text: '  \n ' }), el({ ids: '', text: '' })]));
	assert.deepEqual(probe.empty, ['4 5', '(none)']);
	assert.deepEqual(probe.ids, ['4', '5']);
});

test('probe is self-contained, as page.evaluate needs (no module scope)', () => {
	// A fresh realm whose only globals are the fake window's: any free
	// reference to this module would throw here, as it would in the page.
	const sandbox = vm.createContext({ ...windowOf([el({ ids: '7' })], undefined, { text: () => 'Fee' }) });
	const probe = vm.runInContext(`(${probeValueNodes.toString()})(undefined)`, sandbox);
	assert.deepEqual([...probe.ids], ['7']);
	assert.equal(probe.text, 'Fee');
});

test('RAW_KEY is the operator rule: a lowercase key, a colon, whitespace, at a line start', () => {
	for (const line of ['standard_fee: $15.00', '  apostille_main_branch_street: 243 Israel Rd SE', 'site_title:\tTrang chủ', 'a: b']) {
		assert.ok(RAW_KEY.test(`Header\n${line}\nFooter`), line);
	}
	for (const line of ['Fee per document $15.00', 'Note: ask the office', 'https://www.sos.wa.gov/', '10:30 a.m. cut-off', 'Fee: $15', '#14 · standard_fee: $15', 'fees:$15']) {
		assert.ok(!RAW_KEY.test(`Header\n${line}\nFooter`), line);
	}
	assert.deepEqual(rawKeyLines('Pay\n  standard_fee: $15\nok\nper_document: $8'), ['standard_fee: $15', 'per_document: $8']);
});

test('gp2PageProblems: min(8, n) facts must sit on visible values', () => {
	const ten = facts(10);
	const ids = (n) => Array.from({ length: n }, (_, i) => String(i + 1));
	const pass = gp2PageProblems({ label: '1440', probe: { ids: ids(8), empty: [] }, text: 'Fee per document', facts: ten });
	assert.deepEqual(pass, { problems: [], shown: 8, want: 8, total: 10 });

	const short = gp2PageProblems({ label: '390', probe: { ids: ids(7), empty: [] }, text: '', facts: ten });
	assert.deepEqual(short.problems, ['390: 7/8 facts on a visible [data-fact-ids] value (missing #8 #9 #10)']);

	// Fewer than 8 public facts: every one of them must show.
	const three = [...facts(3), { id: 9, topic: 'phone_script', status: 'accepted' }];
	assert.deepEqual(gp2PageProblems({ label: '1440', probe: { ids: ['1', '2', '3'], empty: [] }, text: '', facts: three }).problems, []);
	assert.equal(gp2PageProblems({ label: '1440', probe: { ids: ['1', '2'], empty: [] }, text: '', facts: three }).problems.length, 1);

	assert.deepEqual(gp2PageProblems({ label: '1440', probe: { ids: [], empty: [] }, text: '', facts: [] }).problems, ['no accepted facts to show']);
});

test('gp2PageProblems: raw keys and empty value nodes are problems', () => {
	const out = gp2PageProblems({
		label: '1440',
		probe: { ids: ['1'], empty: ['2'] },
		text: 'Apostille guide\nsame_day_inperson_fee: Same day services in-person\n',
		facts: facts(1)
	});
	assert.deepEqual(out.problems, [
		'1440: raw key text visible (1 line(s)): "same_day_inperson_fee: Same day services in-person"',
		'1440: 1 visible [data-fact-ids] node(s) without text (ids 2)'
	]);
});

test('raw keys after other text on a line are problems too (plan §2 rule)', () => {
	// A label and a key on one line; an open "How this page was made"
	// records list ("#id · key: value").
	const text = [
		'Apostille guide',
		'Fee per document · standard_fee: $15.00',
		'#20 · authorized_issuer: Apostille Certificates can only be issued by the Ministry',
		'#21 · e_apostille_definition: An e-Apostille is an apostille issued electronically',
		'Closed: Monday',
		'Note at 10:30: the line is open'
	].join('\n');
	assert.equal(RAW_KEY.test(text), false, 'none starts a line');
	assert.equal(RAW_KEY_ANY.test(text), true);
	assert.equal(hasRawKey(text), true);
	assert.deepEqual(rawKeyLines(text), [
		'Fee per document · standard_fee: $15.00',
		'#20 · authorized_issuer: Apostille Certificates can only be issued by the Ministry',
		'#21 · e_apostille_definition: An e-Apostille is an apostille issued electronically'
	]);
	const out = gp2PageProblems({ label: '1440', probe: { ids: ['1'], empty: [] }, text, facts: facts(1) });
	assert.deepEqual(out.problems, [
		'1440: raw key text visible (3 line(s)): "Fee per document · standard_fee: $15.00", "#20 · authorized_issuer: Apostille Certificates can only be ", "#21 · e_apostille_definition: An e-Apostille is an apostille"'
	]);
	// Plain sentences with colons, and a word before a colon mid-line, pass.
	assert.equal(hasRawKey('Closed: Monday\nCall: 360-725-0377\nOpen now · closes 4:00 p.m.'), false);
});
