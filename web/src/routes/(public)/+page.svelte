<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../lib/public-ui/Notice.svelte';
	import VerifiedChip from '../../lib/public-ui/VerifiedChip.svelte';
	import {
		AUTHORITIES,
		RECENT_CHANGES,
		searchAuthorities
	} from '../../lib/public-ui/authorities.js';

	// EN-only until the Week 3+ i18n pass. No backend calls yet: the
	// publish lane (accepted facts) has not landed, so search runs over
	// the placeholder index in lib/public-ui.
	let query = $state('');
	let submitted = $state<string | null>(null);

	const results = $derived(submitted === null ? null : searchAuthorities(submitted));
</script>

<svelte:head>
	<title>Apostille Handbook — Start with your destination</title>
	<meta
		name="description"
		content="Find apostille and authentication requirements by destination authority."
	/>
</svelte:head>

<section class="search-panel" aria-labelledby="start">
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
				<p>
					Search runs over {AUTHORITIES.length} placeholder authorities for now.
					<a href={resolve('/(public)')}>Browse the full index below</a>.
				</p>
			</Notice>
		{:else}
			<ul class="ledger" aria-label="Search results">
				{#each results as authority (authority.slug)}
					<li>
						<a href={resolve('/(public)/authority/[slug]', { slug: authority.slug })}
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

<div class="ledger-grid">
	<section aria-labelledby="index">
		<h2 id="index" class="section-label">Authority index</h2>
		<ul class="ledger">
			{#each AUTHORITIES as authority (authority.slug)}
				<li>
					<a href={resolve('/(public)/authority/[slug]', { slug: authority.slug })}
						>{authority.name}</a
					>
					<span class="kind">{authority.kind}</span>
					<VerifiedChip tier={authority.tier} verified={authority.verified} />
				</li>
			{/each}
		</ul>
	</section>

	<section aria-labelledby="recent">
		<h2 id="recent" class="section-label">Recently verified changes</h2>
		<ul class="ledger feed">
			{#each RECENT_CHANGES as change (change.text)}
				<li>
					<p>{change.text}</p>
					<p class="meta mono">{change.date} · {change.source}</p>
				</li>
			{/each}
		</ul>
	</section>
</div>

<style>
	.search-panel {
		padding: 1.5rem 0;
	}
	h1 {
		font-size: 1.875rem;
		margin: 0 0 1rem;
	}
	.section-label {
		font-size: 0.8rem;
		font-family: var(--font-body);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-muted);
		border-bottom: 1px solid var(--border);
		padding-bottom: 0.4rem;
	}
	form {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}
	.visually-hidden {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip: rect(0 0 0 0);
	}
	input[type='search'] {
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
	}
	:root[data-theme='dark'] button {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	.ledger-grid {
		display: grid;
		gap: 2rem;
		padding-bottom: 2rem;
	}
	.ledger {
		list-style: none;
		margin: 1rem 0 0;
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
		margin-right: auto;
	}
	.feed p {
		margin: 0;
	}
	.meta {
		color: var(--text-muted);
		font-size: 0.8rem;
	}
	.mono {
		font-family: var(--font-mono);
	}
	@media (min-width: 720px) {
		h1 {
			font-size: 2.25rem;
		}
		form {
			flex-direction: row;
		}
		input[type='search'] {
			flex: 1;
		}
		.ledger-grid {
			grid-template-columns: 3fr 2fr;
		}
	}
</style>
