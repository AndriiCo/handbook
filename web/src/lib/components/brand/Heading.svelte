<script lang="ts">
	import type { Snippet } from 'svelte';

	// Levelled heading on the brand type scale. Exactly one h1 per
	// route; every instance carries data-heading for the shots audit.
	let {
		level = 2,
		eyebrow = null,
		children
	}: {
		level?: 1 | 2 | 3;
		eyebrow?: string | null;
		children: Snippet;
	} = $props();

	const tag = $derived(level === 1 ? 'h1' : level === 2 ? 'h2' : 'h3');
	const size = $derived(
		level === 1
			? 'font-brand text-h1 font-medium'
			: level === 2
				? 'font-brand text-h2 font-medium'
				: 'text-h3 font-semibold'
	);
</script>

{#if eyebrow}
	<p class="text-eyebrow font-semibold tracking-[0.06em] text-ink-muted uppercase">{eyebrow}</p>
{/if}
<svelte:element this={tag} data-heading={tag} class={`m-0 text-balance ${size}`}>
	{@render children()}
</svelte:element>
