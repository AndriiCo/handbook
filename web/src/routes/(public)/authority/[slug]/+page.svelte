<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import VerifiedChip from '../../../../lib/public-ui/VerifiedChip.svelte';
	import { findAuthority } from '../../../../lib/public-ui/authorities.js';

	// Dossier layout per site-theme §1.2. EN-only; placeholder content
	// until accepted facts land. No prices render (pricing table open).
	const slug = $derived(page.params.slug ?? '');
	const authority = $derived(findAuthority(slug));

	type Tab = 'requirements' | 'forms' | 'fees' | 'visit' | 'mail';
	const tabs: { id: Tab; label: string }[] = [
		{ id: 'requirements', label: 'Requirements' },
		{ id: 'forms', label: 'Forms' },
		{ id: 'fees', label: 'Fees' },
		{ id: 'visit', label: 'Visit' },
		{ id: 'mail', label: 'Mail' }
	];
	let tab = $state<Tab>('requirements');

	// Receipt-style fee tally (pure Svelte, no dep). Placeholder rows.
	let copies = $state(1);
	const feeRows = [
		{ label: 'Authority fee (cited passthrough)', amount: 0 },
		{ label: 'Per certified copy', amount: 0 }
	];
	const total = $derived(
		feeRows[0].amount + feeRows[1].amount * Math.max(1, copies)
	);
</script>

<svelte:head>
	<title>
		{authority ? authority.name : 'Unknown authority'} — Apostille Handbook
	</title>
</svelte:head>

{#if !authority}
	<Notice variant="warning" heading="Unknown authority “{slug}”.">
		<p>
			This slug is not in the placeholder index.
			<a href={resolve('/(public)')}>Back to the authority index</a>.
		</p>
	</Notice>
{:else}
	<header class="dossier-head">
		<p class="kicker">{authority.kind}</p>
		<h1>{authority.name}</h1>
		<VerifiedChip tier={authority.tier} verified={authority.verified} />
	</header>

	<Notice variant="info" heading="Placeholder dossier.">
		<p>
			Accepted facts have not been published yet, so every section below is
			empty. General information, not legal advice.
		</p>
	</Notice>

	<div class="dossier">
		<div class="main">
			<div class="tabs" role="tablist" aria-label="Dossier sections">
				{#each tabs as t (t.id)}
					<button
						role="tab"
						aria-selected={tab === t.id}
						class:active={tab === t.id}
						onclick={() => (tab = t.id)}
					>
						{t.label}
					</button>
				{/each}
			</div>
			<div role="tabpanel" aria-label={tabs.find((t) => t.id === tab)?.label}>
				{#if tab === 'requirements'}
					<p>No published requirements yet. Check back after the first handbook build.</p>
				{:else if tab === 'forms'}
					<p>No published forms yet. Form downloads will list filename, revision, and hash.</p>
				{:else if tab === 'fees'}
					<p>
						No fee schedule yet. Fees will show cited passthrough and service
						fee separately — never a bare total.
					</p>
				{:else if tab === 'visit'}
					<p>No walk-in intel yet. Address, hours, and parking tips will appear here.</p>
				{:else}
					<p>No mailing instructions yet. Mail-to addresses will carry citations.</p>
				{/if}
			</div>
		</div>

		<aside class="rail" aria-label="Fee estimate and actions">
			<section class="receipt" aria-labelledby="receipt">
				<h2 id="receipt">Fee estimate</h2>
				<label>
					Certified copies
					<input type="number" min="1" max="20" bind:value={copies} />
				</label>
				<table>
					<tbody>
						{#each feeRows as row (row.label)}
							<tr>
								<th scope="row">{row.label}</th>
								<td class="mono">USD $0.00</td>
							</tr>
						{/each}
						<tr class="total">
							<th scope="row">Estimated total</th>
							<td class="mono">USD ${total.toFixed(2)}</td>
						</tr>
					</tbody>
				</table>
				<p class="fine">Placeholder amounts — no prices render until the pricing table lands.</p>
			</section>
			<p><a href={resolve('/(customer)/login')}>Report a change (login required)</a></p>
		</aside>
	</div>
{/if}

<style>
	.dossier-head {
		padding: 1.5rem 0 1rem;
		border-bottom: 1px solid var(--border);
		margin-bottom: 1rem;
	}
	.kicker {
		margin: 0;
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-muted);
	}
	h1 {
		font-size: 1.875rem;
		margin: 0.25rem 0 0.5rem;
	}
	.dossier {
		display: grid;
		gap: 2rem;
		margin-top: 1rem;
		padding-bottom: 2rem;
	}
	.tabs {
		display: flex;
		flex-wrap: wrap;
		gap: 0.25rem;
		border-bottom: 1px solid var(--border);
	}
	.tabs button {
		min-height: 44px;
		padding: 0.5rem 0.9rem;
		font: inherit;
		background: none;
		border: none;
		border-bottom: 3px solid transparent;
		color: var(--text-muted);
		cursor: pointer;
	}
	.tabs button.active {
		color: var(--text);
		border-bottom-color: var(--accent);
		font-weight: 700;
	}
	.rail {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}
	.receipt {
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-md);
		padding: 1rem;
	}
	.receipt h2 {
		font-size: 1rem;
		margin: 0 0 0.5rem;
	}
	.receipt label {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
		font-size: 0.875rem;
	}
	.receipt input {
		width: 4.5rem;
		min-height: 44px;
		font: inherit;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
		padding: 0.4rem 0.6rem;
	}
	.receipt table {
		width: 100%;
		border-collapse: collapse;
		margin-top: 0.75rem;
		font-size: 0.875rem;
	}
	.receipt th {
		text-align: left;
		font-weight: 400;
		padding: 0.35rem 0;
	}
	.receipt td {
		text-align: right;
		padding: 0.35rem 0;
	}
	.receipt .total th,
	.receipt .total td {
		font-weight: 700;
		border-top: 1px solid var(--border);
	}
	.mono {
		font-family: var(--font-mono);
	}
	.fine {
		font-size: 0.8rem;
		color: var(--text-muted);
		margin-bottom: 0;
	}
	@media (min-width: 720px) {
		h1 {
			font-size: 2.25rem;
		}
		.dossier {
			grid-template-columns: 3fr 2fr;
		}
	}
</style>
