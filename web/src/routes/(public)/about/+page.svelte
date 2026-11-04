<script lang="ts">
	import { resolve } from '$app/paths';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import ApostilleCertificate from '../../../lib/components/brand/ApostilleCertificate.svelte';
	import { ABOUT_COPY } from '../../../lib/public-ui/about-copy.js';
	import { shortDate } from '../../../lib/public-ui/home-copy.js';
	import { site } from '../../../lib/locale.svelte.js';

	// "How we verify": the method behind the rules, for readers who ask.
	// The eval line reads the live `/api/v1/quality` run (scoreboard A5
	// checks its percentage here); the three checks name the models.
	const c = $derived(ABOUT_COPY[site.locale]);
	// Certificate field each check highlights (copy per language).
	const STEP_FIELDS = [1, 9, 10];
	const steps = $derived(c.steps.map((step, i) => ({ ...step, field: STEP_FIELDS[i] })));

	let qualityPct = $state<number | null>(null);
	let qualityModel = $state<string | null>(null);
	let qualityRunAt = $state<string | null>(null);

	const apiRoot = resolve('/(public)').replace(/\/$/, '');

	$effect(() => {
		fetch(`${apiRoot}/api/v1/quality`, { credentials: 'same-origin' })
			.then(async (res) => {
				const body = await res.json().catch(() => ({}));
				const rate = body?.quote_found_rate;
				if (res.ok && typeof rate === 'number' && rate >= 0 && rate <= 1) {
					qualityPct = Math.round(rate * 100);
					if (typeof body?.model === 'string') qualityModel = body.model;
					if (typeof body?.run_at === 'string') qualityRunAt = body.run_at;
				}
			})
			.catch(() => {});
	});
</script>

<svelte:head>
	<title>{c.headTitle}</title>
	<meta name="description" content={c.metaDescription} />
</svelte:head>

<section class="py-10 xl:py-16" aria-labelledby="about-title">
	<div class="max-w-[68ch]">
		<Heading level={1} eyebrow={c.eyebrow}><span id="about-title">{c.title}</span></Heading>
		<p class="text-body-lg mt-4 text-ink-muted">{c.intro}</p>
		{#if qualityPct !== null}
			<p
				class="mt-6 inline-flex min-h-11 items-center gap-2 rounded-full border border-line bg-surface px-4 text-sm text-ink"
			>
				<span class="relative inline-flex size-2 flex-none rounded-full bg-verified" aria-hidden="true"
				></span>
				{c.evalLine(qualityPct)}{#if qualityRunAt}
					· {shortDate(qualityRunAt, site.locale)}{/if}{#if qualityModel}
					<span class="font-mono text-xs text-ink-muted">{qualityModel}</span>
				{/if}
			</p>
		{/if}
	</div>
</section>

<section class="pb-10 xl:pb-16" aria-labelledby="how">
	<Heading level={2}><span id="how">{c.howTitle}</span></Heading>
	<ol class="mt-8 grid list-none gap-4 p-0 md:grid-cols-3">
		{#each steps as step (step.field)}
			<li class="m-0 flex flex-col gap-3 rounded-xl border border-line bg-bg p-4">
				<ApostilleCertificate highlight={step.field} />
				<p class="m-0 font-mono text-xs text-ink-muted">{step.model}</p>
				<h3 class="text-h3 m-0 font-semibold">{step.title}</h3>
				<p class="m-0 text-sm text-ink-muted">{step.text}</p>
			</li>
		{/each}
	</ol>
</section>

<section class="pb-10 xl:pb-16" aria-labelledby="sources">
	<Heading level={2}><span id="sources">{c.citationsTitle}</span></Heading>
	<p class="mt-4 max-w-[68ch] text-ink-muted">{c.citations}</p>
	<p class="mt-6">
		<a class="inline-flex min-h-11 items-center font-medium" href={resolve('/(public)')}>{c.backHome}</a>
	</p>
</section>
