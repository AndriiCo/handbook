<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { fade, fly } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import { resolve } from '$app/paths';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import Notice from '#lib/public-ui/Notice.svelte';
	import AuthorityPicker from './AuthorityPicker.svelte';
	import StepIcon from './StepIcon.svelte';
	import { CompileRun, fetchCompileRuns } from './compile.svelte.js';
	import {
		COMPILE_STEPS,
		MAX_SEEDS,
		countChips,
		detailText,
		fmtElapsed,
		parseSeeds,
		sentence,
		summaryLine,
		type CompileRunRow,
		type CompileRuns,
		type CompileStepState
	} from './compile-steps.js';
	import type { AuthorityOption } from './authorities.js';
	import { errorText } from './admin-api.js';
	import { ago, fmtMs } from './format.js';
	import { subscribe } from '#lib/live.svelte.js';
	import WorkflowIcon from '@lucide/svelte/icons/workflow';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import TimerIcon from '@lucide/svelte/icons/timer';
	import XIcon from '@lucide/svelte/icons/x';

	// Compile card (dashboard): one staff button walks an authority
	// through capture → Muse Spark drafts → Muse Wonder cross-check →
	// offices → page plan on the server; the steps move live over the
	// staff-only `run:{id}` topic. Nothing a run makes is accepted
	// here: every draft, office and plan waits in /review.
	let {
		now,
		openRun = null
	}: {
		/** Unix seconds for relative times (the dashboard's clock). */
		now: number;
		/** A run to show on arrival (`?compile=` links). */
		openRun?: number | null;
	} = $props();

	let runs = $state<CompileRuns | null>(null);
	let runsState = $state<'loading' | 'ready' | 'failed'>('loading');
	let runsError = $state<string | null>(null);
	let slug = $state('');
	let seedsText = $state('');
	let authorities = $state<AuthorityOption[]>([]);
	let picker = $state<AuthorityPicker | null>(null);
	let seedsEl = $state<HTMLTextAreaElement | null>(null);
	let root = $state<HTMLElement | null>(null);
	let reduceMotion = $state(false);
	let reloadTimer: ReturnType<typeof setTimeout> | null = null;

	const run = new CompileRun(() => void loadRuns(true));

	async function loadRuns(quiet = false) {
		if (!quiet || !runs) runsState = 'loading';
		try {
			runs = await fetchCompileRuns();
			runsState = 'ready';
			runsError = null;
		} catch (err) {
			runsError = errorText(err, 'Compile runs failed to load.');
			if (!runs) runsState = 'failed';
		}
	}

	function reloadSoon() {
		if (reloadTimer !== null) clearTimeout(reloadTimer);
		reloadTimer = setTimeout(() => {
			reloadTimer = null;
			void loadRuns(true);
		}, 800);
	}

	onMount(() => {
		reduceMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
		void loadRuns();
		// Runs other staff finish move the list and the daily counter.
		const unsub = subscribe('ops', (msg) => {
			if (msg?.type === 'compile.done') reloadSoon();
		});
		return () => {
			unsub();
			if (reloadTimer !== null) clearTimeout(reloadTimer);
			run.dispose();
		};
	});

	// A `?compile=` link (the activity feed, a teammate) opens that run.
	$effect(() => {
		const id = openRun;
		if (typeof id !== 'number' || !(id > 0)) return;
		untrack(() => {
			if (run.runId === id) return;
			run.attach(id);
			void tick().then(() =>
				root?.scrollIntoView({ behavior: reduceMotion ? 'auto' : 'smooth', block: 'start' })
			);
		});
	});

	// A reopened run names its authority once the server answers.
	$effect(() => {
		const named = run.reopened ? run.slug : '';
		if (named && !untrack(() => slug)) slug = named;
	});

	const seeds = $derived(parseSeeds(seedsText));
	const capped = $derived(runs !== null && runs.today >= runs.daily_runs);
	/** A run started here holds the form; a watched one does not. */
	const locked = $derived(run.phase === 'starting' || (run.busy && !run.reopened));
	const canStart = $derived(Boolean(slug) && !locked && !capped && seeds.errors.length === 0);
	const motion = $derived(reduceMotion ? 0 : 200);

	const nameOf = (s: string) => authorities.find((a) => a.slug === s)?.name ?? s;
	const review = resolve('/(admin)/review');
	const reviewHref = $derived(run.slug ? `${review}?authority=${encodeURIComponent(run.slug)}` : review);

	async function go() {
		if (!canStart) return;
		await run.start(slug, seeds.urls);
		// The new row and today's count, from the server.
		if (run.runId !== null) void loadRuns(true);
	}

	function openRow(r: CompileRunRow) {
		if (run.phase === 'starting') return;
		run.attach(r.run_id, r.slug);
	}

	const STATUS_TEXT: Record<CompileStepState['status'], string> = {
		pending: 'Waiting',
		running: 'Running',
		done: 'Done',
		skipped: 'Skipped',
		stopped: 'Stopped',
		failed: 'Failed'
	};

	function stepNote(step: (typeof COMPILE_STEPS)[number], s: CompileStepState): string | null {
		if (s.status === 'skipped' || s.status === 'stopped') {
			return sentence(s.reason) ?? (s.status === 'skipped' ? 'Did not run' : 'Stopped before the end');
		}
		if (s.status === 'failed') return 'Stopped here';
		if (s.status === 'running') return detailText(s.detail) ?? step.hint;
		if (s.status === 'pending') return step.hint;
		return null;
	}

	const rowCls = (s: CompileStepState['status']) =>
		`relative flex gap-3 rounded-md px-2 py-2 transition-colors duration-200 md:px-3 ${
			s === 'running' ? 'bg-surface' : s === 'failed' ? 'bg-error-bg' : ''
		}`;
	const chipCls = (warn: boolean) =>
		`inline-flex items-center rounded-sm border px-2 font-mono text-xs tabular-nums ${
			warn ? 'border-changed bg-changed-bg text-ink' : 'border-line bg-card text-ink'
		}`;
	const badgeCls = (status: string) =>
		status === 'done'
			? 'border-verified text-verified'
			: status === 'failed'
				? 'border-error text-error'
				: 'border-focus text-ink';
	const badgeText = (status: string) => (status === 'done' ? 'Done' : status === 'failed' ? 'Failed' : 'Live');
	const rowIcon = (status: string) => (status === 'done' ? 'done' : status === 'failed' ? 'failed' : 'running');

	const failHeading = $derived(
		run.failKind === 'conflict'
			? `Already compiling ${nameOf(run.slug)}`
			: run.failKind === 'cap'
				? 'Daily cap reached'
				: run.failKind === 'unknown'
					? 'Unknown authority'
					: run.failKind === 'seeds'
						? 'Check the extra URLs'
						: run.failKind === 'run'
							? `Run ${run.runId ?? ''} failed`
							: 'Compile did not start'
	);

	const announce = $derived.by(() => {
		if (run.phase === 'running') return `Compile run ${run.runId ?? ''} running for ${nameOf(run.slug)}.`;
		if (run.phase === 'done') return `Compile run ${run.runId ?? ''} done: ${summaryLine(run.summary)}.`;
		if (run.phase === 'failed') return run.error ?? 'Compile failed.';
		return '';
	});
</script>

<section bind:this={root} aria-labelledby="compile-title" class="scroll-mt-20">
	<Card.Root class="gap-4 shadow-sm">
		<Card.Header class="flex flex-wrap items-start justify-between gap-3">
			<div class="flex min-w-0 flex-1 basis-72 flex-col gap-1">
				<div class="flex flex-wrap items-center gap-2">
					<WorkflowIcon class="size-4 text-accent-strong" aria-hidden="true" />
					<h2 id="compile-title" class="m-0 text-h3 font-semibold">Compile an authority</h2>
					{#if run.runId !== null}
						<Badge variant="outline" class="font-mono">run {run.runId}</Badge>
					{/if}
					{#if run.phase === 'done'}
						<Badge variant="outline" class="border-verified text-verified">Done</Badge>
					{:else if run.phase === 'failed' && run.failKind === 'run'}
						<Badge variant="outline" class="border-error text-error">Failed</Badge>
					{:else if run.phase === 'running'}
						<Badge variant="outline" class="border-focus text-ink">Live</Badge>
					{/if}
				</div>
				<p class="m-0 text-sm text-ink-muted">
					One run captures the official pages, drafts facts with Muse Spark, cross-checks them with
					Muse Wonder, then structures offices and composes the page plan.
				</p>
			</div>
			{#if runs}
				<span
					class={`inline-flex h-7 flex-none items-center rounded-full border px-3 font-mono text-xs tabular-nums ${
						capped ? 'border-warning bg-warning-bg text-ink' : 'border-line bg-surface text-ink'
					}`}
				>
					{runs.today} of {runs.daily_runs} runs today
				</span>
			{:else if runsState === 'loading'}
				<Skeleton class="h-7 w-36 rounded-full" />
			{/if}
		</Card.Header>

		<Card.Content class="grid gap-6 lg:grid-cols-3">
			<div class="flex min-w-0 flex-col gap-4 lg:col-span-2">
				<div class="flex flex-col gap-2 md:flex-row md:items-end">
					<div class="min-w-0 flex-1">
						<AuthorityPicker
							bind:this={picker}
							bind:value={slug}
							id="compile-authority"
							disabled={locked}
							onLoaded={(rows) => (authorities = rows)}
						/>
					</div>
					<Button size="lg" disabled={!canStart} onclick={go}>
						{#if locked}
							<LoaderCircleIcon class="motion-safe:animate-spin" aria-hidden="true" />
							Compiling…
						{:else}
							<WorkflowIcon aria-hidden="true" />
							Compile
						{/if}
					</Button>
				</div>

				<label class="flex min-w-0 flex-col gap-1" for="compile-seeds">
					<span class="flex items-center justify-between gap-2 text-xs font-medium text-ink-muted">
						<span>Extra official URLs, one per line (optional)</span>
						<span class="font-mono tabular-nums" aria-hidden="true">{seeds.urls.length} / {MAX_SEEDS}</span>
					</span>
					<Textarea
						id="compile-seeds"
						bind:ref={seedsEl}
						bind:value={seedsText}
						class="min-h-16 font-mono text-sm"
						rows={2}
						spellcheck={false}
						autocomplete="off"
						disabled={locked}
						aria-invalid={seeds.errors.length > 0}
						aria-describedby={seeds.errors.length > 0 ? 'compile-seeds-error' : undefined}
						placeholder="https://www.sos.wa.gov/…"
					/>
				</label>
				{#if seeds.errors.length > 0}
					<p id="compile-seeds-error" class="-mt-2 mb-0 text-xs text-error" role="status">{seeds.errors[0]}</p>
				{/if}
				{#if capped && run.phase === 'idle'}
					<p class="m-0 text-xs text-ink-muted">
						Today's {runs?.daily_runs} runs are used; the count resets at 00:00 UTC.
						<a class="text-ink underline underline-offset-4" href={resolve('/(admin)/config')}>Raise compile.daily_runs</a>
					</p>
				{/if}

				<p class="m-0 flex items-start gap-2 text-sm text-ink-muted">
					<ShieldCheckIcon class="mt-0.5 size-4 flex-none text-verified" aria-hidden="true" />
					<span>
						Nothing is accepted automatically. Everything lands in Review as a proposal.
						<a class="font-medium text-ink underline underline-offset-4" href={reviewHref}>Open Review</a>
					</span>
				</p>

				<p class="sr-only" role="status" aria-live="polite">{announce}</p>

				{#if run.failKind && run.failKind !== 'run'}
					<div in:fade={{ duration: motion }}>
						<Notice variant={run.failKind === 'conflict' || run.failKind === 'cap' ? 'warning' : 'error'} heading={failHeading}>
							<p class="m-0 break-words">{run.error}</p>
							<p class="mt-3 mb-0">
								{#if run.failKind === 'conflict' && run.conflictRunId !== null}
									<Button variant="outline" onclick={() => run.attach(run.conflictRunId ?? 0, run.slug)}>
										Watch the running one
									</Button>
								{:else if run.failKind === 'cap'}
									<Button variant="outline" href={resolve('/(admin)/config')}>Open config</Button>
								{:else if run.failKind === 'unknown'}
									<Button variant="outline" onclick={() => picker?.focus()}>Choose another authority</Button>
								{:else if run.failKind === 'seeds'}
									<Button variant="outline" onclick={() => seedsEl?.focus()}>Edit the URLs</Button>
								{:else}
									<Button variant="outline" onclick={go} disabled={!canStart}>Try again</Button>
								{/if}
							</p>
						</Notice>
					</div>
				{/if}

				{#if run.runId !== null}
					<div class="flex min-w-0 flex-col gap-2 rounded-lg border border-line p-2" in:fade={{ duration: motion }}>
						<div class="flex items-center gap-2 px-2 pt-1 md:px-3">
							<p class="m-0 min-w-0 flex-1 truncate text-sm font-semibold">
								{run.slug ? nameOf(run.slug) : `Run ${run.runId}`}
							</p>
							<span class="flex flex-none items-center gap-2">
								{#if run.elapsedMs !== null}
									<span class="inline-flex items-center gap-1 font-mono text-xs text-ink-muted tabular-nums">
										<TimerIcon class="size-3" aria-hidden="true" />
										<span class="sr-only">Elapsed</span>
										{fmtElapsed(run.elapsedMs)}
									</span>
								{/if}
								{#if !locked}
									<Button
										variant="ghost"
										size="icon-sm"
										aria-label={`Close run ${run.runId}`}
										onclick={() => run.reset()}
									>
										<XIcon aria-hidden="true" />
									</Button>
								{/if}
							</span>
						</div>
						<ol class="m-0 flex list-none flex-col gap-2 p-0" aria-label={`Compile run ${run.runId} steps`}>
							{#each COMPILE_STEPS as step, i (step.id)}
								{@const s = run.steps[step.id]}
								{@const chips = countChips(step.id, s)}
								{@const note = stepNote(step, s)}
								<li class={rowCls(s.status)} aria-current={s.status === 'running' ? 'step' : undefined}>
									{#if i < COMPILE_STEPS.length - 1 && s.status === 'done'}
										<span class="absolute top-7 -bottom-2 left-4 w-px bg-verified md:left-5" aria-hidden="true"></span>
									{:else if i < COMPILE_STEPS.length - 1}
										<span class="absolute top-7 -bottom-2 left-4 w-px bg-line md:left-5" aria-hidden="true"></span>
									{/if}
									<StepIcon state={s.status} />
									<div class="flex min-w-0 flex-1 flex-col gap-1">
										<div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
											<span class={`text-sm ${s.status === 'pending' ? 'text-ink-muted' : 'font-medium text-ink'}`}>
												{step.label}
											</span>
											<span class="font-mono text-xs text-ink-muted tabular-nums">
												{STATUS_TEXT[s.status]}{s.latency_ms !== null && s.status !== 'running' ? ` · ${fmtMs(s.latency_ms)}` : ''}
											</span>
										</div>
										{#if chips.length > 0}
											<ul class="m-0 flex list-none flex-wrap gap-1 p-0" aria-label={`${step.label} counts`}>
												{#each chips as chip (chip.key)}
													<li class={chipCls(chip.warn)} in:fade={{ duration: motion }}>{chip.text}</li>
												{/each}
											</ul>
										{/if}
										{#if note}
											<p class="m-0 text-xs break-words text-ink-muted">{note}</p>
										{/if}
									</div>
								</li>
							{/each}
						</ol>
					</div>
				{/if}

				{#if run.phase === 'running' && run.stalled}
					<div class="flex flex-wrap items-center gap-2 rounded-md border border-warning bg-warning-bg px-3 py-2 text-sm" role="status">
						<span class="min-w-0 flex-1">No word from run {run.runId} in five minutes.</span>
						<Button variant="outline" size="sm" disabled={run.checking} onclick={() => run.check()}>
							{run.checking ? 'Checking…' : 'Check the run'}
						</Button>
					</div>
				{/if}

				{#if run.phase === 'done'}
					<div
						class="flex flex-col gap-3 rounded-md border border-verified bg-verified-bg px-4 py-3 md:flex-row md:items-center"
						in:fly={{ y: 4, duration: motion }}
					>
						<p class="m-0 min-w-0 flex-1 text-sm text-ink">
							<span class="font-semibold">Run {run.runId} done.</span>
							{summaryLine(run.summary) || 'Nothing new to propose.'}
						</p>
						<Button href={reviewHref}>
							Open Review
							<ArrowRightIcon aria-hidden="true" />
						</Button>
					</div>
				{:else if run.phase === 'failed' && run.failKind === 'run'}
					<Notice variant="error" heading={failHeading}>
						<p class="m-0 break-words">{run.error}</p>
						<p class="mt-3 mb-0 flex flex-wrap gap-2">
							<Button variant="outline" onclick={go} disabled={!canStart}>Compile again</Button>
							<Button variant="ghost" href={reviewHref}>Open Review</Button>
						</p>
					</Notice>
				{/if}
			</div>

			<section aria-labelledby="compile-recent" class="flex min-w-0 flex-col gap-3 lg:border-l lg:border-line lg:pl-6">
				<h3 id="compile-recent" class="m-0 text-sm font-semibold">Recent runs</h3>
				{#if runsState === 'loading'}
					<div class="flex flex-col gap-2" role="status" aria-label="Loading recent runs">
						{#each [0, 1, 2] as i (i)}
							<Skeleton class="h-12 rounded-md" />
						{/each}
					</div>
				{:else if runsState === 'failed'}
					<div class="flex flex-col items-start gap-2">
						<p class="m-0 text-sm text-ink-muted" role="status">{runsError}</p>
						<Button variant="outline" size="sm" onclick={() => loadRuns()}>Try again</Button>
					</div>
				{:else if runs && runs.runs.length === 0}
					<div class="flex flex-col items-start gap-2">
						<p class="m-0 text-sm text-ink-muted">No Compile runs yet.</p>
						<Button variant="outline" size="sm" onclick={() => picker?.focus()}>Choose an authority</Button>
					</div>
				{:else if runs}
					<ul class="m-0 flex list-none flex-col gap-1 p-0">
						{#each runs.runs.slice(0, 5) as r (r.run_id)}
							<li animate:flip={{ duration: motion }} in:fly={{ y: -4, duration: motion }}>
								<button
									type="button"
									class={`flex w-full min-w-0 items-start gap-3 rounded-md border px-3 py-2 text-left transition-colors duration-150 hover:bg-surface focus-visible:outline-2 focus-visible:outline-focus disabled:cursor-not-allowed ${
										run.runId === r.run_id ? 'border-focus bg-surface' : 'border-transparent bg-card'
									}`}
									aria-current={run.runId === r.run_id ? 'true' : undefined}
									disabled={run.phase === 'starting'}
									onclick={() => openRow(r)}
								>
									<StepIcon state={rowIcon(r.status)} />
									<span class="flex min-w-0 flex-1 flex-col gap-1">
										<span class="flex min-w-0 items-center justify-between gap-2">
											<span class="truncate text-sm font-medium text-ink">{nameOf(r.slug)}</span>
											<Badge variant="outline" class={badgeCls(r.status)}>{badgeText(r.status)}</Badge>
										</span>
										<span class="truncate font-mono text-xs text-ink-muted">
											run {r.run_id} · <time datetime={new Date(r.started_at * 1000).toISOString()}>{ago(r.started_at, now)}</time>
										</span>
										{#if r.status !== 'running'}
											{@const line = summaryLine(r.summary)}
											{#if line}
												<span class="truncate text-xs text-ink-muted">{line}</span>
											{/if}
										{/if}
									</span>
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</section>
		</Card.Content>
	</Card.Root>
</section>
