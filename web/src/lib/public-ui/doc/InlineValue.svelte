<script lang="ts">
	import type { Snippet } from 'svelte';
	import { traceAttrs, valueLang } from './items.js';
	import type { Item } from './contract.js';

	// One value inside a sentence ("the office's published number is
	// {phone}"): the same trace attributes as a row's value node, then
	// its citation marker. Keep it on one line in callers' markup so no
	// stray space lands before punctuation.
	//
	// Props:
	// - item: the Item whose display is shown verbatim.
	// - marker: snippet `marker(item)` from the citations layer.
	// - href: makes the value a link (`tel:` for phones).
	let { item, marker, href = null }: { item: Item; marker?: Snippet<[Item]>; href?: string | null } = $props();
</script>

<span class="break-words [overflow-wrap:anywhere]" lang={valueLang(item.display) ?? undefined} {...traceAttrs(item)}
	>{#if href}<a {href} class="font-medium underline underline-offset-2">{item.display}</a>{:else}{item.display}{/if}</span
>{@render marker?.(item)}
