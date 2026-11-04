<script lang="ts">
	import { keepTogether } from './items.js';

	// A value's text, verbatim, with each ZIP+4 code kept on one line
	// ("98504-0234" never breaks at its hyphen on a phone). Display only:
	// the runs join back to the same text, so copying, tracing and
	// screen readers see the value unchanged.
	//
	// Props:
	// - text: the value (an Item's display).
	let { text }: { text: string } = $props();

	const parts = $derived(keepTogether(text));
</script>

{#each parts as part, i (i)}{#if part.keep}<span class="whitespace-nowrap">{part.text}</span>{:else}{part.text}{/if}{/each}
