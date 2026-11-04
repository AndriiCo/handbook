<script lang="ts">
	import type { Snippet } from 'svelte';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import InfoIcon from '@lucide/svelte/icons/info';
	import BadgeCheckIcon from '@lucide/svelte/icons/badge-check';
	import CopyButton from './CopyButton.svelte';
	import InlineValue from './InlineValue.svelte';
	import ValueText from './ValueText.svelte';
	import { DOC_IDS } from './contract.js';
	import { itemLang, partOf, sourceLang, traceAttrs, usable } from './items.js';
	import { guide } from './lang.svelte.js';
	import { contestLead, noteDay, officeAnswer, postalCity, telHref, type EvidenceNote } from './postal.js';
	import type { Item } from './contract.js';

	// A disputed mailing address (plan §3 ConflictBox, D3): the office's
	// own pages print different values, so every one is listed with the
	// pages it comes from, then what the customer can safely do: call
	// first (phone and menu option), or send by courier to the street
	// address instead. While it is open there is no Copy and no label.
	// The heading is the slice spec's and plan §3's wording, "The
	// office's own pages list different mailing addresses". In print the
	// box keeps to one page (`data-print-keep`).
	//
	// After a published staff decision that prefers one of the values
	// (plan §3, §5.4), the box says so in words, citations on or off:
	// "Our recommendation (staff, November 4, 2026): use … — confirmed by
	// phone with the office (…)", then the office's answer it rests on
	// ("The office told us by phone on …: …"). The preferred row is
	// marked, and when the decision rests on the office's answer (a call,
	// an email or a visit note) that row alone gets Copy; the other
	// values stay listed. Labels stay off while the pages disagree (the
	// server gates them). On paper the recommendation line alone says
	// it (the row's mark and the answer's summary are screen only). A `worded` item (the same address written two
	// ways) gets a neutral "Worded differently" box with both texts.
	//
	// Props:
	// - item: the mailing Item in `conflict` or `worded` state, with
	//   `alternatives` (each its own Item with cites; one of another
	//   field the server flags, a return address say, carries its own
	//   label).
	// - evidence: `/plan` `evidence[]` (published staff notes); optional.
	// - phone, phoneOption: `contacts.phone` / `contacts.phone_option`
	//   Items; optional. The send step passes its own copies of them
	//   (`markRepeats`), so their markers repeat the contact step's [n]
	//   under ids of their own.
	// - courier: the `mailing.courier` Item, for "the {city} overnight
	//   address"; optional.
	// - markRepeats: mark the phone and option (they are the send step's
	//   copies); default false (no marker, one marker id each).
	// - marker: snippet `marker(item)` from the citations layer (the
	//   alternatives' gutter).
	// - inline: snippet `inline(item)`, markers inside a sentence (the
	//   phone and option).
	let {
		item,
		evidence = null,
		phone = null,
		phoneOption = null,
		courier = null,
		markRepeats = false,
		marker,
		inline
	}: {
		item: Item;
		evidence?: EvidenceNote[] | null;
		phone?: Item | null;
		phoneOption?: Item | null;
		courier?: Item | null;
		markRepeats?: boolean;
		marker?: Snippet<[Item]>;
		inline?: Snippet<[Item]>;
	} = $props();

	const uid = $props.id();
	const worded = $derived(item.state === 'worded');
	const alternatives = $derived(item.alternatives ?? []);
	const answer = $derived(worded ? null : officeAnswer(item, evidence));
	const recommended = $derived(answer?.preferred ?? null);
	const callable = $derived(phone && usable(phone) ? phone : null);
	const option = $derived(phoneOption && usable(phoneOption) ? phoneOption : null);
	const courierOk = $derived(courier && usable(courier) ? courier : null);
	const city = $derived.by(() => {
		if (!courierOk) return null;
		const name = postalCity(courierOk.display);
		return name ? partOf(courierOk, 'city', name) : null;
	});
	const repeat = $derived(markRepeats ? inline : undefined);
	const t = $derived(guide.t);
	const lang = $derived(guide.lang);
	const how = $derived(answer?.channel === 'email' ? 'email' : answer?.channel === 'visit' ? 'visit' : 'call');
	const decisionDay = $derived(answer ? noteDay(answer.decision.date, lang) : '');
	const noteChannel = $derived((answer?.note?.channel ?? '').trim());

	const valueEls: Record<string, HTMLElement | null> = $state({});

	/** The office pages a value comes from: titles (or hosts), once each. */
	function pagesOf(alt: Item): string {
		const seen = new Set<string>();
		for (const c of alt.cites ?? []) {
			const s = c?.source;
			if (!s || (s.kind && !['web', 'pdf', 'upload'].includes(s.kind))) continue;
			const name = (s.title || s.host || '').trim();
			if (name) seen.add(name);
		}
		return [...seen].join(' · ');
	}
</script>

<div
	class={`rounded-md border p-4 sm:p-5 ${worded ? 'border-line bg-surface' : 'border-changed bg-changed-bg'}`}
	role="group"
	aria-labelledby={`${uid}-h`}
	data-conflict={item.id}
	data-print-keep
>
	<h4 id={`${uid}-h`} class="m-0 flex items-start gap-2 text-body font-semibold text-ink">
		{#if worded}
			<InfoIcon class="mt-1 size-4 flex-none text-ink-muted" aria-hidden="true" />
			{t.conflict.worded}
		{:else}
			<TriangleAlertIcon class="mt-1 size-4 flex-none text-changed" aria-hidden="true" />
			{t.conflict.heading}
		{/if}
	</h4>
	<ul class="m-0 mt-3 list-none p-0">
		{#each alternatives as alt (alt.id)}
			{@const pages = pagesOf(alt)}
			{@const preferred = recommended?.id === alt.id}
			<li
				class="m-0 grid grid-cols-[minmax(0,1fr)_minmax(3.5ch,max-content)] gap-x-3 border-t border-line py-3 first:border-t-0"
				data-alternative={alt.id}
				data-recommended={preferred ? '' : undefined}
			>
				<div class="min-w-0">
					{#if alt.label && alt.label !== item.label}
						<p class="m-0 text-sm font-medium text-ink-muted">{alt.label}</p>
					{/if}
					<span
						bind:this={valueEls[alt.id]}
						class="block break-words whitespace-pre-line font-medium text-ink [overflow-wrap:anywhere]"
						lang={itemLang(alt, lang)}
						{...traceAttrs(alt)}><ValueText text={alt.display} /></span
					>
					{#if pages}
						<p class="m-0 mt-1 text-sm text-ink-muted">{t.conflict.printedOn} <span lang={sourceLang(pages, lang)}>{pages}</span></p>
					{/if}
					{#each alt.contest ?? [] as note (note.id)}
						<p class="m-0 mt-1 text-sm font-medium text-changed" data-note-ids={String(note.id)}>
							{contestLead(note, lang)}: <span lang={sourceLang(note.summary, lang)}>{note.summary}</span>
						</p>
					{/each}
					{#if preferred}
						<!-- Screen only: on paper the recommendation line below names it. -->
						<p class="m-0 mt-2 flex items-center gap-1.5 text-sm font-semibold text-verified print:hidden">
							<BadgeCheckIcon class="size-4 flex-none" aria-hidden="true" />
							{t.conflict.recommendation}
						</p>
						{#if answer?.unlocks}
							<div class="mt-2">
								<CopyButton text={alt.display} name={t.common.copyNames.recommended} target={valueEls[alt.id] ?? null} />
							</div>
						{/if}
					{/if}
				</div>
				<span class="text-right text-sm leading-6">{@render marker?.(alt)}</span>
			</li>
		{/each}
	</ul>
	{#if answer && recommended}
		<div class="mt-3 border-t border-line pt-3" data-recommendation>
			<p class="m-0 text-ink" data-note-ids={String(answer.decision.id)}>
				<span class="font-semibold">{t.conflict.recommendationStaff(decisionDay)}</span>
				{t.conflict.use} <span class="font-medium" lang={itemLang(recommended, lang)} {...traceAttrs(recommended)}
					><ValueText text={recommended.display} /></span
				>{#if answer.note}{t.conflict.confirmed(how, noteChannel)}{/if}.
			</p>
			{#if answer.note}
				<!-- Screen only: on paper the line above says how it was confirmed. -->
				<p class="m-0 mt-1 text-ink print:hidden" data-note-ids={String(answer.note.id)}>
					{t.conflict.toldUs(how, noteDay(answer.note.date, lang))}
					<span lang={sourceLang(answer.note.summary, lang)}>{answer.note.summary}</span>
				</p>
			{/if}
		</div>
	{/if}
	{#if !worded}
		<div class="mt-3 border-t border-line pt-3">
			<p class="m-0 font-semibold text-ink">{t.conflict.whatYouCanDo}</p>
			<ul class="m-0 mt-1 list-disc pl-5 text-ink">
				{#if callable}
					<li class="m-0 mt-1">
						{t.conflict.call[0]}<InlineValue item={callable} href={telHref(callable.display)} marker={repeat} />{#if option}&nbsp;(<InlineValue
								item={option}
								marker={repeat}
							/>){/if}{t.conflict.call[1]}
					</li>
				{/if}
				{#if courierOk}
					<li class="m-0 mt-1">
						{t.conflict.sendCourier(!callable)}{#if city}<InlineValue item={city} />&nbsp;{/if}<a
							href={`#${DOC_IDS.sendCourier}`}
							class="underline underline-offset-2">{t.conflict.overnight}</a
						>.
					</li>
				{/if}
				{#if !callable && !courierOk}
					<li class="m-0 mt-1">
						{t.conflict.askWhich}
						<a href={`#${DOC_IDS.contact}`} class="underline underline-offset-2">{t.conflict.contactLink}</a>.
					</li>
				{/if}
			</ul>
		</div>
	{/if}
</div>
