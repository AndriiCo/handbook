<script lang="ts">
	import CitationMark from './CitationMark.svelte';
	import VerifiedChip from './VerifiedChip.svelte';

	type Item = {
		label: string;
		value: string;
		factIds: unknown[];
		citations: any[];
		conflict: boolean;
		stale: { months: number; warning: boolean } | null;
		draftedBy: string;
	};

	let {
		step = { id: '', title: '', items: [] },
		index = 0,
		flash = false
	}: { step: { id: string; title: string; items: Item[] }; index: number; flash: boolean } =
		$props();

	function verifiedOf(item: Item): string {
		const at = item.citations?.[0]?.captured_at;
		return typeof at === 'string' ? at.split('T')[0] : '';
	}
</script>

<li
	class={`m-0 flex gap-4 rounded-md p-0 py-5 transition-colors first:pt-0 last:pb-0 sm:gap-5 ${flash ? 'guide-flash' : ''}`}
>
	<span
		aria-hidden="true"
		class="flex size-9 shrink-0 items-center justify-center rounded-full border border-line bg-bg font-mono text-sm font-semibold text-ink"
	>
		{index + 1}
	</span>
	<div class="min-w-0 flex-1">
		<h3 class="text-h3 m-0 font-semibold">{step.title}</h3>
		<ul class="m-0 mt-3 list-none p-0">
			{#each step.items as item (item.factIds.join(','))}
				<li class="m-0 mt-3 border-t border-line pt-3 p-0 first:mt-0 first:border-t-0 first:pt-0">
					<p class="text-body-lg m-0 text-ink">
						<span class="font-semibold">{item.label}: </span>{item.value}
					</p>
					<div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-2 text-sm">
						<VerifiedChip tier="Staff approved" verified={verifiedOf(item)} />
						{#if item.conflict}
							<span
								class="inline-block rounded-full border border-changed bg-changed-bg px-2 py-0.5 text-xs font-semibold text-changed"
							>
								Conflicting reports — both shown
							</span>
						{/if}
						{#if item.stale}
							<span class={`text-xs ${item.stale.warning ? 'font-semibold text-changed' : 'text-ink-muted'}`}>
								{#if item.stale.warning}
									Over a year old — confirm before you rely on it
								{:else}
									Checked {item.stale.months} months ago
								{/if}
							</span>
						{/if}
						<CitationMark
							citation={item.citations?.[0] ?? null}
							value={item.value}
							draftedBy={item.draftedBy}
						/>
					</div>
				</li>
			{/each}
		</ul>
	</div>
</li>

<style>
	.guide-flash {
		background-color: var(--color-verified-bg);
	}
	@media (prefers-reduced-motion: reduce) {
		.guide-flash {
			transition: none;
		}
	}
	@media (prefers-reduced-motion: no-preference) {
		.guide-flash {
			transition: background-color 300ms ease-out;
		}
	}
</style>
