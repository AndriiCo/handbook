<script lang="ts">
	import type { Snippet } from 'svelte';
	import MapPinIcon from '@lucide/svelte/icons/map-pin';
	import CalendarXIcon from '@lucide/svelte/icons/calendar-x';
	import DoorOpenIcon from '@lucide/svelte/icons/door-open';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import ValueRow from './ValueRow.svelte';
	import { zoneName, type Office, type PlanFact } from '../plan.js';
	import { OFFICE_SLOTS, SLOT, bySlot, notConfirmed, partOf, traceAttrs } from './items.js';
	import { mapLinks, mapQuery, postalCity } from './postal.js';
	import {
		DEFAULT_ZONE,
		confirmedZone,
		dayLabel,
		instanceName,
		liveStatus,
		sameOffice,
		splitOffices,
		upcomingClosures
	} from './offices.js';
	import type { Item } from './contract.js';

	// "In person" (plan §3 step 5, D4): one card per office, then the
	// next closures.
	//
	// With an accepted office record set (OfficeSetV1, parsed by plan.ts)
	// each office reuses plan/OfficeCard, with Google and Apple map
	// searches built from the record's address, and the timezone caption
	// ("Times in Pacific Standard Time") whenever the zone is quoted or
	// staff-confirmed. Above the card, "Open now" and today's cut-off are
	// worked out in that zone, with a closure for that office dated today
	// overriding its hours (closed, or closing early with the cut-off
	// counted back from the early time). A closure that names another
	// office of the page ("The Cheney office is closed…") does not touch
	// this one; an unclear one (a clock time its text doesn't tie to
	// today alone) leaves nothing calculated. All of that is hidden while
	// a service notice is active; the timezone caption stays.
	//
	// Without a record, each office instance gets a card built from its
	// address, hours and walk-in rows, all as printed: the city as its
	// title (read from the address and traced to it; "Office" when the
	// address names none), the address with Google and Apple map
	// searches, hours as printed, and the rules (cut-off, same-day only,
	// no mail, visits on public record). Nothing is calculated. With no
	// office at all: one "Not confirmed yet" row.
	//
	// Next closures: closures from today on (in `timeZone`), soonest
	// first; three visible, the rest in a details element.
	//
	// Props:
	// - groups: `page.offices` ({instance, name, items}); optional, the
	//   cards are grouped from `items` by instance when it is empty.
	// - items: the send section's items (office rows, cut-off).
	// - closures: `page.closures` (holiday_hours.closure Items).
	// - officeSet: accepted OfficeSetV1 offices (plan.ts Office[]), or
	//   null. The slice spec calls this list `offices`; either list may
	//   arrive under either prop: entries are sorted by shape (a group has
	//   `items`, a record a numeric id and `hours`).
	// - noticeActive: a service notice is active (no live status).
	// - now: the clock (default: page load).
	// - country: the authority's country, for map searches.
	// - timeZone: the zone closures are dated in (default
	//   America/Los_Angeles; an office's confirmed zone wins).
	// - marker: snippet `marker(item)` from the citations layer.
	type Group = { instance: string | null; name?: string | null; items: Item[] };

	let {
		groups = [],
		items = [],
		closures = [],
		officeSet = null,
		noticeActive = false,
		now = new Date(),
		country = null,
		timeZone = DEFAULT_ZONE,
		marker
	}: {
		groups?: Group[] | Office[] | null;
		items?: Item[];
		closures?: Item[];
		officeSet?: Office[] | Group[] | null;
		noticeActive?: boolean;
		now?: Date;
		country?: string | null;
		timeZone?: string;
		marker?: Snippet<[Item]>;
	} = $props();

	const NO_FACTS = new Map<number, PlanFact>();
	const RULE_ORDER: string[] = [SLOT.hours, SLOT.cutoff, SLOT.sameDayOnly, SLOT.noMail, SLOT.publicRecord];
	const PILL = `inline-flex min-h-8 items-center gap-1.5 rounded-full border border-line bg-surface px-3 text-sm font-medium text-ink no-underline transition-colors hover:border-focus`;

	const officeItems = $derived(items.filter((i) => OFFICE_SLOTS.includes(i.slot ?? '')));
	const parts = $derived(splitOffices([...(groups ?? []), ...(officeSet ?? [])]));
	const records = $derived(parts.records as Office[]);

	// plan/OfficeCard draws an accepted record. It loads only when the page
	// has one (most offices have none yet), which keeps it and the old
	// citation chips it carries out of the guide's chunk. Until it loads,
	// and for good if it fails to, every office keeps its card built from
	// the office's own rows (address, hours, rules), so an office never
	// loses its name, address or hours to a missing chunk.
	let OfficeCard = $state.raw<typeof import('../plan/OfficeCard.svelte').default | null>(null);
	$effect(() => {
		if (OfficeCard || records.length === 0) return;
		import('../plan/OfficeCard.svelte')
			.then((m) => (OfficeCard = m.default))
			.catch(() => {});
	});

	/** Office rows by instance; with no instance anywhere, one office. */
	function byInstance(list: Item[]): Group[] {
		const named = list.filter((i) => i.instance);
		if (named.length === 0) return list.length > 0 ? [{ instance: null, items: list }] : [];
		const out = new Map<string, Group>();
		for (const i of named) {
			const key = i.instance as string;
			if (!out.has(key)) out.set(key, { instance: key, items: [] });
			out.get(key)!.items.push(i);
		}
		return [...out.values()];
	}

	function ordered(list: Item[]): Item[] {
		const rank = (i: Item) => {
			const at = RULE_ORDER.indexOf(i.slot ?? '');
			return at < 0 ? RULE_ORDER.length : at;
		};
		return [...list].sort((a, b) => rank(a) - rank(b));
	}

	const sourceGroups = $derived(parts.groups.length > 0 ? parts.groups : byInstance(officeItems));
	const grouped = $derived(new Set(sourceGroups.flatMap((g) => g.items.map((i) => i.id))));
	const shared = $derived(officeItems.filter((i) => !grouped.has(i.id)));

	type Maps = { google: string; apple: string } | null;
	type FactCard = {
		kind: 'facts';
		key: string;
		// The card title: the city read from the address (traced to it),
		// else the presenter's office name, else "Office".
		title: string;
		titleItem: Item | null;
		address: Item | null;
		rows: Item[];
		maps: Maps;
		names: string[];
	};
	type RecordCard = {
		kind: 'record';
		key: string;
		office: Office;
		backing: Item[];
		rows: Item[];
		maps: Maps;
		names: string[];
	};

	/** The names a closure may call an office by (matching only; never shown). */
	function groupNames(g: Group, city: string | null): string[] {
		return [city, g.instance ? instanceName(g.instance) : null].filter((n): n is string => !!n);
	}

	function factCard(g: Group, index: number): FactCard {
		const address = bySlot(g.items, SLOT.office);
		const usableAddress = address && address.state !== 'missing' ? address : null;
		const city = (usableAddress && postalCity(usableAddress.display)) || null;
		const name = (g.name ?? '').trim();
		const query = usableAddress ? mapQuery(usableAddress.display, city ?? name, country) : '';
		return {
			kind: 'facts',
			key: g.instance ?? `office-${index}`,
			title: city || name || 'Office',
			titleItem: city && usableAddress ? partOf(usableAddress, 'city', city) : null,
			address,
			rows: g.items.filter((i) => i !== address),
			maps: mapLinks(query),
			names: groupNames(g, city)
		};
	}

	/** Map searches from a record's address, as OfficeCard prints it. */
	function recordMaps(office: Office): Maps {
		const a = office.address;
		if (!a) return null;
		const lines = [a.line1, a.line2, [a.city, [a.region, a.postal_code].filter(Boolean).join(' ')].filter(Boolean).join(', ')];
		const text = lines.filter((l): l is string => !!l && !!l.trim()).join('\n');
		return text ? mapLinks(mapQuery(text, a.city ?? '', a.country ?? country)) : null;
	}

	// An accepted office record replaces its office's address and hours
	// rows (the card shows them); the office's other rules stay, as
	// printed. Offices the record set lacks keep their own cards.
	const shownRecords = $derived(OfficeCard ? records : []);
	const entries = $derived.by((): (FactCard | RecordCard)[] => {
		const used = new Set<number>();
		const out: (FactCard | RecordCard)[] = [];
		sourceGroups.forEach((g, index) => {
			const ids = new Set(g.items.flatMap((i) => i.factIds));
			const record = shownRecords.find(
				(r) => !used.has(r.id) && (sameOffice(r, g.instance) || (r.fact_ids ?? []).some((id) => ids.has(id)))
			);
			if (!record) {
				out.push(factCard(g, index));
				return;
			}
			used.add(record.id);
			const shown = (i: Item) => i.slot === SLOT.office || i.slot === SLOT.hours;
			const address = bySlot(g.items, SLOT.office);
			const city = (address && address.state !== 'missing' && postalCity(address.display)) || null;
			out.push({
				kind: 'record',
				key: `record-${record.id}`,
				office: record,
				backing: g.items.filter(shown),
				rows: g.items.filter((i) => !shown(i)),
				maps: recordMaps(record),
				names: groupNames(g, city)
			});
		});
		for (const r of shownRecords) {
			if (used.has(r.id)) continue;
			out.push({ kind: 'record', key: `record-${r.id}`, office: r, backing: rowsFor(r), rows: [], maps: recordMaps(r), names: [] });
		}
		// One office: the shared rules join its card.
		if (out.length === 1) out[0] = { ...out[0], rows: [...out[0].rows, ...shared] };
		return out.map((e) => ({ ...e, rows: ordered(e.rows) }));
	});
	const sharedRows = $derived(entries.length > 1 ? ordered(shared) : []);

	/** Every other office's names and records, so a closure naming one office closes only that one. */
	function othersOf(entry: FactCard | RecordCard): (Office | string)[] {
		return entries
			.filter((e) => e !== entry)
			.flatMap((e) => (e.kind === 'record' ? [e.office, ...e.names] : e.names));
	}

	// Closures are dated in the office's confirmed zone when one exists.
	const zone = $derived(records.find((o) => confirmedZone(o))?.timezone ?? timeZone);
	const upcoming = $derived(upcomingClosures(closures, now, zone));
	const shownClosures = $derived(
		upcoming.next.length > 0
			? upcoming.next.map((c) => ({ item: c.item, label: dayLabel(c.date, upcoming.today) }))
			: upcoming.undated.map((item) => ({ item, label: item.label }))
	);
	const foldedClosures = $derived([
		...upcoming.later.map((c) => ({ item: c.item, label: dayLabel(c.date, upcoming.today) })),
		...(upcoming.next.length > 0 ? upcoming.undated.map((item) => ({ item, label: item.label })) : [])
	]);

	/** The office rows a record rests on (its markers), and its cut-off. */
	function rowsFor(office: Office): Item[] {
		const ids = new Set(office.fact_ids ?? []);
		return items.filter(
			(i) => OFFICE_SLOTS.includes(i.slot ?? '') && (i.factIds.some((id) => ids.has(id)) || sameOffice(office, i.instance))
		);
	}
	function cutoffFor(office: Office): Item | null {
		const all = items.filter((i) => i.slot === SLOT.cutoff && (i.state === 'ok' || i.state === 'stale'));
		return all.find((i) => sameOffice(office, i.instance)) ?? all.find((i) => !i.instance) ?? null;
	}
	function zoneLabel(office: Office): string {
		return confirmedZone(office) ? zoneName(office.timezone as string, now) : '';
	}
</script>

{#snippet mapPills(maps: { google: string; apple: string }, title: string)}
	<p class="m-0 flex flex-wrap gap-2 print:hidden">
		<a
			href={maps.google}
			target="_blank"
			rel="noopener noreferrer"
			class={PILL}
			aria-label={`Open in Google Maps: ${title} (opens in a new tab)`}
		>
			<MapPinIcon class="size-4 flex-none text-accent-strong" aria-hidden="true" />Google Maps
		</a>
		<a
			href={maps.apple}
			target="_blank"
			rel="noopener noreferrer"
			class={PILL}
			aria-label={`Open in Apple Maps: ${title} (opens in a new tab)`}
		>
			<MapPinIcon class="size-4 flex-none text-accent-strong" aria-hidden="true" />Apple Maps
		</a>
	</p>
{/snippet}

<div class="flex flex-col gap-6">
	{#each entries as entry (entry.key)}
		{#if entry.kind === 'record'}
			{@const office = entry.office}
			{@const live = noticeActive
				? null
				: liveStatus({ office, closures, cutoff: cutoffFor(office), now, others: othersOf(entry), aliases: entry.names })}
			{@const ids = [...new Set([...(office.fact_ids ?? []), ...entry.backing.flatMap((i) => i.factIds)])]}
			{@const zoneText = zoneLabel(office)}
			<div class="flex flex-col gap-3" data-office={entry.key} data-print-keep>
				<div class="grid grid-cols-[minmax(0,1fr)_minmax(3.5ch,max-content)] gap-x-3">
					<div class="min-w-0" data-fact-ids={ids.length > 0 ? ids.join(' ') : undefined}>
						{#if live}
							<p
								class={`m-0 mb-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm font-semibold ${live.open ? 'text-verified' : 'text-changed'}`}
								data-state="computed"
								data-inputs={live.inputs.map((i) => i.id).join(' ')}
							>
								{#if live.open}
									<DoorOpenIcon class="size-4 flex-none" aria-hidden="true" />
								{:else}
									<ClockIcon class="size-4 flex-none" aria-hidden="true" />
								{/if}
								<span>{live.text}</span>
								{#if live.cutoff}<span class="text-ink">· Today's cut-off: {live.cutoff}</span>{/if}
							</p>
							{#if live.closure}
								<p class="m-0 mb-2 text-sm text-ink" data-fact-ids={live.closure.factIds.join(' ')}>{live.closure.display}</p>
							{/if}
						{/if}
						{#if zoneText}
							<p class="m-0 mb-2 text-sm text-ink-muted" data-office-zone>Times in {zoneText}</p>
						{/if}
						{#if OfficeCard}<OfficeCard
								office={{ ...office, timezone_source: null }}
								facts={NO_FACTS}
								{now}
								heading="h4"
								clocks="guide"
							/>{/if}
						{#if entry.maps}
							<div class="mt-3">{@render mapPills(entry.maps, office.name)}</div>
						{/if}
					</div>
					<!-- One gutter per backing row, stacked: 6 px apart, so each
					     24 px marker box (lifted 6 px) clears the next one. -->
					<span class="flex flex-col items-end gap-1.5 text-sm leading-6">
						{#each entry.backing as row (row.id)}{@render marker?.(row)}{/each}
					</span>
				</div>
				{#if entry.rows.length > 0}
					<dl class="m-0 border-t border-line">
						{#each entry.rows as row (row.id)}
							<ValueRow item={row} {marker} />
						{/each}
					</dl>
				{/if}
			</div>
		{:else}
			<div class="rounded-md border border-line bg-bg p-4 sm:p-5" data-office={entry.key} data-print-keep>
				<h4 class="m-0 text-body-lg font-semibold text-ink">
					{#if entry.titleItem}<span {...traceAttrs(entry.titleItem)}>{entry.title}</span>{:else}{entry.title}{/if}
				</h4>
				{#if entry.maps}
					<div class="mt-2">{@render mapPills(entry.maps, entry.title)}</div>
				{/if}
				<dl class="m-0 mt-3 border-t border-line">
					{#if entry.address}
						<ValueRow
							item={entry.address}
							{marker}
							copyName={entry.titleItem ? `the ${entry.title} office address` : 'the office address'}
						/>
					{/if}
					{#each entry.rows as row (row.id)}
						<ValueRow item={row} {marker} />
					{/each}
				</dl>
			</div>
		{/if}
	{:else}
		<dl class="m-0 border-t border-line">
			<ValueRow item={notConfirmed('visit-office-missing', 'Office address', SLOT.office)} />
		</dl>
	{/each}
	{#if sharedRows.length > 0}
		<div>
			<h4 class="m-0 text-body font-semibold text-ink">For every office</h4>
			<dl class="m-0 mt-2 border-t border-line">
				{#each sharedRows as row (row.id)}
					<ValueRow item={row} {marker} />
				{/each}
			</dl>
		</div>
	{/if}

	{#if shownClosures.length > 0}
		<div>
			<h4 class="m-0 flex items-center gap-2 text-body font-semibold text-ink">
				<CalendarXIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
				Upcoming closures
			</h4>
			<dl class="m-0 mt-2 border-t border-line">
				{#each shownClosures as c (c.item.id)}
					<ValueRow item={c.item} label={c.label} {marker} />
				{/each}
			</dl>
			{#if foldedClosures.length > 0}
				<details class="mt-1">
					<summary class="inline-flex min-h-11 cursor-pointer items-center font-medium text-accent-strong">
						More closures
					</summary>
					<dl class="m-0 border-t border-line">
						{#each foldedClosures as c (c.item.id)}
							<ValueRow item={c.item} label={c.label} {marker} />
						{/each}
					</dl>
				</details>
			{/if}
		</div>
	{/if}
</div>
