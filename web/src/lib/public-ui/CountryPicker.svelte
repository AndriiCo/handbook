<script lang="ts">
	import Flag from '../components/brand/Flag.svelte';
	import { flagFor } from './lanes.js';
	import { filterCountries } from './route.js';
	import XIcon from '@lucide/svelte/icons/x';

	// One country combobox for the guided hero (ARIA 1.2 combobox with a
	// listbox popup): type to filter, arrows to move, Enter to pick. The
	// value is a lowercase alpha-2 code; names come from the caller.
	let {
		id,
		label,
		placeholder,
		options,
		value = $bindable(''),
		noMatch,
		listLabel,
		clearLabel,
		onpick
	}: {
		id: string;
		label: string;
		placeholder: string;
		options: { code: string; name: string }[];
		value?: string;
		noMatch: (query: string) => string;
		listLabel: string;
		clearLabel: string;
		onpick?: (code: string) => void;
	} = $props();

	const LIMIT = 8;
	let query = $state('');
	let open = $state(false);
	let active = $state(0);
	let input = $state<HTMLInputElement | null>(null);

	const selected = $derived(options.find((o) => o.code === value) ?? null);
	const shown = $derived(filterCountries(options, query).slice(0, LIMIT));
	const flag = $derived(selected ? flagFor(selected.name, selected.code) : null);
	const listId = $derived(`${id}-list`);

	function choose(code: string) {
		value = code;
		query = '';
		open = false;
		onpick?.(code);
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'ArrowDown') {
			e.preventDefault();
			open = true;
			active = Math.min(active + 1, shown.length - 1);
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			active = Math.max(active - 1, 0);
		} else if (e.key === 'Enter' && open && shown[active]) {
			e.preventDefault();
			choose(shown[active].code);
		} else if (e.key === 'Escape') {
			open = false;
			query = '';
		}
	}
</script>

<div class="relative min-w-0">
	<label for={id} class="mb-1 block text-sm font-medium text-ink">{label}</label>
	<div
		class="flex h-14 items-center gap-2 rounded-lg border border-line bg-surface px-3 focus-within:border-focus"
	>
		{#if flag && !query}
			<Flag code={flag} label="" class="size-5 flex-none" />
		{/if}
		<input
			bind:this={input}
			{id}
			type="text"
			role="combobox"
			aria-expanded={open}
			aria-controls={listId}
			aria-autocomplete="list"
			aria-activedescendant={open && shown[active] ? `${id}-opt-${shown[active].code}` : undefined}
			autocomplete="off"
			{placeholder}
			value={open ? query : (selected?.name ?? query)}
			oninput={(e) => {
				query = (e.currentTarget as HTMLInputElement).value;
				open = true;
				active = 0;
			}}
			onfocus={(e) => {
				open = true;
				active = 0;
				(e.currentTarget as HTMLInputElement).select();
			}}
			onblur={() => setTimeout(() => ((open = false), (query = '')), 150)}
			{onkeydown}
			class="h-full min-w-0 flex-1 bg-transparent text-base text-ink outline-none placeholder:text-ink-muted"
		/>
		{#if value}
			<button
				type="button"
				class="inline-flex size-11 flex-none cursor-pointer items-center justify-center rounded-full text-ink-muted hover:text-ink"
				aria-label={clearLabel}
				onclick={() => {
					value = '';
					input?.focus();
				}}
			>
				<XIcon class="size-4" aria-hidden="true" />
			</button>
		{/if}
	</div>
	{#if open}
		<ul
			id={listId}
			role="listbox"
			aria-label={listLabel}
			class="absolute inset-x-0 top-full z-30 mt-2 max-h-80 list-none overflow-y-auto rounded-xl border border-line bg-bg p-1 shadow-xl"
		>
			{#each shown as o, i (o.code)}
				{@const f = flagFor(o.name, o.code)}
				<!-- Keys go to the input (aria-activedescendant); the option takes the pointer. -->
				<!-- svelte-ignore a11y_click_events_have_key_events -->
				<li
					id={`${id}-opt-${o.code}`}
					role="option"
					aria-selected={o.code === value}
					class={`m-0 flex min-h-11 cursor-pointer items-center gap-2 rounded-lg px-3 ${i === active ? 'bg-surface' : ''}`}
					onmousedown={(e) => e.preventDefault()}
					onclick={() => choose(o.code)}
					onmouseenter={() => (active = i)}
				>
					{#if f}<Flag code={f} label="" class="size-5 flex-none" />{/if}
					<span class="truncate text-ink">{o.name}</span>
				</li>
			{:else}
				<li class="px-3 py-2 text-sm text-ink-muted">{noMatch(query)}</li>
			{/each}
		</ul>
	{/if}
</div>
