<script lang="ts">
	import { site } from '../locale.svelte.js';
	import { STATUS_BADGE, statusLabel, statusTone } from './orders';

	let { status }: { status: string } = $props();
	const cls = $derived(STATUS_BADGE[statusTone(status)]);
</script>

<span
	class={`inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-sm font-medium whitespace-nowrap ${cls}`}
	data-status={status}
>
	<span aria-hidden="true" class="size-1.5 rounded-full bg-current"></span>
	{statusLabel(status, site.locale)}
</span>
