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

<fieldset class="m-0 border-0 p-0" {disabled}>
	<div class="grid grid-cols-3 gap-3 max-[40rem]:grid-cols-1">
		<label class="col-span-full flex flex-col gap-1 text-sm" for={fieldId('name')}>
			{labels.name}
			<input
				class="rounded-sm border border-line bg-surface px-3 py-2 text-inherit [font:inherit]"
				id={fieldId('name')}
				type="text"
				bind:value={value.name}
				autocomplete="name"
				aria-invalid={showErrors && missing.has('name')}
			/>
		</label>
		<label class="col-span-full flex flex-col gap-1 text-sm" for={fieldId('street')}>
			{labels.street}
			<input
				class="rounded-sm border border-line bg-surface px-3 py-2 text-inherit [font:inherit]"
				id={fieldId('street')}
				type="text"
				bind:value={value.street}
				autocomplete="address-line1"
				aria-invalid={showErrors && missing.has('street')}
			/>
		</label>
		<label class="col-span-full flex flex-col gap-1 text-sm" for={`${idPrefix}-line2`}>
			Apartment, suite, etc. (optional)
			<input
				class="rounded-sm border border-line bg-surface px-3 py-2 text-inherit [font:inherit]"
				id={`${idPrefix}-line2`}
				type="text"
				bind:value={value.line2}
				autocomplete="address-line2"
			/>
		</label>
		<label class="flex flex-col gap-1 text-sm" for={fieldId('city')}>
			{labels.city}
			<input
				class="rounded-sm border border-line bg-surface px-3 py-2 text-inherit [font:inherit]"
				id={fieldId('city')}
				type="text"
				bind:value={value.city}
				autocomplete="address-level2"
				aria-invalid={showErrors && missing.has('city')}
			/>
		</label>
		<label class="flex flex-col gap-1 text-sm" for={fieldId('state')}>
			{labels.state}
			<select
				class="rounded-sm border border-line bg-surface px-3 py-2 text-inherit [font:inherit]"
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
		<label class="flex flex-col gap-1 text-sm" for={fieldId('zip')}>
			{labels.zip}
			<input
				class="rounded-sm border border-line bg-surface px-3 py-2 text-inherit [font:inherit]"
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
		<ul class="m-0 mt-2 pl-5 text-sm text-error">
			{#each ADDRESS_PARTS.filter((part) => missing.has(part)) as part (part)}
				<li>{labels[part]} is missing</li>
			{/each}
		</ul>
	{/if}
</fieldset>

