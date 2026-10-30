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
	class="w-full rounded-sm border border-line bg-surface px-3 py-2 text-inherit"
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

