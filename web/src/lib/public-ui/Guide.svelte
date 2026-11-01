<script lang="ts">
	import VerifiedChip from './VerifiedChip.svelte';

	type GuideStep = {
		title: string;
		statement: string;
		verified: string;
		source: string | null;
		href: string;
	};

	let { steps = [], authorityName = '' }: { steps: GuideStep[]; authorityName: string } =
		$props();
</script>

<section aria-labelledby="howto" class="border-y border-line bg-surface py-10 xl:py-16">
	<div class="mx-auto w-full max-w-5xl px-4 sm:px-6">
		<p class="text-eyebrow mt-0 font-semibold tracking-[0.06em] text-ink-muted uppercase">
			Step by step
		</p>
		<h2 id="howto" class="text-h2 mt-2 mb-0 font-semibold">
			How to get your apostille{#if authorityName}<span class="sr-only"> for {authorityName}</span>{/if}
		</h2>
		{#if steps.length === 0}
			<p class="text-body-lg mt-4 mb-0 max-w-prose text-ink-muted">
				We are still verifying each step for this authority against official
				sources. Every step appears here once staff approve its evidence.
				General information, not legal advice.
			</p>
		{:else}
			<ol class="m-0 mt-8 list-none p-0">
				{#each steps as step, i (step.title)}
					<li class="m-0 flex gap-4 p-0 py-5 first:pt-0 last:pb-0 sm:gap-5">
						<span
							aria-hidden="true"
							class="flex size-9 shrink-0 items-center justify-center rounded-full border border-line bg-bg font-mono text-sm font-semibold text-ink"
						>
							{i + 1}
						</span>
						<div class="min-w-0">
							<h3 class="text-h3 m-0 font-semibold">{step.title}</h3>
							<p class="text-body-lg mt-1 mb-0 text-ink">{step.statement}</p>
							<div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-2 text-sm">
								{#if step.verified}
									<VerifiedChip tier="Staff approved" verified={step.verified} />
								{/if}
								{#if step.source}
									<span class="font-mono text-xs text-ink-muted">{step.source}</span>
								{/if}
								<a
									href={step.href}
									class="inline-flex min-h-11 items-center font-medium text-accent-strong underline decoration-dotted underline-offset-4"
								>
									Evidence
								</a>
							</div>
						</div>
					</li>
				{/each}
			</ol>
		{/if}
	</div>
</section>
