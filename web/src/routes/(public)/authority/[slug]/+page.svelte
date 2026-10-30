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

	// Full class strings per state (not stacked class: directives):
	// the active border/text colors must never co-occur with the
	// idle ones in the cascade.
	const tabCls = (isActive: boolean) =>
		`min-h-11 cursor-pointer border-0 border-b-[3px] bg-transparent px-[0.9rem] py-2 ${
			isActive ? 'border-accent-fill font-bold text-ink' : 'border-transparent text-ink-muted'
		}`;

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
	<header class="mb-4 border-b border-line pt-6 pb-4">
		<p class="m-0 text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">Authority dossier</p>
		<h1 class="mt-1 mb-2 font-sans text-3xl md:text-4xl">{slug}</h1>
	</header>

	{#if facts.length === 0}
		<Notice variant="info" heading="No published facts yet.">
			<p>
				This authority is in the index, but no accepted facts have been
				published. General information, not legal advice.
			</p>
		</Notice>
	{:else}
		<div class="mt-4 grid gap-8 pb-8 md:grid-cols-[3fr_2fr]">
			<div>
				<div class="flex flex-wrap gap-1 border-b border-line" role="tablist" aria-label="Dossier sections">
					{#each groups as g (g.topic)}
						<button
							role="tab"
							aria-selected={tab === g.topic}
							class={tabCls(tab === g.topic)}
							onclick={() => (tab = g.topic)}
						>
							{topicLabel(g.topic)}
						</button>
					{/each}
				</div>
				<div role="tabpanel" aria-label={active ? topicLabel(active.topic) : ''}>
					{#if active}
						<ul class="m-0 mt-4 list-none p-0">
							{#each active.facts as fact (fact.id)}
								<li class="border-b border-line py-3">
									<p class="m-0 mb-2">{fact.statement}</p>
									{#if fact.conflict}
										<Notice variant="warning" heading="Conflicting reports for this item.">
											{#each fact.conflicts ?? [] as sib (sib.id)}
												<p>{sib.statement} <span class="font-mono">[{sib.status}]</span></p>
											{/each}
										</Notice>
									{/if}
								</li>
							{/each}
						</ul>
					{/if}
				</div>
			</div>

			<aside class="flex flex-col gap-4" aria-label="About this dossier">
				<section class="rounded-md border border-line bg-surface p-4" aria-labelledby="about">
					<h2 id="about" class="m-0 mb-2 text-base">About this dossier</h2>
					<p>{factCount(facts.length)} across {groups.length} topic{groups.length === 1 ? '' : 's'}.</p>
					<p class="mb-0 text-[0.8rem] text-ink-muted">General information, not legal advice. No fee totals — the pricing table is still open.</p>
				</section>
				<p><a href={resolve('/(customer)/login')}>Report a change (login required)</a></p>
			</aside>
		</div>
	{/if}
{/if}
