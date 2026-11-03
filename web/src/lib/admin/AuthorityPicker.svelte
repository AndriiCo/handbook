<script lang="ts">
	import { onMount, tick } from 'svelte';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { fetchAuthorities, type AuthorityOption } from './authorities.js';
	import { errorText } from './admin-api.js';
	import ChevronsUpDownIcon from '@lucide/svelte/icons/chevrons-up-down';

	// Searchable authority combobox (shadcn Popover + Command): staff
	// type a country, region or slug. Reads the shared authority list.
	let {
		value = $bindable(''),
		id,
		label = 'Authority',
		disabled = false,
		onLoaded
	}: {
		value?: string;
		id: string;
		label?: string;
		disabled?: boolean;
		onLoaded?: (rows: AuthorityOption[]) => void;
	} = $props();

	let open = $state(false);
	let rows = $state<AuthorityOption[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let trigger = $state<HTMLElement | null>(null);

	const selected = $derived(rows.find((r) => r.slug === value) ?? null);

	async function load() {
		loadState = 'loading';
		try {
			rows = await fetchAuthorities();
			loadState = 'ready';
			onLoaded?.(rows);
		} catch (err) {
			loadError = errorText(err, 'Authorities failed to load.');
			loadState = 'failed';
		}
	}

	onMount(() => {
		void load();
	});

	function choose(slug: string) {
		value = slug;
		open = false;
		void tick().then(() => trigger?.focus());
	}

	/** Focus the trigger (the empty-state next action points here). */
	export function focus() {
		trigger?.focus();
	}
</script>

<div class="flex min-w-0 flex-col gap-1">
	<span id={`${id}-label`} class="text-xs font-medium text-ink-muted">{label}</span>
	{#if loadState === 'loading'}
		<Skeleton class="h-9 w-full rounded-md" />
	{:else if loadState === 'failed'}
		<div class="flex flex-wrap items-center gap-2">
			<span class="text-sm text-ink-muted" role="status">{loadError}</span>
			<Button variant="outline" size="sm" onclick={load}>Try again</Button>
		</div>
	{:else}
		<Popover.Root bind:open>
			<Popover.Trigger bind:ref={trigger} {disabled}>
				{#snippet child({ props })}
					<Button
						{...props}
						{id}
						variant="outline"
						size="lg"
						class="w-full min-w-0 justify-between font-normal"
						role="combobox"
						aria-expanded={open}
						aria-labelledby={`${id}-label ${id}-value`}
					>
						<span id={`${id}-value`} class={`min-w-0 truncate ${selected ? 'text-ink' : 'text-ink-muted'}`}>
							{selected ? selected.name : 'Choose an authority'}
						</span>
						<ChevronsUpDownIcon class="size-4 flex-none opacity-60" aria-hidden="true" />
					</Button>
				{/snippet}
			</Popover.Trigger>
			<Popover.Content class="p-0" align="start">
				<Command.Root>
					<Command.Input placeholder="Search country, region or slug" aria-label="Search authorities" />
					<Command.List aria-label="Authorities">
						<Command.Empty>No authority matches.</Command.Empty>
						<Command.Group>
							{#each rows as row (row.slug)}
								<Command.Item
									value={row.slug}
									keywords={[row.name, row.country, row.region].filter(Boolean)}
									onSelect={() => choose(row.slug)}
									data-checked={row.slug === value}
								>
									<span class="flex min-w-0 flex-1 flex-col">
										<span class="truncate">{row.name}</span>
										<span class="truncate font-mono text-xs text-ink-muted">
											{row.slug} · {row.fact_count} fact{row.fact_count === 1 ? '' : 's'}
										</span>
									</span>
								</Command.Item>
							{/each}
						</Command.Group>
					</Command.List>
				</Command.Root>
			</Popover.Content>
		</Popover.Root>
	{/if}
</div>
