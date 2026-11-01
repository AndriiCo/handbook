<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';

	// Orders list: the caller's form submissions, newest first.
	// Anonymous visitors get the sign-in prompt, never an error dump.
	type Submission = {
		id: number;
		authority: string;
		status: string;
		created_at: number;
		submitted_at: number | null;
	};

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	let rows = $state<Submission[] | null>(null);
	let names = $state<Record<string, string>>({});
	let loadError = $state<string | null>(null);
	let needsLogin = $state(false);

	$effect(() => {
		(async () => {
			try {
				const [subs, auths] = await Promise.all([
					fetch(api('/p2/submissions'), { credentials: 'same-origin' }),
					fetch(api('/authorities'), { credentials: 'same-origin' })
				]);
				if (subs.status === 401) {
					needsLogin = true;
					rows = [];
					return;
				}
				if (!subs.ok) {
					loadError = 'The orders list is unavailable right now. Try again in a bit.';
					rows = [];
					return;
				}
				const body = await subs.json().catch(() => ({}));
				rows = Array.isArray(body?.submissions) ? body.submissions : [];
				if (auths.ok) {
					const index = await auths.json().catch(() => ({}));
					if (Array.isArray(index?.authorities)) {
						for (const a of index.authorities) {
							if (a?.slug && a?.name) names[a.slug] = a.name;
						}
					}
				}
			} catch {
				loadError = 'The orders list is unavailable right now. Try again in a bit.';
				rows = [];
			}
		})();
	});

	const fmtDate = (unix: number) =>
		new Date(unix * 1000).toLocaleDateString(undefined, {
			year: 'numeric',
			month: 'short',
			day: 'numeric'
		});
</script>

<svelte:head>
	<title>Orders — Apostille Handbook</title>
</svelte:head>

<Heading level={1}>Orders</Heading>

{#if needsLogin}
	<Notice variant="info" heading="Sign in to see your orders.">
		<p>
			Order history lives in your account.
			<a href={resolve('/(customer)/login')}>Sign in</a>
			or
			<a href={resolve('/(customer)/order')}>start a new order</a>.
		</p>
	</Notice>
{:else if loadError}
	<Notice variant="error" heading="Orders are unavailable."><p>{loadError}</p></Notice>
{:else if rows === null}
	<div role="status" aria-label="Loading orders" class="mt-4 flex max-w-[32rem] flex-col gap-2">
		<span class="sr-only">Loading orders</span>
		<Skeleton class="h-11 w-full" />
		<Skeleton class="h-11 w-full" />
		<Skeleton class="h-11 w-2/3" />
	</div>
{:else if rows.length === 0}
	<Notice variant="info" heading="No orders yet.">
		<p>
			Submitted orders will appear here with live status.
			<a href={resolve('/(customer)/order')}>Start a new order</a>.
		</p>
	</Notice>
{:else}
	<p class="text-sm text-ink-muted">
		{rows.length}
		{rows.length === 1 ? 'order' : 'orders'}, newest first.
		<a href={resolve('/(customer)/order')}>Start a new order</a>.
	</p>
	<table class="mt-4 block w-full border-collapse md:table">
		<caption class="sr-only">Your orders, newest first</caption>
		<thead class="sr-only md:not-sr-only">
			<tr>
				<th class="border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">Order</th>
				<th class="border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
					Authority
				</th>
				<th class="border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">Status</th>
				<th class="border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">Created</th>
			</tr>
		</thead>
		<tbody class="block w-full md:table-row-group">
			{#each rows as row (row.id)}
				<tr class="mb-3 block rounded-sm border border-line even:bg-surface md:mb-0 md:table-row md:rounded-none md:border-0">
					<td
						data-label="Order"
						class="block border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left font-mono text-[0.9rem] last:border-b-0 md:table-cell md:border"
						>#{row.id}</td
					>
					<td
						data-label="Authority"
						class="block border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left last:border-b-0 md:table-cell md:border"
						>{names[row.authority] ?? row.authority}</td
					>
					<td
						data-label="Status"
						class="block border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left last:border-b-0 md:table-cell md:border"
						><span class="rounded-full border border-line px-2 py-0.5 text-sm capitalize">{row.status}</span></td
					>
					<td
						data-label="Created"
						class="block border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left last:border-b-0 md:table-cell md:border"
						>{fmtDate(row.created_at)}</td
					>
				</tr>
			{/each}
		</tbody>
	</table>
{/if}

<style>
	/* Mobile-only label injection has no clean utility (attr() +
	   string concat); the desktop kill inverts the old max-width
	   query mobile-first. */
	td::before {
		content: attr(data-label) ': ';
		font-weight: 700;
		font-family: var(--font-sans);
	}
	@media (min-width: 720px) {
		td::before {
			content: none;
		}
	}
</style>
