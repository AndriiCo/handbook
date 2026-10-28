<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { fetchConfig, saveConfigKey } from '../../../../lib/admin/config-client.js';

	// Typed config keys per secure-config.md §1.5. SECRET rows are
	// write-only: the page shows Set/Unset and a new-value box only —
	// values are never fetched, rendered, or kept after save.
	// The admin API keeps the /apostille prefix on the admin origin
	// (spec §1.5), so the root is literal — the base-deriving trick
	// other pages use would resolve to '' on the admin host.
	const api = () => `/apostille/api/v1`;

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
<p class="max-w-[44rem] text-ink-muted">
	Encrypted server config. Secrets are set-new-only: this page can never
	display one. Every save writes an audit row.
</p>

{#if loadState === 'loading'}
	<p aria-live="polite">Loading config keys…</p>
{:else if loadState === 'failed'}
	<Notice variant="error" heading="Config unavailable.">
		<p>{loadError}</p>
	</Notice>
	<p>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
			onclick={load}>Retry</button
		>
	</p>
{:else if rows.length === 0}
	<Notice variant="info" heading="No config keys.">
		<p>The server returned an empty key list. Seed keys first, then reload.</p>
	</Notice>
	<p>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
			onclick={load}>Reload</button
		>
	</p>
{:else}
	<ul class="mt-4 mb-0 grid max-w-[44rem] list-none gap-4 p-0 pb-8">
		{#each rows as row (row.key)}
			<li class="rounded-sm border border-line px-4 py-[0.9rem]">
				<div class="mb-2 flex flex-wrap items-baseline gap-x-3 gap-y-1">
					<code class="font-mono font-semibold break-all">{row.key}</code>
					<span class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">{row.type}</span>
				</div>
				{#if row.type === 'bool'}
					<label class="min-h-11">
						<input
							class="mr-2 size-5 accent-accent align-middle"
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
					<label class="flex min-h-11 flex-col gap-[0.35rem] font-bold" for={`cfg-${row.key}`}>
						Value
						<input
							class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-normal text-ink [font:inherit]"
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
					<p class="mt-1 mb-2 font-bold" aria-live="polite">
						{row.set ? 'Set (value hidden)' : 'Not set'}
					</p>
					<label class="flex min-h-11 flex-col gap-[0.35rem] font-bold" for={`cfg-${row.key}`}>
						New value (write-only — overwrites)
						<input
							class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-normal text-ink [font:inherit]"
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
						class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
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
