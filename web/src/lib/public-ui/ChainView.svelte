<script lang="ts">
	import packData from '../../../../packs/egypt-consular.json';

	// Read-only consular chain view (SA-18). Every displayed value
	// comes from packs/egypt-consular.json — this component renders
	// the pack and asserts nothing of its own. Synthetic data only:
	// the pack holds staff observations awaiting review.

	interface ChainStep {
		id: string;
		order: number;
		title: string;
		detail: string;
		authority: string;
		status: string;
		validation?: string;
	}

	interface Authority {
		slug: string;
		name: string;
		region?: string;
	}

	interface Pack {
		title: string;
		chain_steps: ChainStep[];
		authorities: Authority[];
	}

	const pack = packData as unknown as Pack;
	const steps = [...(pack.chain_steps ?? [])].sort((a, b) => a.order - b.order);
	const authorityName = (slug: string) =>
		pack.authorities.find((a) => a.slug === slug)?.name ?? slug;

	function chipLabel(status: string): string {
		if (status === 'needs_review') return 'Needs review';
		if (status === 'verified') return 'Verified';
		return status.replaceAll('_', ' ');
	}

	function chipCls(status: string): string {
		if (status === 'verified')
			return 'border-success bg-success-bg text-success';
		if (status === 'needs_review') return 'border-changed bg-changed-bg text-changed';
		return 'border-line bg-surface-2 text-ink-muted';
	}
</script>

<section aria-label={pack.title} class="mt-6 border-t border-line pt-4">
	<h2 class="m-0 mb-1 text-xl">{pack.title}</h2>
	<p class="m-0 mb-4 text-sm text-ink-muted">
		Read-only chain — {steps.length} steps in pack order. This form covers the
		authentication hop; the other hops happen before and after it.
	</p>
	{#if steps.length === 0}
		<p class="m-0 text-sm text-ink-muted">This pack lists no chain steps.</p>
	{:else}
		<ol class="m-0 list-none p-0">
			{#each steps as step, i (step.id)}
				<li
					class="mt-3 flex gap-4 rounded-md border border-line bg-surface p-4 first:mt-0"
				>
					<span
						aria-hidden="true"
						class="flex size-9 shrink-0 items-center justify-center rounded-full border border-line bg-bg font-mono text-sm font-semibold text-ink"
					>
						{i + 1}
					</span>
					<div class="min-w-0 flex-1">
						<h3 class="m-0 text-base font-semibold">{step.title}</h3>
						<p class="mt-1 mb-0 text-sm break-words text-ink">{step.detail}</p>
						<div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-2 text-sm">
							<span
								class={`inline-block rounded-full border px-2 py-0.5 text-xs font-semibold ${chipCls(step.status)}`}
							>
								{chipLabel(step.status)}
							</span>
							<span class="text-xs text-ink-muted">{authorityName(step.authority)}</span>
							{#if step.validation}
								<span class="font-mono text-xs text-ink-muted">{step.validation}</span>
							{/if}
						</div>
					</div>
				</li>
			{/each}
		</ol>
	{/if}
	<p class="mt-4 mb-0 max-w-prose text-sm text-ink-muted">
		Every step above still needs staff review — confirm each hop against the
		official source before you rely on it. General information, not legal advice.
	</p>
</section>
