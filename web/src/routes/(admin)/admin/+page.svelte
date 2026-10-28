<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
</script>

<svelte:head>
	<title>Admin dashboard — Apostille Handbook</title>
</svelte:head>

<h1>Admin dashboard</h1>

<Notice variant="warning" heading="Thin slice.">
	<p>
		Queues below are shells until the approval and job lanes land.
		Counts are placeholders, not live data.
	</p>
</Notice>

<ul class="m-0 mt-4 list-none p-0 pb-8">
	<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]">
		<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(admin)/admin/triage')}
			>Fact / task triage</a
		>
		<span class="text-sm text-ink-muted">0 awaiting review (placeholder)</span>
	</li>
	<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]">
		<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(admin)/admin/console')}
			>Call console</a
		>
		<span class="text-sm text-ink-muted">no active calls (placeholder)</span>
	</li>
	<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]">
		<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(admin)/admin/config')}
			>Configuration</a
		>
		<span class="text-sm text-ink-muted">typed keys, secrets write-only</span>
	</li>
</ul>
