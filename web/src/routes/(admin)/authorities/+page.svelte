<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { csrfHeaders } from '../../../lib/admin/config-client.js';

	// Staff authorities (P4-2): find, create, edit and hide authorities
	// without a release. Every save is audited (before → after, actor,
	// reason); a hidden row leaves every public page at once. Rows a pack
	// seeded follow their pack on the next load, so a lasting fix to one
	// also goes into the pack. Admin build only; the staff API keeps the
	// /apostille prefix on the admin origin.
	const api = '/apostille/api/v1/admin/authority-records';

	interface Row {
		slug: string;
		name: string;
		kind: string;
		country: string;
		region: string;
		iso2: string;
		subdivision: string;
		level: string;
		name_en: string;
		aliases: string[];
		hidden_at: number | null;
		fact_count: number;
		from_pack: boolean;
	}
	interface HistoryRow {
		at: number;
		actor: string;
		action: string;
		before: string;
		after: string;
		reason: string;
	}
	type Draft = Omit<Row, 'hidden_at' | 'fact_count' | 'from_pack' | 'aliases'> & { aliases: string };

	const KINDS = ['country', 'state', 'province', 'office'];
	const LEVELS = ['', 'federal', 'state', 'regional', 'local', 'consular'];

	let query = $state('');
	let show = $state<'all' | 'public' | 'hidden'>('all');
	let rows = $state<Row[] | null>(null);
	let total = $state(0);
	let listError = $state<string | null>(null);

	let selected = $state<Row | null>(null);
	let history = $state<HistoryRow[]>([]);
	let draft = $state<Draft | null>(null);
	let creating = $state(false);
	let reason = $state('');
	let busy = $state(false);
	let message = $state<{ ok: boolean; text: string } | null>(null);

	const blank = (): Draft => ({
		slug: '',
		name: '',
		kind: 'office',
		country: '',
		region: '',
		iso2: '',
		subdivision: '',
		level: '',
		name_en: '',
		aliases: ''
	});
	const toDraft = (r: Row): Draft => ({ ...r, aliases: r.aliases.join('\n') });

	async function search() {
		listError = null;
		try {
			const q = new URLSearchParams({ q: query, show });
			const res = await fetch(`${api}?${q}`);
			if (!res.ok) throw new Error(`authorities request failed (${res.status})`);
			const body = await res.json();
			rows = body.authorities as Row[];
			total = Number(body.total) || 0;
		} catch (err) {
			listError = err instanceof Error ? err.message : 'Authorities request failed.';
		}
	}

	onMount(search);

	async function open(slug: string) {
		message = null;
		creating = false;
		reason = '';
		const res = await fetch(`${api}/${encodeURIComponent(slug)}`);
		const body = await res.json().catch(() => ({}));
		if (!res.ok) {
			message = { ok: false, text: body?.error ?? `could not open ${slug}` };
			return;
		}
		selected = body.authority as Row;
		history = (body.history ?? []) as HistoryRow[];
		draft = toDraft(selected);
	}

	function startCreate() {
		message = null;
		selected = null;
		history = [];
		creating = true;
		draft = blank();
	}

	async function post(url: string, payload: unknown): Promise<any> {
		busy = true;
		message = null;
		try {
			const res = await fetch(url, {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				body: JSON.stringify(payload)
			});
			const body = await res.json().catch(() => ({}));
			if (!res.ok) throw new Error(body?.error ?? `request failed (${res.status})`);
			return body;
		} catch (err) {
			message = { ok: false, text: err instanceof Error ? err.message : 'Request failed.' };
			return null;
		} finally {
			busy = false;
		}
	}

	async function save() {
		if (!draft) return;
		const payload = { ...draft, aliases: draft.aliases.split('\n') };
		const body = creating
			? await post(api, payload)
			: await post(`${api}/${encodeURIComponent(draft.slug)}`, payload);
		if (!body) return;
		const changed: string[] = body.changed ?? [];
		message = {
			ok: true,
			text: creating
				? `Added ${draft.slug}.`
				: changed.length > 0
					? `Saved: ${changed.join(', ')}.`
					: 'Nothing changed.'
		};
		await open(body.authority.slug);
		await search();
	}

	async function setHidden(hidden: boolean) {
		if (!selected) return;
		const body = await post(`${api}/${encodeURIComponent(selected.slug)}/visibility`, {
			hidden,
			reason
		});
		if (!body) return;
		message = { ok: true, text: hidden ? 'Hidden from public pages.' : 'Shown on public pages again.' };
		await open(selected.slug);
		await search();
	}

	const when = (at: number) => new Date(at * 1000).toISOString().replace('T', ' ').slice(0, 16);
	const place = (r: Row) => [r.region, r.country].filter(Boolean).join(', ');
</script>

<svelte:head>
	<title>Authorities — staff</title>
</svelte:head>

<main class="mx-auto max-w-5xl space-y-6 p-6">
	<div class="flex flex-wrap items-center gap-3">
		<h1 class="m-0 flex-1 text-xl font-bold">Authorities</h1>
		<button
			type="button"
			class="min-h-11 rounded border border-accent-fill bg-accent-fill px-4 py-2 text-accent-ink"
			onclick={startCreate}
		>
			New authority
		</button>
	</div>
	<p class="m-0 text-sm">
		Find, rename, place and hide the offices the handbook publishes. Every save is audited. Hiding
		removes a row from the public index, dossiers, the sitemap and follows at once; its facts and
		history stay. Rows marked <em>from a pack</em> follow their pack on the next load, so fix the
		pack too.
	</p>

	<form
		class="flex flex-wrap items-end gap-3"
		onsubmit={(e) => {
			e.preventDefault();
			void search();
		}}
	>
		<label class="flex min-w-60 flex-1 flex-col text-sm">
			Search name, English or other names, slug, country, region
			<input class="min-h-11 rounded border border-line bg-bg px-3" bind:value={query} />
		</label>
		<label class="flex flex-col text-sm">
			Show
			<select class="min-h-11 rounded border border-line bg-bg px-3" bind:value={show} onchange={search}>
				<option value="all">All</option>
				<option value="public">Public</option>
				<option value="hidden">Hidden</option>
			</select>
		</label>
		<button type="submit" class="min-h-11 rounded border border-line px-4">Search</button>
	</form>

	{#if message}
		<Notice variant={message.ok ? 'info' : 'error'} heading={message.ok ? 'Done' : 'Not saved'}>
			<p>{message.text}</p>
		</Notice>
	{/if}

	<div class="grid gap-6 lg:grid-cols-2">
		<section aria-label="Results">
			{#if listError}
				<Notice variant="error" heading="Authorities unavailable"><p>{listError}</p></Notice>
			{:else if rows === null}
				<div role="status" aria-label="Working" class="space-y-2">
					<div class="h-4 animate-pulse rounded bg-surface-2"></div>
					<div class="h-4 w-2/3 animate-pulse rounded bg-surface-2"></div>
				</div>
			{:else}
				<p class="mt-0 text-sm text-ink-muted">
					{total} match{total === 1 ? '' : 'es'}{#if total > rows.length}, first {rows.length} shown{/if}.
				</p>
				<ul class="m-0 list-none p-0">
					{#each rows as row (row.slug)}
						<li class="m-0 border-b border-line p-0">
							<button
								type="button"
								class={`flex min-h-11 w-full items-center gap-3 py-2 text-left ${selected?.slug === row.slug ? 'bg-surface' : ''}`}
								onclick={() => open(row.slug)}
							>
								<span class="min-w-0 flex-1">
									<span class={`block break-words ${row.hidden_at ? 'text-ink-muted line-through' : 'text-ink'}`}>
										{row.name}
									</span>
									<span class="block font-mono text-xs text-ink-muted">{row.slug} · {place(row)}</span>
								</span>
								{#if row.hidden_at}
									<span class="flex-none rounded-xs border border-line px-2 text-xs text-ink-muted">hidden</span>
								{/if}
								{#if row.level}
									<span class="flex-none text-xs text-ink-muted">{row.level}</span>
								{/if}
							</button>
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		{#if draft}
			<section aria-label={creating ? 'New authority' : 'Edit authority'} class="space-y-3 rounded border border-line p-4">
				<h2 class="m-0 text-lg font-bold">
					{creating ? 'New authority' : selected?.name}
				</h2>
				{#if selected}
					<p class="m-0 text-sm text-ink-muted">
						{selected.fact_count} accepted fact{selected.fact_count === 1 ? '' : 's'}{#if selected.from_pack}
							· from a pack{/if}{#if selected.hidden_at}
							· hidden since {when(selected.hidden_at)}{/if}
					</p>
				{/if}
				<form
					class="space-y-3"
					onsubmit={(e) => {
						e.preventDefault();
						void save();
					}}
				>
					<label class="flex flex-col text-sm">
						Slug {creating ? '(the permanent key: a-z, 0-9, -)' : '(never changes)'}
						<input class="min-h-11 rounded border border-line bg-bg px-3 font-mono" bind:value={draft.slug} readonly={!creating} required />
					</label>
					<label class="flex flex-col text-sm">
						Name (as the office writes it)
						<input class="min-h-11 rounded border border-line bg-bg px-3" bind:value={draft.name} required />
					</label>
					<label class="flex flex-col text-sm">
						English name (searched; when the name is in another language)
						<input class="min-h-11 rounded border border-line bg-bg px-3" bind:value={draft.name_en} />
					</label>
					<label class="flex flex-col text-sm">
						Other names, one per line (searched: "Bayern", "Deutschland")
						<textarea class="min-h-20 rounded border border-line bg-bg px-3 py-2" bind:value={draft.aliases}></textarea>
					</label>
					<div class="grid grid-cols-2 gap-3">
						<label class="flex flex-col text-sm">
							Kind
							<select class="min-h-11 rounded border border-line bg-bg px-3" bind:value={draft.kind}>
								{#each KINDS as kind (kind)}<option value={kind}>{kind}</option>{/each}
							</select>
						</label>
						<label class="flex flex-col text-sm">
							Level
							<select class="min-h-11 rounded border border-line bg-bg px-3" bind:value={draft.level}>
								{#each LEVELS as level (level)}<option value={level}>{level || '—'}</option>{/each}
							</select>
						</label>
						<label class="flex flex-col text-sm">
							Country
							<input class="min-h-11 rounded border border-line bg-bg px-3" bind:value={draft.country} />
						</label>
						<label class="flex flex-col text-sm">
							Region
							<input class="min-h-11 rounded border border-line bg-bg px-3" bind:value={draft.region} />
						</label>
						<label class="flex flex-col text-sm">
							ISO country (US)
							<input class="min-h-11 rounded border border-line bg-bg px-3 font-mono" bind:value={draft.iso2} maxlength="2" />
						</label>
						<label class="flex flex-col text-sm">
							ISO subdivision (US-WA)
							<input class="min-h-11 rounded border border-line bg-bg px-3 font-mono" bind:value={draft.subdivision} />
						</label>
					</div>
					<button
						type="submit"
						class="min-h-11 rounded border border-accent-fill bg-accent-fill px-4 py-2 text-accent-ink disabled:opacity-50"
						disabled={busy}
					>
						{creating ? 'Add authority' : 'Save changes'}
					</button>
				</form>

				{#if selected}
					<div class="space-y-2 border-t border-line pt-3">
						<label class="flex flex-col text-sm">
							Reason ({selected.hidden_at ? 'show again' : 'hide'}; kept in the audit log)
							<input class="min-h-11 rounded border border-line bg-bg px-3" bind:value={reason} maxlength="300" />
						</label>
						<button
							type="button"
							class="min-h-11 rounded border border-line px-4 py-2 disabled:opacity-50"
							disabled={busy || reason.trim() === ''}
							onclick={() => setHidden(!selected?.hidden_at)}
						>
							{selected.hidden_at ? 'Show on public pages' : 'Hide from public pages'}
						</button>
					</div>
					{#if history.length > 0}
						<details class="border-t border-line pt-3">
							<summary class="cursor-pointer text-sm font-medium">History ({history.length})</summary>
							<ol class="m-0 mt-2 list-none space-y-2 p-0 text-xs">
								{#each history as h, i (i)}
									<li class="m-0 rounded border border-line p-2">
										<p class="m-0 font-mono">{when(h.at)} · {h.action} · {h.actor}</p>
										{#if h.before || h.after}
											<p class="m-0 break-words whitespace-pre-wrap text-ink-muted">{h.before} → {h.after}</p>
										{/if}
										{#if h.reason}<p class="m-0 break-words">{h.reason}</p>{/if}
									</li>
								{/each}
							</ol>
						</details>
					{/if}
				{/if}
			</section>
		{/if}
	</div>
</main>
