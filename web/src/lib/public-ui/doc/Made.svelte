<script lang="ts">
	import type { Component } from 'svelte';
	import { changePanels, domain } from '../guide.js';
	import { topicLabel } from '../handbook.js';
	import { CHROME, translateHref } from '../translate.js';
	import type { Provenance } from '../plan.js';

	type Rec = { id: number | null; topic: string; statement: string; acceptedAt: number | null };
	type Translation = { statement: string; model: string };

	// "How this page was made" (plan §3 row 12, screen only): one line on
	// how the values got here, then a details element, closed by default,
	// "The records behind this page (n)". Inside, the verbatim records as
	// "#id · statement" lines grouped under plain h3 topic names (no
	// regions), the machine-translated records for `?lang=es|fr`, the
	// plan's provenance panel and the office pages' "What changed" lists.
	// Nothing inside loads until the reader opens it: the provenance
	// panel and the translation chip are lazy chunks, and `/changes` is
	// read on the first open only (a 404 or an empty body shows nothing).
	//
	// Props:
	// - records: `page.records`.
	// - models: model names that drafted the records ("Muse Spark 1.3").
	// - pages: the pages behind the values, as `pagesPhrase(header.trust)`
	//   words them ("8 official pages", "2 web pages"); '' for none.
	// - provenance, planVersion, planAcceptedAt: the accepted /plan's.
	// - slug, apiBase: where to read `/changes` (apiBase as `apiRoot`).
	// - changes: a `/changes` body the page already read (null: none);
	//   undefined reads it on the first open.
	// - lang: 'es' | 'fr' | null; translations: by record id.
	// - pathname: the page path, for the language links.
	let {
		records = [],
		models = [],
		pages = '',
		provenance = null,
		planVersion = 0,
		planAcceptedAt = null,
		slug,
		apiBase = '',
		changes: known = undefined,
		lang = null,
		translations = {},
		pathname = ''
	}: {
		records?: Rec[];
		models?: string[];
		pages?: string;
		provenance?: Provenance | null;
		planVersion?: number;
		planAcceptedAt?: number | null;
		slug: string;
		apiBase?: string;
		changes?: unknown;
		lang?: 'es' | 'fr' | null;
		translations?: { [id: number]: Translation };
		pathname?: string;
	} = $props();

	let open = $state(false);
	let opened = $state(false);
	let fetched = $state<ReturnType<typeof changePanels>>([]);
	const changes = $derived(known !== undefined ? (known ? changePanels(known) : []) : fetched);
	let Chip = $state<Component<any> | null>(null);
	let Panel = $state<Component<any> | null>(null);

	const n = $derived(records.length);
	const count = (k: number, one: string, many: string) => `${k} ${k === 1 ? one : many}`;
	const groups = $derived.by(() => {
		const out: { topic: string; rows: Rec[] }[] = [];
		for (const r of records) {
			let g = out.find((x) => x.topic === r.topic);
			if (!g) {
				g = { topic: r.topic, rows: [] };
				out.push(g);
			}
			g.rows.push(r);
		}
		return out;
	});
	const drafted = $derived(models.length > 0 ? models.join(' and ') : '');

	function onToggle(e: Event) {
		open = (e.currentTarget as HTMLDetailsElement).open;
		if (!open || opened) return;
		opened = true;
		const current = slug;
		if (current && known === undefined) {
			fetch(`${apiBase}/api/v1/authorities/${encodeURIComponent(current)}/changes`, { credentials: 'same-origin' })
				.then((res) => (res.ok ? res.json().catch(() => null) : null))
				.then((body) => {
					if (current === slug) fetched = body ? changePanels(body) : [];
				})
				.catch(() => {
					fetched = [];
				});
		}
		if (lang) {
			import('../TranslationChip.svelte').then((m) => (Chip = m.default as Component<any>)).catch(() => {});
		}
		if (provenance) {
			import('../plan/ProvenancePanel.svelte').then((m) => (Panel = m.default as Component<any>)).catch(() => {});
		}
	}

	const LANGS = ['es', 'fr'] as const;
</script>

<section id="made" aria-labelledby="made-h" class="border-t border-line py-10 xl:py-16">
	<div class="max-w-3xl">
		<p class="text-eyebrow m-0 font-semibold tracking-[0.06em] text-ink-muted uppercase">Drafted by a model, approved by a person</p>
		<h2 id="made-h" class="text-h2 mt-2 mb-0 font-brand font-medium text-ink">How this page was made</h2>
		<p class="mt-3 mb-0 max-w-prose text-ink-muted">
			{count(n, 'record', 'records')}{#if drafted}, drafted by {drafted}{/if}{#if pages}
				&nbsp;from {pages}{/if}, each approved by a staff reviewer before it
			reached this page.
		</p>
		<details class="mt-6 rounded-md border border-line" ontoggle={onToggle}>
			<summary class="flex min-h-11 cursor-pointer items-center px-4 font-medium text-accent-strong">
				The records behind this page ({n})
			</summary>
			{#if open}
				<div class="border-t border-line px-4 pt-4 pb-6">
					<p class="m-0 flex flex-wrap items-center gap-2 text-sm">
						<span class="text-ink-muted">{CHROME[lang ?? 'en']?.readIn ?? 'Read in'}:</span>
						<a
							href={translateHref(pathname, null)}
							aria-current={!lang ? 'true' : undefined}
							class={`inline-flex min-h-8 items-center rounded-full px-3 no-underline ${!lang ? 'bg-accent-fill font-medium text-accent-ink' : 'text-ink-muted hover:bg-surface hover:text-ink'}`}
							>English</a
						>
						{#each LANGS as code (code)}
							<a
								href={translateHref(pathname, code)}
								aria-current={lang === code ? 'true' : undefined}
								class={`inline-flex min-h-8 items-center rounded-full px-3 no-underline ${lang === code ? 'bg-accent-fill font-medium text-accent-ink' : 'text-ink-muted hover:bg-surface hover:text-ink'}`}
								>{CHROME[code].langName}</a
							>
						{/each}
					</p>
					{#each groups as group (group.topic)}
						<div class="mt-6">
							<h3 class="m-0 text-sm font-semibold text-ink">{topicLabel(group.topic)}</h3>
							<ul class="m-0 mt-2 flex list-none flex-col gap-2 p-0">
								{#each group.rows as r, i (`${r.id ?? 'row'}-${i}`)}
									{@const own = lang && typeof r.id === 'number' ? translations[r.id] : null}
									<li class="m-0 p-0 font-mono text-xs break-words text-ink-muted [overflow-wrap:anywhere]">
										{#if own && Chip && lang}
											<Chip {lang} translated={own.statement} original={r.statement} model={own.model} />
										{:else}
											{#if r.id !== null}#{r.id} ·{/if}
											{r.statement}
										{/if}
									</li>
								{/each}
							</ul>
						</div>
					{/each}
					{#if Panel && provenance}
						<div class="mt-8">
							<Panel {provenance} version={planVersion} acceptedAt={planAcceptedAt} />
						</div>
					{/if}
					{#each changes as change (change.sourceUrl)}
						<div class="mt-8">
							<h3 class="m-0 text-sm font-semibold text-ink">What changed on {change.date}</h3>
							<p class="m-0 mt-1 font-mono text-xs text-ink-muted">
								{domain(change.sourceUrl)}{#if change.since}&nbsp;· since {change.since}{/if}
							</p>
							<ul class="m-0 mt-2 list-none p-0">
								{#each change.added as line, i (`a-${i}`)}
									<li class="m-0 border-l-2 border-verified py-1 pl-3 text-sm break-words text-ink">
										<span class="sr-only">Added: </span><span aria-hidden="true" class="font-mono">+&nbsp;</span>{line}
									</li>
								{/each}
								{#each change.removed as line, i (`r-${i}`)}
									<li class="m-0 border-l-2 border-changed py-1 pl-3 text-sm break-words text-ink-muted line-through">
										<span class="sr-only">Removed: </span><span aria-hidden="true" class="font-mono">−&nbsp;</span>{line}
									</li>
								{/each}
							</ul>
						</div>
					{/each}
				</div>
			{/if}
		</details>
	</div>
</section>
