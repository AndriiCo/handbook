<script lang="ts">
	import { STATUS_BADGE, statusTone } from '#lib/customer/orders.js';
	import { staffStatusLabel } from './orders.js';

	// Compact status pill for the staff Orders table and sheet: the
	// customer palette (verified green, changed amber), staff wording.
	let { status, size = 'sm' }: { status: string; size?: 'sm' | 'md' } = $props();
	const tone = $derived(STATUS_BADGE[statusTone(status)]);
	const box = $derived(size === 'md' ? 'h-7 px-3 text-sm' : 'h-6 px-2 text-xs');
</script>

<span
	class={`inline-flex items-center gap-1.5 rounded-full border font-medium whitespace-nowrap transition-colors duration-200 ${box} ${tone}`}
	data-status={status}
>
	<span aria-hidden="true" class="size-1.5 rounded-full bg-current"></span>
	{staffStatusLabel(status)}
</span>
