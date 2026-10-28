<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { AUTHORITIES } from '../../../../lib/mock/fixtures';

	// Stepped forms-fill flow: destination → document → contact → review.
	// All state is local; submit only flips the confirmation panel.
	let step = $state(1);
	let authority = $state('');
	let docType = $state('');
	let name = $state('');
	let email = $state('');
	let street = $state('');
	let city = $state('');
	let postal = $state('');
	let done = $state(false);

	const authorityName = $derived(AUTHORITIES.find((a) => a.slug === authority)?.name ?? authority);
	const canNext1 = $derived(authority !== '');
	const canNext2 = $derived(docType !== '');
	const canNext3 = $derived(name.trim() !== '' && /.+@.+\..+/.test(email.trim()) && street.trim() !== '');

	// Full class strings per state. Both flags can hold at once (the
	// current step after done): past wins the COLOR (source order in
	// the old CSS) while current keeps its weight.
	const stepCls = (isCurrent: boolean, isPast: boolean) =>
		isPast && isCurrent
			? 'font-bold text-success'
			: isPast
				? 'text-success'
				: isCurrent
					? 'font-bold text-ink'
					: '';
</script>

<svelte:head>
	<title>Mock new order — Apostille Handbook</title>
</svelte:head>

<h1>New order</h1>

<ol class="flex list-none flex-wrap gap-x-4 gap-y-2 p-0 text-ink-muted" aria-label="Order progress">
	{#each ['Destination', 'Document', 'Contact', 'Review'] as label, i (label)}
		<li class={stepCls(step === i + 1, step > i + 1 || done)}>
			{i + 1}. {label}
		</li>
	{/each}
</ol>

{#if done}
	<Notice variant="success" heading="Order ORD-1043 drafted (mock).">
		<p>
			{authorityName} · {docType} · {name}. Nothing was sent — this confirms the flow shape.
			<a href={resolve('/(mock)/mock/uploads')}>Continue to uploads</a>
		</p>
	</Notice>
	<p>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
			onclick={() => (done = false)}>Back to review</button
		>
	</p>
{:else if step === 1}
	<form
		class="flex max-w-[32rem] flex-col gap-2"
		onsubmit={(e) => {
			e.preventDefault();
			if (canNext1) step = 2;
		}}
	>
		<label class="mt-2 font-bold" for="authority">Destination authority</label>
		<select
			class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink [font:inherit]"
			id="authority"
			bind:value={authority}
			required
		>
			<option value="">Choose an authority…</option>
			{#each AUTHORITIES as a (a.slug)}
				<option value={a.slug}>{a.name}</option>
			{/each}
		</select>
		<p>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
				type="submit"
				disabled={!canNext1}>Continue</button
			>
		</p>
	</form>
{:else if step === 2}
	<form
		class="flex max-w-[32rem] flex-col gap-2"
		onsubmit={(e) => {
			e.preventDefault();
			if (canNext2) step = 3;
		}}
	>
		<fieldset class="mx-0 mt-2 mb-0 rounded-sm border border-line px-4 py-3">
			<legend class="px-1 font-bold">Document type — {authorityName}</legend>
			{#each ['Birth certificate', 'Marriage certificate', 'Corporate document', 'Other'] as t (t)}
				<label class="flex min-h-11 items-center gap-[0.6rem]"
					><input
						class="h-5 w-5 accent-accent"
						type="radio"
						name="doc_type"
						value={t}
						bind:group={docType}
						required
					/>
					{t}</label
				>
			{/each}
		</fieldset>
		<p class="flex gap-3">
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
				type="button"
				onclick={() => (step = 1)}>Back</button
			>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
				type="submit"
				disabled={!canNext2}>Continue</button
			>
		</p>
	</form>
{:else if step === 3}
	<form
		class="flex max-w-[32rem] flex-col gap-2"
		onsubmit={(e) => {
			e.preventDefault();
			if (canNext3) step = 4;
		}}
	>
		<label class="mt-2 font-bold" for="name">Full name</label>
		<input
			class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink [font:inherit]"
			id="name"
			type="text"
			bind:value={name}
			required
			autocomplete="name"
		/>
		<label class="mt-2 font-bold" for="email">Email</label>
		<input
			class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink [font:inherit]"
			id="email"
			type="email"
			bind:value={email}
			required
			autocomplete="email"
		/>
		<label class="mt-2 font-bold" for="street">Street address</label>
		<input
			class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink [font:inherit]"
			id="street"
			type="text"
			bind:value={street}
			required
			autocomplete="street-address"
		/>
		<div class="grid grid-cols-2 gap-x-4 gap-y-0">
			<p class="flex flex-col gap-2">
				<label for="city">City</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink [font:inherit]"
					id="city"
					type="text"
					bind:value={city}
					autocomplete="address-level2"
				/>
			</p>
			<p class="flex flex-col gap-2">
				<label for="postal">Postal code</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink [font:inherit]"
					id="postal"
					type="text"
					bind:value={postal}
					autocomplete="postal-code"
				/>
			</p>
		</div>
		<p class="flex gap-3">
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
				type="button"
				onclick={() => (step = 2)}>Back</button
			>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
				type="submit"
				disabled={!canNext3}>Review order</button
			>
		</p>
	</form>
{:else}
	<dl class="grid max-w-[32rem] grid-cols-[auto_1fr] gap-x-4 gap-y-[0.4rem] rounded-sm border border-line bg-surface p-4">
		<dt class="font-bold">Destination</dt>
		<dd class="m-0">{authorityName}</dd>
		<dt class="font-bold">Document</dt>
		<dd class="m-0">{docType}</dd>
		<dt class="font-bold">Name</dt>
		<dd class="m-0">{name}</dd>
		<dt class="font-bold">Email</dt>
		<dd class="m-0">{email}</dd>
		<dt class="font-bold">Address</dt>
		<dd class="m-0">{street}{city ? `, ${city}` : ''}{postal ? ` ${postal}` : ''}</dd>
	</dl>
	<p class="flex gap-3">
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
			onclick={() => (step = 3)}>Back</button
		>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
			onclick={() => (done = true)}>Submit order (mock)</button
		>
	</p>
{/if}

