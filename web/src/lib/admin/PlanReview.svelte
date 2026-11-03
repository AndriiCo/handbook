<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { fly } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import { toast } from 'svelte-sonner';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import Notice from '#lib/public-ui/Notice.svelte';
	import GuardList from './GuardList.svelte';
	import DecisionBar from './DecisionBar.svelte';
	import ComposePanel from './ComposePanel.svelte';
	import type { ComposeSummary, ComposeRun } from './compose.svelte.js';
	import { decide, fetchPlans, type PlanBlock, type PlanProposal } from './proposals.js';
	import { ApiError, errorText } from './admin-api.js';
	import { ago, fmtCents, fmtCompact, fmtMs } from './format.js';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import InfoIcon from '@lucide/svelte/icons/info';

	// Page plans tab: staff start a compose run (ComposePanel) and
	// watch Muse Spark → validator → Muse Wonder live; the finished
	// proposal lands at the top, selected. Accept → `plan_accepted` on
	// `authority:{slug}`; every decision is optimistic with rollback.
	let {
		onCount,
		initialSlug = '',
		initialRun = null
	}: { onCount?: (n: number) => void; initialSlug?: string; initialRun?: number | null } = $props();

	let plans = $state<PlanProposal[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let selectedId = $state<number | null>(null);
	let acting = $state(false);
	let path = $state<string | null>(null);
	let fresh = $state<number | null>(null);
	let composer = $state<ComposePanel | null>(null);
	let reduceMotion = $state(false);

	/** Re-read the queue; keep (or move to) `selectId` when it is listed. */
	async function load(selectId?: number | null, quiet = false) {
		if (!quiet) loadState = 'loading';
		try {
			const rows = await fetchPlans();
			const want = selectId ?? selectedId;
			plans = rows;
			selectedId = rows.some((p) => p.id === want) ? (want as number) : (rows[0]?.id ?? null);
			loadState = 'ready';
			onCount?.(rows.filter((p) => p.status === 'proposed').length);
		} catch (err) {
			loadError = errorText(err, 'Page plans failed to load.');
			if (!quiet || plans.length === 0) loadState = 'failed';
		}
	}

	// A finished run: re-read the list, select the new plan, show it.
	async function onComposed(s: ComposeSummary, run: ComposeRun) {
		const id = s.plan_id ?? null;
		await load(id, true);
		const found = plans.find((p) => (id !== null && p.id === id) || (run.runId !== null && p.run_id === run.runId));
		if (!found) return;
		fresh = found.id;
		selectedId = found.id;
		path = null;
		if (run.reopened) return;
		await tick();
		document.getElementById(`plan-${found.id}`)?.scrollIntoView({ block: 'nearest', behavior: reduceMotion ? 'auto' : 'smooth' });
	}

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		void load();
	});

	const selected = $derived(plans.find((p) => p.id === selectedId) ?? null);
	const visible = $derived(
		selected
			? selected.plan.blocks.filter((b) => path === null || b.show_for.length === 0 || b.show_for.includes(path))
			: []
	);

	function mark(plan: PlanProposal, id: string): 'added' | 'changed' | null {
		if (plan.diff.added.includes(id)) return 'added';
		if (plan.diff.changed.includes(id)) return 'changed';
		return null;
	}

	const KIND: Record<string, string> = {
		step: 'Step',
		callout: 'Callout',
		office: 'Office',
		fees: 'Fees',
		mail: 'Mail',
		forms: 'Forms',
		timing: 'Timing'
	};

	function summary(b: PlanBlock): string {
		if (b.title) return b.title;
		if (b.kind === 'office') return `Office ${(b.office_ids ?? []).map((i) => `#${i}`).join(', ')}`;
		if (b.kind === 'forms') return `Form ${(b.form_ids ?? []).map((i) => `#${i}`).join(', ')}`;
		if (b.kind === 'fees') return (b.lines ?? []).map((l) => l.label).join(', ') || 'Fees';
		return KIND[b.kind] ?? b.kind;
	}

	async function act(plan: PlanProposal, action: 'accept' | 'reject', reason?: string) {
		if (acting) return;
		acting = true;
		const prior = plan.status;
		plan.status = action === 'accept' ? 'accepted' : 'rejected';
		try {
			plan.status = await decide('plans', plan.id, action, reason);
			onCount?.(plans.filter((p) => p.status === 'proposed').length);
			toast.success(
				action === 'accept'
					? `Plan v${plan.version} is live — the ${plan.authority_slug} dossier updates now.`
					: `Plan v${plan.version} rejected.`
			);
		} catch (err) {
			plan.status = prior;
			if (err instanceof ApiError && err.status === 409) {
				toast.info(`Plan v${plan.version} was already decided — refreshing.`);
				void load(plan.id, true);
			} else {
				toast.error(errorText(err, 'Decision failed — nothing changed.'));
			}
		} finally {
			acting = false;
		}
	}

	const motion = $derived(reduceMotion ? 0 : 200);
</script>

<div class="flex flex-col gap-4">
<ComposePanel bind:this={composer} kind="plans" {initialSlug} {initialRun} onDone={onComposed} />


{#if loadState === 'loading'}
	<div class="grid gap-4 md:grid-cols-3" role="status" aria-label="Loading page plans">
		<Skeleton class="h-32 rounded-md" />
		<Skeleton class="h-72 rounded-md md:col-span-2" />
	</div>
{:else if loadState === 'failed'}
	<Notice variant="warning" heading="Page plans are unavailable.">
		<p class="m-0">{loadError}</p>
		<p class="mt-3 mb-0"><Button variant="outline" onclick={() => load()}>Try again</Button></p>
	</Notice>
{:else if plans.length === 0}
	<Card.Root class="items-start px-6 py-10">
		<p class="m-0 text-h3 font-semibold">No page plans waiting</p>
		<p class="m-0 text-sm text-ink-muted">
			Muse Spark composes a plan once an authority has three accepted facts.
		</p>
		<Button variant="outline" onclick={() => composer?.focusPicker()}>Choose an authority to compose</Button>
	</Card.Root>
{:else}
	<div class="grid gap-4 lg:grid-cols-4">
		<ul class="m-0 flex list-none flex-col gap-2 p-0" aria-label="Plan proposals">
			{#each plans as p (p.id)}
				<li animate:flip={{ duration: motion }} in:fly={{ y: -8, duration: motion }}>
					<button
						type="button"
						id={`plan-${p.id}`}
						class={`flex w-full cursor-pointer flex-col gap-1 rounded-md border px-3 py-2 text-left transition-colors duration-150 ${p.id === selectedId ? 'border-focus bg-surface' : 'border-line bg-card hover:bg-surface'}`}
						aria-current={p.id === selectedId ? 'true' : undefined}
						onclick={() => {
							selectedId = p.id;
							path = null;
						}}
					>
						<span class="font-mono text-sm">{p.authority_slug}</span>
						<span class="text-sm font-semibold">
							v{p.version}{p.current_version ? ` replaces v${p.current_version}` : ' (first plan)'}
						</span>
						<span class="flex flex-wrap gap-1">
							{#if p.id === fresh}
								<Badge variant="outline" class="border-focus text-ink">New{p.run_id ? ` · run ${p.run_id}` : ''}</Badge>
							{/if}
							{#if p.status !== 'proposed'}
								<Badge variant="secondary">{p.status}</Badge>
							{:else if p.validator.passed}
								<Badge variant="outline" class="border-verified text-verified">Validator passed</Badge>
							{:else}
								<Badge variant="outline" class="border-changed text-changed">Validator flagged</Badge>
							{/if}
						</span>
					</button>
				</li>
			{/each}
		</ul>

		{#if selected}
			{@const plan = selected}
			<div class="flex min-w-0 flex-col gap-4 lg:col-span-3">
				<div class="flex flex-wrap items-start justify-between gap-3">
					<div class="flex min-w-0 flex-col gap-1">
						<h2 class="m-0 text-h3 font-semibold">
							{plan.authority_slug} · page plan v{plan.version}
						</h2>
						<p class="m-0 text-sm text-ink-muted">
							Composed by Muse Spark {ago(plan.created_at, Math.max(Date.now() / 1000, plan.created_at))}{plan.run_id
								? ` in run ${plan.run_id}`
								: ''} · {plan.plan.blocks.length} blocks
						</p>
						{#if plan.spend}
							<p class="m-0 font-mono text-xs text-ink-muted tabular-nums">
								{fmtMs(plan.spend.latency_ms)} · {fmtCompact(plan.spend.input_tokens)} in · {fmtCompact(plan.spend.cached_tokens)} cached · {fmtCompact(plan.spend.output_tokens)} out · {fmtCompact(plan.spend.reasoning_tokens)} reasoning
							</p>
						{/if}
					</div>
					<DecisionBar
						noun={`plan v${plan.version}`}
						id={`plan-${plan.id}`}
						status={plan.status}
						{acting}
						onAccept={() => act(plan, 'accept')}
						onReject={(reason) => act(plan, 'reject', reason)}
					/>
				</div>

				<Card.Root class="gap-3 shadow-sm">
					<Card.Header>
						<Card.Title class="flex items-center gap-2 text-body font-semibold">
							{#if plan.validator.passed}
								<ShieldCheckIcon class="size-4 text-verified" aria-hidden="true" />
								Validator report: passed
							{:else}
								<TriangleAlertIcon class="size-4 text-changed" aria-hidden="true" />
								Validator report: needs staff
							{/if}
						</Card.Title>
						<Card.Description>Deterministic checks before any block can reach the dossier</Card.Description>
					</Card.Header>
					<Card.Content class="flex flex-col gap-3">
						<GuardList guards={plan.validator.checks} label="Validator checks" />
						{#if plan.validator.dropped_blocks.length > 0}
							<div class="flex flex-col gap-1 rounded-sm bg-changed-bg px-3 py-2 text-sm">
								<p class="m-0 font-semibold">Dropped before review ({plan.validator.dropped_blocks.length})</p>
								<ul class="m-0 flex list-none flex-col gap-1 p-0">
									{#each plan.validator.dropped_blocks as d (d.id)}
										<li class="break-words"><span class="font-mono">{d.id}</span>{d.reason ? ` — ${d.reason}` : ''}</li>
									{/each}
								</ul>
							</div>
						{/if}
					</Card.Content>
				</Card.Root>

				<div class="grid gap-4 md:grid-cols-2">
					<Card.Root class="gap-3 shadow-sm">
						<Card.Header>
							<Card.Title class="text-body font-semibold">
								Changes{plan.current_version ? ` vs live v${plan.current_version}` : ''}
							</Card.Title>
							<Card.Description>
								{plan.diff.added.length} added · {plan.diff.changed.length} changed · {plan.diff.removed.length} removed
							</Card.Description>
						</Card.Header>
						<Card.Content>
							<ol class="m-0 flex list-none flex-col gap-1 p-0" aria-label="Plan blocks">
								{#each plan.plan.blocks as b (b.id)}
									{@const m = mark(plan, b.id)}
									<li
										class={`flex items-start gap-2 rounded-sm px-2 py-1 text-sm ${m === 'added' ? 'bg-verified-bg' : m === 'changed' ? 'bg-changed-bg' : ''}`}
									>
										<span class="w-16 flex-none text-xs font-semibold text-ink-muted uppercase">{KIND[b.kind] ?? b.kind}</span>
										<span class="min-w-0 flex-1">
											{summary(b)}
											<span class="block font-mono text-xs text-ink-muted">
												{b.fact_ids.map((f) => `#${f}`).join(' ')}{b.show_for.length ? ` · only ${b.show_for.join(', ')}` : ''}
											</span>
										</span>
										{#if m === 'added'}
											<Badge variant="outline" class="border-verified text-verified">New</Badge>
										{:else if m === 'changed'}
											<Badge variant="outline" class="border-changed text-changed">Changed</Badge>
										{/if}
									</li>
								{/each}
								{#each plan.diff.removed as id (id)}
									<li class="flex items-center gap-2 rounded-sm bg-error-bg px-2 py-1 text-sm">
										<span class="w-16 flex-none text-xs font-semibold text-ink-muted uppercase">Removed</span>
										<span class="font-mono line-through">{id}</span>
									</li>
								{/each}
							</ol>
						</Card.Content>
					</Card.Root>

					<Card.Root class="gap-3 bg-surface shadow-sm">
						<Card.Header>
							<Card.Title class="text-body font-semibold">Visitor preview</Card.Title>
							<Card.Description>What the dossier shows once accepted</Card.Description>
						</Card.Header>
						<Card.Content class="flex flex-col gap-3">
							<p class="m-0 text-body font-semibold text-balance">{plan.plan.headline}</p>
							{#if plan.plan.paths.length > 0}
								<div class="flex flex-wrap gap-1" role="group" aria-label="Document path">
									<Button size="sm" variant={path === null ? 'default' : 'outline'} aria-pressed={path === null} onclick={() => (path = null)}>
										All paths
									</Button>
									{#each plan.plan.paths as p (p.id)}
										<Button size="sm" variant={path === p.id ? 'default' : 'outline'} aria-pressed={path === p.id} onclick={() => (path = p.id)}>
											{p.label}
										</Button>
									{/each}
								</div>
							{/if}
							<ol class="m-0 flex list-none flex-col gap-2 p-0" aria-label="Preview blocks">
								{#each visible as b (b.id)}
									<li class="rounded-sm border border-line bg-card px-3 py-2 text-sm">
										{#if b.kind === 'callout'}
											<p class="m-0 flex items-center gap-2 font-semibold">
												{#if b.tone === 'warning'}
													<TriangleAlertIcon class="size-4 text-changed" aria-hidden="true" />
												{:else}
													<InfoIcon class="size-4 text-accent-strong" aria-hidden="true" />
												{/if}
												{b.title}
											</p>
											{#if b.body}<p class="m-0 text-ink-muted">{b.body}</p>{/if}
										{:else if b.kind === 'fees'}
											<p class="m-0 font-semibold">Fees</p>
											<ul class="m-0 list-none p-0">
												{#each b.lines ?? [] as l, li (li)}
													<li class="flex justify-between gap-2">
														<span>{l.label}</span>
														<span class="font-mono">{fmtCents(l.amount_cents)}{l.per ? ` per ${l.per}` : ''}</span>
													</li>
												{/each}
											</ul>
										{:else if b.kind === 'office'}
											<p class="m-0 font-semibold">Where to go</p>
											<p class="m-0 text-ink-muted">Accepted office {(b.office_ids ?? []).map((i) => `#${i}`).join(', ')}: address, hours and directions</p>
										{:else if b.kind === 'forms'}
											<p class="m-0 font-semibold">Fill the form online</p>
											<p class="m-0 text-ink-muted">Mapped form {(b.form_ids ?? []).map((i) => `#${i}`).join(', ')}</p>
										{:else}
											<p class="m-0 font-semibold">{b.title ?? KIND[b.kind]}</p>
											{#if b.body}<p class="m-0 text-ink-muted">{b.body}</p>{/if}
										{/if}
									</li>
								{/each}
							</ol>
							{#if plan.plan.hide_topics.length > 0}
								<p class="m-0 text-xs text-ink-muted">Hidden topics: {plan.plan.hide_topics.join(', ')}</p>
							{/if}
						</Card.Content>
					</Card.Root>
				</div>
			</div>
		{/if}
	</div>
{/if}
</div>
