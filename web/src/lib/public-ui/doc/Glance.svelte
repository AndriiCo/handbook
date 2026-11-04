<script lang="ts">
	import type { Snippet } from 'svelte';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import InfoIcon from '@lucide/svelte/icons/info';
	import ArrowDownIcon from '@lucide/svelte/icons/arrow-down';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import FileDownIcon from '@lucide/svelte/icons/file-down';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import CopyButton from './CopyButton.svelte';
	import ValueText from './ValueText.svelte';
	import { GLANCE_ROWS } from '../present.js';
	import { DOC_IDS } from './contract.js';
	import { SLOT, formLink, itemLang, traceAttrs } from './items.js';
	import { guide } from './lang.svelte.js';
	import type { Item } from './contract.js';

	// "At a glance" (plan §3 row 1): cost, time, pay, send and form in
	// one summary list, each value verbatim with its marker. A disputed
	// value, or one a staff note contests, is never shown here: a pointer
	// names the problem and links to its step. Values worded differently
	// show every wording. A value's caveats show here as in its step
	// ("May be out of date…", "Source not on file"), in text with an
	// icon. The office cities are read out of their addresses (traced to
	// them). The payee keeps its Copy button. The cost row points to the
	// faster options when the fees step lists any; the form row links to
	// the form ("Download (PDF)" only when the cited address is a PDF,
	// else the office's page), as step 3 does.
	//
	// Props:
	// - items: `page.glance` (present.js; each Item's slot names its row).
	// - faster: the fees step's number when it lists an expedited or
	//   same-day fee (null step on an unnumbered page); null for none.
	// - marker: snippet `marker(item)` (the row gutter).
	// - inline: snippet `inline(item)` (markers inside a sentence).
	let {
		items = [],
		faster = null,
		marker,
		inline
	}: {
		items?: Item[];
		faster?: { step: number | null } | null;
		marker?: Snippet<[Item]>;
		inline?: Snippet<[Item]>;
	} = $props();

	const t = $derived(guide.t);
	const lang = $derived(guide.lang);
	const partLabels: Record<string, string> = $derived(t.glance.parts);

	type Part = (typeof GLANCE_ROWS)[number]['parts'][number];
	type Entry = { key: string; part: Part; items: Item[] };

	const rows = $derived(
		GLANCE_ROWS.map((row) => {
			const entries: Entry[] = [];
			for (const part of row.parts) {
				const own = items.filter((i) => i.slot === part.slot);
				if (own.length > 0) entries.push({ key: `${row.id}-${part.slot}`, part, items: own });
			}
			const labels: Record<string, string> = t.glance.rows;
			return { id: row.id, label: labels[row.id] ?? row.label, entries };
		}).filter((r) => r.entries.length > 0)
	);
</script>

<dl class="m-0 border-t border-line">
	{#each rows as row (row.id)}
		<div class="grid gap-x-6 gap-y-1 border-b border-line py-4 sm:grid-cols-[8rem_minmax(0,1fr)]" data-glance={row.id}>
			<dt class="text-sm font-semibold text-ink-muted sm:pt-0.5">{row.label}</dt>
			<dd class="m-0 min-w-0">
				<ul class="m-0 flex list-none flex-col gap-2 p-0">
					{#each row.entries as entry (entry.key)}
						{@const first = entry.items[0]}
						<li
							class="m-0 grid grid-cols-[minmax(0,1fr)_minmax(3.5ch,max-content)] gap-x-3 p-0"
							data-item={first.id}
						>
							<div class="min-w-0 break-words [overflow-wrap:anywhere]">
								{#if entry.part.label}
									<span class="mr-2 text-sm font-medium text-ink-muted">{partLabels[entry.part.slot] ?? entry.part.label}</span>
								{/if}
								{#if entry.part.slot === 'address.office'}
									{#each entry.items as city, i (city.id)}{#if i > 0}<span class="text-ink-muted"
												>&nbsp;{t.glance.or}&nbsp;</span
											>{/if}<span class="font-medium text-ink" lang={itemLang(city, lang)} {...traceAttrs(city)}>{city.display}</span>{@render inline?.(
											city
										)}{/each}
								{:else if first.state === 'conflict' || first.state === 'contested'}
									<a
										href={`#${entry.part.anchor}`}
										class="inline-flex items-start gap-1.5 font-medium text-changed underline underline-offset-2"
										{...traceAttrs(first)}
									>
										<TriangleAlertIcon class="mt-1 size-4 flex-none" aria-hidden="true" />
										<span>{first.display}</span>
										<ArrowDownIcon class="mt-1 size-4 flex-none" aria-hidden="true" />
									</a>
								{:else if first.state === 'worded' && first.alternatives}
									<span class="inline-flex items-start gap-1.5">
										<InfoIcon class="mt-1 size-4 flex-none text-ink-muted" aria-hidden="true" />
										<span>
											{#each first.alternatives as alt, i (alt.id)}{#if i > 0}<span class="text-ink-muted"
														>&nbsp;/&nbsp;</span
													>{/if}<span class="text-ink" lang={itemLang(alt, lang)} {...traceAttrs(alt)}
													><ValueText text={alt.display} /></span
												>{@render inline?.(alt)}{/each}
											<span class="text-sm text-ink-muted">{t.glance.worded}</span>
										</span>
									</span>
								{:else}
									<span
										class="font-medium whitespace-pre-line text-ink"
										lang={itemLang(first, lang)}
										{...traceAttrs(first)}><ValueText text={first.display} /></span
									>
									{#if first.copy}
										<span class="ml-2 inline-block align-middle"><CopyButton text={first.display} name={t.common.copyNames.payee} /></span>
									{/if}
									{#if first.stale}
										<span
											class={`mt-1 flex items-start gap-1.5 text-sm ${first.stale.level === 'warning' ? 'font-medium text-changed' : 'text-ink-muted'}`}
										>
											{#if first.stale.level === 'warning'}
												<TriangleAlertIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
											{:else}
												<ClockIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
											{/if}
											{first.stale.text}
										</span>
									{/if}
									{#if first.state === 'uncited'}
										<span class="mt-1 flex items-start gap-1.5 text-sm text-ink-muted">
											<CircleAlertIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
											{t.common.sourceNotOnFile}
										</span>
									{/if}
								{/if}
								{#if entry.part.slot === SLOT.form}
									{@const link = formLink(first)}
									{#if link}
										<a
											href={link.url}
											target={link.pdf ? undefined : '_blank'}
											rel={link.pdf ? undefined : 'noopener noreferrer'}
											class="mt-1 flex w-fit min-h-6 items-center gap-1.5 text-sm font-medium text-accent-strong underline underline-offset-2 print:hidden"
											data-glance-form={link.pdf ? 'pdf' : 'page'}
										>
											{#if link.pdf}
												<FileDownIcon class="size-4 flex-none" aria-hidden="true" />{t.glance.downloadPdf}
											{:else}
												{t.glance.getOnPage}<ExternalLinkIcon class="size-4 flex-none" aria-hidden="true" /><span
													class="sr-only">{t.common.newTab}</span
												>
											{/if}
										</a>
									{/if}
								{/if}
							</div>
							<span class="text-right text-sm leading-6">
								{#if entry.part.slot !== 'address.office' && first.state !== 'worded'}{@render marker?.(first)}{/if}
							</span>
						</li>
						<!-- The cost's pointer to the faster options sits on a line of
						     its own after the cost's marker, so the marker stays the
						     row's first stop for the keyboard. -->
						{#if entry.part.slot === SLOT.perDocument && faster && first.state !== 'conflict' && first.state !== 'contested'}
							<li class="m-0 p-0 print:hidden">
								<a
									href={`#${DOC_IDS.fees}`}
									class="inline-flex min-h-6 items-center text-sm font-medium text-accent-strong underline underline-offset-2"
									data-glance-faster
									>{#if faster.step !== null}{t.glance.fasterStep(faster.step)}{:else}{t.glance.fasterFees}{/if}</a
								>
							</li>
						{/if}
					{/each}
				</ul>
			</dd>
		</div>
	{/each}
</dl>
