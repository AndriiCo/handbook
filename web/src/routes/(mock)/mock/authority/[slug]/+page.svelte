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
	// Full class strings per state (not stacked class: directives):
	// the active border/text colors must never co-occur with the
	// idle ones in the cascade.
	const tabCls = (isActive: boolean) =>
		`min-h-11 cursor-pointer border-0 border-b-[3px] bg-transparent px-[0.9rem] py-2 ${
			isActive ? 'border-accent-fill font-bold text-ink' : 'border-transparent text-ink-muted'
		}`;
</script>

<svelte:head>
	<title>{authority ? authority.name : 'Unknown authority'} — mock dossier</title>
</svelte:head>

{#if !authority}
	<Notice variant="warning" heading="Unknown authority “{slug}”.">
		<p><a href={resolve('/(mock)/mock/search')}>Back to the mock index</a>.</p>
	</Notice>
{:else}
	<header class="mb-4 border-b border-line pt-6 pb-4">
		<p class="m-0 text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">{authority.kind}</p>
		<h1 class="mt-1 mb-2 font-sans text-3xl md:text-4xl">{authority.name}</h1>
		<VerifiedChip tier={authority.tier} verified={authority.verified} />
	</header>

	<div class="mt-4 grid gap-8 pb-8 md:grid-cols-[3fr_2fr]">
		<div>
			<div class="flex flex-wrap gap-1 border-b border-line" role="tablist" aria-label="Dossier sections">
				{#each tabs as t (t.id)}
					<button
						role="tab"
						aria-selected={tab === t.id}
						class={tabCls(tab === t.id)}
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
								<li>{form.name} — {form.revision} — <span class="font-mono">{form.hash}</span></li>
							{/each}
						</ul>
					{/if}
				{:else if tab === 'fees'}
					<ul>
						{#each authority.fees as fee (fee.label)}
							<li>{fee.label}: <span class="font-mono">{money(fee.amount)}</span></li>
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

		<aside class="flex flex-col gap-4" aria-label="Fee estimate and actions">
			<section class="rounded-md border border-line bg-surface p-4" aria-labelledby="receipt">
				<h2 class="m-0 mb-2 text-base" id="receipt">Fee estimate</h2>
				<label class="flex items-center justify-between gap-4 text-sm">
					Certified copies
					<input
						class="min-h-11 w-[4.5rem] rounded-xs border border-line bg-bg px-[0.6rem] py-[0.4rem] text-ink"
						type="number"
						min="1"
						max="20"
						bind:value={copies}
					/>
				</label>
				<table class="mt-3 w-full border-collapse text-sm">
					<tbody>
						{#each authority.fees as row (row.label)}
							<tr>
								<th class="px-0 py-[0.35rem] text-left font-normal" scope="row">{row.label}</th>
								<td class="px-0 py-[0.35rem] text-right font-mono">{money(row.amount)}</td>
							</tr>
						{/each}
						<tr>
							<th class="border-t border-line px-0 py-[0.35rem] text-left font-bold" scope="row">
								Estimated total
							</th>
							<td class="border-t border-line px-0 py-[0.35rem] text-right font-mono font-bold">
								{money(total)}
							</td>
						</tr>
					</tbody>
				</table>
				<p class="mb-0 text-[0.8rem] text-ink-muted">Fixture amounts for layout review — not real fees.</p>
			</section>
			<p><a href={resolve('/(mock)/mock/order')}>Start an order for this authority</a></p>
			<p><a href={resolve('/(mock)/mock/login')}>Report a change (login required)</a></p>
		</aside>
	</div>
{/if}

