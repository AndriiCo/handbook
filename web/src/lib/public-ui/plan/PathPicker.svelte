<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import type { PlanPath } from '../plan.js';

	// Document-type picker: plain links to `?path=<id>` (shareable,
	// back-button friendly); the chosen chip links back to the general
	// plan. Paths come from the accepted plan only.
	let {
		paths = [],
		selected = null,
		hrefFor
	}: {
		paths?: PlanPath[];
		selected?: string | null;
		hrefFor: (id: string | null) => string;
	} = $props();
</script>

{#if paths.length > 0}
	<nav aria-label="Your document" class="mt-6">
		<p class="m-0 text-sm font-medium text-ink">What are you getting apostilled?</p>
		<ul class="m-0 mt-3 flex list-none flex-wrap gap-2 p-0">
			{#each paths as p (p.id)}
				{@const on = selected === p.id}
				<li class="m-0 p-0">
					<a
						href={hrefFor(on ? null : p.id)}
						aria-current={on ? 'true' : undefined}
						data-sveltekit-noscroll
						data-sveltekit-replacestate
						class={`inline-flex min-h-11 items-center gap-2 rounded-full border px-4 text-sm font-medium no-underline transition-colors duration-200 ${
							on
								? 'border-accent-fill bg-accent-fill text-accent-ink'
								: 'border-line bg-bg text-ink hover:border-focus'
						}`}
					>
						{#if on}<CheckIcon class="size-4" aria-hidden="true" />{/if}
						{p.label}
					</a>
				</li>
			{/each}
		</ul>
		{#if !selected}
			<p class="mt-2 mb-0 text-sm text-ink-muted">Pick one to add the steps specific to it.</p>
		{/if}
	</nav>
{/if}
