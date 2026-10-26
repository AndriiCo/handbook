<script lang="ts">
	// Static placeholders only — no backend calls yet.
	let query = $state('');
	let submitted = $state<string | null>(null);

	const popular = [
		{ name: 'Placeholder Authority One', note: 'Verified date TBD' },
		{ name: 'Placeholder Authority Two', note: 'Verified date TBD' },
		{ name: 'Placeholder Authority Three', note: 'Verified date TBD' }
	];
</script>

<svelte:head>
	<title>Apostille Handbook — Start with your destination</title>
	<meta
		name="description"
		content="Find apostille and authentication requirements by destination authority."
	/>
</svelte:head>

<section class="hero">
	<h1>Start with your destination</h1>
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
	{#if submitted !== null}
		<p class="notice">Search is not wired up yet — showing nothing for “{submitted}”.</p>
	{/if}
</section>

<section aria-labelledby="popular">
	<h2 id="popular">Popular authorities</h2>
	<ul class="cards">
		{#each popular as authority (authority.name)}
			<li>
				<span class="name">{authority.name}</span>
				<span class="note">{authority.note}</span>
			</li>
		{/each}
	</ul>
</section>

<style>
	.hero {
		padding: 1.5rem 0;
	}
	h1 {
		font-size: 1.875rem;
		margin: 0 0 1rem;
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
	.notice {
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
	}
	.cards {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 0.75rem;
	}
	.cards li {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
	}
	.name {
		font-weight: 700;
	}
	.note {
		color: var(--text-muted);
		font-size: 0.875rem;
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
		.cards {
			grid-template-columns: repeat(3, 1fr);
		}
	}
</style>
