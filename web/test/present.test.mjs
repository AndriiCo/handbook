import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse as parseJs } from 'acorn';
import {
	COPY_MAX,
	GLANCE_ROWS,
	STATUS_DAYS,
	UPDATE_DAYS,
	allItems,
	changedItems,
	cleanValue,
	notConfirmedLabels,
	present,
	publicFacts
} from '../src/lib/public-ui/present.js';
import { factsDisagree } from '../src/lib/public-ui/compare.js';
import { SECTIONS, SLOTS } from '../src/lib/public-ui/slots.js';
import { describeSource, numberPage, numberSources, orderedItems, trustLine } from '../src/lib/public-ui/cite.js';
import { estimate, estimateItem, estimateText, feeModel, money } from '../src/lib/public-ui/fees.js';
import { upcomingClosures, DEFAULT_ZONE } from '../src/lib/public-ui/doc/offices.js';
import { VISIBLE_UPDATES, updateEntries } from '../src/lib/public-ui/doc/updates.js';
import { NOT_CONFIRMED } from '../src/lib/public-ui/doc/items.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const fixture = (name) => JSON.parse(readFileSync(join(HERE, 'fixtures', 'dossier', `${name}.json`), 'utf8'));

const NOW = Date.parse('2026-11-04T18:00:00Z');
const DAY = 86_400_000;

/**
 * R1's field rule on the server (compare.js is its twin): a fact is
 * flagged when another public fact of the authority disagrees with it.
 * The production fixtures carry the pre-R1 topic-level flags.
 */
function withR1Flags(facts) {
	const live = publicFacts(facts);
	return facts.map((f) => ({
		...f,
		conflict: live.some((g) => g !== f && factsDisagree(f.topic, f.statement, g.topic, g.statement))
	}));
}

/** present() over a fixture, the way the page calls it. */
function build(name, { now = NOW, facts = null, evidence = undefined, r1 = true } = {}) {
	const f = fixture(name);
	const plan = f.plan ?? {};
	const raw = facts ?? f.facts?.facts ?? [];
	return present({
		authority: plan.authority ?? f.authority ?? f.row ?? null,
		facts: r1 ? withR1Flags(raw) : raw,
		plan: plan.plan ?? null,
		offices: plan.offices ?? [],
		forms: plan.forms ?? [],
		evidence: evidence ?? plan.evidence ?? [],
		options: f.options ?? null,
		now
	});
}

const RAW_KEY = /\b[a-z][a-z0-9]*_[a-z0-9_]+:\s/;
const DOM_ID = /^[A-Za-z][\w-]*$/;

/**
 * What the page shows on screen without opening anything: the status
 * band, the glance, every section row with its alternatives, office
 * cards (their closures: the next three upcoming, or the undated ones
 * when none is dated), and the first three updates. Exactly these are
 * not shown: closures folded into "More closures", updates beyond three
 * (past closures among them: present() moves a closure whose dates have
 * all passed to the updates), and resolved notices older than ninety
 * days (present() leaves those out).
 */
function visibleFactIds(page, now) {
	const ids = new Set();
	const add = (item) => {
		for (const id of item.factIds) ids.add(id);
		for (const alt of item.alternatives ?? []) add(alt);
	};
	const closures = new Set(page.closures.map((i) => i.id));
	let shownClosures = new Set(closures);
	if (!page.thin) {
		const soon = upcomingClosures(page.closures, new Date(now), DEFAULT_ZONE);
		shownClosures = new Set((soon.next.length > 0 ? soon.next.map((c) => c.item) : soon.undated).map((i) => i.id));
	}
	for (const item of [...page.status.active, ...page.status.resolved, ...page.glance]) add(item);
	for (const s of page.sections) {
		for (const item of [...s.items, ...s.also]) {
			if (closures.has(item.id) && !shownClosures.has(item.id)) continue;
			add(item);
		}
	}
	const entries = updateEntries({ items: page.updates, panels: [] });
	for (const e of entries.slice(0, VISIBLE_UPDATES)) add(e.item);
	return ids;
}

function assertVisible(page, facts, now, { exempt = [] } = {}) {
	const shown = visibleFactIds(page, now);
	const want = publicFacts(facts)
		.map((f) => f.id)
		.filter((id) => typeof id === 'number' && !exempt.includes(id));
	const missing = want.filter((id) => !shown.has(id));
	assert.deepEqual(missing, [], `facts on no visible value: ${missing.join(', ')}`);
	return { shown: want.length - missing.length, total: want.length };
}

// ------------------------------------------------------------ wa-prod

test('wa-prod (R1 flags): the production shape reads as a guide with no false conflicts', () => {
	const page = build('wa-prod');
	assert.equal(page.empty, null);
	assert.equal(page.profile, 'us_state');
	assert.equal(page.thin, false);
	assert.equal(page.header.eyebrow, 'Apostille guide');
	assert.equal(page.header.h1, 'Get an apostille in Washington');
	assert.equal(page.header.lede, 'Issued by the Washington Secretary of State — Apostilles & Authentications.');
	assert.equal(page.header.flag, 'us');
	// The index row lists no official site: the page names none (a cited
	// host is never called the office's site).
	assert.equal(page.header.officialHost, null);
	assert.equal(page.header.officialUrl, null);
	assert.deepEqual([page.header.trust.pages, page.header.trust.otherPages], [0, 3]);
	// R1's /plan row lists it: the header links it, and its pages count
	// as official.
	const listed = present({
		authority: { ...fixture('wa-prod').row, official_url: 'https://www.sos.wa.gov/' },
		facts: withR1Flags(fixture('wa-prod').facts.facts),
		now: NOW
	});
	assert.equal(listed.header.officialHost, 'sos.wa.gov');
	assert.equal(listed.header.officialUrl, 'https://www.sos.wa.gov/');
	assert.deepEqual([listed.header.trust.pages, listed.header.trust.otherPages], [3, 0]);
	assert.deepEqual(
		page.sections.map((s) => [s.id, s.step]),
		[
			['before', 1],
			['prepare', 2],
			['form', 3],
			['fees', 4],
			['send', 5],
			['after', 6],
			['contact', null]
		]
	);
	const items = allItems(page);
	assert.equal(items.filter((i) => i.state === 'conflict').length, 0, 'no conflicts after R1');
	// The two same-day fee statements agree on $150: one row, both cites.
	const sameDay = items.find((i) => i.id === 'fees-same_day');
	assert.deepEqual(sameDay?.factIds, [11, 17]);
	assert.equal(sameDay?.cites.length, 2);
	assert.equal(sameDay?.display, 'We charge a $150 same-day service fee');
	// Two pages name the form differently: worded, never a conflict.
	const form = items.find((i) => i.id === 'forms-form');
	assert.equal(form?.state, 'worded');
	assert.equal(form?.alternatives?.length, 2);
	// Offices by instance from the v1 alias keys.
	assert.deepEqual(
		page.offices.map((o) => [o.instance, o.name]),
		[
			['tumwater', 'Tumwater'],
			['cheney', 'Cheney']
		]
	);
	assert.deepEqual(
		page.routes.map((r) => r.id),
		['mail', 'visit']
	);
	// Values verbatim, a trailing colon trimmed.
	assert.equal(items.find((i) => i.id === 'mailing-enclose')?.display, 'Send all documents with Request form and payment');
	assert.equal(items.find((i) => i.id === 'mailing-usps')?.copy, true);
	assert.deepEqual(assertVisible(page, fixture('wa-prod').facts.facts, NOW), { shown: 14, total: 14 });
});

test('wa-prod: raw keys never reach a display or a label', () => {
	const page = build('wa-prod');
	for (const item of allItems(page)) {
		assert.doesNotMatch(item.display, RAW_KEY, item.id);
		assert.doesNotMatch(item.label, /_/, item.id);
		assert.doesNotMatch(item.label, RAW_KEY, item.id);
	}
	// The records keep the verbatim statements for "How this page was made".
	assert.equal(page.records.length, 14);
	assert.ok(page.records.some((r) => r.statement.startsWith('standard_fee: ')));
});

test('wa-prod (pre-R1 flags): a fact the server flags is always a conflict', () => {
	const f = fixture('wa-prod');
	const page = build('wa-prod', { r1: false });
	const flagged = f.facts.facts.filter((x) => x.conflict).map((x) => x.id);
	for (const id of flagged) {
		const holder = allItems(page).find((i) => i.factIds.includes(id) && !i.id.startsWith('glance-'));
		assert.ok(holder, `#${id} shown`);
		const top = page.sections.flatMap((s) => [...s.items, ...s.also]).find((i) => i.factIds.includes(id));
		assert.equal(top?.state, 'conflict', `#${id}`);
	}
});

// ------------------------------------------------------------ wa-guide

test('wa-guide: the three USPS variants are one conflict with three alternatives', () => {
	const page = build('wa-guide');
	const send = page.sections.find((s) => s.id === 'send');
	const usps = send?.items.find((i) => i.slot === 'mailing.usps');
	assert.ok(usps);
	assert.equal(usps.state, 'conflict');
	assert.equal(usps.copy, false);
	assert.deepEqual(usps.cites, [], 'a disputed value keeps its cites on its alternatives');
	assert.deepEqual([...usps.factIds].sort(), [216, 217, 218]);
	assert.equal(usps.alternatives?.length, 3);
	assert.deepEqual(
		usps.alternatives?.map((a) => a.display),
		['PO BOX 40220 Olympia, WA 98504-0220', 'PO Box 40234, Olympia, WA 98504-0234', 'PO Box 40220, Olympia, WA 98504-0234']
	);
	for (const alt of usps.alternatives ?? []) {
		assert.equal(alt.copy, false);
		assert.equal(alt.factIds.length, 1);
		assert.ok(alt.cites.some((c) => c.source.kind !== 'call' && c.source.kind !== 'decision'), 'cited by a page');
	}
	// The glance never shows a disputed box: it points at the mail step.
	const pointer = page.glance.find((i) => i.slot === 'mailing.usps');
	assert.equal(pointer?.state, 'conflict');
	assert.deepEqual(pointer?.cites, []);
	assert.match(pointer?.display ?? '', /different PO boxes/);
	assert.ok(!page.glance.some((i) => /40220|40234/.test(i.display)));
	// The courier street and the office cities do show.
	assert.equal(page.glance.find((i) => i.slot === 'mailing.courier')?.display, 'Town Center 3, 243 Israel Rd SE, Tumwater, WA 98501');
	assert.deepEqual(
		page.glance.filter((i) => i.slot === 'address.office').map((i) => i.display),
		['Tumwater', 'Cheney']
	);
});

test('wa-guide: offices by instance, closures soonest first, sections numbered', () => {
	const page = build('wa-guide');
	assert.deepEqual(
		page.offices.map((o) => [o.instance, o.items.map((i) => i.slot)]),
		[
			['tumwater', ['address.office', 'hours.weekday']],
			['cheney', ['address.office', 'hours.weekday', 'walkin.same_day_only', 'walkin.no_mail', 'walkin.public_record']]
		]
	);
	assert.equal(page.closures.length, 6);
	assert.deepEqual(
		page.closures.map((i) => i.instance),
		['2026_11_11', '2026_11_13', '2026_11_26', '2026_11_27', '2026_12_11', '2026_12_25']
	);
	assert.deepEqual(
		page.sections.map((s) => [s.id, s.step]),
		[
			['before', 1],
			['prepare', 2],
			['form', 3],
			['fees', 4],
			['send', 5],
			['after', 6],
			['contact', null]
		]
	);
	assert.equal(page.sections.find((s) => s.id === 'send')?.also[0]?.label, 'Traceable');
	assert.deepEqual(
		page.routes,
		[
			{ id: 'mail', label: 'By mail', href: '#send-mail' },
			{ id: 'courier', label: 'By courier', href: '#send-courier' },
			{ id: 'visit', label: 'In person, same day', href: '#visit' }
		]
	);
});

test('wa-guide: a paired notice is a status line, then an update, then hidden', () => {
	const f = fixture('wa-guide');
	const restoredAt = Date.parse('2026-11-03T20:30:00Z');
	// Within seven days of the restore: the resolved status line.
	const now = build('wa-guide');
	assert.deepEqual(now.status.active, []);
	assert.deepEqual(
		now.status.resolved.map((i) => [i.slot, i.instance, i.factIds]),
		[
			['service_notice.notice', '2026_11_02', [248]],
			['service_notice.restored', '2026_11_02', [249]]
		]
	);
	assert.deepEqual(now.status.lastRead, {
		capturedAt: '2026-11-03T20:30:00Z',
		url: 'https://www.sos.wa.gov/corporations-charities/public-notices/online-filing-system-outage'
	});
	assert.deepEqual(now.updates, []);
	// From seven to ninety days: Updates.
	const later = build('wa-guide', { now: restoredAt + (STATUS_DAYS + 1) * DAY });
	assert.deepEqual(later.status.resolved, []);
	assert.deepEqual(
		later.updates.map((i) => i.factIds[0]),
		[248, 249]
	);
	assert.ok(later.status.lastRead);
	// After ninety days: hidden (the one visibility exemption present() makes).
	const old = build('wa-guide', { now: restoredAt + (UPDATE_DAYS + 1) * DAY });
	assert.deepEqual(old.status.resolved, []);
	assert.deepEqual(old.updates.filter((i) => i.slot?.startsWith('service_notice')), []);
	assert.ok(!allItems(old).some((i) => i.factIds.includes(248) || i.factIds.includes(249)));
	// An unpaired notice is active: the banner.
	const unpaired = build('wa-guide', { facts: f.facts.facts.filter((x) => x.id !== 249) });
	assert.deepEqual(
		unpaired.status.active.map((i) => i.factIds),
		[[248]]
	);
	assert.equal(unpaired.status.lastRead, null, 'no pair from the notice log, no last-read line');
	assert.equal(unpaired.order[0], unpaired.status.active[0].id, 'the status band numbers first');
});

test('wa-guide: staff notes become cites on the value they speak to, never a value', () => {
	const page = build('wa-guide');
	const usps = page.sections.find((s) => s.id === 'send')?.items.find((i) => i.slot === 'mailing.usps');
	const box = usps?.alternatives?.find((a) => a.factIds.includes(217));
	const keys = box?.cites.map((c) => c.source.key);
	assert.deepEqual(keys?.filter((k) => k.startsWith('note:')), ['note:7', 'note:8']);
	const call = box?.cites.find((c) => c.source.key === 'note:7');
	assert.equal(call?.source.kind, 'call');
	assert.equal(call?.quote, null);
	assert.match(call?.summary ?? '', /PO Box 40234/);
	assert.equal(call?.source.recorded, false);
	assert.equal(call?.source.dayOnly, true);
	const decision = box?.cites.find((c) => c.source.key === 'note:8');
	assert.equal(decision?.source.kind, 'decision');
	assert.deepEqual(decision?.source.supports, ['note:7']);
	for (const item of allItems(page)) {
		assert.ok(!item.cites.some((c) => c.source.key.startsWith('note:') && item.display === c.summary), item.id);
	}
	// Numbered: the decision is based on the call.
	const { sources } = numberPage(page);
	const callSource = sources.find((s) => s.key === 'note:7');
	const decisionSource = sources.find((s) => s.key === 'note:8');
	assert.ok(callSource && decisionSource);
	assert.ok(decisionSource.basedOn.includes(callSource.n));
	assert.equal(page.header.trust.calls, 1);
	assert.equal(page.header.trust.emails, 0);
	// An absence nothing fills becomes an `absent` row, not a value.
	const absence = { id: 9, kind: 'absence', stance: 'context', slot: 'fees.same_day', date: '2026-11-04', channel: 'FAQ and fee pages', summary: 'Neither page names a same-day fee.' };
	const facts = fixture('wa-guide').facts.facts.filter((x) => x.id !== 212);
	const absent = build('wa-guide', { facts, evidence: [absence] });
	const row = absent.sections.find((s) => s.id === 'fees')?.items.find((i) => i.slot === 'fees.same_day');
	assert.equal(row?.state, 'absent');
	assert.deepEqual(row?.factIds, []);
	assert.deepEqual(row?.cites.map((c) => c.source.key), ['note:9']);
	assert.notEqual(row?.display, absence.summary);
});

test('wa-guide: never public, copy rules, and the records', () => {
	const page = build('wa-guide');
	const items = allItems(page);
	assert.ok(!items.some((i) => i.factIds.includes(250)), 'call scripts never render');
	assert.ok(!items.some((i) => i.factIds.includes(251)), 'proposals never render');
	assert.equal(page.records.length, 49);
	const byId = (id) => items.find((i) => i.id === id);
	assert.equal(byId('payment-payee')?.copy, true);
	assert.equal(byId('mailing-courier')?.copy, true);
	assert.equal(byId('address-office-cheney')?.copy, true);
	assert.equal(byId('fees-per_document')?.copy, false, 'Copy is for payees and addresses only');
	assert.equal(byId('glance-payment-payee')?.copy, true);
	assert.equal(byId('glance-mailing-courier')?.copy, false, 'the glance copies the payee only');
	// A whole sentence never gets Copy.
	const long = fixture('wa-guide').facts.facts.map((x) =>
		x.id === 214 ? { ...x, statement: `payee: ${'Make your check payable to the Secretary of State, '.repeat(3)}` } : x
	);
	const longPage = build('wa-guide', { facts: long });
	const payee = allItems(longPage).find((i) => i.id === 'payment-payee');
	assert.ok((payee?.display.length ?? 0) > COPY_MAX);
	assert.equal(payee?.copy, false);
	// Every id is DOM-safe and unique; every value rests on facts or says why not.
	const ids = items.map((i) => i.id);
	assert.equal(new Set(ids).size, ids.length);
	for (const item of items) {
		assert.match(item.id, DOM_ID);
		assert.ok(['ok', 'conflict', 'worded', 'stale', 'missing', 'uncited', 'computed', 'absent'].includes(item.state), item.id);
		if (item.factIds.length === 0) assert.ok(['missing', 'absent', 'computed'].includes(item.state), item.id);
	}
	assert.equal(new Set(page.order).size, page.order.length);
});

test('wa-guide: the fee rows feed the calculator (D2)', () => {
	const page = build('wa-guide');
	const fees = page.sections.find((s) => s.id === 'fees');
	const model = feeModel(fees?.items ?? []);
	const three = estimate(model, 3, 'expedite');
	assert.equal(three.ask, false);
	if (!three.ask) assert.equal(money(three.totalCents), '$145.00');
	const twelve = estimate(model, 12, 'same_day');
	assert.equal(twelve.ask, false);
	if (!twelve.ask) assert.equal(money(twelve.totalCents), '$430.00');
	assert.equal(estimate(model, 25, 'expedite').ask, true);
	// The estimate is numbered after every real source, with its terms.
	const est = estimateItem(model, 3, 'expedite');
	assert.ok(est);
	if (!est) return;
	assert.equal(estimateText(estimate(model, 3, 'expedite')), est.display);
	assert.deepEqual(
		est.terms?.map((t) => t.text),
		['3 × $15.00', '+ $100.00 expedite fee (1–10 documents)', '= $145.00']
	);
	const before = numberPage(page);
	const after = numberSources([...orderedItems(page), est]);
	const own = after.sources.filter((s) => s.kind !== 'computed');
	assert.deepEqual(
		own.map((s) => [s.key, s.n]),
		before.sources.map((s) => [s.key, s.n]),
		'the estimate never renumbers the office pages'
	);
	assert.equal(after.sources.at(-1)?.kind, 'computed');
});

test('wa-guide: a missing critical value says so, with the office phone', () => {
	const facts = fixture('wa-guide').facts.facts.filter((x) => x.id !== 214);
	const page = build('wa-guide', { facts });
	const payee = page.sections.find((s) => s.id === 'fees')?.items.find((i) => i.slot === 'payment.payee');
	assert.equal(payee?.state, 'missing');
	assert.equal(payee?.display, `${NOT_CONFIRMED}: 360-725-0377`);
	assert.deepEqual(payee?.factIds, [241], 'the number is the phone fact');
	assert.equal(payee?.copy, false);
	assert.deepEqual(notConfirmedLabels(page), ['Payable to']);
	assert.ok(page.sections.some((s) => s.id === 'contact'));
	// A thin page lists nothing as not confirmed.
	assert.deepEqual(notConfirmedLabels(build('vietnam-prod')), []);
	// Without the office's contacts: no number, and the contact step holds
	// only the "Not confirmed yet" list.
	const quiet = fixture('wa-guide').facts.facts.filter((x) => x.topic !== 'contacts');
	const noPhone = build('wa-guide', { facts: quiet.filter((x) => x.id !== 214) });
	const bare = noPhone.sections.find((s) => s.id === 'fees')?.items.find((i) => i.slot === 'payment.payee');
	assert.equal(bare?.display, NOT_CONFIRMED);
	assert.deepEqual(bare?.factIds, []);
	const contact = noPhone.sections.find((s) => s.id === 'contact');
	assert.deepEqual([contact?.items.length, contact?.also.length], [0, 0]);
	assert.deepEqual(notConfirmedLabels(noPhone), ['Payable to']);
	// Every critical value filled and no contacts: no contact step at all
	// (the remote-notarization row is not a "Not confirmed yet" value).
	const full = build('wa-guide', { facts: quiet });
	assert.ok(!full.sections.some((s) => s.id === 'contact'));
	assert.deepEqual(notConfirmedLabels(full), []);
});

/** Fact ids of updates folded into "Older updates" (beyond three). */
const foldedUpdates = (page) =>
	updateEntries({ items: page.updates, panels: [] })
		.slice(VISIBLE_UPDATES)
		.flatMap((e) => e.item.factIds);

test('wa-guide: the visibility invariant, with exactly the screen exemptions', () => {
	const f = fixture('wa-guide');
	const page = build('wa-guide');
	// Three closures are folded into "More closures" (Nov 27, Dec 11, Dec 25).
	const folded = [233, 234, 235];
	assert.throws(() => assertVisible(page, f.facts.facts, NOW));
	assert.deepEqual(assertVisible(page, f.facts.facts, NOW, { exempt: folded }), { shown: 46, total: 46 });
	// Closures already past leave the visit card for Updates, newest
	// first: on Nov 30 the four past ones lead the list, so the oldest of
	// them and the resolved pair fold into "Older updates".
	const late = Date.parse('2026-11-30T18:00:00Z');
	const latePage = build('wa-guide', { now: late });
	assert.deepEqual(
		latePage.updates.map((i) => i.factIds[0]),
		[248, 249, 230, 231, 232, 233]
	);
	assert.deepEqual(latePage.closures.map((i) => i.factIds[0]), [234, 235]);
	assert.deepEqual(foldedUpdates(latePage).sort(), [230, 248, 249]);
	assert.deepEqual(assertVisible(latePage, f.facts.facts, late, { exempt: foldedUpdates(latePage) }), { shown: 46, total: 46 });
	// Every closure the page numbers is one it shows: no numbered item
	// sits outside the sections, the glance, the status band and Updates.
	const placed = new Set(
		[...latePage.status.active, ...latePage.status.resolved, ...latePage.glance, ...latePage.sections.flatMap((s) => [...s.items, ...s.also]), ...latePage.updates].map((i) => i.id)
	);
	assert.deepEqual(orderedItems(latePage).filter((i) => !placed.has(i.id)).map((i) => i.id), []);
	// A resolved notice older than ninety days is the exemption present() makes.
	const old = Date.parse('2027-03-01T18:00:00Z');
	const oldPage = build('wa-guide', { now: old });
	assert.deepEqual(oldPage.closures, []);
	assert.deepEqual(
		assertVisible(oldPage, f.facts.facts, old, { exempt: [...foldedUpdates(oldPage), 248, 249] }),
		{ shown: 44, total: 44 }
	);
	// Updates beyond three fold away.
	const notices = [];
	for (let k = 0; k < 4; k++) {
		const id = `2026_10_0${k + 1}`;
		const at = `2026-10-0${k + 2}T18:00:00Z`;
		const cite = [{ kind: 'web', capture_id: 900 + k, captured_at: at, quote: 'x', source_url: 'https://www.sos.wa.gov/corporations-charities/public-notices/online-filing-system-outage' }];
		notices.push(
			{ id: 300 + 2 * k, topic: 'service_notice', status: 'accepted', statement: `notice__${id}: Outage ${k + 1}.`, citations: cite },
			{ id: 301 + 2 * k, topic: 'service_notice', status: 'accepted', statement: `restored__${id}: Restored ${k + 1}.`, citations: cite }
		);
	}
	const many = [...f.facts.facts, ...notices];
	const manyPage = build('wa-guide', { facts: many });
	assert.equal(manyPage.updates.length, 8);
	const beyond = updateEntries({ items: manyPage.updates, panels: [] })
		.slice(VISIBLE_UPDATES)
		.flatMap((e) => e.item.factIds);
	assert.ok(beyond.length > 0);
	assertVisible(manyPage, many, NOW, { exempt: [...folded, ...beyond] });
});

// ------------------------------------------------------------ vietnam, J1, empty

test('vietnam-prod: thin, the office name as h1, 10 of 10 facts visible', () => {
	const page = build('vietnam-prod');
	assert.equal(page.profile, null);
	assert.equal(page.thin, true);
	assert.equal(page.header.h1, 'Viet Nam — receiving authorities');
	assert.equal(page.header.lede, null);
	assert.deepEqual(page.glance, []);
	assert.deepEqual(page.routes, []);
	assert.ok(page.sections.every((s) => s.step === null));
	assert.deepEqual(
		page.sections.map((s) => s.id),
		['before', 'prepare']
	);
	assert.equal(allItems(page).filter((i) => i.state === 'conflict').length, 0);
	assert.deepEqual(assertVisible(page, fixture('vietnam-prod').facts.facts, NOW), { shown: 10, total: 10 });
	for (const item of allItems(page)) assert.doesNotMatch(item.label, /_/);
	// Before R1 the same shape still shows every fact (flagged, as the server says).
	const pre = build('vietnam-prod', { r1: false });
	assert.deepEqual(assertVisible(pre, fixture('vietnam-prod').facts.facts, NOW), { shown: 10, total: 10 });
});

test('j1: thin, one section named for fees, holding the unkeyed statement', () => {
	const page = build('j1');
	assert.equal(page.thin, true);
	assert.equal(page.header.h1, 'Example Authority');
	const fees = page.sections.filter((s) => /fees/i.test(s.heading));
	assert.equal(fees.length, 1);
	assert.equal(fees[0].heading, 'Pay the fees');
	assert.equal(fees[0].step, null);
	const rows = [...fees[0].items, ...fees[0].also];
	assert.deepEqual(
		rows.map((i) => [i.label, i.display, i.state]),
		[['', 'Eight dollars per document.', 'uncited']]
	);
	assert.ok(page.sections.some((s) => s.also.some((i) => i.display === 'Walk-ins nine to five.')));
});

test('empty: the listed official site, as text, and no values', () => {
	const page = build('empty');
	assert.deepEqual(page.empty, {
		name: 'Oregon Secretary of State — Corporation Division',
		officialHost: 'sos.oregon.gov',
		officialUrl: 'https://sos.oregon.gov/'
	});
	assert.equal(page.header.h1, 'Oregon Secretary of State — Corporation Division');
	assert.equal(page.header.officialHost, null);
	assert.deepEqual(page.sections, []);
	assert.deepEqual(page.glance, []);
	assert.deepEqual(page.order, []);
	assert.deepEqual(present({}).empty, { name: '', officialHost: null, officialUrl: null });
});

test('values are verbatim: whitespace and one trailing colon trimmed, never shortened', () => {
	assert.equal(cleanValue('  Send all documents with Request form and payment:  '), 'Send all documents with Request form and payment');
	assert.equal(cleanValue('Fee: $15.00'), 'Fee: $15.00');
	assert.equal(cleanValue('a\nb'), 'a\nb');
});

test('changedItems: new and changed rows flash, with their labels', () => {
	const f = fixture('wa-guide');
	const before = build('wa-guide');
	const facts = f.facts.facts.map((x) => (x.id === 224 ? { ...x, statement: 'weekday__tumwater: Monday through Friday, 8:00 a.m. to 3:00 p.m.' } : x));
	const after = build('wa-guide', { facts });
	const { ids, labels } = changedItems(before, after);
	assert.ok(ids.includes('hours-weekday-tumwater'));
	assert.deepEqual(labels, ['Opening hours']);
	assert.deepEqual(changedItems(after, after), { ids: [], labels: [] });
});

// ------------------------------------------------------------ copy rules

test('template copy: no digits or $ in present.js strings, slots.json labels or headings', () => {
	const src = readFileSync(join(HERE, '..', 'src', 'lib', 'public-ui', 'present.js'), 'utf8');
	const ast = parseJs(src, { ecmaVersion: 'latest', sourceType: 'module' });
	const strings = [];
	const visit = (node) => {
		if (!node || typeof node !== 'object') return;
		if (Array.isArray(node)) {
			for (const n of node) visit(n);
			return;
		}
		if (node.type === 'ImportDeclaration') return;
		if (node.type === 'Literal' && typeof node.value === 'string' && !node.regex) strings.push(node.value);
		if (node.type === 'TemplateElement') strings.push(node.value.cooked ?? node.value.raw);
		for (const [key, value] of Object.entries(node)) {
			if (key !== 'start' && key !== 'end') visit(value);
		}
	};
	visit(ast);
	assert.ok(strings.length > 50, `only ${strings.length} strings read`);
	const BANNED = /\b(?:dossier|slot|topic|fact|skeleton|lorem|todo|coming soon)\b/i;
	for (const text of strings) {
		assert.doesNotMatch(text, /[\d$]/, `present.js: ${JSON.stringify(text)}`);
		if (text.trim().split(/\s+/).length >= 2) assert.doesNotMatch(text, BANNED, `present.js: ${JSON.stringify(text)}`);
	}
	for (const s of SLOTS) {
		assert.doesNotMatch(s.label, /[\d$]/, s.id);
		assert.doesNotMatch(s.label, BANNED, s.id);
	}
	for (const s of SECTIONS) {
		if (s.heading) assert.doesNotMatch(s.heading, /[\d$]|\bfact\b|\bslot\b|\btopic\b|\bdossier\b/i, s.id);
	}
	for (const row of GLANCE_ROWS) {
		assert.doesNotMatch(row.label, /[\d$]/);
		for (const part of row.parts) if (part.label) assert.doesNotMatch(part.label, /[\d$]/);
	}
});

test('present() template copy stays out of the page above the records', () => {
	const page = build('wa-guide');
	const copy = [
		page.header.eyebrow,
		page.header.h1,
		page.header.lede ?? '',
		...allItems(page).map((i) => i.label),
		...allItems(page)
			.filter((i) => i.factIds.length === 0 || i.state === 'missing')
			.map((i) => i.display)
	];
	for (const text of copy) assert.doesNotMatch(text, /\b(?:dossier|slot|topic|fact)\b/i, text);
});

// ------------------------------------------------------------ R2 review fixes

const WA_ROW = fixture('wa-guide').authority;

/** A public fact with one web citation of its own page. */
function fact(id, topic, statement, over = {}) {
	return {
		id,
		topic,
		status: 'accepted',
		statement,
		accepted_at: 1793750400 + id,
		citations: [
			{
				capture_id: 900 + id,
				captured_at: '2026-11-02T18:00:00Z',
				kind: 'web',
				quote: statement.replace(/^[a-z_]+(?:__[a-z0-9_]+)?: /, ''),
				source_url: `https://www.sos.wa.gov/corporations-charities/page-${id}`
			}
		],
		...over
	};
}

/**
 * R1's public read: `conflict` and `conflicts[]` (the accepted siblings
 * that disagree under the field rule, cross-key USPS rule included).
 */
function withSiblings(facts) {
	const live = publicFacts(facts);
	return facts.map((f) => {
		const conflicts = live
			.filter((g) => g !== f && factsDisagree(f.topic, f.statement, g.topic, g.statement))
			.map((g) => ({ id: g.id, topic: g.topic, statement: g.statement, status: 'accepted' }));
		return { ...f, conflict: conflicts.length > 0, conflicts };
	});
}

/** The section Item of a slot (and instance), never a glance copy. */
const itemOf = (page, slot, instance = null) =>
	page.sections.flatMap((s) => [...s.items, ...s.also]).find((i) => i.slot === slot && (instance === null || i.instance === instance));

test('clusters: a vaguer value never merges two values that disagree (box ZIPs, hours)', () => {
	const facts = withSiblings([
		fact(10, 'mailing', 'apostille_mailing_address: PO Box 40220, Olympia, WA 98504-0220', { accepted_at: 100 }),
		fact(11, 'mailing', 'apostille_mailing_address: PO Box 40220, Olympia, WA 98504-0234', { accepted_at: 101 }),
		// Newest, so the preferred one: it agrees with both.
		fact(12, 'mailing', 'apostille_mailing_address: PO Box 40220, Olympia, WA', { accepted_at: 102 }),
		fact(20, 'hours', 'weekday__tumwater: Monday through Friday', { accepted_at: 202 }),
		fact(21, 'hours', 'weekday__tumwater: Monday through Friday, 8 a.m. to 4 p.m.', { accepted_at: 200 }),
		fact(22, 'hours', 'weekday__tumwater: Monday through Friday, 8 a.m. to 5 p.m.', { accepted_at: 201 })
	]);
	assert.deepEqual(
		facts.map((f) => [f.id, f.conflict]),
		[[10, true], [11, true], [12, false], [20, false], [21, true], [22, true]]
	);
	const page = present({ authority: WA_ROW, facts, now: NOW });
	const usps = itemOf(page, 'mailing.usps');
	assert.equal(usps.state, 'conflict');
	assert.deepEqual(usps.alternatives.map((a) => a.factIds).sort(), [[10], [11], [12]]);
	assert.ok(usps.alternatives.some((a) => a.display.endsWith('98504-0220')));
	assert.ok(usps.alternatives.some((a) => a.display.endsWith('98504-0234')));
	const hours = itemOf(page, 'hours.weekday', 'tumwater');
	assert.equal(hours.state, 'conflict');
	assert.deepEqual(
		hours.alternatives.map((a) => a.display).sort(),
		['Monday through Friday', 'Monday through Friday, 8 a.m. to 4 p.m.', 'Monday through Friday, 8 a.m. to 5 p.m.']
	);
	// Values that agree still merge, with every cite.
	const merged = present({
		authority: WA_ROW,
		facts: withSiblings([
			fact(30, 'mailing', 'apostille_mailing_address: PO Box 40234, Olympia, WA 98504-0234'),
			fact(31, 'mailing', 'apostille_mailing_address: PO Box 40234, Olympia, WA')
		]),
		now: NOW
	});
	const one = itemOf(merged, 'mailing.usps');
	assert.deepEqual([one.state, one.factIds.sort(), one.cites.length], ['ok', [30, 31], 2]);
	for (const p of [page, merged]) {
		assert.ok(allItems(p).every((i) => i.state !== 'conflict' || (i.alternatives?.length ?? 0) >= 2), 'a conflict lists two values or more');
	}
});

test('a fact the server flags with one value lists the server-named sibling (cross-key USPS rule)', () => {
	const facts = withSiblings([
		fact(216, 'mailing', 'apostille_mailing_address: PO BOX 40220 Olympia, WA 98504-0220'),
		fact(239, 'mailing', 'return: Returned from PO Box 40228, Olympia, WA 98504-0228')
	]);
	assert.deepEqual(facts.map((f) => [f.id, f.conflict, f.conflicts.map((c) => c.id)]), [[216, true, [239]], [239, true, [216]]]);
	const page = present({ authority: WA_ROW, facts, now: NOW });
	const usps = itemOf(page, 'mailing.usps');
	assert.equal(usps.state, 'conflict');
	assert.deepEqual(usps.alternatives.map((a) => [a.factIds, a.label]), [[[216], 'Mailing address'], [[239], 'How it comes back']]);
	assert.ok(usps.alternatives.every((a) => a.copy === false));
	const back = itemOf(page, 'mailing.return');
	assert.equal(back.state, 'conflict');
	assert.deepEqual(back.alternatives.map((a) => [a.factIds, a.label]), [[[239], 'How it comes back'], [[216], 'Mailing address']]);
	// On a full page the glance points to the step; it never names one box.
	const full = present({
		authority: WA_ROW,
		facts: withSiblings(
			fixture('wa-guide')
				.facts.facts.filter((x) => x.id !== 217 && x.id !== 218)
				.map((x) => (x.id === 239 ? { ...x, statement: 'return: Returned from PO Box 40228, Olympia, WA 98504-0228' } : x))
		),
		now: NOW
	});
	assert.equal(full.thin, false);
	const send = full.glance.find((i) => i.slot === 'mailing.usps');
	assert.equal(send.display, "The office's pages list different PO boxes — read the mailing step first");
	assert.deepEqual(itemOf(full, 'mailing.usps').alternatives.map((a) => a.factIds), [[216], [239]]);
	// Flagged, but no other value on the page: the value alone, no Copy.
	const lone = present({
		authority: WA_ROW,
		facts: [{ ...fact(217, 'mailing', 'apostille_mailing_address: PO Box 40234, Olympia, WA 98504-0234'), conflict: true, conflicts: [] }],
		now: NOW
	});
	const alone = itemOf(lone, 'mailing.usps');
	assert.deepEqual([alone.state, alone.copy, alone.alternatives], ['ok', false, null]);
});

test('a staff note that contradicts a value contests it: a visible line, no Copy, no calculator, no paid gate', () => {
	const note = {
		id: 9,
		kind: 'call',
		stance: 'contradicts',
		fact_id: 210,
		slot: 'fees.per_document',
		date: '2026-11-04',
		channel: 'Apostille desk',
		summary: 'The desk said the fee went up to $20.00 per document on November 1.',
		status: 'published'
	};
	const page = build('wa-guide', { evidence: [note] });
	const fees = page.sections.find((s) => s.id === 'fees');
	const fee = fees.items.find((i) => i.slot === 'fees.per_document');
	assert.equal(fee.state, 'contested');
	assert.equal(fee.copy, false);
	assert.deepEqual(fee.contest, [{ id: 9, kind: 'call', date: '2026-11-04', summary: note.summary }]);
	assert.equal(fee.display, '$15.00 per document', 'the value stays verbatim');
	// The calculator asks the office for every speed; the estimate is no input.
	const model = feeModel(fees.items);
	for (const speed of ['standard', 'expedite', 'same_day']) assert.deepEqual(estimate(model, 3, speed), { ask: true, reason: 'contested' });
	assert.equal(estimateItem(model, 1, 'standard'), null);
	// The glance points to the step instead of the amount.
	const cost = page.glance.find((i) => i.slot === 'fees.per_document');
	assert.equal(cost.display, 'The office told us something different — read the step below first');
	assert.deepEqual([cost.cites, cost.copy], [[], false]);
	// The card and the endnote say the note contests the value.
	const src = numberPage(page).sources.find((x) => x.key === 'note:9');
	assert.equal(src.stance, 'contradicts');
	assert.match(describeSource(src).lead, /contradicts the value shown/);
	assert.match(describeSource(src).head, /contradicts the value shown/);
	// A confirming note leaves the value as it was.
	const confirmed = build('wa-guide', { evidence: [{ ...note, stance: 'confirms' }] });
	const ok = confirmed.sections.find((s) => s.id === 'fees').items.find((i) => i.slot === 'fees.per_document');
	assert.deepEqual([ok.state, ok.contest ?? null], ['ok', null]);
});

test('the header names an official site only from the listed URL (Viet Nam lists none)', () => {
	const page = build('vietnam-prod');
	assert.equal(page.header.officialHost, null);
	assert.equal(page.header.officialUrl, null);
	assert.equal(page.header.trust.pages, 0);
	assert.equal(page.header.trust.otherPages, 2);
	assert.equal(trustLine(page.header.trust, { timeZone: 'America/Los_Angeles', now: NOW }), 'Sources captured Nov 1 · 2 web pages');
	// A listed site the page never cites is still the office's own.
	const listed = present({
		authority: { ...fixture('vietnam-prod').row, official_url: 'https://lanhsuvietnam.gov.vn/' },
		facts: withR1Flags(fixture('vietnam-prod').facts.facts),
		now: NOW
	});
	assert.equal(listed.header.officialHost, 'lanhsuvietnam.gov.vn');
	assert.equal(listed.header.trust.pages, 0);
	// An http listing is never linked.
	const plain = present({ authority: { ...WA_ROW, official_url: 'http://www.sos.wa.gov/' }, facts: fixture('wa-guide').facts.facts, now: NOW });
	assert.equal(plain.header.officialHost, null);
	// The lede never puts "the" before a country's name.
	assert.equal(build('wa-guide').header.lede, 'Issued by the Washington Secretary of State — Apostilles & Authentications.');
});

test('the glance and the route cards follow the send step\'s D3 rules', () => {
	const f = fixture('wa-guide');
	const facts = f.facts.facts
		.filter((x) => x.id !== 217 && x.id !== 218)
		.map((x) =>
			x.id === 216
				? { ...x, statement: 'apostille_mailing_address: 801 Capitol Way S, Olympia, WA 98501' }
				: x.id === 219
					? { ...x, statement: 'courier: PO Box 40234, Olympia, WA 98504-0234' }
					: x
		);
	const page = build('wa-guide', { facts });
	assert.equal(page.thin, false);
	assert.deepEqual(page.glance.filter((i) => i.slot === 'mailing.usps' || i.slot === 'mailing.courier'), []);
	assert.deepEqual(page.routes.map((r) => r.id), ['visit']);
	// Undisputed, a PO box by mail and a street by courier are shown.
	const normal = build('wa-guide', { facts: f.facts.facts.filter((x) => x.id !== 216 && x.id !== 218) });
	assert.deepEqual(
		normal.glance.filter((i) => i.slot === 'mailing.usps' || i.slot === 'mailing.courier').map((i) => i.display),
		['PO Box 40234, Olympia, WA 98504-0234', 'Town Center 3, 243 Israel Rd SE, Tumwater, WA 98501']
	);
	assert.deepEqual(normal.routes.map((r) => r.id), ['mail', 'courier', 'visit']);
});
