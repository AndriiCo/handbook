<script lang="ts">
	import GlobeIcon from '@lucide/svelte/icons/globe';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import ScanSearchIcon from '@lucide/svelte/icons/scan-search';
	import ListChecksIcon from '@lucide/svelte/icons/list-checks';
	import UserCheckIcon from '@lucide/svelte/icons/user-check';
	import { domain } from '../guide.js';
	import { count, day, modelLabel, roleLabel, seconds, type Provenance } from '../plan.js';

	// "How this page was made": the Meta models and their roles, the
	// deterministic validator, the cross-check tally, the run's latency
	// and tokens, and the staff approval date — all from the plan's own
	// `provenance` record. No staff identity is shown.
	let {
		provenance,
		version = 0,
		acceptedAt = null
	}: { provenance: Provenance; version?: number; acceptedAt?: number | null } = $props();

	type Stage = { icon: typeof GlobeIcon; title: string; text: string; detail?: string; mono?: string };

	const stages = $derived.by(() => {
		const out: Stage[] = [];
		const src = provenance.sources ?? [];
		if (src.length > 0) {
			const hosts = [...new Set(src.map((s) => domain(s.url)))];
			const newest = src.map((s) => s.captured_at ?? '').sort().at(-1)?.slice(0, 10) ?? '';
			out.push({
				icon: GlobeIcon,
				title: `${src.length} official page${src.length === 1 ? '' : 's'}`,
				text: newest ? `Captured ${newest}` : 'Captured from the authority',
				mono: hosts.join(' · ')
			});
		}
		const cc = provenance.cross_check;
		for (const m of provenance.models ?? []) {
			const stage: Stage = {
				icon: m.role === 'cross_check' ? ScanSearchIcon : SparklesIcon,
				title: modelLabel(m.model),
				text: roleLabel(m.role),
				mono: m.model
			};
			if (m.role === 'cross_check' && cc) {
				stage.detail = `${count(cc.supported)} supported · ${count(cc.contradicted)} contradicted · ${count(cc.not_found)} not found`;
			}
			out.push(stage);
		}
		const v = provenance.validator;
		if (v && typeof v.checks === 'number') {
			out.push({
				icon: ListChecksIcon,
				title: v.passed ? `${count(v.checks)} checks passed` : `${count(v.checks)} checks run`,
				text: 'Deterministic validator: every block cites accepted facts of this authority',
				detail: `${count(v.dropped_blocks ?? 0)} block${v.dropped_blocks === 1 ? '' : 's'} dropped`
			});
		}
		const at = day(provenance.approved?.at ?? acceptedAt);
		out.push({
			icon: UserCheckIcon,
			title: 'Approved by staff reviewer',
			text: at ? `Accepted ${at}${version ? ` · plan v${version}` : ''}` : 'Accepted before publishing'
		});
		return out;
	});

	const run = $derived(provenance.run ?? null);
	const tokens = $derived(
		run
			? [
					{ k: 'Input', v: run.input_tokens },
					{ k: 'Output', v: run.output_tokens },
					{ k: 'Cached', v: run.cached_tokens },
					{ k: 'Reasoning', v: run.reasoning_tokens }
				].filter((t) => typeof t.v === 'number')
			: []
	);
</script>

<ol class="m-0 grid list-none gap-3 p-0 sm:grid-cols-2 lg:grid-cols-3" data-graphic="pipeline">
	{#each stages as s, i (i)}
		{@const Icon = s.icon}
		<li class="relative m-0 flex gap-3 rounded-xl border border-line bg-bg p-4">
			<span
				class="flex size-9 flex-none items-center justify-center rounded-full bg-surface text-accent-strong"
				aria-hidden="true"
			>
				<Icon class="size-4" />
			</span>
			<div class="min-w-0 flex-1">
				<p class="m-0 font-mono text-xs text-ink-muted">Step {i + 1}</p>
				<p class="m-0 font-semibold break-words text-ink">{s.title}</p>
				<p class="mt-0.5 mb-0 text-sm text-ink-muted">{s.text}</p>
				{#if s.detail}<p class="mt-1 mb-0 text-sm font-medium text-ink">{s.detail}</p>{/if}
				{#if s.mono}<p class="mt-1 mb-0 font-mono text-xs break-words text-ink-muted">{s.mono}</p>{/if}
			</div>
		</li>
	{/each}
</ol>
{#if run}
	<dl class="m-0 mt-4 grid grid-cols-2 gap-3 rounded-xl border border-line bg-bg p-4 sm:grid-cols-3 lg:grid-cols-6">
		<div class="m-0">
			<dt class="text-xs font-semibold tracking-wide text-ink-muted uppercase">Run time</dt>
			<dd class="m-0 mt-1 font-mono text-sm text-ink">{seconds(run.latency_ms)}</dd>
		</div>
		{#if typeof run.p95_ms === 'number'}
			<div class="m-0">
				<dt class="text-xs font-semibold tracking-wide text-ink-muted uppercase">p95</dt>
				<dd class="m-0 mt-1 font-mono text-sm text-ink">{seconds(run.p95_ms)}</dd>
			</div>
		{/if}
		{#each tokens as t (t.k)}
			<div class="m-0">
				<dt class="text-xs font-semibold tracking-wide text-ink-muted uppercase">{t.k} tokens</dt>
				<dd class="m-0 mt-1 font-mono text-sm text-ink">{count(t.v)}</dd>
			</div>
		{/each}
	</dl>
{/if}
