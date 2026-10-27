<script lang="ts">
	import Notice from '../../../../../lib/public-ui/Notice.svelte';
	import { CONFIG } from '../../../../../lib/mock/fixtures';

	// Mirrors the real config contract: bool/text rows edit in place,
	// SECRET rows are write-only — Set/Unset plus a new-value box, and
	// the typed value is cleared right after the mock save.
	interface Row {
		key: string;
		type: 'bool' | 'text' | 'secret';
		value?: boolean | string;
		set?: boolean;
		hint: string;
		draft: string;
		boolDraft: boolean;
		saved: boolean;
	}

	let rows = $state<Row[]>(
		CONFIG.map((r) => ({
			...r,
			draft: typeof r.value === 'string' ? r.value : '',
			boolDraft: r.value === true,
			saved: false
		}))
	);

	function save(row: Row) {
		if (row.type === 'bool') row.value = row.boolDraft;
		if (row.type === 'text') row.value = row.draft;
		if (row.type === 'secret') {
			if (!row.draft) return;
			row.set = true;
			row.draft = '';
		}
		row.saved = true;
	}
</script>

<svelte:head>
	<title>Mock config — Apostille Handbook</title>
</svelte:head>

<h1>Configuration</h1>

<Notice variant="info" heading="Fixture keys.">
	<p>Saves only flip the Saved marker on this page. Secret values are never displayed.</p>
</Notice>

<ul class="rows">
	{#each rows as row (row.key)}
		<li>
			<div class="head">
				<code>{row.key}</code>
				<span class="kind">{row.type}</span>
				{#if row.saved}<span class="saved" role="status">Saved (mock)</span>{/if}
			</div>
			<p class="hint">{row.hint}</p>
			{#if row.type === 'bool'}
				<label class="inline">
					<input type="checkbox" bind:checked={row.boolDraft} />
					Enabled
				</label>
			{:else if row.type === 'text'}
				<label class="stack">
					Value
					<input type="text" bind:value={row.draft} />
				</label>
			{:else}
				<p>Status: <strong>{row.set ? 'Set' : 'Unset'}</strong></p>
				<label class="stack">
					New secret value (write-only)
					<input
						type="password"
						bind:value={row.draft}
						placeholder="enter a new value to replace"
						autocomplete="new-password"
					/>
				</label>
			{/if}
			<p>
				<button onclick={() => save(row)} disabled={row.type === 'secret' && !row.draft}>
					Save
				</button>
			</p>
		</li>
	{/each}
</ul>

<style>
	.rows {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0 0 2rem;
		display: grid;
		gap: 1rem;
	}
	.rows li {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.9rem 1rem;
	}
	.head {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.25rem 0.75rem;
	}
	code {
		font-family: var(--font-mono);
		font-weight: 700;
	}
	.kind {
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-muted);
	}
	.saved {
		margin-left: auto;
		font-size: 0.875rem;
		color: var(--success);
		font-weight: 700;
	}
	.hint {
		font-size: 0.875rem;
		color: var(--text-muted);
		margin: 0.25rem 0 0.5rem;
	}
	.stack {
		display: grid;
		gap: 0.3rem;
		font-size: 0.875rem;
		max-width: 28rem;
	}
	.inline {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		min-height: 44px;
	}
	input[type='checkbox'] {
		width: 1.25rem;
		height: 1.25rem;
		accent-color: var(--accent);
	}
	input[type='text'],
	input[type='password'] {
		font: inherit;
		color: var(--text);
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.5rem 0.75rem;
		min-height: 44px;
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
	button:disabled {
		opacity: 0.55;
		cursor: not-allowed;
	}
</style>
