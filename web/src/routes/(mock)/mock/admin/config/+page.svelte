<script lang="ts">
	import Notice from '../../../../../lib/public-ui/Notice.svelte';
	import SecretInput from '../../../../../lib/admin/SecretInput.svelte';
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

<ul class="mt-4 mb-0 grid list-none gap-4 p-0 pb-8">
	{#each rows as row (row.key)}
		<li class="rounded-sm border border-line px-4 py-[0.9rem]">
			<div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
				<code class="font-mono font-bold">{row.key}</code>
				<span class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">{row.type}</span>
				{#if row.saved}<span class="ml-auto text-sm font-bold text-success" role="status">Saved (mock)</span>{/if}
			</div>
			<p class="mt-1 mb-2 text-sm text-ink-muted">{row.hint}</p>
			{#if row.type === 'bool'}
				<label class="flex min-h-11 items-center gap-[0.6rem]">
					<input class="size-5 accent-accent" type="checkbox" bind:checked={row.boolDraft} />
					Enabled
				</label>
			{:else if row.type === 'text'}
				<label class="grid max-w-[28rem] gap-[0.3rem] text-sm">
					Value
					<input
						class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
						type="text"
						bind:value={row.draft}
					/>
				</label>
			{:else}
				<p>Status: <strong>{row.set ? 'Set' : 'Unset'}</strong></p>
				<div class="grid max-w-[28rem] gap-[0.3rem] text-sm">
					<label for={`mock-${row.key}`}>New value for {row.key} (write-only)</label>
					<SecretInput
						id={`mock-${row.key}`}
						class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
						bind:value={row.draft}
						placeholder="enter a new value to replace"
					/>
				</div>
			{/if}
			<p>
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
					onclick={() => save(row)}
					disabled={row.type === 'secret' && !row.draft}
				>
					Save
				</button>
			</p>
		</li>
	{/each}
</ul>
