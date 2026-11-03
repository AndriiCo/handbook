<script lang="ts">
	import { fmtTime } from './orders';

	// Shown once the bus has answered on this screen's topics; the
	// time is the last ids-only event that triggered a re-read.
	let { live, lastAt = null }: { live: boolean; lastAt?: number | null } = $props();
</script>

{#if live}
	<span
		class="inline-flex h-7 items-center gap-2 rounded-full border border-verified bg-verified-bg px-3 text-sm font-medium text-verified"
		role="status"
	>
		<span class="relative flex size-2" aria-hidden="true">
			<span class="absolute inline-flex size-full animate-ping rounded-full bg-verified opacity-60 motion-reduce:animate-none"></span>
			<span class="relative inline-flex size-2 rounded-full bg-verified"></span>
		</span>
		{lastAt ? `Live · updated ${fmtTime(Math.floor(lastAt / 1000))}` : 'Live'}
	</span>
{/if}
