<script lang="ts">
	import { searchCountries } from './countries.js';

	// Framework country autocomplete: filters the static ISO list as
	// the user types. Binds the full country name; free text stays
	// allowed and validation decides (never blocks typing).

	interface Props {
		value: string;
		id?: string;
		name?: string;
		placeholder?: string;
		required?: boolean;
		disabled?: boolean;
	}

	let {
		value = $bindable(''),
		id,
		name,
		placeholder = 'Start typing a country…',
		required = false,
		disabled = false
	}: Props = $props();

	let open = $state(false);
	let active = $state(0);
	const countryListId = `country-options-${Math.random().toString(36).slice(2)}`;
	const matches = $derived(searchCountries(value, 8));

	function choose(name: string) {
		value = name;
		open = false;
	}

	// Full class strings per state: the active option colors must
	// never co-occur with the idle ones in the cascade.
	const optionCls = (isActive: boolean) =>
		`block w-full cursor-pointer rounded-xs border-0 px-2 py-[0.375rem] text-left [font:inherit] ${
			isActive ? 'bg-accent text-accent-ink' : 'bg-transparent text-inherit'
		}`;

	function onKeyDown(event: KeyboardEvent) {
		if (event.key === 'ArrowDown' && matches.length > 0) {
			event.preventDefault();
			open = true;
			active = (active + 1) % matches.length;
		} else if (event.key === 'ArrowUp' && matches.length > 0) {
			event.preventDefault();
			open = true;
			active = (active + matches.length - 1) % matches.length;
		} else if (event.key === 'Enter' && open && matches.length > 0) {
			event.preventDefault();
			choose(matches[Math.min(active, matches.length - 1)].name);
		} else if (event.key === 'Escape') {
			open = false;
		}
	}
</script>

<div class="relative">
	<input
		class="w-full rounded-sm border border-line bg-surface px-3 py-2 text-inherit [font:inherit]"
		{id}
		{name}
		type="text"
		bind:value
		{placeholder}
		{required}
		{disabled}
		role="combobox"
		aria-expanded={open}
		aria-autocomplete="list"
		aria-controls={countryListId}
		autocomplete="off"
		onfocus={() => {
			open = true;
			active = 0;
		}}
		oninput={() => {
			open = true;
			active = 0;
		}}
		onkeydown={onKeyDown}
		onblur={() => {
			// Let a pointer click on an option land before closing.
			setTimeout(() => (open = false), 120);
		}}
	/>
	{#if open && matches.length > 0}
		<ul
			class="absolute right-0 left-0 z-20 m-0 mt-1 max-h-48 list-none overflow-y-auto rounded-sm border border-line bg-surface p-1"
			role="listbox"
			id={countryListId}
		>
			{#each matches as match, i (match.code)}
				<li role="option" aria-selected={i === active}>
					<button
						class={optionCls(i === active)}
						type="button"
						tabindex="-1"
						onmousedown={(event) => event.preventDefault()}
						onclick={() => choose(match.name)}
						onmouseenter={() => (active = i)}
					>
						{match.name}
					</button>
				</li>
			{/each}
		</ul>
	{/if}
</div>
