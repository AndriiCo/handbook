<script lang="ts">
	import { STRINGS, resolveLocale, type Locale } from '../../../lib/i18n';

	let locale = $state<Locale>(resolveLocale());
	let submitted = $state(false);

	const t = $derived(STRINGS[locale]);
</script>

<svelte:head>
	<title>{t.orderLabel} — Apostille Handbook</title>
</svelte:head>

<h1>{t.orderLabel}</h1>

{#if submitted}
	<p class="notice" role="status">Order submit is not wired yet — nothing was sent. Your input above is preserved.</p>
{/if}

<form
	onsubmit={(e) => {
		e.preventDefault();
		submitted = true;
	}}
>
	<label for="authority">Destination authority</label>
	<select id="authority" name="authority" required>
		<option value="">Choose an authority…</option>
		<option value="placeholder-1">Placeholder Authority One</option>
		<option value="placeholder-2">Placeholder Authority Two</option>
		<option value="placeholder-3">Placeholder Authority Three</option>
	</select>

	<fieldset>
		<legend>Document type</legend>
		<label><input type="radio" name="doc_type" value="birth" required /> Birth certificate</label>
		<label><input type="radio" name="doc_type" value="marriage" required /> Marriage certificate</label>
		<label><input type="radio" name="doc_type" value="corporate" required /> Corporate document</label>
		<label><input type="radio" name="doc_type" value="other" required /> Other</label>
	</fieldset>

	<label for="name">Full name</label>
	<input id="name" name="name" type="text" required autocomplete="name" />

	<label for="email">Email</label>
	<input id="email" name="email" type="email" required autocomplete="email" />

	<label for="street">Street address</label>
	<input id="street" name="street" type="text" required autocomplete="street-address" />

	<label for="city">City</label>
	<input id="city" name="city" type="text" required autocomplete="address-level2" />

	<label for="postal">Postal code</label>
	<input id="postal" name="postal" type="text" required autocomplete="postal-code" />

	<button type="submit">Submit order</button>
</form>

<noscript>
	<p class="notice">Submitting an order needs JavaScript for now; field validation above still works without it.</p>
</noscript>

<style>
	form {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		max-width: 32rem;
	}
	form > label {
		font-weight: 700;
		margin-top: 0.5rem;
	}
	input[type='text'],
	input[type='email'],
	select {
		min-height: 44px;
		padding: 0.6rem 0.8rem;
		font: inherit;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
	}
	fieldset {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		margin: 0.5rem 0 0;
		padding: 0.75rem 1rem;
	}
	legend {
		font-weight: 700;
		padding: 0 0.25rem;
	}
	fieldset label {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		min-height: 44px;
	}
	input[type='radio'] {
		width: 1.25rem;
		height: 1.25rem;
		accent-color: var(--accent);
	}
	button {
		min-height: 44px;
		margin-top: 0.75rem;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
	}
	:root[data-theme='dark'] button {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	.notice {
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
	}
</style>
