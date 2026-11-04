<script lang="ts">
	import MapPinIcon from '@lucide/svelte/icons/map-pin';
	import NavigationIcon from '@lucide/svelte/icons/navigation';
	import DoorOpenIcon from '@lucide/svelte/icons/door-open';
	import CalendarClockIcon from '@lucide/svelte/icons/calendar-clock';
	import MailIcon from '@lucide/svelte/icons/mail';
	import CitationChips from './CitationChips.svelte';
	import {
		hasGroundedZone,
		hoursRows,
		openStatus,
		zoneName,
		type Office,
		type PlanFact
	} from '../plan.js';

	// One accepted office record: how to visit (walk-in / appointment),
	// "Open now" in the office's own grounded timezone, the hours table,
	// the street address with an outbound directions link. Every value is
	// a field of the accepted office row; its facts sit one tap away.
	//
	// The customer guide passes `heading` (its office cards are h4s under
	// the "In person" h3) and `clocks="guide"` (times read "8:00 a.m.",
	// as the rest of the guide prints them); the plan view keeps a
	// paragraph title and "8:00 AM".
	let {
		office,
		facts,
		now,
		factIds = [],
		heading = 'p',
		clocks = 'plan'
	}: {
		office: Office;
		facts: Map<number, PlanFact>;
		now: Date;
		factIds?: number[];
		heading?: 'p' | 'h3' | 'h4';
		clocks?: 'plan' | 'guide';
	} = $props();

	/** "8:00 AM – 4:00 PM" → "8:00 a.m. – 4:00 p.m." in the guide. */
	const clockText = (text: string) =>
		clocks === 'guide' ? text.replace(/\s?\bAM\b/g, ' a.m.').replace(/\s?\bPM\b/g, ' p.m.') : text;

	const status = $derived(openStatus(office, now));
	const rowsOfHours = $derived(hoursRows(office).map((r) => ({ ...r, time: clockText(r.time) })));
	const zone = $derived(hasGroundedZone(office) ? zoneName(office.timezone!, now) : '');
	const visit = $derived(
		office.visit?.appointment_required || office.visit?.kind === 'appointment'
			? { label: 'Appointment required', icon: CalendarClockIcon }
			: office.visit?.kind === 'walk_in'
				? { label: 'Walk-in', icon: DoorOpenIcon }
				: office.visit?.kind === 'mail_only'
					? { label: 'Mail only', icon: MailIcon }
					: null
	);
	const street = $derived(
		office.address
			? [
					office.address.line1,
					office.address.line2,
					[office.address.city, [office.address.region, office.address.postal_code].filter(Boolean).join(' ')]
						.filter(Boolean)
						.join(', ')
				].filter((l): l is string => !!l)
			: []
	);
	const directions = $derived(
		typeof office.directions_url === 'string' && office.directions_url.startsWith('https://')
			? office.directions_url
			: null
	);
	const cited = $derived([
		...factIds,
		...(office.visit?.fact_ids ?? []),
		office.address?.fact_id,
		...office.hours.map((h) => h.fact_id)
	]);
</script>

<div class="rounded-xl border border-line bg-bg p-5 shadow-sm">
	<div class="flex flex-wrap items-center gap-2">
		{#if visit}
			{@const Icon = visit.icon}
			<span
				class="inline-flex min-h-7 items-center gap-1.5 rounded-full border border-line bg-surface px-3 text-xs font-semibold text-ink"
			>
				<Icon class="size-3.5" aria-hidden="true" />
				{visit.label}
			</span>
		{/if}
		{#if status}
			<span
				class={`inline-flex min-h-7 items-center gap-2 rounded-full border px-3 text-xs font-semibold ${
					status.open
						? 'border-verified bg-verified-bg text-verified'
						: 'border-changed bg-changed-bg text-changed'
				}`}
				data-open={status.open ? 'yes' : 'no'}
			>
				<span class="relative flex size-2 flex-none" aria-hidden="true">
					{#if status.open}
						<span
							class="absolute inline-flex size-full animate-ping rounded-full bg-verified opacity-60 motion-reduce:animate-none"
						></span>
					{/if}
					<span class={`relative inline-flex size-2 rounded-full ${status.open ? 'bg-verified' : 'bg-changed'}`}></span>
				</span>
				{status.text}
			</span>
		{/if}
	</div>
	<svelte:element this={heading} class="text-h3 mt-3 mb-0 font-semibold text-ink">{office.name}</svelte:element>
	{#if office.visit?.notes}
		<p class="mt-1 mb-0 text-sm text-ink-muted">{office.visit.notes}</p>
	{/if}

	<div class="mt-4 grid gap-4 sm:grid-cols-2">
		{#if street.length > 0}
			<div class="min-w-0">
				<p class="m-0 flex items-center gap-1.5 text-xs font-semibold tracking-wide text-ink-muted uppercase">
					<MapPinIcon class="size-3.5" aria-hidden="true" /> Address
				</p>
				<address class="mt-1 text-sm break-words text-ink not-italic">
					{#each street as line, i (i)}<span class="block">{line}</span>{/each}
				</address>
				{#if directions}
					<a
						href={directions}
						target="_blank"
						rel="noopener noreferrer"
						class="mt-2 inline-flex min-h-11 items-center gap-1.5 text-sm font-medium text-accent-strong"
					>
						<NavigationIcon class="size-4" aria-hidden="true" />
						Directions<span class="sr-only"> to {office.name} (opens in a new tab)</span>
					</a>
				{/if}
			</div>
		{/if}
		{#if rowsOfHours.length > 0}
			<div class="min-w-0">
				<table class="w-full border-collapse text-sm">
					<caption class="text-left text-xs font-semibold tracking-wide text-ink-muted uppercase">
						Hours
					</caption>
					<tbody>
						{#each rowsOfHours as row (row.days)}
							<tr class="border-b border-line last:border-b-0">
								<th scope="row" class="py-1.5 pr-3 text-left font-medium text-ink">{row.days}</th>
								<td class="py-1.5 text-right font-mono text-xs text-ink">{row.time}</td>
							</tr>
						{/each}
					</tbody>
				</table>
				{#if zone}
					<p class="mt-1 mb-0 text-xs text-ink-muted">Times in {zone}</p>
				{/if}
			</div>
		{/if}
	</div>
	{#if office.phone}
		<p class="mt-3 mb-0 text-sm">
			<a href={`tel:${office.phone.replace(/[^\d+]/g, '')}`} class="font-medium">{office.phone}</a>
		</p>
	{/if}
	<div class="mt-4">
		<CitationChips ids={cited} {facts} />
	</div>
</div>
