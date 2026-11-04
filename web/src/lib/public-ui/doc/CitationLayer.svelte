<script lang="ts">
	// CitationLayer: one shared citation card for the whole page (R2).
	//
	// Props ($props):
	// - sources: NumberedSource[], i.e. `numberSources(items).sources`
	//   (the contract's Source plus the extras documented in cite.js).
	// - container: HTMLElement | null, the element holding the markers
	//   (the page's main column). Pointer, focus and click events are
	//   delegated there; null means the whole document.
	//
	// Behaviour (WCAG 1.4.13: dismissible, hoverable, persistent):
	// - one bits-ui Popover; customAnchor is the active marker; trapFocus
	//   is false and open auto-focus is prevented, so focus never jumps;
	// - a mouse hover opens it after 350 ms and closes it 400 ms after
	//   leaving; the card itself is hoverable;
	// - a click or tap opens it (and pins a hover-opened card); a second
	//   click on the same marker closes it;
	// - keyboard focus on a marker opens it; Enter follows the link to
	//   the endnote and focuses it, whose backlink returns focus;
	// - Esc or Close closes it and returns focus to the marker when focus
	//   was on the marker or in the card;
	// - pressing another marker while a card is open moves the card
	//   rather than closing it (the outside-click guard);
	// - a live update that renumbers the page keeps the card on the same
	//   item and source (found again by data-cite-item and data-cite-key),
	//   or closes it when that marker is gone.
	// Width min(20rem, 100vw - 2rem). Never a paid control in a card; only
	// https sources get a link.
	import XIcon from '@lucide/svelte/icons/x';
	import * as Popover from '../../components/ui/popover/index.js';
	import { citations } from '../citations.svelte.js';
	import {
		datesLine,
		describeSource,
		sourceCount,
		type CiteQuote,
		type NumberedSource
	} from '../cite.js';
	import { markQuote, sourceUrl } from '../guide.js';
	import { sourceLang } from './items.js';
	import { guide } from './lang.svelte.js';

	let {
		sources = [],
		container = null
	}: {
		sources?: NumberedSource[];
		container?: HTMLElement | null;
	} = $props();

	const OPEN_DELAY = 350;
	const CLOSE_DELAY = 400;
	const MARKER = 'a.cite[data-cite-n]';
	const uid = $props.id();
	const leadId = `${uid}-lead`;
	const copy = $derived(guide.t);
	const lang = $derived(guide.lang);

	type Mode = 'hover' | 'focus' | 'click';

	let open = $state(false);
	let anchor = $state<HTMLElement | null>(null);
	// The item and source the open card shows, so a re-render that
	// renumbers or reuses marker elements cannot swap its content.
	let shows: { item: string; key: string } | null = null;
	let card = $state<HTMLElement | null>(null);
	let mode: Mode | null = null;
	let restoreFocus = false;
	let suppressFocus: HTMLElement | null = null;
	let flagged: HTMLElement | null = null;
	let openTimer: ReturnType<typeof setTimeout> | undefined;
	let closeTimer: ReturnType<typeof setTimeout> | undefined;

	const byN = $derived(new Map(sources.map((s) => [s.n, s])));

	const active = $derived.by(() => {
		const m = anchor;
		if (!m) return null;
		const source = byN.get(Number(m.dataset.citeN));
		if (!source) return null;
		const letters = (m.dataset.citeLetters ?? '').split(/\s+/).filter(Boolean);
		const picked = source.quotes.filter((q) => letters.includes(q.letter));
		const quotes = picked.length ? picked : source.quotes.slice(0, 1);
		const info = describeSource(source, { lang });
		// Pages and PDFs show when this quote was captured and approved;
		// staff notes carry their own date in the lead line.
		const page = source.kind === 'web' || source.kind === 'pdf' || source.kind === 'upload';
		const one = quotes.length === 1 ? quotes[0] : null;
		return {
			source,
			info,
			quotes,
			dates: !page ? '' : one && (one.capturedAt || one.acceptedAt) ? datesLine(one, { lang }) : info.dates,
			href: linkFor(info, quotes[0] ?? null)
		};
	});

	$effect(() => {
		citations.count = sourceCount(sources);
	});

	function linkFor(info: ReturnType<typeof describeSource>, quote: CiteQuote | null): string | null {
		const url = info.href;
		if (!info.link || !url) return null;
		if (!info.fragment) return url;
		return sourceUrl(url, quote?.text ?? '') ?? url;
	}

	function markerOf(target: EventTarget | null): HTMLElement | null {
		if (!(target instanceof Element)) return null;
		const m = target.closest(MARKER);
		if (!(m instanceof HTMLElement)) return null;
		return !container || container.contains(m) ? m : null;
	}

	function focusVisible(el: Element): boolean {
		try {
			return el.matches(':focus-visible');
		} catch {
			return true;
		}
	}

	function clearTimers() {
		clearTimeout(openTimer);
		clearTimeout(closeTimer);
		openTimer = undefined;
		closeTimer = undefined;
	}

	function flag(m: HTMLElement | null) {
		if (flagged && flagged !== m) flagged.removeAttribute('data-cite-active');
		flagged = m;
		m?.setAttribute('data-cite-active', '');
	}

	function show(m: HTMLElement, how: Mode) {
		clearTimers();
		anchor = m;
		shows = { item: m.dataset.citeItem ?? '', key: m.dataset.citeKey ?? '' };
		mode = how;
		flag(m);
		open = true;
	}

	// After any close: drop timers and the active flag; give focus back
	// to the marker when the reader was on it or inside the card.
	function settle(restore: boolean) {
		clearTimers();
		mode = null;
		flag(null);
		const a = anchor;
		if (restore && a?.isConnected && document.activeElement !== a) {
			suppressFocus = a;
			a.focus({ preventScroll: true });
		}
	}

	function hide(restore = false) {
		open = false;
		settle(restore);
	}

	function scheduleClose() {
		clearTimeout(closeTimer);
		closeTimer = setTimeout(() => {
			if (mode === 'hover') hide();
		}, CLOSE_DELAY);
	}

	// bits-ui closed it (Esc or a press outside).
	function onOpenChange(next: boolean) {
		if (next) return;
		settle(restoreFocus);
		restoreFocus = false;
	}

	function onEscape() {
		const a = document.activeElement;
		restoreFocus = !!anchor && (a === anchor || (!!card && !!a && card.contains(a)));
	}

	// A press on another marker is not "outside": the click moves the card.
	function guardOutside(e: PointerEvent) {
		if (markerOf(e.target)) e.preventDefault();
	}

	function onPointerOver(e: PointerEvent) {
		if (e.pointerType !== 'mouse') return;
		const m = markerOf(e.target);
		if (!m) return;
		clearTimeout(closeTimer);
		if (open && anchor === m) return;
		if (open && mode === 'click') return;
		clearTimeout(openTimer);
		if (open) {
			show(m, 'hover');
			return;
		}
		openTimer = setTimeout(() => show(m, 'hover'), OPEN_DELAY);
	}

	function onPointerOut(e: PointerEvent) {
		if (e.pointerType !== 'mouse') return;
		const m = markerOf(e.target);
		if (!m) return;
		if (e.relatedTarget instanceof Node && m.contains(e.relatedTarget)) return;
		clearTimeout(openTimer);
		if (open && mode === 'hover') scheduleClose();
	}

	function onCardEnter(e: PointerEvent) {
		if (e.pointerType === 'mouse') clearTimeout(closeTimer);
	}

	function onCardLeave(e: PointerEvent) {
		if (e.pointerType === 'mouse' && mode === 'hover') scheduleClose();
	}

	function onCardFocusOut(e: FocusEvent) {
		if (mode !== 'focus') return;
		const next = e.relatedTarget;
		if (next instanceof Node && (card?.contains(next) || next === anchor)) return;
		hide();
	}

	function onFocusIn(e: FocusEvent) {
		const m = markerOf(e.target);
		if (!m) return;
		if (suppressFocus === m) {
			suppressFocus = null;
			return;
		}
		suppressFocus = null;
		if (!focusVisible(m)) return;
		if (open && anchor === m) return;
		show(m, 'focus');
	}

	function onFocusOut(e: FocusEvent) {
		if (mode !== 'focus' || !markerOf(e.target)) return;
		const next = e.relatedTarget;
		if (next instanceof Node && card?.contains(next)) return;
		if (markerOf(next)) return;
		hide();
	}

	function onClick(e: MouseEvent) {
		const m = markerOf(e.target);
		if (!m || e.defaultPrevented) return;
		if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
		const n = Number(m.dataset.citeN);
		if (e.detail === 0) {
			// Keyboard: follow the link to the endnote, then focus it.
			const note = document.getElementById(`src-${n}`);
			if (note) {
				hide();
				citations.rememberReturn(n, m.id);
				requestAnimationFrame(() => note.focus({ preventScroll: true }));
				return;
			}
		}
		e.preventDefault();
		if (open && anchor === m) {
			if (mode === 'click') hide();
			else {
				clearTimers();
				mode = 'click';
			}
			return;
		}
		show(m, 'click');
	}

	$effect(() => {
		if (!citations.on) return;
		const root: Document | HTMLElement = container ?? document;
		const over = (e: Event) => onPointerOver(e as PointerEvent);
		const out = (e: Event) => onPointerOut(e as PointerEvent);
		const focusIn = (e: Event) => onFocusIn(e as FocusEvent);
		const focusOut = (e: Event) => onFocusOut(e as FocusEvent);
		const click = (e: Event) => onClick(e as MouseEvent);
		root.addEventListener('pointerover', over);
		root.addEventListener('pointerout', out);
		root.addEventListener('focusin', focusIn);
		root.addEventListener('focusout', focusOut);
		// Capture: runs before the router's link handling.
		root.addEventListener('click', click, true);
		return () => {
			root.removeEventListener('pointerover', over);
			root.removeEventListener('pointerout', out);
			root.removeEventListener('focusin', focusIn);
			root.removeEventListener('focusout', focusOut);
			root.removeEventListener('click', click, true);
			clearTimers();
			flag(null);
			open = false;
			mode = null;
			anchor = null;
		};
	});

	// The marker for this item and source, wherever a re-render put it
	// (its number and id may have changed).
	function markerFor(item: string, key: string): HTMLElement | null {
		if (!item || !key) return null;
		for (const el of (container ?? document).querySelectorAll(MARKER)) {
			if (el instanceof HTMLElement && el.dataset.citeItem === item && el.dataset.citeKey === key) {
				return el;
			}
		}
		return null;
	}

	// A live update re-renders and may renumber the markers: keep the
	// card on the same item and source, or close it when that is gone.
	// Never follow the old id: after a renumbering it names another
	// value's marker.
	$effect(() => {
		void sources;
		const a = anchor;
		const was = shows;
		if (!a || !was) return;
		if (a.isConnected && a.dataset.citeItem === was.item && a.dataset.citeKey === was.key) return;
		const again = markerFor(was.item, was.key);
		if (again) {
			anchor = again;
			if (open) flag(again);
		} else if (open) {
			hide();
		}
	});
</script>

{#if citations.on}
	<Popover.Root bind:open {onOpenChange}>
		<Popover.Content
			bind:ref={card}
			customAnchor={anchor}
			trapFocus={false}
			onOpenAutoFocus={(e: Event) => e.preventDefault()}
			onCloseAutoFocus={(e: Event) => e.preventDefault()}
			onInteractOutside={guardOutside}
			onEscapeKeydown={onEscape}
			onpointerenter={onCardEnter}
			onpointerleave={onCardLeave}
			onfocusout={onCardFocusOut}
			side="bottom"
			align="start"
			sideOffset={6}
			collisionPadding={16}
			role="dialog"
			aria-labelledby={leadId}
			data-cite-card=""
			class="gap-3 p-4 text-sm"
			style="width: min(20rem, calc(100vw - 2rem));"
		>
			{#if active}
				<p id={leadId} class="m-0 text-xs font-semibold break-words text-ink-muted">
					{active.info.lead}
				</p>
				{#if active.info.title}
					<p class="m-0 font-medium break-words text-ink">{active.info.title}</p>
				{/if}
				{#each active.quotes as q (q.letter)}
					{#if q.text}
						<blockquote
							class="cite-quote font-quote m-0 border-l-2 border-verified pl-3 break-words text-ink italic"
							lang={sourceLang(q.text, lang)}
						>
							{@html markQuote(q.text, q.value)}
						</blockquote>
					{:else if q.inputs}
						<p class="m-0 font-semibold break-words text-ink">
							{#if q.terms}
								{#each q.terms as t, i (i)}{i > 0 ? ' ' : ''}{t.text}{#each t.refs as r (`${r.n}${r.letter}`)}{' '}<a
											href={`#src-${r.n}`}
											class="font-mono text-xs font-normal text-accent-strong underline underline-offset-2"
											onclick={() => hide()}>[{r.n}{r.letter}]</a
										>{/each}{/each}
							{:else}
								{q.summary ?? q.value}
							{/if}
						</p>
						{#if q.inputs.length > 0}
							<ul class="m-0 flex list-none flex-col gap-1 p-0">
								{#each q.inputs as input, i (i)}
									<li class="m-0 p-0 break-words text-ink">
										{input.label}{input.label && input.display ? ': ' : ''}{input.display}
										{#each input.refs as r (`${r.n}${r.letter}`)}
											<a
												href={`#src-${r.n}`}
												class="font-mono text-xs text-accent-strong underline underline-offset-2"
												onclick={() => hide()}>[{r.n}{r.letter}]</a
											>{' '}
										{/each}
									</li>
								{/each}
							</ul>
						{/if}
					{:else if q.summary}
						<p class="m-0 break-words text-ink" lang={sourceLang(q.summary, lang)}>{q.summary}</p>
					{/if}
				{/each}
				{#if active.dates}
					<p class="m-0 font-mono text-xs text-ink-muted">{active.dates}</p>
				{/if}
				{#if active.info.check}
					<p class="m-0 text-ink-muted">{active.info.check}</p>
				{/if}
				{#if active.source.basedOn.length > 0}
					<p class="m-0 text-ink-muted">
						{copy.endnotes.basedOn} {#each active.source.basedOn as n, i (n)}{i > 0 ? ', ' : ''}<a
								href={`#src-${n}`}
								class="font-mono text-accent-strong underline underline-offset-2"
								onclick={() => hide()}>[{n}]</a
							>{/each}
					</p>
				{/if}
				<div class="flex flex-wrap items-center justify-between gap-2">
					{#if active.href}
						<a
							href={active.href}
							target="_blank"
							rel="noopener noreferrer"
							class="inline-flex min-h-6 items-center gap-1 font-medium text-accent-strong underline underline-offset-4"
						>
							{active.info.link}<span aria-hidden="true">↗</span><span class="sr-only">
								{copy.common.newTab}</span
							>
						</a>
					{/if}
					<button
						type="button"
						class="ml-auto inline-flex min-h-8 cursor-pointer items-center gap-1 rounded-sm px-2 text-ink-muted transition-colors hover:bg-surface hover:text-ink"
						onclick={() => hide(true)}
					>
						<XIcon class="size-4" aria-hidden="true" />
						{copy.card.close}
					</button>
				</div>
			{/if}
		</Popover.Content>
	</Popover.Root>
{/if}

<style>
	/* markQuote() wraps the cited value; the quote keeps its own breaks. */
	.cite-quote :global(mark) {
		background: var(--changed-bg);
		border-radius: 2px;
		color: inherit;
		padding-inline: 1px;
	}
</style>
