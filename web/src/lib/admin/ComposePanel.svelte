<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { fade, fly } from 'svelte/transition';
	import { resolve } from '$app/paths';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import Notice from '#lib/public-ui/Notice.svelte';
	import AuthorityPicker from './AuthorityPicker.svelte';
	import { ComposeRun, type ComposeKind, type ComposeSummary, type StepDef, type StepState } from './compose.svelte.js';
	import type { AuthorityOption } from './authorities.js';
	import { fmtCompact, fmtMs, fmtPct, modelName } from './format.js';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import CircleIcon from '@lucide/svelte/icons/circle';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import CircleXIcon from '@lucide/svelte/icons/circle-x';
	import CircleDashedIcon from '@lucide/svelte/icons/circle-dashed';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';

	// Compose card (MISSION WIN W1e): pick an authority, start a Muse
	// Spark run, watch it move through the validator (and Muse Wonder
	// for plans) live over `run:{id}`, then read the numbers and the
	// per-call rows. `compact` is the dashboard form: one line of
	// steps, one line of numbers, a link to the proposal.
	let {
		kind,
		compact = false,
		initialSlug = '',
		initialRun = null,
		busy = $bindable(false),
		onDone
	}: {
		kind: ComposeKind;
		compact?: boolean;
		initialSlug?: string;
		initialRun?: number | null;
		busy?: boolean;
		onDone?: (summary: ComposeSummary, run: ComposeRun) => void | Promise<void>;
	} = $props();

	const run = new ComposeRun(
		untrack(() => kind),
		(s, r) => onDone?.(s, r)
	);
	let slug = $state(untrack(() => initialSlug));
	let authorities = $state<AuthorityOption[]>([]);
	let picker = $state<AuthorityPicker | null>(null);
	const pickerId = untrack(() => `compose-${kind}${compact ? '-compact' : ''}`);

	onMount(() => {
		if (typeof initialRun === 'number' && initialRun > 0) run.attach(initialRun, initialSlug);
		return () => run.dispose();
	});

	$effect(() => {
		busy = run.busy;
	});
	// A reopened run names its authority once the server answers.
	$effect(() => {
		if (run.slug && !slug) slug = run.slug;
	});

	export function focusPicker() {
		picker?.focus();
	}

	const isPlans = untrack(() => kind === 'plans');
	const title = isPlans ? 'Compose a page plan' : 'Structure offices';
	const action = isPlans ? 'Compose plan with Muse Spark' : 'Structure offices';
	const working = isPlans ? 'Composing' : 'Structuring';
	const description = isPlans
		? 'Muse Spark drafts the visitor plan from accepted facts. The validator and Muse Wonder check every block before it reaches this queue.'
		: 'Muse Spark turns accepted address, hours and mail facts into office cards. Guards keep only what the cited fact says.';

	const authority = $derived(authorities.find((a) => a.slug === slug) ?? null);
	// Plans need three accepted facts (plan.rs MIN_FACTS); say so
	// before the run, the server still decides.
	const thinFacts = $derived(isPlans && authority !== null && authority.fact_count < 3);

	const sumLatency = (prefix: string): number | null => {
		const rows = run.callsBy(prefix);
		return rows.length ? rows.reduce((n, c) => n + (c.latency_ms ?? 0), 0) : null;
	};
	const firstToken = (prefix: string): number | null => run.callsBy(prefix).find((c) => c.ttft_ms !== null)?.ttft_ms ?? null;

	function secs(ms: number | null): string {
		return ms === null ? '' : `${(ms / 1000).toFixed(1)} s`;
	}

	function detail(step: StepDef, state: StepState): string {
		const s = run.summary;
		if (state === 'failed') return step.id === 'queued' ? 'Not started' : 'Stopped here';
		if (state === 'skipped') return 'Did not run';
		if (state === 'active') {
			if (step.id === 'compose') return run.elapsedMs === null ? 'Waiting for the run' : secs(run.elapsedMs);
			return step.hint;
		}
		if (state !== 'done') return step.hint;
		switch (step.id) {
			case 'queued':
				return `Run ${run.runId ?? ''}${run.slug ? ` · ${run.slug}` : ''}`;
			case 'compose': {
				const ms = sumLatency('muse-spark');
				const ttft = firstToken('muse-spark');
				if (ms !== null) return `${fmtMs(ms)}${ttft !== null ? ` · first token ${fmtMs(ttft)}` : ''}`;
				return run.phase === 'done' && run.callsState === 'loading' ? 'Reading calls' : modelName(run.model);
			}
			case 'validate':
				if (!s) return step.hint;
				if (isPlans) return s.passed === false ? 'Flagged for staff' : 'Passed';
				return `${s.proposed.length} kept · ${s.dropped} dropped`;
			case 'entail': {
				const ms = sumLatency('muse-wonder');
				return ms !== null ? `Every block checked · ${fmtMs(ms)}` : 'Every block checked';
			}
			case 'done':
				if (!s) return step.hint;
				if (isPlans) return s.version !== null ? `Plan v${s.version}${s.passed === false ? ' · needs staff' : ''}` : 'Plan stored';
				return `${s.proposed.length} office${s.proposed.length === 1 ? '' : 's'} proposed`;
		}
		return step.hint;
	}

	// Full class strings per state (one background wins per element).
	const stepCls = (state: StepState) =>
		`flex min-w-0 items-start gap-2 rounded-md border px-3 py-2 transition-colors duration-200 ${
			state === 'active'
				? 'border-focus bg-surface shadow-sm'
				: state === 'done'
					? 'border-line bg-card'
					: state === 'failed'
						? 'border-error bg-error-bg'
						: 'border-dashed border-line bg-card'
		}`;
	const pillCls = (state: StepState) =>
		`inline-flex items-center gap-1 rounded-full border px-2 py-1 text-xs ${
			state === 'active'
				? 'border-focus bg-surface font-semibold text-ink'
				: state === 'done'
					? 'border-line bg-card text-ink'
					: state === 'failed'
						? 'border-error bg-error-bg text-ink'
						: 'border-dashed border-line bg-card text-ink-muted'
		}`;
	const gridCls = isPlans ? 'm-0 grid list-none gap-2 p-0 md:grid-cols-2 lg:grid-cols-5' : 'm-0 grid list-none gap-2 p-0 md:grid-cols-2 lg:grid-cols-4';

	const summaryLine = (s: ComposeSummary): string => {
		const head = isPlans
			? `${s.blocks ?? 0} block${s.blocks === 1 ? '' : 's'} kept · ${s.dropped} dropped`
			: `${s.proposed.length} office${s.proposed.length === 1 ? '' : 's'} · ${s.dropped} dropped`;
		return `${head} · ${fmtMs(s.latency_ms)} · ${fmtCompact(s.input_tokens)} in (${fmtCompact(s.cached_tokens)} cached) · ${fmtCompact(s.output_tokens)} out · ${fmtCompact(s.reasoning_tokens)} reasoning`;
	};

	const metrics = (s: ComposeSummary) => [
		isPlans
			? { label: 'Blocks kept', value: String(s.blocks ?? 0), hint: s.passed === false ? 'needs staff' : 'validator passed' }
			: { label: 'Offices proposed', value: String(s.proposed.length), hint: 'guards applied' },
		{ label: 'Dropped', value: String(s.dropped), hint: isPlans ? 'validator + Wonder' : 'nothing grounded' },
		{ label: 'Latency', value: fmtMs(s.latency_ms), hint: 'model time, all calls' },
		{ label: 'Input tokens', value: fmtCompact(s.input_tokens), hint: 'prompt and facts' },
		{
			label: 'Cached',
			value: fmtCompact(s.cached_tokens),
			hint: s.input_tokens > 0 ? `${fmtPct(s.cached_tokens / s.input_tokens)} of input` : 'of input'
		},
		{ label: 'Output tokens', value: fmtCompact(s.output_tokens), hint: 'structured JSON' },
		{ label: 'Reasoning', value: fmtCompact(s.reasoning_tokens), hint: 'thinking tokens' }
	];

	const maxLatency = $derived(Math.max(1, ...run.calls.map((c) => c.latency_ms ?? 0)));
	const OUTCOME: Record<string, string> = {
		ok: 'OK',
		refusal: 'Refusal',
		length: 'Cut off',
		http_4xx: 'HTTP 4xx',
		http_5xx: 'HTTP 5xx',
		timeout: 'Timeout',
		transport: 'Network',
		parse_error: 'Parse error'
	};

	const announce = $derived.by(() => {
		if (run.phase === 'running') return `Run ${run.runId ?? ''} started for ${run.slug || 'the authority'}.`;
		if (run.phase === 'done' && run.summary) return `Run ${run.runId ?? ''} done: ${summaryLine(run.summary)}.`;
		if (run.phase === 'failed') return run.error ?? 'The run failed.';
		return '';
	});

	const failHeading = $derived(
		run.failKind === 'cap'
			? 'Daily Meta call cap reached'
			: run.failKind === 'unknown'
				? 'Unknown authority'
				: run.failKind === 'facts'
					? 'Not enough accepted facts'
					: 'The run failed'
	);

	const review = resolve('/(admin)/review');
	const reviewHref = $derived(
		isPlans && run.runId !== null ? `${review}?tab=plans&run=${run.runId}` : `${review}?tab=offices`
	);
	const reviewLabel = $derived(
		run.summary
			? isPlans
				? `Review plan v${run.summary.version ?? ''}`
				: `Review ${run.summary.proposed.length} office${run.summary.proposed.length === 1 ? '' : 's'}`
			: 'Open the review queue'
	);
	const motion = typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 0 : 200;
</script>

{#snippet stepIcon(state: StepState)}
	{#if state === 'active'}
		<LoaderCircleIcon class="mt-0.5 size-4 flex-none text-accent-strong motion-safe:animate-spin" aria-hidden="true" />
	{:else if state === 'done'}
		<CircleCheckIcon class="mt-0.5 size-4 flex-none text-verified" aria-hidden="true" />
	{:else if state === 'failed'}
		<CircleXIcon class="mt-0.5 size-4 flex-none text-error" aria-hidden="true" />
	{:else if state === 'skipped'}
		<CircleDashedIcon class="mt-0.5 size-4 flex-none text-ink-muted" aria-hidden="true" />
	{:else}
		<CircleIcon class="mt-0.5 size-4 flex-none text-ink-muted" aria-hidden="true" />
	{/if}
{/snippet}

{#snippet failure()}
	<Notice variant={run.failKind === 'cap' ? 'warning' : 'error'} heading={failHeading}>
		<p class="m-0 break-words">{run.error}</p>
		<p class="mt-3 mb-0">
			{#if run.failKind === 'cap'}
				<Button variant="outline" href={resolve('/(admin)/config')}>Open config</Button>
			{:else if run.failKind === 'unknown'}
				<Button variant="outline" onclick={focusPicker}>Choose another authority</Button>
			{:else if run.failKind === 'facts'}
				<Button variant="outline" href={`${review}?authority=${encodeURIComponent(slug)}`} data-sveltekit-reload>
					Review facts for {slug}
				</Button>
			{:else}
				<Button variant="outline" onclick={() => run.start(slug)} disabled={!slug}>Try again</Button>
			{/if}
		</p>
	</Notice>
{/snippet}

<Card.Root class={compact ? 'h-full gap-3 shadow-sm' : 'gap-4 shadow-sm'}>
	<Card.Header>
		<Card.Title class="flex flex-wrap items-center gap-2 text-body font-semibold">
			<SparklesIcon class="size-4 text-accent-strong" aria-hidden="true" />
			<svelte:element this={compact ? 'h3' : 'h2'} class="m-0 text-body font-semibold">{title}</svelte:element>
			{#if run.runId !== null}
				<Badge variant="outline" class="font-mono">run {run.runId}</Badge>
			{/if}
			{#if run.phase === 'done'}
				<Badge variant="outline" class="border-verified text-verified">Done</Badge>
			{:else if run.phase === 'failed'}
				<Badge variant="outline" class="border-error text-error">Failed</Badge>
			{:else if run.busy}
				<Badge variant="outline" class="border-focus text-ink">Live</Badge>
			{/if}
		</Card.Title>
		{#if !compact}
			<Card.Description>{description}</Card.Description>
		{/if}
	</Card.Header>
	<Card.Content class="flex flex-col gap-4">
		<div class={compact ? 'flex flex-col gap-2 md:flex-row md:items-end lg:max-w-3xl' : 'flex flex-col gap-2 md:flex-row md:items-end'}>
			<div class="min-w-0 flex-1">
				<AuthorityPicker
					bind:this={picker}
					bind:value={slug}
					id={pickerId}
					disabled={run.busy}
					onLoaded={(rows) => (authorities = rows)}
				/>
			</div>
			<Button size="lg" disabled={!slug || run.busy} onclick={() => run.start(slug)}>
				{#if run.busy}
					<LoaderCircleIcon class="motion-safe:animate-spin" aria-hidden="true" />
					{working}…
				{:else}
					<SparklesIcon aria-hidden="true" />
					{action}
				{/if}
			</Button>
		</div>
		{#if thinFacts && authority && run.phase === 'idle'}
			<p class="m-0 text-xs text-ink-muted">
				{authority.name} shows {authority.fact_count} accepted fact{authority.fact_count === 1 ? '' : 's'}; a plan needs three.
			</p>
		{/if}

		<p class="sr-only" role="status" aria-live="polite">{announce}</p>

		{#if run.phase !== 'idle'}
			{#if compact}
				<ol class="m-0 flex list-none flex-wrap gap-1 p-0" aria-label="Run timeline" in:fade={{ duration: motion }}>
					{#each run.steps as step, i (step.id)}
						{@const state = run.stepState(i)}
						<li class={pillCls(state)} aria-current={state === 'active' ? 'step' : undefined}>
							{@render stepIcon(state)}
							<span>{state === 'active' ? step.active : step.label}</span>
							{#if state === 'active' && step.id === 'compose' && run.elapsedMs !== null}
								<span class="font-mono tabular-nums">{secs(run.elapsedMs)}</span>
							{/if}
						</li>
					{/each}
				</ol>
			{:else}
				<ol class={gridCls} aria-label="Run timeline" in:fade={{ duration: motion }}>
					{#each run.steps as step, i (step.id)}
						{@const state = run.stepState(i)}
						<li class={stepCls(state)} aria-current={state === 'active' ? 'step' : undefined}>
							{@render stepIcon(state)}
							<span class="flex min-w-0 flex-col">
								<span class="text-sm font-medium">{state === 'active' ? step.active : step.label}</span>
								<span class="font-mono text-xs break-words text-ink-muted tabular-nums">{detail(step, state)}</span>
							</span>
						</li>
					{/each}
				</ol>
			{/if}
		{/if}

		{#if run.phase === 'running' && run.stalled}
			<div class="flex flex-wrap items-center gap-2 rounded-md border border-warning bg-warning-bg px-3 py-2 text-sm" role="status">
				<span class="min-w-0 flex-1">No word from run {run.runId} in five minutes.</span>
				<Button variant="outline" size="sm" disabled={run.checking} onclick={() => run.check()}>
					{run.checking ? 'Checking…' : 'Check the run'}
				</Button>
			</div>
		{:else if run.phase === 'running' && run.slow}
			<p class="m-0 text-xs text-ink-muted">
				High-reasoning runs can take a minute or two. The timeline moves the moment Muse Spark answers.
			</p>
		{/if}

		{#if run.phase === 'failed'}
			{@render failure()}
		{/if}

		{#if run.phase === 'done' && run.summary}
			{@const s = run.summary}
			{#if compact}
				<div class="flex flex-col gap-3" in:fly={{ y: 4, duration: motion }}>
					<p class="m-0 font-mono text-xs text-ink-muted tabular-nums">{summaryLine(s)}</p>
					<div>
						<Button href={reviewHref}>
							{reviewLabel}
							<ArrowRightIcon aria-hidden="true" />
						</Button>
					</div>
				</div>
			{:else}
				<dl class="m-0 grid grid-cols-2 gap-3 md:grid-cols-4 xl:grid-cols-7" in:fly={{ y: 4, duration: motion }}>
					{#each metrics(s) as m (m.label)}
						<div class="flex min-w-0 flex-col rounded-md bg-surface px-3 py-2">
							<dt class="truncate text-xs text-ink-muted">{m.label}</dt>
							<dd class="m-0 text-h3 font-semibold tabular-nums">{m.value}</dd>
							<dd class="m-0 truncate text-xs text-ink-muted">{m.hint}</dd>
						</div>
					{/each}
				</dl>
			{/if}
		{/if}

		{#if !compact && run.runId !== null && (run.phase === 'done' || run.phase === 'failed')}
			<section class="flex flex-col gap-2" aria-label={`Model calls in run ${run.runId}`}>
				<h3 class="m-0 text-sm font-semibold">Calls in run {run.runId}</h3>
				{#if run.callsState === 'loading' || run.callsState === 'idle'}
					<Skeleton class="h-16 rounded-md" />
				{:else if run.callsState === 'failed'}
					<p class="m-0 text-sm text-ink-muted" role="status">Per-call rows could not load; the trace console has them.</p>
				{:else if run.calls.length === 0}
					<p class="m-0 text-sm text-ink-muted">No model call was recorded for this run.</p>
				{:else}
					<div class="overflow-hidden rounded-md border border-line">
						<table class="block w-full border-collapse text-sm md:table">
							<caption class="sr-only">Model calls in run {run.runId}, in order</caption>
							<thead class="sr-only bg-surface md:not-sr-only">
								<tr class="text-left text-xs text-ink-muted">
									<th scope="col" class="px-3 py-2 font-medium">Call</th>
									<th scope="col" class="px-3 py-2 font-medium">Latency</th>
									<th scope="col" class="px-3 py-2 text-right font-medium">TTFT</th>
									<th scope="col" class="px-3 py-2 text-right font-medium">Tokens in / cached / out / reasoning</th>
									<th scope="col" class="px-3 py-2 text-right font-medium">Outcome</th>
								</tr>
							</thead>
							<tbody class="block md:table-row-group">
								{#each run.calls as c (c.id)}
									<tr class="flex flex-col gap-1 border-t border-line px-3 py-2 md:table-row md:px-0 md:py-0">
										<td class="md:px-3 md:py-2">
											<span class="font-semibold">{modelName(c.model)}</span>
											<span class="block font-mono text-xs text-ink-muted">{c.model} · {c.pipeline} · attempt {c.attempt}</span>
										</td>
										<td class="md:w-48 md:px-3 md:py-2">
											<span class="flex items-center gap-2">
												<span class="h-2 flex-1" aria-hidden="true">
													<span
														class={`block h-full rounded-r-xs ${c.outcome === 'ok' ? 'bg-accent-strong' : 'bg-changed'}`}
														style={`width:${Math.max(2, ((c.latency_ms ?? 0) / maxLatency) * 100)}%`}
													></span>
												</span>
												<span class="w-14 text-right font-mono text-xs tabular-nums">{fmtMs(c.latency_ms)}</span>
											</span>
										</td>
										<td class="font-mono text-xs tabular-nums md:px-3 md:py-2 md:text-right">
											<span class="text-ink-muted md:sr-only">TTFT </span>{fmtMs(c.ttft_ms)}
										</td>
										<td class="font-mono text-xs tabular-nums md:px-3 md:py-2 md:text-right">
											<span class="text-ink-muted md:sr-only">Tokens </span>{fmtCompact(c.input_tokens)} / {fmtCompact(c.cached_tokens)} / {fmtCompact(c.output_tokens)} / {fmtCompact(c.reasoning_tokens)}
										</td>
										<td class="md:px-3 md:py-2 md:text-right">
											<Badge variant="outline" class={c.outcome === 'ok' ? 'border-verified text-verified' : 'border-changed text-changed'}>
												{OUTCOME[c.outcome] ?? c.outcome}
											</Badge>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</section>
		{/if}
	</Card.Content>
</Card.Root>
