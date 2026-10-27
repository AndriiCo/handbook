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
</script>

<svelte:head>
	<title>Mock new order — Apostille Handbook</title>
</svelte:head>

<h1>New order</h1>

<ol class="steps" aria-label="Order progress">
	{#each ['Destination', 'Document', 'Contact', 'Review'] as label, i (label)}
		<li class:current={step === i + 1} class:past={step > i + 1 || done}>
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
	<p><button class="secondary" onclick={() => (done = false)}>Back to review</button></p>
{:else if step === 1}
	<form
		onsubmit={(e) => {
			e.preventDefault();
			if (canNext1) step = 2;
		}}
	>
		<label for="authority">Destination authority</label>
		<select id="authority" bind:value={authority} required>
			<option value="">Choose an authority…</option>
			{#each AUTHORITIES as a (a.slug)}
				<option value={a.slug}>{a.name}</option>
			{/each}
		</select>
		<p><button type="submit" disabled={!canNext1}>Continue</button></p>
	</form>
{:else if step === 2}
	<form
		onsubmit={(e) => {
			e.preventDefault();
			if (canNext2) step = 3;
		}}
	>
		<fieldset>
			<legend>Document type — {authorityName}</legend>
			{#each ['Birth certificate', 'Marriage certificate', 'Corporate document', 'Other'] as t (t)}
				<label><input type="radio" name="doc_type" value={t} bind:group={docType} required /> {t}</label>
			{/each}
		</fieldset>
		<p class="row">
			<button type="button" class="secondary" onclick={() => (step = 1)}>Back</button>
			<button type="submit" disabled={!canNext2}>Continue</button>
		</p>
	</form>
{:else if step === 3}
	<form
		onsubmit={(e) => {
			e.preventDefault();
			if (canNext3) step = 4;
		}}
	>
		<label for="name">Full name</label>
		<input id="name" type="text" bind:value={name} required autocomplete="name" />
		<label for="email">Email</label>
		<input id="email" type="email" bind:value={email} required autocomplete="email" />
		<label for="street">Street address</label>
		<input id="street" type="text" bind:value={street} required autocomplete="street-address" />
		<div class="grid">
			<p>
				<label for="city">City</label>
				<input id="city" type="text" bind:value={city} autocomplete="address-level2" />
			</p>
			<p>
				<label for="postal">Postal code</label>
				<input id="postal" type="text" bind:value={postal} autocomplete="postal-code" />
			</p>
		</div>
		<p class="row">
			<button type="button" class="secondary" onclick={() => (step = 2)}>Back</button>
			<button type="submit" disabled={!canNext3}>Review order</button>
		</p>
	</form>
{:else}
	<dl class="review">
		<dt>Destination</dt>
		<dd>{authorityName}</dd>
		<dt>Document</dt>
		<dd>{docType}</dd>
		<dt>Name</dt>
		<dd>{name}</dd>
		<dt>Email</dt>
		<dd>{email}</dd>
		<dt>Address</dt>
		<dd>{street}{city ? `, ${city}` : ''}{postal ? ` ${postal}` : ''}</dd>
	</dl>
	<p class="row">
		<button class="secondary" onclick={() => (step = 3)}>Back</button>
		<button onclick={() => (done = true)}>Submit order (mock)</button>
	</p>
{/if}

<style>
	.steps {
		list-style: none;
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem 1rem;
		padding: 0;
		color: var(--text-muted);
	}
	.steps li.current {
		color: var(--text);
		font-weight: 700;
	}
	.steps li.past {
		color: var(--success);
	}
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
	.grid {
		display: grid;
		gap: 0 1rem;
		grid-template-columns: 1fr 1fr;
	}
	.grid p {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}
	.row {
		display: flex;
		gap: 0.75rem;
	}
	.review {
		display: grid;
		grid-template-columns: auto 1fr;
		gap: 0.4rem 1rem;
		max-width: 32rem;
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 1rem;
	}
	.review dt {
		font-weight: 700;
	}
	.review dd {
		margin: 0;
	}
	button {
		min-height: 44px;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
		padding: 0 1.25rem;
	}
	:root[data-theme='dark'] button:not(.secondary) {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	button.secondary {
		color: var(--text);
		background: var(--surface);
		border-color: var(--border);
	}
	button:disabled {
		opacity: 0.55;
		cursor: not-allowed;
	}
</style>
