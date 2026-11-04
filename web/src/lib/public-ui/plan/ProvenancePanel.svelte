<script lang="ts">
	import GlobeIcon from '@lucide/svelte/icons/globe';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import ScanSearchIcon from '@lucide/svelte/icons/scan-search';
	import ListChecksIcon from '@lucide/svelte/icons/list-checks';
	import UserCheckIcon from '@lucide/svelte/icons/user-check';
	import { domain } from '../guide.js';
	import { count, day, modelLabel, roleLabel, seconds, type Provenance } from '../plan.js';
	import { guideCopy } from '../doc/copy/index.js';

	// "How this page was made": the Meta models and their roles, the
	// deterministic validator, the cross-check tally, the run's latency
	// and tokens, and the staff approval date — all from the plan's own
	// `provenance` record. No staff identity is shown.
	let {
		provenance,
		version = 0,
		acceptedAt = null,
		lang = 'en'
	}: { provenance: Provenance; version?: number; acceptedAt?: number | null; lang?: string } = $props();

	const t = $derived(guideCopy(lang).provenance);
	const n = (v: number | undefined) => count(v, lang);
	const kinds: Record<string, string> = $derived(t.tokenKinds);

	type Stage = { icon: typeof GlobeIcon; title: string; text: string; detail?: string; mono?: string };

	const stages = $derived.by(() => {
		const out: Stage[] = [];
		const src = provenance.sources ?? [];
		if (src.length > 0) {
			const hosts = [...new Set(src.map((s) => domain(s.url)))];
			const newest = src.map((s) => s.captured_at ?? '').sort().at(-1)?.slice(0, 10) ?? '';
			out.push({
				icon: GlobeIcon,
				title: t.pages(src.length),
				text: newest ? t.captured(newest) : t.capturedAuthority,
				mono: hosts.join(' · ')
			});
		}
		const cc = provenance.cross_check;
		for (const m of provenance.models ?? []) {
			const stage: Stage = {
				icon: m.role === 'cross_check' ? ScanSearchIcon : SparklesIcon,
				title: modelLabel(m.model),
				text: roleLabel(m.role, lang),
				mono: m.model
			};
			if (m.role === 'cross_check' && cc) {
				stage.detail = t.crossCheck(n(cc.supported), n(cc.contradicted), n(cc.not_found));
			}
			out.push(stage);
		}
		const v = provenance.validator;
		if (v && typeof v.checks === 'number') {
			out.push({
				icon: ListChecksIcon,
				title: v.passed ? t.checksPassed(n(v.checks)) : t.checksRun(n(v.checks)),
				text: t.validator,
				detail: t.dropped(n(v.dropped_blocks ?? 0), v.dropped_blocks ?? 0)
			});
		}
		const at = day(provenance.approved?.at ?? acceptedAt);
		out.push({
			icon: UserCheckIcon,
			title: t.approved,
			text: at ? t.accepted(at, version) : t.acceptedBefore
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
				<p class="m-0 font-mono text-xs text-ink-muted">{t.step(i + 1)}</p>
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
			<dt class="text-xs font-semibold tracking-wide text-ink-muted uppercase">{t.runTime}</dt>
			<dd class="m-0 mt-1 font-mono text-sm text-ink">{seconds(run.latency_ms, lang)}</dd>
		</div>
		{#if typeof run.p95_ms === 'number'}
			<div class="m-0">
				<dt class="text-xs font-semibold tracking-wide text-ink-muted uppercase">p95</dt>
				<dd class="m-0 mt-1 font-mono text-sm text-ink">{seconds(run.p95_ms, lang)}</dd>
			</div>
		{/if}
		{#each tokens as tok (tok.k)}
			<div class="m-0">
				<dt class="text-xs font-semibold tracking-wide text-ink-muted uppercase">{t.tokens(kinds[tok.k] ?? tok.k)}</dt>
				<dd class="m-0 mt-1 font-mono text-sm text-ink">{n(tok.v)}</dd>
			</div>
		{/each}
	</dl>
{/if}
