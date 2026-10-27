<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { fetchConfig, saveConfigKey } from '../../../../lib/admin/config-client.js';

	// Typed config keys per secure-config.md §1.5. SECRET rows are
	// write-only: the page shows Set/Unset and a new-value box only —
	// values are never fetched, rendered, or kept after save.
	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const api = () => `${resolve('/(public)').replace(/\/$/, '')}/api/v1`;

	interface Row {
		key: string;
		type: 'bool' | 'text' | 'secret';
		value?: boolean | string;
		set?: boolean;
		draft?: string;
		dirty: boolean;
		saving: boolean;
		error: string | null;
		saved: boolean;
	}

	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let rows = $state<Row[]>([]);

	async function load() {
		loadState = 'loading';
		loadError = null;
		try {
			const list = await fetchConfig(api());
			rows = list.map((row) => ({ ...row, saved: false }));
			loadState = 'ready';
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Config load failed.';
			loadState = 'failed';
		}
	}

	onMount(load);

	function markDirty(row: Row) {
		row.dirty = true;
		row.saved = false;
		row.error = null;
	}

	async function save(row: Row) {
		row.saving = true;
		row.error = null;
		try {
			await saveConfigKey(api(), row);
			row.dirty = false;
			row.saved = true;
			if (row.type === 'secret') {
				row.set = true;
				row.draft = '';
			}
		} catch (err) {
			row.error = err instanceof Error ? err.message : 'Save failed.';
		} finally {
			row.saving = false;
		}
	}
</script>

<svelte:head>
	<title>Configuration — Admin — Apostille Handbook</title>
</svelte:head>

<h1>Configuration</h1>
<p class="lede">
	Encrypted server config. Secrets are set-new-only: this page can never
	display one. Every save writes an audit row.
</p>

{#if loadState === 'loading'}
	<p aria-live="polite">Loading config keys…</p>
{:else if loadState === 'failed'}
	<Notice variant="error" heading="Config unavailable.">
		<p>{loadError}</p>
	</Notice>
	<p><button onclick={load}>Retry</button></p>
{:else if rows.length === 0}
	<Notice variant="info" heading="No config keys.">
		<p>The server returned an empty key list. Seed keys first, then reload.</p>
	</Notice>
	<p><button onclick={load}>Reload</button></p>
{:else}
	<ul class="keys">
		{#each rows as row (row.key)}
			<li>
				<div class="key-head">
					<code class="mono">{row.key}</code>
					<span class="type">{row.type}</span>
				</div>
				{#if row.type === 'bool'}
					<label>
						<input
							type="checkbox"
							checked={row.value === true}
							onchange={(e) => {
								row.value = e.currentTarget.checked;
								markDirty(row);
							}}
						/>
						Enabled
					</label>
				{:else if row.type === 'text'}
					<label class="stack" for={`cfg-${row.key}`}>
						Value
						<input
							id={`cfg-${row.key}`}
							type="text"
							value={typeof row.value === 'string' ? row.value : ''}
							maxlength={4096}
							oninput={(e) => {
								row.value = e.currentTarget.value;
								markDirty(row);
							}}
						/>
					</label>
				{:else}
					<p class="secret-state" aria-live="polite">
						{row.set ? 'Set (value hidden)' : 'Not set'}
					</p>
					<label class="stack" for={`cfg-${row.key}`}>
						New value (write-only — overwrites)
						<input
							id={`cfg-${row.key}`}
							type="password"
							autocomplete="new-password"
							maxlength={8192}
							value={row.draft ?? ''}
							oninput={(e) => {
								row.draft = e.currentTarget.value;
								markDirty(row);
							}}
						/>
					</label>
				{/if}
				{#if row.error}
					<Notice variant="error" heading="Save failed."><p>{row.error}</p></Notice>
				{/if}
				{#if row.saved}
					<Notice variant="success" heading="Saved.">
						<p>“{row.key}” updated and audited.</p>
					</Notice>
				{/if}
				<p>
					<button
						onclick={() => save(row)}
						disabled={!row.dirty || row.saving || (row.type === 'secret' && !row.draft)}
					>
						{row.saving ? 'Saving…' : 'Save'}
					</button>
				</p>
			</li>
		{/each}
	</ul>
{/if}

<style>
	.lede {
		color: var(--text-muted);
		max-width: 44rem;
	}
	.keys {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0 0 2rem;
		display: grid;
		gap: 1rem;
		max-width: 44rem;
	}
	.keys li {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.9rem 1rem;
	}
	.key-head {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.25rem 0.75rem;
		margin-bottom: 0.5rem;
	}
	.mono {
		font-family: var(--font-mono);
		font-weight: 600;
		word-break: break-all;
	}
	.type {
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-muted);
	}
	.stack {
		display: flex;
		flex-direction: column;
		gap: 0.35rem;
		font-weight: 700;
	}
	label {
		min-height: 44px;
	}
	input[type='text'],
	input[type='password'] {
		min-height: 44px;
		padding: 0.6rem 0.8rem;
		font: inherit;
		font-weight: 400;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
	}
	input[type='checkbox'] {
		width: 1.25rem;
		height: 1.25rem;
		accent-color: var(--accent);
		vertical-align: middle;
		margin-right: 0.5rem;
	}
	.secret-state {
		font-weight: 700;
		margin: 0.25rem 0 0.5rem;
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
