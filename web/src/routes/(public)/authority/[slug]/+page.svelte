<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import {
		factCount,
		groupFactsByTopic,
		publicError,
		readDossier,
		topicLabel
	} from '../../../../lib/public-ui/handbook.js';

	// Slice A: live dossier over GET /api/v1/authorities/{slug}/facts
	// (accepted-only + conflict banners). Same-origin, no auth.
	// No prices render (pricing table open): the rail shows counts
	// only, never a fee total.
	const slug = $derived(page.params.slug ?? '');

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	let facts = $state<any[] | null>(null);
	let loadError = $state<string | null>(null);
	let notFound = $state(false);
	let tab = $state('');

	const groups = $derived(groupFactsByTopic(facts ?? []));
	const active = $derived(groups.find((g) => g.topic === tab) ?? groups[0] ?? null);

	$effect(() => {
		const current = slug;
		facts = null;
		loadError = null;
		notFound = false;
		tab = '';
		readDossier(current, { get })
			.then((res) => {
				if (current !== slug) return; // stale slug, ignore
				if (!res.ok) {
					if (res.status === 404) notFound = true;
					else loadError = publicError(res.status, 'dossier');
					return;
				}
				const rows = res.body?.facts;
				const list = Array.isArray(rows) ? rows : [];
				facts = list;
				tab = groupFactsByTopic(list)[0]?.topic ?? '';
			})
			.catch(() => {
				if (current !== slug) return;
				loadError = publicError(500, 'dossier');
			});
	});
</script>

<svelte:head>
	<title>{slug} — Apostille Handbook</title>
</svelte:head>

{#if notFound}
	<Notice variant="warning" heading="Unknown authority “{slug}”.">
		<p>
			This slug is not in the handbook index.
			<a href={resolve('/(public)')}>Back to the authority index</a>.
		</p>
	</Notice>
{:else if loadError}
	<Notice variant="error" heading="This dossier is unavailable."><p>{loadError}</p></Notice>
{:else if facts === null}
	<p role="status">Loading dossier for “{slug}”…</p>
{:else}
	<header class="dossier-head">
		<p class="kicker">Authority dossier</p>
		<h1>{slug}</h1>
	</header>

	{#if facts.length === 0}
		<Notice variant="info" heading="No published facts yet.">
			<p>
				This authority is in the index, but no accepted facts have been
				published. General information, not legal advice.
			</p>
		</Notice>
	{:else}
		<div class="dossier">
			<div class="main">
				<div class="tabs" role="tablist" aria-label="Dossier sections">
					{#each groups as g (g.topic)}
						<button
							role="tab"
							aria-selected={tab === g.topic}
							class:active={tab === g.topic}
							onclick={() => (tab = g.topic)}
						>
							{topicLabel(g.topic)}
						</button>
					{/each}
				</div>
				<div role="tabpanel" aria-label={active ? topicLabel(active.topic) : ''}>
					{#if active}
						<ul class="facts">
							{#each active.facts as fact (fact.id)}
								<li>
									<p>{fact.statement}</p>
									{#if fact.conflict}
										<Notice variant="warning" heading="Conflicting reports for this item.">
											{#each fact.conflicts ?? [] as sib (sib.id)}
												<p>{sib.statement} <span class="mono">[{sib.status}]</span></p>
											{/each}
										</Notice>
									{/if}
								</li>
							{/each}
						</ul>
					{/if}
				</div>
			</div>

			<aside class="rail" aria-label="About this dossier">
				<section class="receipt" aria-labelledby="about">
					<h2 id="about">About this dossier</h2>
					<p>{factCount(facts.length)} across {groups.length} topic{groups.length === 1 ? '' : 's'}.</p>
					<p class="fine">General information, not legal advice. No fee totals — the pricing table is still open.</p>
				</section>
				<p><a href={resolve('/(customer)/login')}>Report a change (login required)</a></p>
			</aside>
		</div>
	{/if}
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
	.facts {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0;
	}
	.facts > li {
		padding: 0.75rem 0;
		border-bottom: 1px solid var(--border);
	}
	.facts p {
		margin: 0 0 0.5rem;
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
