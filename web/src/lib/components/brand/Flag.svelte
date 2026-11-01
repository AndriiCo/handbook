<script lang="ts">
	import { resolve } from '$app/paths';

	// Self-hosted 1x1 flag (flag-icons, MIT) in static/flags/.
	let {
		code,
		label,
		class: klass = 'size-5'
	}: {
		code: string;
		label: string;
		class?: string;
	} = $props();

	// Static assets live under the public base (/apostille), which the
	// route id resolves to; plain string concat, not a route resolve.
	const base = resolve('/(public)').replace(/\/$/, '');
	const src = $derived(`${base}/flags/${code}.svg`);
</script>

<img
	{src}
	alt={label}
	width="20"
	height="20"
	loading="lazy"
	class={`inline-block flex-none rounded-full object-cover ${klass}`}
	data-graphic="flag"
/>
