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
		role="search"
		onsubmit={(e) => {
			e.preventDefault();
			submitted = query;
		}}
	>
		<label class="visually-hidden" for="q">Search the handbook</label>
		<input
			id="q"
			name="q"
			type="search"
			bind:value={query}
			placeholder="Country, state, or authority…"
			autocomplete="off"
		/>
		<button type="submit">Search</button>
	</form>
	{#if results !== null}
		{#if results.length === 0}
			<Notice variant="info" heading="No authority matches “{submitted}”.">
				<p>Try “secretary”, “consulate”, or “clerk”. Fixture index holds {AUTHORITIES.length} entries.</p>
			</Notice>
		{:else}
			<ul class="ledger" aria-label="Search results">
				{#each results as authority (authority.slug)}
					<li>
						<a href={resolve('/(mock)/mock/authority/[slug]', { slug: authority.slug })}
							>{authority.name}</a
						>
						<span class="kind">{authority.kind}</span>
						<VerifiedChip tier={authority.tier} verified={authority.verified} />
					</li>
				{/each}
			</ul>
		{/if}
	{/if}
</section>

<section aria-labelledby="index">
	<h2 id="index" class="section-label">Authority index</h2>
	<ul class="ledger">
		{#each AUTHORITIES as authority (authority.slug)}
			<li>
				<a href={resolve('/(mock)/mock/authority/[slug]', { slug: authority.slug })}
					>{authority.name}</a
				>
				<span class="kind">{authority.kind}</span>
				<VerifiedChip tier={authority.tier} verified={authority.verified} />
			</li>
		{/each}
	</ul>
</section>

<style>
	.visually-hidden {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip: rect(0 0 0 0);
	}
	form {
		display: flex;
		gap: 0.5rem;
		margin: 1rem 0;
	}
	input[type='search'] {
		flex: 1;
		min-height: 44px;
		padding: 0.6rem 0.8rem;
		font: inherit;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
	}
	button {
		min-height: 44px;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
		padding: 0 1.25rem;
	}
	:root[data-theme='dark'] button {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	.section-label {
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-muted);
	}
	.ledger {
		list-style: none;
		margin: 0 0 2rem;
		padding: 0;
	}
	.ledger li {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.25rem 0.75rem;
		padding: 0.7rem 0;
		border-bottom: 1px solid var(--border);
	}
	.ledger a {
		font-weight: 700;
		min-height: 44px;
		display: inline-flex;
		align-items: center;
	}
	.kind {
		color: var(--text-muted);
		font-size: 0.875rem;
	}
</style>
