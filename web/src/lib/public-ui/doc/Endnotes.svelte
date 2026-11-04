<script lang="ts">
	// Endnotes: the numbered sources (R2 citations mode).
	//
	// Props ($props):
	// - sources: NumberedSource[], i.e. `numberSources(items).sources`.
	//
	// Renders only while citations are on and a source exists, so off
	// prints clean. A top-level band: place it directly in <main>, after
	// the guide and before "How this page was made".
	//   <section id="sources" role="doc-endnotes" aria-labelledby="sources-h">
	//     <h2 id="sources-h">Sources</h2>
	//     <ol><li id="src-N"> "Title — host. https://… Captured … · approved …"
	// URLs print without their `#:~:text=` fragment; only https ones are
	// links (an http address is plain text, anything else is dropped).
	// Dates always carry the year: a printed sheet outlives this year.
	// Each quote is a <blockquote> under its letter, the value's label and
	// a backlink "↩" (role="doc-backlink", href="#refId"); the backlink
	// returns focus to the marker the reader came from, opening any
	// closed <details> around it first. Staff notes (calls, emails,
	// decisions) show their summary as a paragraph, never as a quote; a
	// calculation shows its terms with their [n letter] (or its display)
	// and lists its inputs.
	import { citations } from '../citations.svelte.js';
	import { describeSource, sourceCount, type CiteQuote, type NumberedSource } from '../cite.js';
	import { markQuote } from '../guide.js';
	import { sourceLang } from './items.js';
	import { guide } from './lang.svelte.js';

	let { sources = [] }: { sources?: NumberedSource[] } = $props();

	const shown = $derived(citations.on && sources.length > 0);
	const copy = $derived(guide.t.endnotes);
	const lang = $derived(guide.lang);
	const paper = $derived({ year: 'always' as const, lang });

	$effect(() => {
		citations.count = sourceCount(sources);
	});

	function marker(id: string | null): HTMLElement | null {
		if (!id) return null;
		const el = document.getElementById(id);
		return el instanceof HTMLElement && el.matches('a.cite') ? el : null;
	}

	function rendered(el: HTMLElement): boolean {
		if (typeof el.checkVisibility === 'function') return el.checkVisibility();
		return el.getClientRects().length > 0;
	}

	// Open every closed <details> around the marker (a marker in a closed
	// one is not rendered, so focus() would do nothing); true when it can
	// then take focus.
	function reveal(el: HTMLElement): boolean {
		for (let d = el.closest('details'); d; d = d.parentElement?.closest('details') ?? null) {
			if (!d.open) d.open = true;
		}
		return rendered(el);
	}

	// Back to the marker the reader followed here when it shows this
	// quote, else the quote's first marker on screen, else its first
	// marker at all. When none can take focus (e.g. inside a hidden
	// panel), the link's own navigation runs instead.
	function back(e: MouseEvent, n: number, q: CiteQuote) {
		const came = marker(citations.returnFor(n));
		const cameHere = came && (came.dataset.citeLetters ?? '').split(/\s+/).includes(q.letter);
		const all = [q.refId, ...q.uses.map((u) => u.refId)]
			.map(marker)
			.filter((el): el is HTMLElement => el !== null);
		const target = (cameHere ? came : null) ?? all.find(rendered) ?? all[0] ?? null;
		if (!target || !reveal(target)) return;
		e.preventDefault();
		target.scrollIntoView({ block: 'center' });
		target.focus({ preventScroll: true });
	}
</script>

{#if shown}
	<section
		id="sources"
		role="doc-endnotes"
		aria-labelledby="sources-h"
		class="border-t border-line py-10 xl:py-16"
	>
		<div class="max-w-3xl">
			<h2 id="sources-h" class="text-h2 m-0 font-semibold">{copy.heading}</h2>
			<p class="mt-2 mb-0 max-w-prose text-sm text-ink-muted">{copy.intro}</p>
			<ol
				class="m-0 mt-6 flex list-decimal flex-col gap-6 pl-8 marker:font-mono marker:text-sm marker:text-ink-muted"
			>
				{#each sources as s (s.key)}
					{@const info = describeSource(s, paper)}
					<li id={`src-${s.n}`} value={s.n} tabindex="-1" class="source m-0 scroll-mt-20 pl-1 text-sm">
						<p class="m-0 text-ink">
							<span class="font-medium">{info.head}</span>
							{#if info.href}
								<a
									href={info.href}
									target="_blank"
									rel="noopener noreferrer"
									class="text-accent-strong underline underline-offset-2">{info.href}</a
								>.
							{:else if info.url}
								<span class="font-mono text-xs">{info.url}</span>.
							{/if}
							{#if info.dates}
								<span class="text-ink-muted">{info.dates}.</span>
							{/if}
						</p>
						{#each s.quotes as q (q.letter)}
							<div class="mt-3">
								<p class="m-0 flex flex-wrap items-center gap-x-2 text-xs text-ink-muted">
									<span class="font-mono font-semibold text-ink">{q.letter}</span>
									{#if q.label}<span>{q.label}</span>{/if}
									<a
										href={`#${q.refId}`}
										role="doc-backlink"
										aria-label={q.label ? copy.backTo(q.label) : copy.backToCitation}
										class="inline-flex min-h-6 min-w-6 items-center justify-center rounded-xs text-accent-strong no-underline hover:bg-surface"
										onclick={(e) => back(e, s.n, q)}>↩</a
									>
								</p>
								{#if q.text}
									<blockquote
										class="cite-quote font-quote m-0 mt-1 border-l-2 border-verified pl-3 text-ink italic"
										lang={sourceLang(q.text, lang)}
									>
										{@html markQuote(q.text, q.value)}
									</blockquote>
								{:else if q.inputs}
									<p class="m-0 mt-1 font-semibold text-ink">
										{#if q.terms}
											{#each q.terms as t, i (i)}{i > 0 ? ' ' : ''}{t.text}{#each t.refs as r (`${r.n}${r.letter}`)}{' '}<a
														href={`#src-${r.n}`}
														class="font-mono text-xs font-normal text-accent-strong underline underline-offset-2"
														>[{r.n}{r.letter}]</a
													>{/each}{/each}
										{:else}
											{q.summary ?? q.value}
										{/if}
									</p>
									{#if q.inputs.length > 0}
										<ul class="m-0 mt-1 flex list-none flex-col gap-1 p-0">
											{#each q.inputs as input, i (i)}
												<li class="m-0 p-0 text-ink">
													{input.label}{input.label && input.display ? ': ' : ''}{input.display}
													{#each input.refs as r (`${r.n}${r.letter}`)}
														<a
															href={`#src-${r.n}`}
															class="font-mono text-xs text-accent-strong underline underline-offset-2"
															>[{r.n}{r.letter}]</a
														>{' '}
													{/each}
												</li>
											{/each}
										</ul>
									{/if}
								{:else if q.summary}
									<p class="m-0 mt-1 text-ink" lang={sourceLang(q.summary, lang)}>{q.summary}</p>
								{/if}
							</div>
						{/each}
						{#if s.basedOn.length > 0}
							<p class="m-0 mt-2 text-ink-muted">
								{copy.basedOn} {#each s.basedOn as n, i (n)}{i > 0 ? ', ' : ''}<a
										href={`#src-${n}`}
										class="font-mono text-accent-strong underline underline-offset-2">[{n}]</a
									>{/each}
							</p>
						{/if}
						{#if info.check}
							<p class="m-0 mt-2 text-ink-muted">{info.check}</p>
						{/if}
					</li>
				{/each}
			</ol>
		</div>
	</section>
{/if}

<style>
	.source {
		overflow-wrap: anywhere;
	}

	.cite-quote :global(mark) {
		background: var(--changed-bg);
		border-radius: 2px;
		color: inherit;
		padding-inline: 1px;
	}
</style>
