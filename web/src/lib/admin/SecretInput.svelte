<script lang="ts">
	import { onMount } from 'svelte';

	// Write-only secret field for staff pages (API keys, tokens, setup
	// keys). Never a password-type field: browsers and password
	// managers treat that as a login and offer to save the value,
	// which a config secret must never be. Instead a text field masked
	// with the webkit text-security property, autofill off, and each
	// manager's own opt-out attribute. A Show/Hide toggle (aria-pressed) unmasks it;
	// where the browser cannot mask a text field the value shows as
	// typed and the hint says so.
	let {
		id,
		value = $bindable(''),
		placeholder = '',
		maxlength = 8192,
		required = false,
		class: className = '',
		hint = 'Write-only: stored encrypted on the server and never shown again. Your browser will not offer to save it.',
		oninput
	}: {
		id: string;
		value?: string;
		placeholder?: string;
		maxlength?: number;
		required?: boolean;
		class?: string;
		hint?: string;
		oninput?: (e: Event & { currentTarget: HTMLInputElement }) => void;
	} = $props();

	let shown = $state(false);
	let maskable = $state(true);
	const hintId = $derived(`${id}-hint`);

	onMount(() => {
		maskable =
			typeof CSS !== 'undefined' &&
			CSS.supports('-webkit-text-security', 'disc');
	});
</script>

<span class="flex items-center gap-2">
	<input
		{id}
		type="text"
		class={`min-w-0 flex-1 ${shown || !maskable ? '' : 'secret-masked'} ${className}`}
		bind:value
		{placeholder}
		{maxlength}
		{required}
		{oninput}
		autocomplete="off"
		autocapitalize="off"
		autocorrect="off"
		spellcheck={false}
		inputmode="text"
		aria-describedby={hintId}
		data-secret-input=""
		data-1p-ignore=""
		data-lpignore="true"
		data-bwignore="true"
		data-form-type="other"
		data-protonpass-ignore="true"
	/>
	{#if maskable}
		<button
			type="button"
			class="min-h-11 flex-none cursor-pointer rounded-xs border border-line bg-bg px-3 text-sm font-medium text-ink hover:border-focus"
			aria-controls={id}
			aria-pressed={shown}
			aria-label={shown ? 'Hide value' : 'Show value'}
			onclick={() => (shown = !shown)}
		>
			{shown ? 'Hide' : 'Show'}
		</button>
	{/if}
</span>
<span id={hintId} class="mt-1 block text-xs font-normal text-ink-muted">
	{hint}{#if !maskable}
		This browser shows it while you type.{/if}
</span>

<style>
	.secret-masked {
		-webkit-text-security: disc;
	}
</style>
