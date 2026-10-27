<script lang="ts">
	import { ADDRESS_PARTS, US_STATES, missingAddressParts } from './address.js';

	// Framework address_block editor: the five parts the
	// address_block rule checks (name/street/city/state/zip) plus an
	// optional second street line (display-only, never validated).
	// Binds the whole value object; per-part errors render inline
	// once `showErrors` is set (the route flips it on review).

	interface AddressValue {
		name?: string;
		street?: string;
		line2?: string;
		city?: string;
		state?: string;
		zip?: string;
	}

	interface Props {
		value: AddressValue;
		idPrefix: string;
		showErrors?: boolean;
		disabled?: boolean;
	}

	let {
		value = $bindable(),
		idPrefix,
		showErrors = false,
		disabled = false
	}: Props = $props();

	const missing = $derived(new Set(missingAddressParts(value)));
	const labels: Record<string, string> = {
		name: 'Full name',
		street: 'Street address',
		city: 'City',
		state: 'State',
		zip: 'ZIP code'
	};

	function fieldId(part: string): string {
		return `${idPrefix}-${part}`;
	}
</script>

<fieldset class="address" {disabled}>
	<div class="grid">
		<label class="span" for={fieldId('name')}>
			{labels.name}
			<input
				id={fieldId('name')}
				type="text"
				bind:value={value.name}
				autocomplete="name"
				aria-invalid={showErrors && missing.has('name')}
			/>
		</label>
		<label class="span" for={fieldId('street')}>
			{labels.street}
			<input
				id={fieldId('street')}
				type="text"
				bind:value={value.street}
				autocomplete="address-line1"
				aria-invalid={showErrors && missing.has('street')}
			/>
		</label>
		<label class="span" for={`${idPrefix}-line2`}>
			Apartment, suite, etc. (optional)
			<input
				id={`${idPrefix}-line2`}
				type="text"
				bind:value={value.line2}
				autocomplete="address-line2"
			/>
		</label>
		<label for={fieldId('city')}>
			{labels.city}
			<input
				id={fieldId('city')}
				type="text"
				bind:value={value.city}
				autocomplete="address-level2"
				aria-invalid={showErrors && missing.has('city')}
			/>
		</label>
		<label for={fieldId('state')}>
			{labels.state}
			<select
				id={fieldId('state')}
				bind:value={value.state}
				autocomplete="address-level1"
				aria-invalid={showErrors && missing.has('state')}
			>
				<option value="">Select…</option>
				{#each US_STATES as state (state.code)}
					<option value={state.code}>{state.name}</option>
				{/each}
			</select>
		</label>
		<label for={fieldId('zip')}>
			{labels.zip}
			<input
				id={fieldId('zip')}
				type="text"
				inputmode="numeric"
				bind:value={value.zip}
				autocomplete="postal-code"
				aria-invalid={showErrors && missing.has('zip')}
			/>
		</label>
	</div>
	{#if showErrors}
		<ul class="part-errors">
			{#each ADDRESS_PARTS.filter((part) => missing.has(part)) as part (part)}
				<li>{labels[part]} is missing</li>
			{/each}
		</ul>
	{/if}
</fieldset>

<style>
	.address {
		border: 0;
		margin: 0;
		padding: 0;
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(3, 1fr);
		gap: 0.75rem;
	}
	.span {
		grid-column: 1 / -1;
	}
	label {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		font-size: 0.875rem;
	}
	input,
	select {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.5rem 0.75rem;
		background: var(--surface);
		color: inherit;
		font: inherit;
	}
	input[aria-invalid='true'],
	select[aria-invalid='true'] {
		border-color: var(--error);
	}
	.part-errors {
		margin: 0.5rem 0 0;
		padding-left: 1.25rem;
		color: var(--error);
		font-size: 0.875rem;
	}
	@media (max-width: 40rem) {
		.grid {
			grid-template-columns: 1fr;
		}
	}
</style>
