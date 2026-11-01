<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import CheckCircleIcon from '@lucide/svelte/icons/circle-check';
	import XCircleIcon from '@lucide/svelte/icons/circle-x';

	// Demo photo precheck (A2): synthetic samples, recorded live-model
	// verdicts. SAM boxes animate over the sample; the verdict panel
	// names the models and the recording date. Zero console noise:
	// verdicts run only on button press, never on load.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	type Sample = { id: string; label: string; thumb_url: string };
	type Box = { label: string; x: number; y: number; w: number; h: number };
	type Verdict = {
		sample: string;
		verdict: 'pass' | 'retake';
		issues: string[];
		boxes: Box[];
		models: string[];
		recorded_at: string;
	};

	let samples = $state<Sample[] | null>(null);
	let loadError = $state<string | null>(null);
	let chosen = $state<string | null>(null);
	let running = $state(false);
	let result = $state<Verdict | null>(null);
	let runError = $state<string | null>(null);

	const chosenSample = $derived(samples?.find((s) => s.id === chosen) ?? null);

	const sampleBtnCls = (active: boolean) =>
		`flex min-h-11 w-full max-w-[36rem] cursor-pointer items-center gap-3 rounded-sm border bg-bg px-3 py-2 text-left text-ink transition-colors ${active ? `border-accent-fill` : `border-line hover:border-focus`}`;

	const verdictPillCls = (pass: boolean) =>
		`inline-block rounded-xs border px-3 py-1 text-sm font-bold ${pass ? `border-verified bg-verified-bg text-verified` : `border-changed bg-changed-bg text-changed`}`;

	$effect(() => {
		fetch(api('/demo/precheck/samples'), { credentials: 'same-origin' })
			.then((res) => (res.ok ? res.json() : Promise.reject(new Error(String(res.status)))))
			.then((body) => {
				const list = body?.samples;
				samples = Array.isArray(list) ? list : [];
				chosen ??= samples[0]?.id ?? null;
			})
			.catch(() => {
				loadError = 'Could not load the demo samples — try again.';
			});
	});

	async function run() {
		if (!chosen || running) return;
		running = true;
		result = null;
		runError = null;
		try {
			const res = await fetch(api('/demo/precheck'), {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				credentials: 'same-origin',
				body: JSON.stringify({ sample: chosen })
			});
			if (res.status === 429) {
				runError = 'Too many runs — wait a minute and try again.';
				return;
			}
			if (res.status === 503) {
				runError = 'Demo verdicts are being recorded — check back after the next deploy.';
				return;
			}
			if (!res.ok) throw new Error(String(res.status));
			const body = await res.json();
			if (body?.verdict !== 'pass' && body?.verdict !== 'retake') throw new Error('verdict');
			result = {
				sample: chosen,
				verdict: body.verdict,
				issues: Array.isArray(body.issues) ? body.issues.map(String) : [],
				boxes: Array.isArray(body.boxes) ? body.boxes : [],
				models: Array.isArray(body.models) ? body.models.map(String) : [],
				recorded_at: typeof body.recorded_at === 'string' ? body.recorded_at : ''
			};
		} catch {
			runError = 'The precheck run failed — try again.';
		} finally {
			running = false;
		}
	}
</script>

<svelte:head>
	<title>Precheck — Apostille Handbook</title>
</svelte:head>

<Heading level={1}>Document precheck</Heading>
<p class="max-w-[36rem] text-ink-muted">
	Try the photo check on a synthetic sample. SAM boxes draw over the page;
	the verdict names its models and recording date.
</p>

{#if loadError}
	<Notice variant="error" heading="Samples unavailable."><p>{loadError}</p></Notice>
{:else if samples === null}
	<div role="status" aria-label="Loading samples" class="flex flex-col gap-2">
		<span class="sr-only">Loading samples</span>
		<Skeleton class="h-24 w-full max-w-[36rem]" />
	</div>
{:else}
	<ul class="m-0 list-none p-0" aria-label="Demo samples">
		{#each samples as s (s.id)}
			<li class="mb-2">
				<button
					type="button"
					onclick={() => {
						chosen = s.id;
						result = null;
						runError = null;
					}}
					aria-pressed={chosen === s.id}
					class={sampleBtnCls(chosen === s.id)}
				>
					<img
						src={s.thumb_url}
						alt=""
						width="48"
						height="64"
						class="h-16 w-12 flex-none rounded-xs border border-line object-cover"
					/>
					<span class="font-bold">{s.label}</span>
				</button>
			</li>
		{/each}
	</ul>

	<p>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
			disabled={!chosen || running}
			onclick={run}
		>
			{running ? 'Checking…' : result ? 'Run again' : 'Run precheck'}
		</button>
	</p>

	{#if runError}
		<Notice variant="warning" heading="Run unavailable."><p>{runError}</p></Notice>
	{/if}

	{#if chosenSample}
		<section aria-label="Precheck result" class="max-w-[36rem] py-10 xl:py-16">
			{#if result}
				<p class={verdictPillCls(result.verdict === 'pass')} aria-live="polite">
					{result.verdict === 'pass' ? 'Pass — good to submit' : 'Retake — fix the issues below'}
				</p>
			{/if}
			<div class="relative mt-2 overflow-hidden rounded-md border border-line">
				<img
					src={chosenSample.thumb_url}
					alt={chosenSample.label}
					width="480"
					height="640"
					class="block h-auto w-full"
				/>
				<svg
					class="pointer-events-none absolute inset-0 h-full w-full"
					viewBox="0 0 100 100"
					preserveAspectRatio="none"
					aria-hidden="true"
					data-graphic="precheck-mask"
				>
					{#if result}
						{#each result.boxes as b, i (b.label + i)}
							{@const left = Math.max(0, Math.min(100, b.x * 100))}
							{@const top = Math.max(0, Math.min(100, b.y * 100))}
							{@const width = Math.max(0, Math.min(100 - left, b.w * 100))}
							{@const height = Math.max(0, Math.min(100 - top, b.h * 100))}
							<rect
								x={left}
								y={top}
								width={Math.max(width, 0.5)}
								height={Math.max(height, 0.5)}
								fill="none"
								stroke="var(--color-brand-500)"
								stroke-width="2"
								vector-effect="non-scaling-stroke"
								pathLength="1"
								class="mask-trace"
								style={`animation-delay: ${i * 220}ms`}
							/>
						{/each}
					{/if}
				</svg>
				{#if result}
					{#each result.boxes as b, i (b.label + i)}
						{@const left = Math.max(0, Math.min(100, b.x * 100))}
						{@const top = Math.max(0, Math.min(100, b.y * 100))}
						<span
							class="box-in absolute rounded-xs bg-accent-fill px-1 font-mono text-xs whitespace-nowrap text-accent-ink"
							style="left: {left}%; top: {top}%; animation-delay: {i * 220 + 500}ms"
						>
							{b.label}
						</span>
					{/each}
				{/if}
			</div>
			{#if result}
			<ul class="mt-3 flex list-none flex-col gap-2 p-0" aria-label="Precheck checklist">
				{#if result.verdict === 'pass'}
					<li class="flex min-h-11 items-center gap-2">
						<CheckCircleIcon class="size-5 flex-none text-verified" aria-hidden="true" />
						<span>Ready to submit — no defects found</span>
					</li>
				{/if}
				{#each result.issues as issue (issue)}
					<li class="flex min-h-11 items-center gap-2">
						<XCircleIcon class="size-5 flex-none text-changed" aria-hidden="true" />
						<span>{issue}</span>
					</li>
				{/each}
			</ul>
			<p class="font-mono text-xs text-ink-muted">
				models {result.models.join(' + ') || 'unrecorded'} · recorded {result.recorded_at ||
					'—'} · replay of a live run on synthetic samples
			</p>
			{/if}
		</section>
	{/if}
{/if}

<style>
	.box-in {
		animation: box-in 260ms ease-out both;
	}
	@keyframes box-in {
		from {
			opacity: 0;
			transform: scale(0.96);
		}
		to {
			opacity: 1;
			transform: scale(1);
		}
	}
	/* SAM mask trace: the outline draws itself (dashoffset), then the
	   verdict checklist below takes over. */
	.mask-trace {
		stroke-dasharray: 1;
		stroke-dashoffset: 1;
		animation: mask-trace 600ms ease-out both;
	}
	@keyframes mask-trace {
		to {
			stroke-dashoffset: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.mask-trace {
			animation: none;
			stroke-dashoffset: 0;
		}
	}
</style>
