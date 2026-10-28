<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { TRIAGE } from '../../../../lib/mock/fixtures';

	const awaiting = $derived(TRIAGE.length);
	const conflicts = $derived(TRIAGE.filter((t) => t.conflict).length);
</script>

<svelte:head>
	<title>Mock admin dashboard — Apostille Handbook</title>
</svelte:head>

<h1>Admin dashboard</h1>

<Notice variant="warning" heading="Fixture queues.">
	<p>Counts below come from the mock fixture set, not live data.</p>
</Notice>

<ul class="m-0 mt-4 list-none p-0 pb-8">
	<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]">
		<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/admin/triage')}
			>Fact / task triage</a
		>
		<span class="text-sm text-ink-muted">{awaiting} awaiting review ({conflicts} with conflict)</span>
	</li>
	<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]">
		<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/admin/console')}
			>Call console</a
		>
		<span class="text-sm text-ink-muted">1 demo transcript loaded</span>
	</li>
	<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]">
		<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/admin/config')}
			>Configuration</a
		>
		<span class="text-sm text-ink-muted">typed keys, secrets write-only</span>
	</li>
</ul>
