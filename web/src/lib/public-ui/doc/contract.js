// Customer guide contract (R2): the shapes `present.js` produces and
// every `doc/*` component, `cite.js` and `fees.js` consume.
//
// A visible value is exactly one of: a fact value shown verbatim, a
// calculation over cited values, a labelled staff note, or an honest
// "not confirmed". Every value node in the DOM carries
// `data-fact-ids="<space-separated ids>"` or
// `data-state="computed|missing|absent"`, so the page stays traceable
// (GP2-page v2 and the CUJ specs read those attributes).
//
// Typedefs only: no runtime exports beyond the frozen DOM ids below.

/**
 * Where a citation's evidence comes from.
 * @typedef {'web'|'pdf'|'upload'|'call'|'email'|'visit'|'decision'|'absence'|'computed'} EvidenceKind
 */

/**
 * One source, numbered once per page by `cite.js`.
 * `key` is the canonical URL without fragment (web, pdf, upload) or
 * `note:<id>` for a staff evidence note.
 * @typedef {{
 *   key: string,
 *   kind: EvidenceKind,
 *   title: string,
 *   url: string|null,
 *   host: string|null,
 *   capturedAt: string|null,
 *   acceptedAt: number|null,
 *   channel: string|null,
 *   occurredAt: number|null,
 *   recorded: boolean|null,
 *   supports?: string[],
 *   dayOnly?: boolean,
 *   stance?: 'confirms'|'contradicts'|'context'|null
 * }} SourceRef
 * - capturedAt: ISO capture time (web, pdf, upload).
 * - acceptedAt: unix seconds of the staff accept (fact) or publish (note).
 * - channel: the call, email or visit counterpart, e.g. "Apostille
 *   desk"; on an absence note, the pages staff checked ("FAQ and fee
 *   pages").
 * - occurredAt: unix seconds of the call, email or visit.
 * - recorded: calls only; always false before a consent lane exists.
 * - supports (optional): on a staff decision, the source keys
 *   (`note:<id>`) of the notes it rests on.
 * - dayOnly (optional): `occurredAt` carries a day, not a clock time
 *   (published notes keep the day only); cards print the day alone.
 * - stance (optional): a call, email or visit note's stance on the
 *   value it is about; `contradicts` reads as contesting it.
 */

/**
 * One citation of one value: which source, which words.
 * `quote` is the verbatim official text (web, pdf, upload); `summary`
 * is a staff note (call, email, visit, decision, absence), never shown
 * as a quotation.
 * @typedef {{ source: SourceRef, quote: string|null, summary: string|null, factId: number|null }} Cite
 */

/**
 * One rendered value: a row, a glance cell, an address, a total.
 * @typedef {{
 *   id: string,
 *   slot: string|null,
 *   label: string,
 *   display: string,
 *   kind: string,
 *   factIds: number[],
 *   cites: Cite[],
 *   state: 'ok'|'conflict'|'worded'|'stale'|'missing'|'uncited'|'computed'|'absent'|'contested',
 *   stale: null|{ level: 'muted'|'warning', text: string },
 *   alternatives: Item[]|null,
 *   inputs: Item[]|null,
 *   copy: boolean,
 *   route: 'mail'|'courier'|'counter'|null,
 *   instance: string|null,
 *   terms?: { text: string, itemId: string|null }[],
 *   contest?: { id: number, kind: string, date: string|null, summary: string }[]|null
 * }} Item
 * - id: stable, DOM-safe, unique per page, e.g. "fees-per_document" or
 *   "address-office-tumwater".
 * - slot: slots.json id; null for facts the registry doesn't map.
 * - label: plain-language label from slots.json or guide.js fieldLabel
 *   ('' for an unkeyed statement, which shows its value alone).
 * - display: the fact value verbatim (only whitespace and a trailing
 *   colon trimmed), or the computed text.
 * - kind: the slot kind (money, money_tier, postal, payee, phone, ...).
 * - factIds: facts behind the value (empty for computed and absent).
 * - cites: every citation, in display order. A disputed item (conflict,
 *   worded) keeps its cites on its alternatives.
 * - alternatives: conflict and worded states; every distinct value with
 *   its own cites, most sources first.
 * - inputs: computed state; the cited inputs.
 * - copy: show a Copy button (payee, undisputed postal values of at
 *   most 120 characters).
 * - instance: the office or date instance (e.g. "tumwater",
 *   "2026_11_11"), or null.
 * - state `contested`: a published call, email or visit note says the
 *   office told staff otherwise (`contest` lists those notes). It is
 *   shown with that note as a visible line, never with Copy, never as
 *   a calculator input, label or paid-offer gate.
 * - alternatives may hold a value of another field the server flags as
 *   disagreeing (a return address naming another PO box): its `label`
 *   is that field's.
 * - terms (optional, computed items): the calculation in reading order,
 *   each term with the item id of the input it reads ("3 × $15.00"
 *   reads the per-document fee), so the card and the endnote print
 *   "3 × $15.00 [1a] + … = $145.00".
 */

/**
 * One guide step or section.
 * `step` is the visible step number, or null for unnumbered sections
 * (contact, thin pages). `also` holds unmapped facts of this section's
 * topics, labelled with guide.js fieldLabel.
 * @typedef {{ id: string, step: number|null, heading: string, items: Item[], also: Item[] }} Section
 */

/**
 * Header data for DocHeader.
 * Counts are distinct https pages on the listed official site
 * (`pages`), other https pages (`otherPages`), and evidence notes by
 * kind. `officialHost` and `officialUrl` come from the authority's
 * listed https official site only (null without one).
 * @typedef {{
 *   eyebrow: string,
 *   h1: string,
 *   lede: string|null,
 *   crumbs: { label: string, href: string|null }[],
 *   flag: string|null,
 *   officialHost: string|null,
 *   officialUrl: string|null,
 *   trust: { capturedMin: string|null, capturedMax: string|null, acceptedMax: number|null, pages: number, otherPages: number, calls: number, emails: number }
 * }} Header
 */

/**
 * Service status for the header band.
 * - active: unpaired notices (banner).
 * - resolved: paired notices within 7 days of the restore's capture,
 *   the notice then its restore, both with the pair id as `instance`.
 * - lastRead: set only when a paired notice from the office's notice
 *   log exists: "No open service notice in the office's notice log
 *   (last read {date})".
 * @typedef {{ active: Item[], resolved: Item[], lastRead: null|{ capturedAt: string, url: string } }} Status
 */

/**
 * The whole page, built by `present()` from accepted facts.
 * - empty: 0 public facts. The listed site renders as plain text, "not
 *   yet checked by us", never as a link.
 * - thin: no profile for this kind/country, or fewer than 2 glance
 *   rows. Then there's no glance, routes, calculator or "Not confirmed"
 *   list, but sections still render as regions.
 * - updates: resolved notices from 7 to 90 days after the restore
 *   (older ones are hidden), and closures whose every date has passed;
 *   `/changes` notes join them on the page.
 * - records: every public fact, verbatim, for "How this page was made";
 *   `id` is null for a fact the payload sends without one.
 * - order: item ids in canonical reading order: the status band first
 *   (active, then resolved), then the glance, then sections top to
 *   bottom, then updates. `cite.js` numbers sources in this order; the
 *   calculator's estimate stays out of it and is numbered last.
 * @typedef {{
 *   empty: null|{ name: string, officialHost: string|null, officialUrl: string|null },
 *   thin: boolean,
 *   profile: string|null,
 *   header: Header,
 *   status: Status,
 *   glance: Item[],
 *   routes: { id: string, label: string, href: string }[],
 *   sections: Section[],
 *   offices: { instance: string, name: string, items: Item[] }[],
 *   closures: Item[],
 *   updates: Item[],
 *   records: { id: number|null, topic: string, statement: string, acceptedAt: number|null }[],
 *   order: string[]
 * }} Page
 */

/**
 * One numbered endnote (cite.js numberSources output). The contract
 * fields, plus the superset cite.js fills (see its NumberedSource and
 * CiteQuote typedefs):
 * - capturedMin / acceptedMin: earliest capture and accept across its
 *   quotes (capturedAt / acceptedAt hold the latest);
 * - basedOn: for a staff decision, the numbers of the sources it rests on;
 * - each quote's value, capturedAt, acceptedAt, uses (every marker that
 *   shows it), inputs and terms (calculations).
 * @typedef {SourceRef & {
 *   n: number,
 *   quotes: { letter: string, text: string|null, summary: string|null, label: string, itemId: string, refId: string }[],
 *   capturedMin?: string|null,
 *   acceptedMin?: number|null,
 *   basedOn?: number[]
 * }} Source
 */

/**
 * A marker reference for one item: source number, quote letter, the
 * DOM id of the marker (target of the endnote backlink), every letter
 * of that source the item shows, and the source key (a card finds its
 * marker again by item and key after a live update renumbers).
 * @typedef {{ n: number, letter: string, refId: string, letters: string[], key: string }} Ref
 */

/** DOM ids the page, the specs and the print sheet rely on. */
export const DOC_IDS = Object.freeze({
	before: 'before',
	prepare: 'prepare',
	form: 'form',
	fees: 'fees',
	send: 'send',
	sendMail: 'send-mail',
	sendCourier: 'send-courier',
	visit: 'visit',
	after: 'after',
	contact: 'contact',
	updates: 'updates',
	sources: 'sources',
	sourcesHeading: 'sources-h',
	made: 'made'
});
