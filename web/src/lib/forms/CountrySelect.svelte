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

<div class="combo">
	<input
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
		<ul class="options" role="listbox" id={countryListId}>
			{#each matches as match, i (match.code)}
				<li role="option" aria-selected={i === active} class:active={i === active}>
					<button
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

<style>
	.combo {
		position: relative;
	}
	input {
		width: 100%;
		box-sizing: border-box;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.5rem 0.75rem;
		background: var(--surface);
		color: inherit;
		font: inherit;
	}
	.options {
		position: absolute;
		z-index: 20;
		left: 0;
		right: 0;
		margin: 0.25rem 0 0;
		padding: 0.25rem;
		list-style: none;
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		max-height: 12rem;
		overflow-y: auto;
	}
	.options button {
		display: block;
		width: 100%;
		text-align: left;
		border: 0;
		background: transparent;
		color: inherit;
		font: inherit;
		padding: 0.375rem 0.5rem;
		border-radius: var(--radius-xs);
		cursor: pointer;
	}
	.options li.active button {
		background: var(--accent);
		color: var(--accent-ink);
	}
</style>
