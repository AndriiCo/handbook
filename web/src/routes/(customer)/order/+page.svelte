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
	<p class="rounded-sm border border-line bg-surface px-4 py-3" role="status">
		Order submit is not wired yet — nothing was sent. Your input above is preserved.
	</p>
{/if}

<form
	class="flex max-w-[32rem] flex-col gap-2"
	onsubmit={(e) => {
		e.preventDefault();
		submitted = true;
	}}
>
	<label class="mt-2 font-bold" for="authority">Destination authority</label>
	<select
		class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
		id="authority"
		name="authority"
		required
	>
		<option value="">Choose an authority…</option>
		<option value="placeholder-1">Placeholder Authority One</option>
		<option value="placeholder-2">Placeholder Authority Two</option>
		<option value="placeholder-3">Placeholder Authority Three</option>
	</select>

	<fieldset class="mx-0 mt-2 mb-0 rounded-sm border border-line px-4 py-3">
		<legend class="px-1 font-bold">Document type</legend>
		<label class="flex min-h-11 items-center gap-[0.6rem]"
			><input class="h-5 w-5 accent-accent" type="radio" name="doc_type" value="birth" required /> Birth
			certificate</label
		>
		<label class="flex min-h-11 items-center gap-[0.6rem]"
			><input class="h-5 w-5 accent-accent" type="radio" name="doc_type" value="marriage" required />
			Marriage certificate</label
		>
		<label class="flex min-h-11 items-center gap-[0.6rem]"
			><input class="h-5 w-5 accent-accent" type="radio" name="doc_type" value="corporate" required />
			Corporate document</label
		>
		<label class="flex min-h-11 items-center gap-[0.6rem]"
			><input class="h-5 w-5 accent-accent" type="radio" name="doc_type" value="other" required /> Other</label
		>
	</fieldset>

	<label class="mt-2 font-bold" for="name">Full name</label>
	<input
		class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
		id="name"
		name="name"
		type="text"
		required
		autocomplete="name"
	/>

	<label class="mt-2 font-bold" for="email">Email</label>
	<input
		class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
		id="email"
		name="email"
		type="email"
		required
		autocomplete="email"
	/>

	<label class="mt-2 font-bold" for="street">Street address</label>
	<input
		class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
		id="street"
		name="street"
		type="text"
		required
		autocomplete="street-address"
	/>

	<label class="mt-2 font-bold" for="city">City</label>
	<input
		class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
		id="city"
		name="city"
		type="text"
		required
		autocomplete="address-level2"
	/>

	<label class="mt-2 font-bold" for="postal">Postal code</label>
	<input
		class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
		id="postal"
		name="postal"
		type="text"
		required
		autocomplete="postal-code"
	/>

	<button
		class="mt-3 min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill font-bold text-accent-ink"
		type="submit">Submit order</button
	>
</form>

<noscript>
	<p class="rounded-sm border border-line bg-surface px-4 py-3">
		Submitting an order needs JavaScript for now; field validation above still works without it.
	</p>
</noscript>
