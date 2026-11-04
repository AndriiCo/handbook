<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import Notice from '#lib/public-ui/Notice.svelte';
	import { canonicalKeys, splitKey } from '#lib/public-ui/slots.js';
	import AuthorityPicker from './AuthorityPicker.svelte';
	import { ApiError, errorText, getJson, postJson } from './admin-api.js';
	import { keyNote } from './review-keys.js';
	import {
		KEY,
		MAX_MATCHES,
		MAX_QUOTE_CHARS,
		MIN_QUOTE_CHARS,
		chipKey,
		findAll,
		lineBreakMap,
		nextMatch,
		parseCaptureText,
		parseCaptures,
		parsePrepared,
		proposeOutcome,
		quoteBody,
		quoteProblem,
		quoteTopics,
		selectionQuote,
		sizeLabel,
		splitStatement,
		statementOf,
		tidy,
		uriPath,
		valueInQuote,
		type CaptureRow,
		type CaptureText,
		type Outcome,
		type PreparedQuote,
		type PreparedRow
	} from './quote-capture.js';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import ChevronUpIcon from '@lucide/svelte/icons/chevron-up';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

	// "Quote a page" tab of /review: staff pick a captured official page
	// of an authority, select the exact words in its text and propose
	// them as a fact. The quote goes with its UTF-8 byte offsets into
	// the capture's text sidecar, so the fact carries a real citation;
	// it lands as proposed and publishes only when accepted on the
	// Facts tab. "Prepared quotes" sends quotes written elsewhere (JSON
	// Lines): a row with `start`/`end` sends those byte offsets, one
	// without them lets the server find the quote in its capture.
	// `initialSlug` / `initialCapture` come from `?slug=&capture=`.
	let { initialSlug = '', initialCapture = null }: { initialSlug?: string; initialCapture?: number | null } = $props();

	const LIST_LIMIT = 100;
	const TOPIC_CHOICES = quoteTopics();
	const EXAMPLE = '{"capture_id":130,"quote":"…","topic":"hours","statement":"weekday__tumwater: …"}';

	let slug = $state(untrack(() => initialSlug));
	let captures = $state<CaptureRow[]>([]);
	let listState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let listError = $state<string | null>(null);

	let captureId = $state<number | null>(untrack(() => initialCapture));
	let doc = $state.raw<CaptureText | null>(null);
	let docState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let docError = $state<string | null>(null);
	let textEl = $state<HTMLTextAreaElement | null>(null);
	// The textarea's selection, in its own (line-break normalized) indices.
	let sel = $state({ start: 0, end: 0 });
	let needle = $state('');

	let topic = $state('');
	let key = $state('');
	let value = $state('');
	// The value tracks the selection until staff edit it.
	let valueFollows = $state(true);
	let proposing = $state(false);
	let result = $state<Outcome | null>(null);

	let preparedOpen = $state(false);
	let preparedText = $state('');
	// Per line (keyed by its text): the server's answer, or a request out.
	let rowOutcome = $state<Record<string, Outcome>>({});
	let rowBusy = $state<Record<string, boolean>>({});
	let batchRunning = $state(false);
	let batchNote = $state('');

	const lines = $derived(doc ? lineBreakMap(doc.text) : null);
	const shown = $derived(lines?.shown ?? '');
	const current = $derived(doc && lines ? selectionQuote(doc.text, lines.toOriginal, sel.start, sel.end) : null);
	const statement = $derived(statementOf(key, value));
	const problem = $derived.by(() => {
		if (!doc) return 'Open a captured page first.';
		if (!doc.uri.startsWith('https://')) return 'This capture has no https source page to cite.';
		return quoteProblem({ quote: current?.quote ?? '', topic, statement });
	});
	const keys = $derived(topic ? canonicalKeys(topic) : []);
	const lengthOk = $derived(!!current && current.chars >= MIN_QUOTE_CHARS && current.chars <= MAX_QUOTE_CHARS);
	const valueText = $derived(splitStatement(statement)?.value ?? '');
	const matches = $derived(doc && needle.trim() ? findAll(shown, needle) : []);
	const findStatus = $derived.by(() => {
		if (!doc || !needle.trim()) return '';
		if (matches.length === 0) return 'No match';
		const total = `${matches.length}${matches.length >= MAX_MATCHES ? '+' : ''}`;
		const at = matches.findIndex((m) => m.start === sel.start && m.end === sel.end);
		return at >= 0 ? `${at + 1} of ${total}` : `${total} ${matches.length === 1 ? 'match' : 'matches'}`;
	});
	const preparedRows = $derived(parsePrepared(preparedText));
	// A line pasted twice counts once: outcomes are kept per line text.
	const readyRows = $derived(
		preparedRows.filter(
			(r, i, all) => r.row && !r.error && !rowOutcome[r.raw] && !rowBusy[r.raw] && all.findIndex((o) => o.raw === r.raw) === i
		)
	);

	async function loadCaptures(s: string) {
		listState = 'loading';
		listError = null;
		try {
			const body = await getJson<unknown>(
				`/admin/authorities/${encodeURIComponent(s)}/captures?limit=${LIST_LIMIT}`,
				'Captured pages'
			);
			if (s !== slug) return;
			captures = parseCaptures(body);
			listState = 'ready';
		} catch (err) {
			if (s !== slug) return;
			listError = errorText(err, 'Captured pages failed to load.');
			listState = 'failed';
		}
	}

	async function openCapture(id: number) {
		captureId = id;
		docState = 'loading';
		docError = null;
		result = null;
		try {
			const parsed = parseCaptureText(await getJson<unknown>(`/admin/captures/${id}/text`, `Capture #${id}`));
			if (captureId !== id) return;
			if (!parsed) throw new Error(`Capture #${id} answered in an unknown shape.`);
			doc = parsed;
			sel = { start: 0, end: 0 };
			if (valueFollows) value = '';
			docState = 'ready';
			if (parsed.slug && parsed.slug !== slug) {
				slug = parsed.slug;
				void loadCaptures(parsed.slug);
			}
			syncUrl();
			await tick();
			if (textEl) {
				textEl.setSelectionRange(0, 0);
				textEl.scrollTop = 0;
			}
		} catch (err) {
			if (captureId !== id) return;
			doc = null;
			docError = errorText(err, `Capture #${id} could not load.`);
			docState = 'failed';
		}
	}

	function pickAuthority(next: string) {
		captures = [];
		// Open, still loading or failed: a capture of another authority
		// closes, so its late answer cannot switch the picker back.
		if (captureId !== null && doc?.slug !== next) {
			doc = null;
			captureId = null;
			docState = 'idle';
			docError = null;
			sel = { start: 0, end: 0 };
			result = null;
		}
		void loadCaptures(next);
		syncUrl();
	}

	// `slug` and `capture` stay in the address, so a reload or a shared
	// link reopens this capture; the page's own keys are left alone.
	function syncUrl() {
		const params = new URLSearchParams(location.search);
		for (const [name, v] of [
			['slug', slug],
			['capture', doc ? String(doc.id) : '']
		] as const) {
			if (v) params.set(name, v);
			else params.delete(name);
		}
		const query = params.toString();
		history.replaceState(history.state, '', query ? `?${query}` : location.pathname);
	}

	function readSelection() {
		if (!textEl) return;
		const { selectionStart: start, selectionEnd: end } = textEl;
		if (start === sel.start && end === sel.end) return;
		sel = { start, end };
		if (valueFollows) value = tidy(current?.quote ?? '');
	}

	onMount(() => {
		if (slug) void loadCaptures(slug);
		if (captureId !== null) void openCapture(captureId);
		// Mouse drags and Shift+arrows: the textarea's own events miss some.
		const onSelectionChange = () => {
			if (textEl && document.activeElement === textEl) readSelection();
		};
		document.addEventListener('selectionchange', onSelectionChange);
		return () => document.removeEventListener('selectionchange', onSelectionChange);
	});

	// Scroll the text so the character at `at` sits a third of the way
	// down: a hidden copy with the textarea's width and font measures
	// where that character wraps to.
	function scrollToOffset(el: HTMLTextAreaElement, at: number) {
		const css = getComputedStyle(el);
		const width = el.clientWidth - parseFloat(css.paddingLeft) - parseFloat(css.paddingRight);
		if (!(width > 0)) return;
		const mirror = document.createElement('div');
		mirror.style.cssText = `position:absolute;top:0;left:-99999px;visibility:hidden;white-space:pre-wrap;overflow-wrap:break-word;box-sizing:content-box;width:${width}px`;
		for (const p of ['font-family', 'font-size', 'font-weight', 'font-style', 'line-height', 'letter-spacing', 'word-spacing', 'tab-size', 'word-break']) {
			mirror.style.setProperty(p, css.getPropertyValue(p));
		}
		mirror.textContent = el.value.slice(0, at);
		const mark = mirror.appendChild(document.createElement('span'));
		mark.textContent = '|';
		document.body.appendChild(mirror);
		const top = mark.offsetTop + parseFloat(css.paddingTop);
		mirror.remove();
		el.scrollTop = Math.max(0, top - el.clientHeight / 3);
	}

	// Enter selects the next match (Shift+Enter the previous), scrolls
	// it into view and hands focus to the text, where Shift+arrows
	// adjust the selection.
	function findNext(backward = false) {
		const el = textEl;
		if (!el || matches.length === 0) return;
		const m = matches[nextMatch(matches, sel.start, sel.end, backward)];
		el.focus();
		el.setSelectionRange(m.start, m.end, 'forward');
		scrollToOffset(el, m.start);
		readSelection();
	}

	function onFindKey(e: KeyboardEvent) {
		if (e.key !== 'Enter') return;
		e.preventDefault();
		findNext(e.shiftKey);
	}

	function useSelection() {
		value = tidy(current?.quote ?? '');
		valueFollows = true;
	}

	/** The answer of a failed POST, as an outcome. */
	function failure(err: unknown, fallback: string): Outcome {
		return err instanceof ApiError
			? proposeOutcome(err.status, err.body, err.message)
			: { kind: 'error', text: errorText(err, fallback), factId: null };
	}

	async function propose() {
		if (!doc || !current || proposing) return;
		const built = quoteBody({ quote: current.quote, start: current.start, end: current.end, topic, statement });
		if ('error' in built) {
			result = { kind: 'error', text: built.error, factId: null };
			return;
		}
		const id = doc.id;
		proposing = true;
		result = null;
		try {
			const out = await postJson<{ fact_id?: number }>(`/admin/captures/${id}/quote`, built.body, 'Propose');
			result = proposeOutcome(201, out);
			toast.success(`${result.text.replace(/\.$/, '')} — accept it on the Facts tab`);
			// Ready for the next span of the same page.
			key = '';
			value = '';
			valueFollows = true;
			textEl?.setSelectionRange(sel.end, sel.end);
			sel = { start: sel.end, end: sel.end };
		} catch (err) {
			result = failure(err, 'Propose failed.');
		} finally {
			proposing = false;
		}
	}

	async function proposeRow(r: PreparedRow) {
		const row = r.row;
		if (!row || r.error || rowBusy[r.raw]) return;
		const built = quoteBody({ quote: row.quote, start: row.start, end: row.end, topic: row.topic, statement: row.statement });
		if ('error' in built) {
			rowOutcome[r.raw] = { kind: 'error', text: built.error, factId: null };
			return;
		}
		rowBusy[r.raw] = true;
		try {
			const out = await postJson<{ fact_id?: number }>(`/admin/captures/${row.capture_id}/quote`, built.body, `Line ${r.line}`);
			rowOutcome[r.raw] = proposeOutcome(201, out);
		} catch (err) {
			rowOutcome[r.raw] = failure(err, `Line ${r.line} failed.`);
		} finally {
			rowBusy[r.raw] = false;
		}
	}

	// One request at a time, in line order: each row reports its own
	// answer, and a refused row never stops the rest.
	async function proposeAll() {
		if (batchRunning) return;
		const rows = [...readyRows];
		if (rows.length === 0) return;
		batchRunning = true;
		try {
			for (const [i, r] of rows.entries()) {
				batchNote = `Proposing ${i + 1} of ${rows.length}…`;
				await proposeRow(r);
			}
			const made = rows.filter((r) => rowOutcome[r.raw]?.kind === 'proposed').length;
			batchNote = `${made} of ${rows.length} proposed${made > 0 ? ' — accept them on the Facts tab' : ''}.`;
		} finally {
			batchRunning = false;
		}
	}

	// A prepared quote the server could not place: open its capture with
	// the form filled and Find on the quote, to select the right span.
	async function findRow(row: PreparedQuote) {
		topic = row.topic;
		const parts = splitStatement(row.statement);
		key = parts?.key ?? '';
		value = parts ? tidy(parts.value) : '';
		valueFollows = !parts;
		needle = tidy(row.quote);
		if (!doc || doc.id !== row.capture_id) await openCapture(row.capture_id);
		if (!doc || doc.id !== row.capture_id) return;
		await tick();
		findNext();
	}

	function capturedOn(at: number): string {
		return at > 0 ? new Date(at * 1000).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' }) : '';
	}

	const short = (text: string, n: number) => (text.length > n ? `${text.slice(0, n)}…` : text);
	const outcomeCls = (o: Outcome | undefined) =>
		o?.kind === 'proposed' ? 'text-verified' : o?.kind === 'duplicate' ? 'text-ink-muted' : 'text-error';
</script>

<div class="flex flex-col gap-4">
	<p class="m-0 max-w-prose text-sm text-ink-muted">
		Quote the exact words of a captured official page to fill a value the drafter skipped. The fact lands
		as proposed with the quote as its citation, and reaches the public guide only once it is accepted on
		the Facts tab.
	</p>
	<div class="max-w-md">
		<AuthorityPicker id="quote-authority" bind:value={slug} onChange={pickAuthority} />
	</div>

	{#if !slug && captureId === null}
		<p class="m-0 text-sm text-ink-muted">Pick an authority to list its captured pages.</p>
	{:else}
		<div class="grid items-start gap-4 lg:grid-cols-5">
			<div class="flex min-w-0 flex-col gap-4 lg:col-span-3">
				<section aria-labelledby="quote-pages-h" class="flex flex-col gap-2">
					<h2 id="quote-pages-h" class="m-0 text-h3 font-semibold">Captured pages</h2>
					{#if !slug}
						<p class="m-0 text-sm text-ink-muted">Pick an authority to list its captured pages.</p>
					{:else if listState === 'loading' || listState === 'idle'}
						<div class="flex flex-col gap-2" role="status" aria-label="Loading captured pages">
							{#each [0, 1, 2] as i (i)}
								<Skeleton class="h-12 rounded-sm" />
							{/each}
						</div>
					{:else if listState === 'failed'}
						<div class="flex flex-wrap items-center gap-2" role="status">
							<p class="m-0 text-sm text-ink-muted">{listError}</p>
							<Button variant="outline" size="sm" onclick={() => loadCaptures(slug)}>Try again</Button>
						</div>
					{:else if captures.length === 0}
						<p class="m-0 text-sm text-ink-muted">No captured page of {slug} has text to quote yet.</p>
					{:else}
						<ul
							class="m-0 flex max-h-64 list-none flex-col overflow-y-auto rounded-md border border-line p-0"
							aria-label={`Captured pages of ${slug}, newest first`}
						>
							{#each captures as c (c.id)}
								<li class="border-t border-line first:border-t-0">
									<button
										type="button"
										class={`flex w-full min-w-0 cursor-pointer flex-col gap-0.5 border-l-2 px-3 py-2 text-left ${c.id === captureId ? 'border-l-accent-strong bg-surface' : 'border-l-transparent hover:bg-surface'}`}
										aria-current={c.id === captureId ? 'true' : undefined}
										onclick={() => openCapture(c.id)}
									>
										<span class="block max-w-full truncate font-mono text-xs text-ink">{uriPath(c.uri)}</span>
										<span class="text-xs text-ink-muted">
											#{c.id} · {capturedOn(c.captured_at)} · {sizeLabel(c.bytes)}{c.kind ? ` · ${c.kind}` : ''}{c.trust ? ` · ${c.trust}` : ''}
										</span>
									</button>
								</li>
							{/each}
						</ul>
					{/if}
				</section>

				<section aria-labelledby="quote-text-h" class="flex flex-col gap-2">
					<div class="flex flex-wrap items-center justify-between gap-2">
						<h2 id="quote-text-h" class="m-0 text-h3 font-semibold">{doc ? `Capture #${doc.id}` : 'Capture text'}</h2>
						{#if doc}
							<span class="flex flex-wrap items-center gap-1 text-xs text-ink-muted">
								{#if doc.kind}<Badge variant="outline">{doc.kind}</Badge>{/if}
								{#if doc.trust}<Badge variant="outline">{doc.trust}</Badge>{/if}
								{capturedOn(doc.captured_at)}
								<Button variant="ghost" size="sm" onclick={() => doc && openCapture(doc.id)}>Reload text</Button>
							</span>
						{/if}
					</div>
					{#if docState === 'loading'}
						<Skeleton class="h-96 rounded-md" />
					{:else if docState === 'failed'}
						<Notice variant="error" heading="The capture text could not load.">
							<p class="m-0">{docError}</p>
							{#if captureId !== null}
								{@const id = captureId}
								<p class="mt-3 mb-0"><Button variant="outline" onclick={() => openCapture(id)}>Try again</Button></p>
							{/if}
						</Notice>
					{:else if !doc}
						<p class="m-0 text-sm text-ink-muted">Pick a captured page to read its text.</p>
					{:else}
						{#if doc.uri}
							<p class="m-0 flex min-w-0 items-center gap-1 text-xs">
								<a class="truncate underline underline-offset-2" href={doc.uri} target="_blank" rel="noopener noreferrer">{doc.uri}</a>
								<ExternalLinkIcon class="size-3 flex-none text-ink-muted" aria-hidden="true" />
							</p>
						{/if}
						<div class="flex items-end gap-2">
							<label class="flex min-w-0 flex-1 flex-col gap-1 text-xs font-medium text-ink-muted" for="quote-find">
								Find in the text
								<Input
									id="quote-find"
									type="search"
									class="h-9"
									bind:value={needle}
									autocomplete="off"
									spellcheck={false}
									aria-describedby="quote-find-status"
									onkeydown={onFindKey}
								/>
							</label>
							<Button variant="outline" size="icon-lg" aria-label="Previous match" disabled={matches.length === 0} onclick={() => findNext(true)}>
								<ChevronUpIcon aria-hidden="true" />
							</Button>
							<Button variant="outline" size="icon-lg" aria-label="Next match" disabled={matches.length === 0} onclick={() => findNext()}>
								<ChevronDownIcon aria-hidden="true" />
							</Button>
						</div>
						<p id="quote-find-status" class="m-0 min-h-4 text-xs text-ink-muted" role="status">{findStatus}</p>
						<label class="text-sm font-medium" for="quote-text">Capture text</label>
						<Textarea
							id="quote-text"
							bind:ref={textEl}
							value={shown}
							readonly
							rows={24}
							spellcheck={false}
							class="field-sizing-fixed resize-y font-mono text-sm leading-6"
							aria-describedby="quote-text-help"
							onselect={readSelection}
							onkeyup={readSelection}
							onmouseup={readSelection}
							onfocus={readSelection}
						/>
						<p id="quote-text-help" class="m-0 text-xs text-ink-muted">
							Read-only. Select the words to quote with the mouse, or with Shift and the arrow keys. Enter in
							Find selects the next match, Shift+Enter the previous.
						</p>
					{/if}
				</section>
			</div>

			<aside class="min-w-0 lg:sticky lg:top-20 lg:col-span-2" aria-labelledby="quote-form-h">
				<Card.Root class="gap-4 shadow-md">
					<Card.Header>
						<Card.Title><h2 id="quote-form-h" class="m-0 text-h3 font-semibold">Propose a fact</h2></Card.Title>
						<Card.Description>
							{doc ? `From capture #${doc.id} of ${doc.slug || slug}.` : 'Open a captured page, then select the words to quote.'}
						</Card.Description>
					</Card.Header>
					<Card.Content class="flex flex-col gap-4">
						<section aria-labelledby="quote-sel-h" class="flex flex-col gap-1">
							<h3 id="quote-sel-h" class="m-0 text-sm font-semibold">Quote</h3>
							{#if current && current.quote}
								<blockquote class="m-0 rounded-sm border-l-2 border-verified bg-surface px-3 py-2 font-quote text-body break-words whitespace-pre-wrap italic">{short(current.quote, 600)}</blockquote>
								<p class={`m-0 flex items-start gap-1 text-xs ${lengthOk ? 'text-ink-muted' : 'font-semibold text-error'}`}>
									{#if lengthOk}<CheckIcon class="mt-0.5 size-3 flex-none text-verified" aria-hidden="true" />{:else}<TriangleAlertIcon class="mt-0.5 size-3 flex-none" aria-hidden="true" />{/if}
									<span>{current.chars} characters (a public citation is {MIN_QUOTE_CHARS} to {MAX_QUOTE_CHARS}) · bytes {current.start}–{current.end}</span>
								</p>
							{:else}
								<p class="m-0 text-sm text-ink-muted">Nothing selected yet.</p>
							{/if}
						</section>

						<div class="flex flex-col gap-1">
							<span id="quote-topic-label" class="text-sm font-medium">Topic</span>
							<Select.Root
								type="single"
								value={topic}
								onValueChange={(v) => {
									if (v) topic = v;
								}}
							>
								<Select.Trigger id="quote-topic" class="h-9 w-full font-mono" aria-labelledby="quote-topic-label quote-topic">
									{#if topic}{topic}{:else}<span class="font-sans text-ink-muted">Choose a topic</span>{/if}
								</Select.Trigger>
								<Select.Content>
									{#each TOPIC_CHOICES as choice (choice.topic)}
										<Select.Item value={choice.topic} label={choice.topic}>
											<span class="flex min-w-0 flex-col">
												<span class="font-mono">{choice.topic}</span>
												<span class="text-xs text-ink-muted">{choice.where}</span>
											</span>
										</Select.Item>
									{/each}
								</Select.Content>
							</Select.Root>
						</div>

						<div class="flex flex-col gap-1">
							{#if keys.length > 0}
								<div class="flex flex-wrap items-center gap-1" role="group" aria-labelledby="quote-keys-label">
									<span id="quote-keys-label" class="mr-1 text-xs font-medium text-ink-muted">Keys of {topic}</span>
									{#each keys as chip (chip.key)}
										{@const on = KEY.test(key.trim()) && splitKey(key.trim()).base === chip.key}
										<Button
											size="xs"
											variant={on ? 'secondary' : 'outline'}
											class="font-mono"
											title={chip.label}
											aria-pressed={on}
											onclick={() => (key = chipKey(key, chip.key, topic))}
										>
											{#if on}<CheckIcon aria-hidden="true" />{/if}{chip.key}
										</Button>
									{/each}
								</div>
							{/if}
							<label class="flex flex-col gap-1 text-sm font-medium" for="quote-key">
								Key
								<Input
									id="quote-key"
									class="h-9 font-mono"
									bind:value={key}
									autocomplete="off"
									spellcheck={false}
									aria-describedby="quote-key-note"
								/>
							</label>
							<p id="quote-key-note" class="m-0 text-xs text-ink-muted">
								{topic ? keyNote(statement, topic) : 'Pick a topic first: its keys show here.'} An office instance goes after two underscores (weekday__tumwater).
							</p>
						</div>

						<div class="flex flex-col gap-1">
							<label class="flex flex-col gap-1 text-sm font-medium" for="quote-value">
								Value
								<Textarea
									id="quote-value"
									rows={2}
									bind:value
									spellcheck={false}
									aria-describedby="quote-value-note"
									oninput={() => (valueFollows = false)}
								/>
							</label>
							<div class="flex flex-wrap items-center justify-between gap-2">
								<p id="quote-value-note" class="m-0 text-xs text-ink-muted">
									Starts as the selection: trim it to the words the page should show.
								</p>
								<Button variant="ghost" size="xs" disabled={!current?.quote} onclick={useSelection}>Use the selection</Button>
							</div>
							{#if valueText && current?.quote}
								{#if valueInQuote(valueText, current.quote)}
									<p class="m-0 flex items-start gap-1 text-xs text-verified">
										<CheckIcon class="mt-0.5 size-3 flex-none" aria-hidden="true" /><span>The value is in the quote.</span>
									</p>
								{:else}
									<p class="m-0 flex items-start gap-1 text-xs font-semibold text-error">
										<TriangleAlertIcon class="mt-0.5 size-3 flex-none" aria-hidden="true" /><span>The value is not in the quote (case and spacing aside).</span>
									</p>
								{/if}
							{/if}
						</div>

						<div class="flex flex-col gap-1">
							<span class="text-sm font-medium">Statement</span>
							<p class="m-0 rounded-sm border border-line px-3 py-2 font-mono text-sm break-words">{statement}</p>
						</div>

						<div class="flex flex-col gap-2">
							<div>
								<Button
									size="lg"
									disabled={problem !== null || proposing}
									aria-describedby={problem ? 'quote-problem' : undefined}
									onclick={propose}
								>
									{proposing ? 'Proposing…' : 'Propose'}
								</Button>
							</div>
							{#if problem}
								<p id="quote-problem" class="m-0 text-xs text-ink-muted">Propose is off: {problem}</p>
							{/if}
							<p class={`m-0 text-sm font-semibold ${outcomeCls(result ?? undefined)}`} role="status">
								{result ? (result.kind === 'proposed' ? `${result.text} Accept it on the Facts tab.` : result.text) : ''}
							</p>
						</div>
					</Card.Content>
				</Card.Root>
			</aside>
		</div>
	{/if}

	<section aria-labelledby="prepared-h" class="flex flex-col gap-3 border-t border-line pt-4">
		<h2 id="prepared-h" class="m-0 text-h3 font-semibold">
			<button
				type="button"
				class="inline-flex min-h-6 cursor-pointer items-center gap-1 rounded-sm text-left focus-visible:outline-2 focus-visible:outline-focus"
				aria-expanded={preparedOpen}
				aria-controls={preparedOpen ? 'prepared-body' : undefined}
				onclick={() => (preparedOpen = !preparedOpen)}
			>
				<ChevronRightIcon class={`size-4 flex-none transition-transform ${preparedOpen ? 'rotate-90' : ''}`} aria-hidden="true" />
				Prepared quotes
			</button>
		</h2>
		{#if !preparedOpen}
			<p class="m-0 text-xs text-ink-muted">Quotes written elsewhere, one JSON object per line: the server finds each one in its capture.</p>
		{:else}
			<div id="prepared-body" class="flex flex-col gap-3">
				<p class="m-0 max-w-prose text-sm text-ink-muted">
					One object per line. The server finds each quote in its capture's text (exact case, any spacing
					between words); a quote that appears more than once needs its byte offsets (<code
						class="font-mono">"start"</code> and <code class="font-mono">"end"</code>) or has to be selected in the
					text instead.
				</p>
				<code class="block rounded-sm bg-surface px-3 py-2 font-mono text-xs break-all">{EXAMPLE}</code>
				<label class="flex flex-col gap-1 text-sm font-medium" for="prepared-lines">
					JSON Lines
					<Textarea id="prepared-lines" rows={6} class="field-sizing-fixed font-mono text-xs" bind:value={preparedText} spellcheck={false} />
				</label>
				<div class="flex flex-wrap items-center gap-2">
					<Button disabled={batchRunning || readyRows.length === 0} onclick={proposeAll}>
						Propose all ready rows ({readyRows.length})
					</Button>
					<p class="m-0 text-sm" role="status">{batchNote}</p>
				</div>
				{#if preparedRows.length > 0}
					<ol class="m-0 flex list-none flex-col p-0" aria-label="Prepared rows">
						{#each preparedRows as r (r.line)}
							{@const outcome = rowOutcome[r.raw]}
							{@const busy = rowBusy[r.raw] === true}
							<li class="flex flex-col gap-1 border-t border-line py-2 text-sm">
								<div class="flex flex-wrap items-start justify-between gap-2">
									<span class="flex min-w-0 flex-1 flex-col gap-0.5">
										<span class="font-mono text-xs text-ink-muted">Line {r.line}{r.row ? ` · capture #${r.row.capture_id} · ${r.row.topic}` : ''}</span>
										{#if r.row}
											<span class="break-words">{r.row.statement}</span>
											<span class="text-xs break-words text-ink-muted italic">“{short(r.row.quote, 200)}”</span>
										{/if}
									</span>
									{#if r.row}
										{@const row = r.row}
										<span class="flex flex-none flex-wrap gap-1">
											{#if outcome?.kind === 'ambiguous' || outcome?.kind === 'not_found'}
												<Button size="sm" variant="ghost" onclick={() => findRow(row)}>Find in the text</Button>
											{/if}
											<Button
												size="sm"
												variant="outline"
												disabled={!!r.error || busy || batchRunning || outcome?.kind === 'proposed' || outcome?.kind === 'duplicate'}
												onclick={() => proposeRow(r)}
											>
												{busy ? 'Proposing…' : 'Propose'}<span class="sr-only"> line {r.line}</span>
											</Button>
										</span>
									{/if}
								</div>
								{#if r.error}
									<p class="m-0 text-xs font-semibold break-words text-error">{r.error}</p>
								{/if}
								<p class={`m-0 text-xs font-semibold break-words ${outcomeCls(outcome)}`} role="status">{outcome?.text ?? ''}</p>
							</li>
						{/each}
					</ol>
				{/if}
			</div>
		{/if}
	</section>
</div>
