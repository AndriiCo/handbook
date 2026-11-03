<script lang="ts">
	import { flip } from 'svelte/animate';
	import { fade, fly } from 'svelte/transition';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import PathPicker from './PathPicker.svelte';
	import CitationChips from './CitationChips.svelte';
	import OfficeCard from './OfficeCard.svelte';
	import FeeSummary from './FeeSummary.svelte';
	import MailBlock from './MailBlock.svelte';
	import FormsBlock from './FormsBlock.svelte';
	import Callout from './Callout.svelte';
	import {
		blocksFor,
		feeTotal,
		lineCurrency,
		money,
		openStatus,
		type Office,
		type PlanBlock,
		type PlanForm,
		type PlanView
	} from '../plan.js';

	// Allowlisted renderer for an accepted plan: blocks in plan order,
	// filtered by the chosen path; each block kind maps to exactly one
	// component and every one shows its citations. A summary card
	// repeats the live numbers (open now, total, timing) beside it.
	let {
		view,
		path = null,
		hrefFor,
		orderHref,
		fillHref
	}: {
		view: PlanView;
		path?: string | null;
		hrefFor: (id: string | null) => string;
		orderHref: (kind?: string) => string;
		fillHref: (form: PlanForm) => string;
	} = $props();

	let now = $state(new Date());
	$effect(() => {
		const t = setInterval(() => (now = new Date()), 30_000);
		return () => clearInterval(t);
	});

	const reduce =
		typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
	const dur = (ms: number) => (reduce ? 0 : ms);

	let docs = $state(1);

	const blocks = $derived(blocksFor(view.plan, path));
	const officeOf = (id: number) => view.offices.get(id) as Office;
	const formOf = (id: number) => view.forms.get(id) as PlanForm;
	const planOffices = $derived(
		view.plan.blocks.flatMap((b) => (b.kind === 'office' ? b.office_ids : [])).map(officeOf)
	);
	const mailOffice = $derived(
		planOffices.find((o) => o?.mail) ?? [...view.offices.values()].find((o) => o.mail) ?? null
	);

	// Summary card: the first office's live status, the fee total for
	// the chosen document count, the first timing line.
	const glanceOffice = $derived(planOffices.find((o) => o && openStatus(o, now)) ?? null);
	const glanceStatus = $derived(glanceOffice ? openStatus(glanceOffice, now) : null);
	const feeBlock = $derived(blocks.find((b) => b.kind === 'fees') ?? null);
	const glanceTotal = $derived.by(() => {
		if (!feeBlock || feeBlock.kind !== 'fees') return null;
		const cur =
			feeBlock.lines
				.map((l) => lineCurrency(l, l.fact_id !== undefined ? view.facts.get(l.fact_id) : undefined))
				.find(Boolean) ?? null;
		return money(feeTotal(feeBlock.lines, docs), cur);
	});
	const timing = $derived(blocks.find((b) => b.kind === 'timing') ?? null);

	function titleOf(b: PlanBlock): string {
		if (b.kind === 'office') return b.office_ids.length > 1 ? 'Visit an office' : 'Visit the office';
		if (b.kind === 'fees') return 'Pay the fee';
		if (b.kind === 'forms') return b.form_ids.length > 1 ? 'Fill in the forms' : 'Fill in the form';
		return b.title;
	}
	const numbered = $derived(blocks.filter((b) => b.kind !== 'callout').map((b) => b.id));
</script>

{#if view.plan.headline}
	<p class="text-body-lg mt-3 mb-0 max-w-prose text-ink-muted">{view.plan.headline}</p>
{/if}
<PathPicker paths={view.plan.paths} selected={path} {hrefFor} />

<div class="mt-8 grid gap-8 lg:grid-cols-12">
	<ol class="m-0 list-none p-0 lg:col-span-8" aria-label="Plan steps">
		{#each blocks as block (block.id)}
			<li
				class="relative m-0 pb-8 last:pb-0"
				animate:flip={{ duration: dur(250) }}
				in:fly={{ y: 8, duration: dur(250) }}
				out:fade={{ duration: dur(150) }}
				data-block={block.kind}
			>
				{#if block.kind === 'callout'}
					<div class="sm:pl-14">
						<Callout tone={block.tone} title={block.title} body={block.body} facts={view.facts} factIds={block.fact_ids} />
					</div>
				{:else}
					<div class="flex items-start gap-4 sm:gap-5">
						<span
							aria-hidden="true"
							class="flex size-9 shrink-0 items-center justify-center rounded-full border border-line bg-bg font-mono text-sm font-semibold text-ink"
						>
							{#if block.kind === 'timing'}
								<ClockIcon class="size-4" />
							{:else}
								{numbered.indexOf(block.id) + 1}
							{/if}
						</span>
						<h3 class="text-h3 m-0 min-w-0 flex-1 pt-1 font-semibold">{titleOf(block)}</h3>
					</div>
					<div class="mt-3 min-w-0 sm:pl-14">
						{#if block.kind === 'step' || block.kind === 'timing'}
							{#if block.body}<p class="text-body-lg m-0 break-words text-ink">{block.body}</p>{/if}
							<div class="mt-3"><CitationChips ids={block.fact_ids} facts={view.facts} /></div>
						{:else if block.kind === 'office'}
							<div class="flex flex-col gap-4">
								{#each block.office_ids as id (id)}
									<OfficeCard office={officeOf(id)} facts={view.facts} {now} factIds={block.fact_ids} />
								{/each}
							</div>
						{:else if block.kind === 'fees'}
							<FeeSummary lines={block.lines} facts={view.facts} factIds={block.fact_ids} bind:docs />
						{:else if block.kind === 'forms'}
							<FormsBlock forms={block.form_ids.map(formOf)} facts={view.facts} factIds={block.fact_ids} {fillHref} />
						{:else if block.kind === 'mail'}
							<MailBlock
								body={block.body}
								mail={mailOffice?.mail ?? null}
								facts={view.facts}
								factIds={block.fact_ids}
								{orderHref}
							/>
						{/if}
					</div>
				{/if}
			</li>
		{/each}
	</ol>

	<aside class="order-first min-w-0 lg:order-last lg:col-span-4" aria-label="Plan at a glance">
		<div class="glass flex flex-col gap-4 rounded-xl border border-line p-5 shadow-md lg:sticky lg:top-20">
			<p class="m-0 text-eyebrow font-semibold tracking-wide text-ink-muted uppercase">At a glance</p>
			{#if glanceOffice && glanceStatus}
				<div>
					<p class="m-0 text-sm text-ink-muted">{glanceOffice.name}</p>
					<p class={`m-0 mt-1 font-semibold ${glanceStatus.open ? 'text-verified' : 'text-changed'}`}>
						{glanceStatus.text}
					</p>
				</div>
			{/if}
			{#if glanceTotal}
				<div>
					<p class="m-0 text-sm text-ink-muted">
						Fees for {docs} document{docs === 1 ? '' : 's'}
					</p>
					<p class="font-brand text-h2 m-0 mt-1 font-medium text-ink">{glanceTotal}</p>
				</div>
			{/if}
			{#if timing && timing.kind === 'timing'}
				<div>
					<p class="m-0 text-sm text-ink-muted">Timing</p>
					<p class="m-0 mt-1 font-semibold text-ink">{timing.title}</p>
				</div>
			{/if}
			<a
				href={orderHref()}
				class="inline-flex min-h-11 items-center justify-center gap-2 rounded-sm bg-accent-fill px-5 font-medium text-accent-ink no-underline transition-colors hover:bg-accent-hover"
			>
				Start an order
				<ArrowRightIcon class="size-4" aria-hidden="true" />
			</a>
			<a href="#made" class="text-sm font-medium text-accent-strong">How this page was made</a>
		</div>
	</aside>
</div>
