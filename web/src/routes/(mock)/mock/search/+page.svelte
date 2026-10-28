<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import VerifiedChip from '../../../../lib/public-ui/VerifiedChip.svelte';
	import { AUTHORITIES, searchMock } from '../../../../lib/mock/fixtures';

	let query = $state('');
	let submitted = $state<string | null>(null);

	const results = $derived(submitted === null ? null : searchMock(submitted));
</script>

<svelte:head>
	<title>Mock search — Apostille Handbook</title>
</svelte:head>

<section aria-labelledby="start">
	<h1 id="start">Start with your destination</h1>
	<form
		class="my-4 flex gap-2"
		role="search"
		onsubmit={(e) => {
			e.preventDefault();
			submitted = query;
		}}
	>
		<label class="sr-only" for="q">Search the handbook</label>
		<input
			class="min-h-11 flex-1 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink [font:inherit]"
			id="q"
			name="q"
			type="search"
			bind:value={query}
			placeholder="Country, state, or authority…"
			autocomplete="off"
		/>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit]"
			type="submit">Search</button
		>
	</form>
	{#if results !== null}
		{#if results.length === 0}
			<Notice variant="info" heading="No authority matches “{submitted}”.">
				<p>Try “secretary”, “consulate”, or “clerk”. Fixture index holds {AUTHORITIES.length} entries.</p>
			</Notice>
		{:else}
			<ul class="m-0 mb-8 list-none p-0" aria-label="Search results">
				{#each results as authority (authority.slug)}
					<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]">
						<a
							class="inline-flex min-h-11 items-center font-bold"
							href={resolve('/(mock)/mock/authority/[slug]', { slug: authority.slug })}
							>{authority.name}</a
						>
						<span class="text-sm text-ink-muted">{authority.kind}</span>
						<VerifiedChip tier={authority.tier} verified={authority.verified} />
					</li>
				{/each}
			</ul>
		{/if}
	{/if}
</section>

<section aria-labelledby="index">
	<h2 id="index" class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">Authority index</h2>
	<ul class="m-0 mb-8 list-none p-0">
		{#each AUTHORITIES as authority (authority.slug)}
			<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]">
				<a
					class="inline-flex min-h-11 items-center font-bold"
					href={resolve('/(mock)/mock/authority/[slug]', { slug: authority.slug })}
					>{authority.name}</a
				>
				<span class="text-sm text-ink-muted">{authority.kind}</span>
				<VerifiedChip tier={authority.tier} verified={authority.verified} />
			</li>
		{/each}
	</ul>
</section>
