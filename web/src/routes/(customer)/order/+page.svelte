<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import { listAuthorities, publicError } from '../../../lib/public-ui/handbook.js';
	import { STRINGS, resolveLocale, type Locale } from '../../../lib/i18n';

	let locale = $state<Locale>(resolveLocale());

	// Stepped order draft: destination → document → contact → review.
	// Promoted from the mock flow; all state is local until a submit
	// endpoint exists, and the confirmation says so honestly.
	let step = $state(1);
	let authority = $state('');
	let docType = $state('');
	let name = $state('');
	let email = $state('');
	let street = $state('');
	let city = $state('');
	let postal = $state('');
	let done = $state(false);

	let index = $state<{ slug: string; name: string; country: string }[] | null>(null);
	let loadError = $state<string | null>(null);

	const t = $derived(STRINGS[locale]);
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	$effect(() => {
		listAuthorities({ get })
			.then((res) => {
				if (!res.ok) {
					loadError = publicError(res.status, 'index');
					return;
				}
				const rows = res.body?.authorities;
				index = Array.isArray(rows) ? rows : [];
			})
			.catch(() => {
				loadError = publicError(500, 'index');
			});
	});

	const authorityName = $derived(
		index?.find((a) => a.slug === authority)?.name ?? authority
	);
	const canNext1 = $derived(authority !== '');
	const canNext2 = $derived(docType !== '');
	const canNext3 = $derived(
		name.trim() !== '' && /.+@.+\..+/.test(email.trim()) && street.trim() !== ''
	);

		const badgeCls = (isCurrent: boolean, isPast: boolean) =>
		'flex size-7 items-center justify-center rounded-full border text-sm font-bold ' +
		(isCurrent
			? 'border-accent-fill bg-accent-fill text-accent-ink'
			: isPast
				? 'border-success text-success'
				: 'border-line text-ink-muted');
	const stepLabelCls = (isCurrent: boolean, isPast: boolean) =>
		isCurrent ? 'font-bold text-ink' : isPast ? 'text-ink' : 'text-ink-muted';
</script>

<svelte:head>
	<title>{t.orderLabel} — Apostille Handbook</title>
</svelte:head>

<h1>{t.orderLabel}</h1>

{#if loadError}
	<Notice variant="error" heading="The order form is unavailable."><p>{loadError}</p></Notice>
{:else if index === null}
	<div role="status" aria-label="Loading order form" class="flex max-w-[32rem] flex-col gap-2">
		<span class="sr-only">Loading order form</span>
		<Skeleton class="h-11 w-full" />
		<Skeleton class="h-11 w-full" />
		<Skeleton class="h-11 w-2/3" />
	</div>
{:else if index.length === 0}
	<Notice variant="info" heading="No destinations yet.">
		<p>
			The handbook has no published authorities, so there is nothing to order yet.
			<a href={resolve('/(public)')}>Back to the authority index</a>.
		</p>
	</Notice>
{:else}
	<ol class="flex list-none flex-wrap items-center gap-y-2 p-0" aria-label="Order progress">
		{#each ['Destination', 'Document', 'Contact', 'Review'] as label, i (label)}
			{@const n = i + 1}
			{@const isCurrent = step === n && !done}
			{@const isPast = step > n || done}
			<li class="flex items-center" aria-current={isCurrent ? 'step' : undefined}>
				<span aria-hidden="true" class={badgeCls(isCurrent, isPast)}>{n}</span>
				<span class={'ml-2 ' + stepLabelCls(isCurrent, isPast)}>{label}</span>
				{#if n < 4}
					<span aria-hidden="true" class="mx-2 h-px w-6 bg-line sm:w-10"></span>
				{/if}
			</li>
		{/each}
	</ol>

	{#if done}
		<Notice variant="success" heading="Order drafted.">
			<p>
				{authorityName} · {docType} · {name}. Nothing was sent — sign-in opens
				order submission.
				<a href={resolve('/(customer)/login')}>Continue to {t.loginLabel}</a>
			</p>
		</Notice>
		<p>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink disabled:cursor-not-allowed disabled:opacity-55"
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
			<select class="field" id="authority" bind:value={authority} required>
				<option value="">Choose an authority…</option>
				{#each index as a (a.slug)}
					<option value={a.slug}>{a.name}</option>
				{/each}
			</select>
			<p>
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
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
					<label class="flex min-h-11 items-center gap-2"
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
					class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink disabled:cursor-not-allowed disabled:opacity-55"
					type="button"
					onclick={() => (step = 1)}>Back</button
				>
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
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
			<input class="field" id="name" type="text" bind:value={name} required autocomplete="name" />
			<label class="mt-2 font-bold" for="email">Email</label>
			<input
				class="field"
				id="email"
				type="email"
				bind:value={email}
				required
				autocomplete="email"
			/>
			<label class="mt-2 font-bold" for="street">Street address</label>
			<input
				class="field"
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
						class="field"
						id="city"
						type="text"
						bind:value={city}
						autocomplete="address-level2"
					/>
				</p>
				<p class="flex flex-col gap-2">
					<label for="postal">Postal code</label>
					<input
						class="field"
						id="postal"
						type="text"
						bind:value={postal}
						autocomplete="postal-code"
					/>
				</p>
			</div>
			<p class="flex gap-3">
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink disabled:cursor-not-allowed disabled:opacity-55"
					type="button"
					onclick={() => (step = 2)}>Back</button
				>
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
					type="submit"
					disabled={!canNext3}>Review order</button
				>
			</p>
		</form>
	{:else}
		<dl
			class="grid max-w-[32rem] grid-cols-[auto_1fr] gap-x-4 gap-y-2 rounded-sm border border-line bg-surface p-4"
		>
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
				class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink disabled:cursor-not-allowed disabled:opacity-55"
				onclick={() => (step = 3)}>Back</button
			>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
				onclick={() => (done = true)}>Draft order</button
			>
		</p>
	{/if}
{/if}

<noscript>
	<p class="rounded-sm border border-line bg-surface px-4 py-3">
		Placing an order needs JavaScript for now; field validation above still works without it.
	</p>
</noscript>
