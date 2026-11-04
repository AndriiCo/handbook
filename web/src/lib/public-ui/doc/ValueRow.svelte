<script lang="ts">
	import type { Snippet } from 'svelte';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import CircleQuestionMarkIcon from '@lucide/svelte/icons/circle-question-mark';
	import InfoIcon from '@lucide/svelte/icons/info';
	import CopyButton from './CopyButton.svelte';
	import ValueText from './ValueText.svelte';
	import { traceAttrs, valueLang } from './items.js';
	import { contestLead } from './postal.js';
	import type { Item } from './contract.js';

	// One row of the guide: label · value · citation gutter (plan §3
	// "Value rows", grid minmax(9rem,14rem) 1fr minmax(3.5ch,max-content):
	// the gutter grows to a group of markers, never over the value). It is
	// always reserved, so turning citations on reflows nothing. Every
	// section renders its values through this row.
	//
	// Markup: a `<div>` group of `<dt>` + `<dd>` (value) + `<dd>` (gutter),
	// so a row must sit inside a `<dl>`. At phone width the label stacks
	// above the value; the gutter stays at the right.
	//
	// The value node carries `data-fact-ids` (or `data-state` for
	// computed, missing and absent values, plus `data-inputs` for a
	// computed one). A disputed value (`conflict`, `worded`) lists each
	// alternative as its own value node with its own marker, and never
	// gets Copy. Stale, uncited and missing values say so in text with an
	// icon, never by colour alone; so does a value a staff note contests
	// (`contested`: the office told staff otherwise), which never gets
	// Copy either. A value in another language carries its `lang`; ZIP+4
	// codes never break at their hyphen.
	//
	// Props:
	// - item: the Item (doc/contract.js).
	// - marker: snippet `marker(item)` the citations layer fills with
	//   `[n]` markers; rendered in the gutter (and per alternative).
	// - label: overrides `item.label`.
	// - copy: overrides `item.copy` (Copy shows only for an undisputed
	//   value resting on facts).
	// - copyName: what Copy copies, for its accessible name; defaults
	//   to the label.
	// - emphasis: set the value in semibold (payee, totals).
	// - extra: snippet rendered under the value, inside the value cell.
	let {
		item,
		marker,
		label,
		copy,
		copyName,
		emphasis = false,
		extra
	}: {
		item: Item;
		marker?: Snippet<[Item]>;
		label?: string;
		copy?: boolean;
		copyName?: string;
		emphasis?: boolean;
		extra?: Snippet;
	} = $props();

	let valueEl = $state<HTMLElement | null>(null);

	const rowLabel = $derived(label ?? item.label);
	const alternatives = $derived(item.alternatives ?? []);
	const disputed = $derived((item.state === 'conflict' || item.state === 'worded') && alternatives.length > 0);
	const canCopy = $derived(
		(copy ?? item.copy) &&
			!disputed &&
			(item.state === 'ok' || item.state === 'stale') &&
			item.factIds.length > 0 &&
			item.display.trim() !== ''
	);
	const missing = $derived(item.state === 'missing');
	const name = $derived(copyName ?? rowLabel.charAt(0).toLowerCase() + rowLabel.slice(1));
</script>

<div
	class="grid grid-cols-[minmax(0,1fr)_minmax(3.5ch,max-content)] gap-x-3 gap-y-1 border-b border-line py-3 sm:grid-cols-[minmax(9rem,14rem)_minmax(0,1fr)_minmax(3.5ch,max-content)]"
	data-item={item.id}
>
	<!-- An unkeyed statement has no label: its value takes the label's room. -->
	<dt class={rowLabel ? 'col-span-2 text-sm font-semibold text-ink-muted sm:col-span-1 sm:pt-0.5' : 'sr-only'}>{rowLabel}</dt>
	{#if disputed}
		<dd class={rowLabel ? 'col-span-2 m-0 min-w-0' : 'col-span-2 m-0 min-w-0 sm:col-span-3'}>
			{#if item.state === 'conflict'}
				<p class="m-0 flex items-start gap-1.5 text-sm font-semibold text-changed">
					<TriangleAlertIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
					The office's pages give different values
				</p>
			{:else}
				<p class="m-0 flex items-start gap-1.5 text-sm font-medium text-ink-muted">
					<InfoIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
					Worded differently on the office's pages
				</p>
			{/if}
			<ul class="m-0 mt-2 list-none p-0">
				{#each alternatives as alt (alt.id)}
					<li class="m-0 grid grid-cols-[minmax(0,1fr)_minmax(3.5ch,max-content)] gap-x-3 py-1">
						<div class="min-w-0">
							{#if alt.label && alt.label !== item.label}
								<span class="mr-1.5 text-sm font-medium text-ink-muted">{alt.label}:</span>
							{/if}
							<span
								class="break-words whitespace-pre-line text-ink [overflow-wrap:anywhere]"
								lang={valueLang(alt.display) ?? undefined}
								{...traceAttrs(alt)}><ValueText text={alt.display} /></span
							>
							{#each alt.contest ?? [] as note (note.id)}
								<p class="m-0 mt-1 flex items-start gap-1.5 text-sm font-medium text-changed" data-note-ids={String(note.id)}>
									<TriangleAlertIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
									<span class="min-w-0 break-words [overflow-wrap:anywhere]">{contestLead(note)}: {note.summary}</span>
								</p>
							{/each}
						</div>
						<span class="text-right text-sm leading-6">{@render marker?.(alt)}</span>
					</li>
				{/each}
			</ul>
			{@render extra?.()}
		</dd>
	{:else}
		<dd class={rowLabel ? 'm-0 min-w-0' : 'm-0 min-w-0 sm:col-span-2'}>
			{#if missing}
				<span class="flex items-start gap-1.5 text-ink-muted">
					<CircleQuestionMarkIcon class="mt-1 size-4 flex-none" aria-hidden="true" />
					<span class="min-w-0 break-words" {...traceAttrs(item)}>{item.display}</span>
				</span>
			{:else}
				<span
					bind:this={valueEl}
					class={`block min-w-0 break-words whitespace-pre-line text-ink [overflow-wrap:anywhere] ${emphasis ? 'font-semibold' : ''}`}
					lang={valueLang(item.display) ?? undefined}
					{...traceAttrs(item)}><ValueText text={item.display} /></span
				>
			{/if}
			{#if item.stale}
				<p
					class={`m-0 mt-1 flex items-start gap-1.5 text-sm ${item.stale.level === 'warning' ? 'font-medium text-changed' : 'text-ink-muted'}`}
				>
					{#if item.stale.level === 'warning'}
						<TriangleAlertIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
					{:else}
						<ClockIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
					{/if}
					{item.stale.text}
				</p>
			{/if}
			{#each item.contest ?? [] as note (note.id)}
				<p class="m-0 mt-1 flex items-start gap-1.5 text-sm font-medium text-changed" data-note-ids={String(note.id)}>
					<TriangleAlertIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
					<span class="min-w-0 break-words [overflow-wrap:anywhere]">{contestLead(note)}: {note.summary}</span>
				</p>
			{/each}
			{#if item.state === 'uncited'}
				<p class="m-0 mt-1 flex items-start gap-1.5 text-sm text-ink-muted">
					<CircleAlertIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
					Source not on file
				</p>
			{/if}
			{#if canCopy}
				<div class="mt-2">
					<CopyButton text={item.display} {name} target={valueEl} />
				</div>
			{/if}
			{@render extra?.()}
		</dd>
		<dd class="m-0 text-right text-sm leading-6">{@render marker?.(item)}</dd>
	{/if}
</div>
