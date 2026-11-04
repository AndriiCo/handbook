#!/usr/bin/env node
// Washington data metrics for the apostille guide (plan sections 2 and 9):
// coverage of docs/test-plans/wa-checklist.json, false conflicts, closures
// known and capture freshness, read from the public API. Read-only: two
// GETs (/facts and /plan), nothing written.
//
//   node web/scripts/dossier-metrics.mjs                  # production
//   node web/scripts/dossier-metrics.mjs --base http://127.0.0.1:18318/apostille
//   node web/scripts/dossier-metrics.mjs --json           # the numbers as JSON
//
// Options: --base <site base> (default https://andrii.ai/apostille),
// --slug <authority> (default: the checklist's), --checklist <path>.
//
// A fact fills a checklist slot by its statement key, `field[__instance]:
// value` (guide.js grammar):
//   1. by the registry: R1's web/src/lib/public-ui/slots.js (slots.json)
//      when it is in the tree, else the small ALIASES map below;
//   2. otherwise by the canonical convention `{topic}.{field}`, which is
//      how slots.json spells every canonical slot id.
// R1's slots.json replaces ALIASES: delete the map once it lands.
//
// Only what the page shows from cited facts counts. A value's `plan`
// path covers it only for staff-accepted /plan data (form maps); a
// `shown` path (the authority row, which is pack data) is reported as
// "shown, not cited" and never counted. A closure date is known from a
// fact's instance or from a date its statement's value names, never
// from a date that appears only in a citation quote.

import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseStatement } from '../src/lib/public-ui/guide.js';

const ROOT = fileURLToPath(new URL('../..', import.meta.url));
const DEFAULT_BASE = 'https://andrii.ai/apostille';
const CHECKLIST = 'docs/test-plans/wa-checklist.json';

// Alias keys only, per topic: production's v1 keys and the judge-seed
// keys, mirroring R1's draft slots.json. Canonical keys need no entry
// (the convention maps them). [slot, instance] when the alias implies
// the office.
/** @type {Record<string, Record<string, string | [string, string]>>} */
export const ALIASES = {
	fees: {
		standard_fee: 'fees.per_document',
		standard_authentication_fee: 'fees.per_document',
		apostille_fee: 'fees.per_document',
		fee_amount: 'fees.per_document',
		expedited_fee: 'fees.expedite',
		expedited_handling: 'fees.expedite',
		expedited_note: 'fees.expedite',
		same_day_fee: 'fees.same_day',
		same_day_inperson_fee: 'fees.same_day'
	},
	payment: { accepted_methods: 'payment.payee', cash_policy: 'payment.methods_counter' },
	mailing: {
		apostille_mailing_address: 'mailing.usps',
		mail_instruction: 'mailing.usps',
		instruction: 'mailing.usps',
		mail_destination: 'mailing.usps',
		destination: 'mailing.usps',
		submission_requirement: 'mailing.enclose',
		return_postage_requirement: 'mailing.return'
	},
	po_box: { po_box: 'mailing.usps', po_box_address: 'mailing.usps' },
	address: {
		apostille_main_branch_street: ['address.office', 'tumwater'],
		apostille_eastern_branch_street: ['address.office', 'cheney'],
		location: 'address.office',
		address: 'address.office',
		office_address: 'address.office'
	},
	walkin: { location: 'address.office', counter_service_cutoff: 'walkin.cutoff' },
	hours: {
		counter_hours: 'hours.weekday',
		walkin_hours: 'hours.weekday',
		hours: 'hours.weekday',
		counter_cutoff: 'walkin.cutoff',
		phone_hours: 'contacts.phone_hours'
	},
	holiday_hours: { holiday_closure: 'holiday_hours.closure', early_closure: 'holiday_hours.closure' },
	forms: {
		request_form: 'forms.form',
		request_form_name: 'forms.form',
		required_form: 'forms.form',
		current_form: 'forms.form',
		current_form_instruction: 'forms.form',
		superseded_revision: 'forms.revision',
		obsolete_version_status: 'forms.revision',
		required_enclosure: 'mailing.enclose'
	},
	processing_time: { standard_time: 'processing_time.standard', complete_applications: 'processing_time.standard' },
	contacts: { apostille_phone: 'contacts.phone', authentication_questions_phone: 'contacts.phone' },
	authentication: {
		certificate_type: 'authentication.product',
		original_return: 'mailing.return',
		return_policy: 'mailing.return'
	},
	copies: { retention_and_return: 'mailing.return', copy_retention_and_return: 'mailing.return' },
	jurisdiction: { federal_scope: 'jurisdiction.federal_route', federal_issuer_status: 'jurisdiction.federal_route' },
	notarization: {
		notarization: 'notarization.wa_notary',
		seal_requirement: 'notarization.wa_notary',
		requirement: 'notarization.wa_notary',
		private_documents: 'notarization.wa_notary',
		separate_certificate: 'notarization.wa_notary'
	},
	rejections: { not_accepted: 'jurisdiction.refusals' }
};

/** An instance after `__`: words or digits joined by `_` (`cheney`, `2026_11_11`). */
const INSTANCE = /^[a-z0-9]+(?:_[a-z0-9]+)*$/;

/** Topics whose box-carrying facts conflict across keys (plan section 12). */
const USPS_TOPICS = new Set(['mailing', 'po_box']);

/**
 * `weekday__cheney` → `{ base: 'weekday', instance: 'cheney' }`.
 * @param {string} key
 */
export function splitKey(key) {
	const k = String(key ?? '');
	const at = k.indexOf('__');
	if (at > 0 && INSTANCE.test(k.slice(at + 2))) return { base: k.slice(0, at), instance: k.slice(at + 2) };
	return { base: k, instance: null };
}

/**
 * The embedded registry: ALIASES, exact key first, then the key's base
 * with its `__instance` (an explicit suffix wins over the alias's).
 * @param {string} topic
 * @param {string} key
 * @returns {{ slot: string, instance: string|null, via: string } | null}
 */
export function aliasResolver(topic, key) {
	const table = Object.hasOwn(ALIASES, topic) ? ALIASES[topic] : null;
	if (!table) return null;
	const pick = (entry, instance) => {
		const [slot, implied] = Array.isArray(entry) ? entry : [entry, null];
		return { slot, instance: instance ?? implied, via: 'registry' };
	};
	if (Object.hasOwn(table, key)) return pick(table[key], null);
	const { base, instance } = splitKey(key);
	if (instance !== null && Object.hasOwn(table, base)) return pick(table[base], instance);
	return null;
}

/**
 * A fact key's slot: the registry, else `{topic}.{base}`.
 * @param {string} topic
 * @param {string} key
 * @param {(topic: string, key: string) => ({ slot: string, instance: string|null, via: string } | null)} registry
 */
export function resolveFact(topic, key, registry = aliasResolver) {
	const hit = registry(topic, key);
	if (hit) return hit;
	const { base, instance } = splitKey(key);
	return { slot: `${topic}.${base}`, instance, via: 'convention' };
}

/**
 * Facts on the page: accepted (no status counts as accepted) and not
 * call scripts.
 * @param {unknown} facts
 * @returns {any[]}
 */
export function publicFacts(facts) {
	return (Array.isArray(facts) ? facts : []).filter(
		(f) => f && typeof f === 'object' && (f.status ?? 'accepted') === 'accepted' && f.topic !== 'phone_script'
	);
}

const keyOf = (statement) => parseStatement(String(statement ?? '')).fieldKey;

/** PO-box numbers and ZIP+4 codes in a statement. */
function boxTokens(text) {
	const s = String(text ?? '');
	return {
		boxes: new Set([...s.matchAll(/\bP\.?\s*O\.?\s*BOX\s+(\d{3,6})\b/gi)].map((m) => m[1])),
		zips: new Set(s.match(/\b\d{5}-\d{4}\b/g) ?? [])
	};
}

const sameSet = (a, b) => a.size === b.size && [...a].every((x) => b.has(x));

/**
 * Two postal statements disagree when both name a box and the boxes, or
 * both ZIP+4 codes, differ (plan section 12, cross-key USPS rule).
 */
export function uspsDisagree(a, b) {
	const x = boxTokens(a);
	const y = boxTokens(b);
	if (!x.boxes.size || !y.boxes.size) return false;
	if (!sameSet(x.boxes, y.boxes)) return true;
	return x.zips.size > 0 && y.zips.size > 0 && !sameSet(x.zips, y.zips);
}

/**
 * Sort the server's conflict flags. A flagged pair is real when the two
 * keys are equal (`__instance` included), either side is unkeyed (legacy
 * pins), or both are postal statements whose boxes or ZIP+4 differ;
 * otherwise the server compared different fields: a false conflict. A
 * flag that lists no sibling cannot be checked and counts as real.
 * @param {any[]} facts public facts
 */
export function classifyConflicts(facts) {
	/** @type {Map<number, { real: number[], false: number[] }>} */
	const byFact = new Map();
	/** @type {Map<string, { a: number, b: number, keyA: string|null, keyB: string|null, topic: string, real: boolean }>} */
	const pairs = new Map();
	for (const f of facts) {
		if (f?.conflict !== true) continue;
		const entry = { real: [], false: [] };
		const mine = keyOf(f.statement);
		const siblings = Array.isArray(f.conflicts) ? f.conflicts : [];
		if (!siblings.length) entry.real.push(Number.NaN);
		for (const s of siblings) {
			const theirs = keyOf(s?.statement);
			const real =
				mine === null ||
				theirs === null ||
				mine === theirs ||
				(USPS_TOPICS.has(f.topic) && uspsDisagree(f.statement, s?.statement));
			(real ? entry.real : entry.false).push(s?.id);
			const forward = Number(f.id) <= Number(s?.id);
			const [a, b] = forward ? [f.id, s?.id] : [s?.id, f.id];
			const [keyA, keyB] = forward ? [mine, theirs] : [theirs, mine];
			pairs.set(`${a}-${b}`, { a, b, keyA, keyB, topic: f.topic, real });
		}
		byFact.set(f.id, entry);
	}
	const flagged = [...byFact.keys()];
	return {
		byFact,
		flagged,
		falseFacts: flagged.filter((id) => byFact.get(id)?.real.length === 0),
		realFacts: flagged.filter((id) => (byFact.get(id)?.real.length ?? 0) > 0),
		pairs: [...pairs.values()].sort((p, q) => p.a - q.a || p.b - q.b)
	};
}

const MONTHS = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'];

/**
 * Calendar dates named in text: `November 26-27, 2026`, `Nov. 11`,
 * `11/13/2026`, `2026-12-25`. A range or pair names every day in it;
 * a year left out stays null.
 * @param {unknown} text
 * @returns {{ y: number|null, m: number, d: number }[]}
 */
export function datesIn(text) {
	const s = String(text ?? '');
	const out = [];
	const push = (y, m, d) => {
		if (m >= 1 && m <= 12 && d >= 1 && d <= 31) out.push({ y, m, d });
	};
	const year = (v) => (v === undefined ? null : v.length === 2 ? 2000 + Number(v) : Number(v));
	for (const m of s.matchAll(/\b(20\d\d)-(\d\d)-(\d\d)\b/g)) push(Number(m[1]), Number(m[2]), Number(m[3]));
	for (const m of s.matchAll(/(?<![\d/])(1[0-2]|0?[1-9])\/(3[01]|[12]\d|0?[1-9])(?:\/(\d{4}|\d{2}))?(?![\d/])/g)) {
		push(year(m[3]), Number(m[1]), Number(m[2]));
	}
	// Whole month names or their abbreviations only: "Marriage 12" is not March.
	const named =
		/\b(january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|nov|dec)\.?\s+(\d{1,2})(?:st|nd|rd|th)?(?:\s*(-|–|—|through|to|and|&)\s*(\d{1,2})(?:st|nd|rd|th)?)?(?:,?\s+(20\d\d))?\b/gi;
	for (const m of s.matchAll(named)) {
		const month = MONTHS.indexOf(m[1].slice(0, 3).toLowerCase()) + 1;
		const from = Number(m[2]);
		const to = m[4] === undefined ? from : Number(m[4]);
		const y = year(m[5]);
		if (/and|&/.test(m[3] ?? '')) {
			push(y, month, from);
			push(y, month, to);
		} else {
			for (let d = from; d <= Math.max(from, Math.min(to, from + 31)); d += 1) push(y, month, d);
		}
	}
	return out;
}

/** Canonical URL for comparing sources: no fragment, no trailing slash. */
function canonical(url) {
	try {
		const u = new URL(String(url));
		u.hash = '';
		return u.href.replace(/\/$/, '').toLowerCase();
	} catch {
		return null;
	}
}

/** The value at a dotted path of `/plan`, or undefined. */
function at(obj, path) {
	return String(path)
		.split('.')
		.reduce((o, k) => (o && typeof o === 'object' ? o[k] : undefined), obj);
}

const present = (v) =>
	Array.isArray(v) ? v.length > 0 : typeof v === 'string' ? v.trim() !== '' : v !== null && v !== undefined && v !== false;

/**
 * The whole measure.
 * @param {{ facts: unknown, plan: any, checklist: any, resolve?: typeof aliasResolver, now?: number }} input
 */
export function analyse({ facts, plan, checklist, resolve = aliasResolver, now = Date.now() }) {
	const sources = checklist.sources ?? {};
	const pub = publicFacts(facts);
	const rows = pub.map((f) => {
		// `value` is what the page shows: the statement after its key
		// (the whole statement when it has none), never the quotes.
		const { fieldKey: key, value } = parseStatement(String(f.statement ?? ''));
		const hit = key ? resolveFact(String(f.topic), key, resolve) : null;
		return { fact: f, key, value, slot: hit?.slot ?? null, instance: hit?.instance ?? null, via: hit?.via ?? 'unkeyed' };
	});
	const conflicts = classifyConflicts(pub);
	const evidence = Array.isArray(plan?.evidence) ? plan.evidence : [];
	const decisions = evidence.filter((n) => n?.kind === 'decision' && n.preferred_fact_id != null);

	const cites = (f, alias) => {
		const want = canonical(sources[alias]);
		return (f.citations ?? []).some((c) => want !== null && canonical(c?.source_url) === want);
	};
	const fits = (r, v) =>
		r.slot === v.slot &&
		(v.instance == null || r.instance === v.instance || (v.unscoped_ok === true && r.instance === null)) &&
		(!v.match?.source || cites(r.fact, v.match.source));

	/** @type {Map<string, any>} */
	const verdict = new Map();
	const judge = (v) => {
		const hits = v.slot ? rows.filter((r) => fits(r, v)) : [];
		const ids = hits.map((r) => r.fact.id);
		if (hits.length) {
			const disputed = hits.filter((r) => (conflicts.byFact.get(r.fact.id)?.real.length ?? 0) > 0);
			if (v.critical && disputed.length) {
				const decision = decisions.find((d) => ids.includes(d.preferred_fact_id));
				if (!decision) {
					const against = [...new Set(disputed.flatMap((r) => conflicts.byFact.get(r.fact.id)?.real ?? []))];
					return { state: 'conflict', by: null, facts: ids, against: against.filter(Number.isFinite) };
				}
				return { state: 'covered', by: 'decision', facts: ids, decision: decision.id };
			}
			return { state: 'covered', by: 'facts', facts: ids };
		}
		if (v.plan && present(at(plan, v.plan))) return { state: 'covered', by: 'plan', facts: [] };
		const note = evidence.find((n) => v.slot && n?.slot === v.slot);
		const extra = note ? { note: note.id } : {};
		// Pack data (the /plan authority row): on the page, but not cited.
		if (v.shown && present(at(plan, v.shown))) return { state: 'shown', by: null, facts: [], path: v.shown, ...extra };
		return { state: 'missing', by: null, facts: [], ...extra };
	};
	for (const v of checklist.values) if (!v.computed_from) verdict.set(v.id, judge(v));
	for (const v of checklist.values) {
		if (!v.computed_from) continue;
		const ok = v.computed_from.every((id) => verdict.get(id)?.state === 'covered');
		verdict.set(v.id, ok ? { state: 'covered', by: 'computed', facts: [], from: v.computed_from } : { state: 'missing', by: null, facts: [] });
	}

	const values = checklist.values.map((v) => ({ id: v.id, section: v.section, slot: v.slot, critical: v.critical === true, ...verdict.get(v.id) }));
	const covered = values.filter((v) => v.state === 'covered');
	const shown = values.filter((v) => v.state === 'shown');
	const count = (by) => covered.filter((v) => v.by === by).length;
	const tag = (v) => (v.state === 'conflict' ? `${v.id} (conflict)` : v.state === 'shown' ? `${v.id} (shown, not cited)` : v.id);
	const sectionIds = [...new Set(values.map((v) => v.section))];
	const critical = (checklist.critical_slots ?? []).map((slot) => {
		const v = values.find((x) => x.critical && x.slot === slot);
		return { slot, value: v?.id ?? null, state: v?.state ?? 'missing', by: v?.by ?? null, facts: v?.facts ?? [], against: v?.against ?? [], decision: v?.decision ?? null };
	});

	const closureRows = rows.filter((r) => r.slot === 'holiday_hours.closure' || r.fact.topic === 'holiday_hours');
	const closures = (checklist.closures ?? []).map((c) => {
		const [y, m, d] = String(c.date).split('-').map(Number);
		// By the key's instance, or by a date the statement's value names.
		// A quote may list the office's whole holiday calendar, but the
		// page shows only the statement, so its dates do not count.
		const hit = closureRows.find(
			(r) => r.instance === c.instance || datesIn(r.value).some((t) => t.m === m && t.d === d && (t.y === null || t.y === y))
		);
		return { id: c.id, date: c.date, known: Boolean(hit), fact: hit?.fact.id ?? null };
	});

	const captured = pub.flatMap((f) => (f.citations ?? []).map((c) => Date.parse(c?.captured_at ?? ''))).filter(Number.isFinite);
	const accepted = pub.map((f) => f.accepted_at).filter((t) => typeof t === 'number' && t > 0).map((t) => t * 1000);
	const span = (list) =>
		list.length
			? { newest: iso(Math.max(...list)), oldest: iso(Math.min(...list)), newestHours: hours(now - Math.max(...list)), oldestHours: hours(now - Math.min(...list)) }
			: null;

	return {
		facts: {
			public: pub.length,
			byRegistry: rows.filter((r) => r.via === 'registry').length,
			byConvention: rows.filter((r) => r.via === 'convention').length,
			unkeyed: rows.filter((r) => r.via === 'unkeyed').length
		},
		coverage: {
			covered: covered.length,
			total: values.length,
			byFacts: count('facts'),
			byDecision: count('decision'),
			byComputed: count('computed'),
			byPlan: count('plan'),
			// Not counted: pack data the page shows without a citation.
			shown: shown.length,
			shownIds: shown.map((v) => v.id),
			sections: sectionIds.map((id) => {
				const mine = values.filter((v) => v.section === id);
				return {
					id,
					covered: mine.filter((v) => v.state === 'covered').length,
					total: mine.length,
					missing: mine.filter((v) => v.state !== 'covered').map(tag)
				};
			})
		},
		critical: { covered: critical.filter((c) => c.state === 'covered').length, total: critical.length, slots: critical },
		conflicts: {
			flagged: conflicts.flagged.length,
			false: conflicts.falseFacts.length,
			real: conflicts.realFacts.length,
			pairs: conflicts.pairs
		},
		closures: { known: closures.filter((c) => c.known).length, total: closures.length, dates: closures, facts: closureRows.length },
		captures: span(captured),
		approvals: span(accepted),
		notes: evidence.length,
		targets: checklist.targets ?? {},
		values
	};
}

function iso(ms) {
	return new Date(ms).toISOString().replace('.000Z', 'Z');
}

function hours(ms) {
	return Math.round((ms / 3.6e6) * 10) / 10;
}

function age(h) {
	return h < 48 ? `${h} h old` : `${Math.round((h / 24) * 10) / 10} days old`;
}

/**
 * The plain-text report.
 * @param {ReturnType<typeof analyse> & { slug: string, base: string, at: string, registry: string }} r
 */
export function report(r) {
	const t = r.targets ?? {};
	const met = (ok) => (ok ? 'met' : 'NOT MET');
	const lines = [];
	const c = r.coverage;
	lines.push(`Dossier metrics  ${r.slug}  ${r.base}  ${r.at}`);
	lines.push(
		`Facts: ${r.facts.public} public; slot from the ${r.registry}: ${r.facts.byRegistry}, from the canonical {topic}.{key}: ${r.facts.byConvention}, unkeyed: ${r.facts.unkeyed}`
	);
	lines.push('');
	lines.push(
		`Coverage: ${c.covered}/${c.total} values (${c.byFacts} from facts, ${c.byDecision} by staff decision, ${c.byComputed} computed, ${c.byPlan} from /plan)  target >= ${t.covered ?? '?'}: ${met(c.covered >= (t.covered ?? Infinity))}`
	);
	for (const s of c.sections) {
		const tally = `${s.covered}/${s.total}`.padStart(5);
		lines.push(`  ${s.id.padEnd(13)}${tally}${s.missing.length ? `  missing: ${s.missing.join(', ')}` : ''}`);
	}
	if (c.shown) {
		lines.push(`  Shown, not cited (pack data in /plan, not counted): ${c.shown} (${c.shownIds.join(', ')})`);
	}
	const k = r.critical;
	lines.push(`Critical slots: ${k.covered}/${k.total} covered  target ${t.critical ?? '?'}: ${met(k.covered >= (t.critical ?? Infinity))}`);
	for (const s of k.slots) {
		const facts = s.facts.map((id) => `#${id}`).join(' ');
		const why =
			s.state === 'conflict'
				? `conflict: ${facts} disputed by ${s.against.map((id) => `#${id}`).join(' ') || 'an unlisted fact'}, no staff decision`
				: s.state === 'covered'
					? `covered by ${s.by === 'decision' ? `staff decision ${s.decision} over ` : ''}${facts || s.by}`
					: 'missing';
		lines.push(`  ${s.slot.padEnd(26)}${why}`);
	}
	const x = r.conflicts;
	lines.push(
		`False conflicts: ${x.false} of ${r.facts.public} facts flagged only against other keys (${x.flagged} flagged, ${x.real} real)  target ${t.false_conflicts ?? '?'}: ${met(x.false <= (t.false_conflicts ?? -1))}`
	);
	for (const p of x.pairs) {
		lines.push(`  ${p.real ? 'real ' : 'false'}  ${p.topic}: #${p.a} ${p.keyA ?? '(unkeyed)'}  vs  #${p.b} ${p.keyB ?? '(unkeyed)'}`);
	}
	const cl = r.closures;
	const missing = cl.dates.filter((d) => !d.known).map((d) => d.date);
	lines.push(
		`Closures known: ${cl.known}/${cl.total} (${cl.facts} closure fact(s))  target ${t.closures ?? '?'}: ${met(cl.known >= (t.closures ?? Infinity))}${missing.length ? `  missing ${missing.join(', ')}` : ''}`
	);
	if (r.captures) {
		lines.push(
			`Captures: newest ${r.captures.newest} (${age(r.captures.newestHours)}), oldest ${r.captures.oldest} (${age(r.captures.oldestHours)})  target newest <= ${t.newest_capture_hours ?? '?'} h: ${met(r.captures.newestHours <= (t.newest_capture_hours ?? -1))}`
		);
	} else lines.push('Captures: none cited');
	lines.push(
		r.approvals
			? `Approvals: newest ${r.approvals.newest} (${age(r.approvals.newestHours)}), oldest ${r.approvals.oldest}`
			: 'Approvals: not in this read (/facts has no accepted_at)'
	);
	lines.push(`Staff notes: ${r.notes} in /plan evidence[]`);
	return lines.join('\n');
}

/** R1's registry when it is in the tree, else ALIASES. */
async function loadRegistry() {
	const file = new URL('../src/lib/public-ui/slots.js', import.meta.url);
	if (!existsSync(fileURLToPath(file))) return { name: 'embedded aliases (R1 slots.json not in this tree)', resolve: aliasResolver };
	try {
		const m = await import(file.href);
		if (typeof m.resolveSlot !== 'function') throw new Error('no resolveSlot export');
		return {
			name: 'slots.json registry',
			resolve: (topic, key) => {
				const hit = m.resolveSlot(topic, key);
				return hit ? { slot: hit.slot.id, instance: hit.instance ?? null, via: 'registry' } : null;
			}
		};
	} catch (err) {
		return { name: `embedded aliases (slots.js did not load: ${String(err?.message ?? err).slice(0, 80)})`, resolve: aliasResolver };
	}
}

async function getJson(url) {
	try {
		const r = await fetch(url, { headers: { accept: 'application/json' }, signal: AbortSignal.timeout(30_000) });
		let body = null;
		try {
			body = await r.json();
		} catch {
			/* not JSON */
		}
		return { status: r.status, body };
	} catch (err) {
		return { status: 0, body: null, error: String(err?.message ?? err) };
	}
}

function parseArgs(argv) {
	const out = {};
	for (let i = 0; i < argv.length; i += 1) {
		const m = argv[i].match(/^--([a-z-]+)(?:=(.*))?$/);
		if (!m) continue;
		out[m[1]] = m[2] ?? (argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[++i] : true);
	}
	return out;
}

async function main(argv) {
	const args = parseArgs(argv);
	const checklist = JSON.parse(readFileSync(resolve(ROOT, String(args.checklist ?? CHECKLIST)), 'utf8'));
	const base = String(args.base ?? DEFAULT_BASE).replace(/\/+$/, '');
	const slug = String(args.slug ?? checklist.authority);
	const api = `${base}/api/v1/authorities/${encodeURIComponent(slug)}`;
	const facts = await getJson(`${api}/facts`);
	if (facts.status !== 200 || !Array.isArray(facts.body?.facts)) {
		console.error(`dossier-metrics: ${api}/facts -> ${facts.status}${facts.error ? ` (${facts.error})` : ''}`);
		process.exit(1);
	}
	const plan = await getJson(`${api}/plan`); // 404 or empty: no plan, no notes
	const registry = await loadRegistry();
	const result = {
		slug,
		base,
		at: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
		registry: registry.name,
		...analyse({ facts: facts.body.facts, plan: plan.status === 200 ? plan.body : null, checklist, resolve: registry.resolve })
	};
	console.log(args.json ? JSON.stringify(result, null, 2) : report(result));
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
	await main(process.argv.slice(2));
}
