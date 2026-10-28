<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../lib/public-ui/Notice.svelte';
	import {
		factCount,
		filterAuthorities,
		listAuthorities,
		publicError
	} from '../../lib/public-ui/handbook.js';

	// Slice A: live search over GET /api/v1/authorities, filtered
	// client-side as you type. Same-origin, no auth. EN-only until
	// the Week 3+ i18n pass.
	let query = $state('');
	let index = $state<
		{ slug: string; name: string; country: string; fact_count: number }[] | null
	>(null);
	let loadError = $state<string | null>(null);

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	$effect(() => {
		listAuthorities({ get })
			.then((res) => {
				if (!res.ok) {
					loadError = publicError(res.status, 'index');
					return;
				}
				const rows = res.body?.authorities;
				index = Array.isArray(rows) ? rows : [];
			})
			.catch(() => {
				loadError = publicError(500, 'index');
			});
	});

	const blank = $derived(query.trim() === '');
	const results = $derived(index === null ? null : filterAuthorities(index, query));
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
	<form role="search" onsubmit={(e) => e.preventDefault()}>
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
</section>

<section aria-labelledby="index">
	<h2 id="index" class="section-label">Authority index</h2>
	{#if loadError}
		<Notice variant="error" heading="Search is unavailable."><p>{loadError}</p></Notice>
	{:else if results === null}
		<p role="status">Loading the authority index…</p>
	{:else if results.length === 0 && blank}
		<Notice variant="info" heading="The handbook has no published authorities yet.">
			<p>Check back after the first handbook build.</p>
		</Notice>
	{:else if results.length === 0}
		<Notice variant="info" heading="No authority matches “{query}”.">
			<p>Try a country name or a shorter term.</p>
		</Notice>
	{:else}
		<ul class="ledger" aria-label={blank ? 'Authority index' : 'Search results'}>
			{#each results as authority (authority.slug)}
				<li>
					<a href={resolve('/(public)/authority/[slug]', { slug: authority.slug })}
						>{authority.name}</a
					>
					<span class="kind">{authority.country} · {factCount(authority.fact_count ?? 0)}</span>
				</li>
			{/each}
		</ul>
	{/if}
</section>

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
	section[aria-labelledby='index'] {
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
	}
</style>
