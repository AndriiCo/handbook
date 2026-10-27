<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../../../lib/public-ui/Notice.svelte';
	import VerifiedChip from '../../../../../lib/public-ui/VerifiedChip.svelte';
	import { findMock, money } from '../../../../../lib/mock/fixtures';

	const slug = $derived(page.params.slug ?? '');
	const authority = $derived(findMock(slug));

	type Tab = 'requirements' | 'forms' | 'fees' | 'visit' | 'mail';
	const tabs: { id: Tab; label: string }[] = [
		{ id: 'requirements', label: 'Requirements' },
		{ id: 'forms', label: 'Forms' },
		{ id: 'fees', label: 'Fees' },
		{ id: 'visit', label: 'Visit' },
		{ id: 'mail', label: 'Mail' }
	];
	let tab = $state<Tab>('requirements');
	let copies = $state(1);

	const total = $derived.by(() => {
		if (!authority) return 0;
		const base = authority.fees[0]?.amount ?? 0;
		const perCopy = authority.fees[1]?.amount ?? 0;
		return base + perCopy * Math.max(1, copies);
	});
</script>

<svelte:head>
	<title>{authority ? authority.name : 'Unknown authority'} — mock dossier</title>
</svelte:head>

{#if !authority}
	<Notice variant="warning" heading="Unknown authority “{slug}”.">
		<p><a href={resolve('/(mock)/mock/search')}>Back to the mock index</a>.</p>
	</Notice>
{:else}
	<header class="dossier-head">
		<p class="kicker">{authority.kind}</p>
		<h1>{authority.name}</h1>
		<VerifiedChip tier={authority.tier} verified={authority.verified} />
	</header>

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
					<ul>
						{#each authority.requirements as req (req)}<li>{req}</li>{/each}
					</ul>
				{:else if tab === 'forms'}
					{#if authority.forms.length === 0}
						<p>No published forms yet for this office.</p>
					{:else}
						<ul>
							{#each authority.forms as form (form.name)}
								<li>{form.name} — {form.revision} — <span class="mono">{form.hash}</span></li>
							{/each}
						</ul>
					{/if}
				{:else if tab === 'fees'}
					<ul>
						{#each authority.fees as fee (fee.label)}
							<li>{fee.label}: <span class="mono">{money(fee.amount)}</span></li>
						{/each}
					</ul>
				{:else if tab === 'visit'}
					{#if authority.visit.length === 0}
						<p>No walk-in intel yet.</p>
					{:else}
						<ul>
							{#each authority.visit as line (line)}<li>{line}</li>{/each}
						</ul>
					{/if}
				{:else if authority.mail.length === 0}
					<p>No mailing instructions yet.</p>
				{:else}
					<ul>
						{#each authority.mail as line (line)}<li>{line}</li>{/each}
					</ul>
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
						{#each authority.fees as row (row.label)}
							<tr>
								<th scope="row">{row.label}</th>
								<td class="mono">{money(row.amount)}</td>
							</tr>
						{/each}
						<tr class="total">
							<th scope="row">Estimated total</th>
							<td class="mono">{money(total)}</td>
						</tr>
					</tbody>
				</table>
				<p class="fine">Fixture amounts for layout review — not real fees.</p>
			</section>
			<p><a href={resolve('/(mock)/mock/order')}>Start an order for this authority</a></p>
			<p><a href={resolve('/(mock)/mock/login')}>Report a change (login required)</a></p>
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
