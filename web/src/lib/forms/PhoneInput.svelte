<script lang="ts">
	import { formatPhone } from './phone.js';

	// Framework phone input: plain text field that formats NANP on
	// blur. Binds raw text; never blocks typing or rejects input —
	// the `tel` rule owns validity.

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
		placeholder = '(555) 123-4567',
		required = false,
		disabled = false
	}: Props = $props();
</script>

<input
	{id}
	{name}
	type="tel"
	bind:value
	{placeholder}
	{required}
	{disabled}
	autocomplete="tel"
	onblur={() => {
		value = formatPhone(value);
	}}
/>

<style>
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
</style>
