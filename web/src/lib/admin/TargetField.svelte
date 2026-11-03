<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { scale } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import LinkIcon from '@lucide/svelte/icons/link';
	import MailIcon from '@lucide/svelte/icons/mail';
	import PhoneIcon from '@lucide/svelte/icons/phone';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import CheckIcon from '@lucide/svelte/icons/check';
	import XIcon from '@lucide/svelte/icons/x';
	import {
		MAX_SWEEP_URLS,
		classifyEntry,
		countLine,
		countryName,
		entryKey,
		extractEntries,
		planSubmit,
		type ChipState,
		type Entry
	} from './contact-entry.js';

	// Chip input for the watch sweep: one field takes URLs, office
	// emails and office phone numbers, typed or pasted as a block.
	// Each entry becomes a chip with its detected type; invalid ones
	// stay visible with the reason, and nothing here talks to the API.
	let {
		entries = $bindable([]),
		defaultCountry = null,
		savedKeys = new Set<string>(),
		maxUrls = MAX_SWEEP_URLS,
		id,
		disabled = false
	}: {
		entries?: Entry[];
		defaultCountry?: string | null;
		savedKeys?: Set<string>;
		maxUrls?: number;
		id: string;
		disabled?: boolean;
	} = $props();

	let draft = $state('');
	let input = $state<HTMLInputElement | null>(null);
	let armed = $state<string | null>(null);
	let flash = $state<string | null>(null);
	let note = $state('');
	let reduceMotion = $state(false);

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
	});

	const motion = $derived(reduceMotion ? 0 : 150);
	const plan = $derived(planSubmit(entries, savedKeys, maxUrls));
	const counter = $derived(
		countLine({
			urls: plan.urls.length,
			emails: plan.contacts.filter((e) => e.kind === 'email').length,
			phones: plan.contacts.filter((e) => e.kind === 'phone').length,
			onFile: plan.onFile.length,
			invalid: plan.invalid.length,
			overLimit: plan.overLimit.length
		})
	);
	const where = $derived(defaultCountry ? countryName(defaultCountry) : null);

	function add(list: Entry[]): number {
		const have = new Set(entries.map(entryKey));
		const fresh: Entry[] = [];
		for (const e of list) {
			const key = entryKey(e);
			if (have.has(key)) {
				flash = key;
				setTimeout(() => {
					if (flash === key) flash = null;
				}, 300);
				continue;
			}
			have.add(key);
			fresh.push(e);
		}
		if (fresh.length) entries = [...entries, ...fresh];
		return fresh.length;
	}

	function commit(): boolean {
		const text = draft.trim();
		if (!text) return false;
		const list = /[,;\n\t]/.test(text)
			? extractEntries(text, defaultCountry).entries
			: [classifyEntry(text, defaultCountry)];
		add(list);
		draft = '';
		armed = null;
		return true;
	}

	function remove(e: Entry) {
		const key = entryKey(e);
		entries = entries.filter((x) => entryKey(x) !== key);
		armed = null;
		note = `Removed ${e.display}`;
		input?.focus();
	}

	async function edit(e: Entry) {
		remove(e);
		draft = e.raw;
		await tick();
		input?.focus();
		input?.select();
	}

	function onKeydown(event: KeyboardEvent) {
		const key = event.key;
		if (key === 'Enter') {
			event.preventDefault();
			if (!commit()) input?.form?.requestSubmit();
		} else if ((key === ',' || key === ';') && draft.trim()) {
			event.preventDefault();
			commit();
		} else if (key === 'Tab' && !event.shiftKey && draft.trim()) {
			event.preventDefault();
			commit();
		} else if (key === 'Escape' && draft) {
			event.preventDefault();
			draft = '';
		} else if (key === 'Backspace' && !draft && entries.length > 0) {
			event.preventDefault();
			const last = entries[entries.length - 1];
			if (armed === entryKey(last)) remove(last);
			else {
				armed = entryKey(last);
				note = `Press Backspace again to remove ${last.display}`;
			}
		} else if (armed) {
			armed = null;
		}
	}

	function onPaste(event: ClipboardEvent) {
		const text = event.clipboardData?.getData('text') ?? '';
		if (!text.trim()) return;
		const { entries: found, ignored } = extractEntries(text, defaultCountry);
		if (found.length === 0) return;
		event.preventDefault();
		const added = add(found);
		const parts = [`Added ${added} entr${added === 1 ? 'y' : 'ies'}`];
		if (found.length > added) parts.push(`${found.length - added} already listed`);
		if (ignored) parts.push(`ignored ${ignored} word${ignored === 1 ? '' : 's'}`);
		note = parts.join(' · ');
	}

	function stateOf(e: Entry): ChipState {
		return plan.states.get(entryKey(e)) ?? (e.sendable ? 'ok' : 'invalid');
	}

	// Full class strings per state (the tailwind checker reads literals).
	function chipCls(state: ChipState, key: string): string {
		const base =
			'inline-flex min-h-7 max-w-full items-center gap-1 rounded-full border py-0.5 pr-0.5 pl-2 text-sm transition-shadow';
		const ring = armed === key || flash === key ? 'ring-2 ring-focus' : '';
		if (state === 'saved') return `${base} border-transparent bg-verified-bg text-verified ${ring}`;
		if (state === 'warning' || state === 'over') return `${base} border-transparent bg-changed-bg text-changed ${ring}`;
		if (state === 'invalid') return `${base} border-destructive/40 bg-destructive/10 text-destructive ${ring}`;
		return `${base} border-line bg-card text-ink ${ring}`;
	}

	function kindLabel(e: Entry, state: ChipState): string {
		if (state === 'invalid') return 'Needs fixing';
		if (state === 'saved') return 'Already on file';
		return e.kind === 'url' ? 'URL' : e.kind === 'email' ? 'Email' : 'Phone';
	}

	function titleFor(e: Entry, state: ChipState): string {
		if (state === 'invalid') return `${e.raw}: ${e.error ?? 'needs fixing'}`;
		if (state === 'over') return `${e.value}: over the ${maxUrls}-URL limit, kept for the next sweep`;
		if (state === 'saved') return `${e.value}: already on file for this office`;
		if (e.kind === 'phone') return `${e.value}${e.country ? ` · ${countryName(e.country)}` : ''}`;
		return e.warning ? `${e.value}: ${e.warning}` : e.value;
	}

	export function focus() {
		input?.focus();
	}

	/** Commit any typed text (the page calls this before it submits). */
	export function flush() {
		commit();
	}
</script>

<div class="flex min-w-0 flex-col gap-1.5">
	<label id={`${id}-label`} for={id} class="text-xs font-medium text-ink-muted">URLs, emails and phone numbers</label>
	<!-- Clicking the box focuses the text input; chips keep their own buttons. -->
	<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
	<div
		role="group"
		aria-labelledby={`${id}-label`}
		class="flex min-h-11 w-full min-w-0 cursor-text flex-wrap items-center gap-1.5 rounded-md border border-input bg-transparent px-2 py-1.5 transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50 dark:bg-input/30"
		onclick={(e) => {
			if (e.target === e.currentTarget) input?.focus();
		}}
	>
		<ul aria-label="Entries" class="contents">
			{#each entries as e (entryKey(e))}
				{@const state = stateOf(e)}
				{@const key = entryKey(e)}
				<li class="flex max-w-full min-w-0" in:scale={{ duration: motion, start: 0.9 }} animate:flip={{ duration: motion }}>
					<span class={chipCls(state, key)} title={titleFor(e, state)} aria-invalid={state === 'invalid' ? 'true' : undefined}>
						{#if state === 'invalid'}
							<CircleAlertIcon class="size-3.5 flex-none" aria-hidden="true" />
						{:else if state === 'saved'}
							<CheckIcon class="size-3.5 flex-none" aria-hidden="true" />
						{:else if e.kind === 'url'}
							<LinkIcon class="size-3.5 flex-none" aria-hidden="true" />
						{:else if e.kind === 'email'}
							<MailIcon class="size-3.5 flex-none" aria-hidden="true" />
						{:else}
							<PhoneIcon class="size-3.5 flex-none" aria-hidden="true" />
						{/if}
						<button
							type="button"
							class="min-w-0 rounded-xs text-left whitespace-normal wrap-anywhere outline-none focus-visible:ring-2 focus-visible:ring-focus"
							{disabled}
							onclick={() => edit(e)}
						>
							<span class="sr-only">{kindLabel(e, state)}: </span>{e.display}<span class="sr-only">. Edit</span>
						</button>
						<button
							type="button"
							class="inline-flex size-6 flex-none items-center justify-center rounded-full outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-focus"
							aria-label={`Remove ${e.display}`}
							{disabled}
							onclick={() => remove(e)}
						>
							<XIcon class="size-3.5" aria-hidden="true" />
						</button>
					</span>
				</li>
			{/each}
		</ul>
		<input
			bind:this={input}
			{id}
			bind:value={draft}
			{disabled}
			class="h-7 min-w-0 flex-1 basis-40 bg-transparent px-1 text-sm text-ink outline-none placeholder:text-muted-foreground"
			type="text"
			inputmode="text"
			autocomplete="off"
			spellcheck={false}
			placeholder={entries.length ? "Add another…" : "https://…  desk@office.gov  +1 360 555 0100"}
			aria-describedby={`${id}-hint ${id}-count`}
			onkeydown={onKeydown}
			onpaste={onPaste}
			onblur={() => commit()}
		/>
	</div>
	<p id={`${id}-hint`} class="m-0 text-xs text-ink-muted">
		Enter, comma or Tab adds an entry; paste a whole contact block to split it.
		{#if where}Numbers without + are read as {where}.{:else}Start numbers with + and the country code.{/if}
		Office contacts only, never a customer's.
	</p>
	<p id={`${id}-count`} class={`m-0 text-sm ${entries.length ? "font-medium" : "text-ink-muted"}`} aria-live="polite">
		{counter}<span class="sr-only">{note ? `. ${note}` : ''}</span>
	</p>
	{#if plan.invalid.length > 0 || plan.overLimit.length > 0}
		<ul class="m-0 flex list-none flex-col gap-1 p-0 text-sm" aria-label="Entries to fix">
			{#each plan.invalid as e (entryKey(e))}
				<li class="flex items-start gap-2 text-destructive">
					<CircleAlertIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
					<span class="min-w-0 wrap-anywhere"><span class="font-mono">{e.raw}</span> — {e.error}</span>
				</li>
			{/each}
			{#if plan.overLimit.length > 0}
				<li class="flex items-start gap-2 text-changed">
					<CircleAlertIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
					<span>
						A sweep fetches {maxUrls} URLs at most; {plan.overLimit.length} stay{plan.overLimit.length === 1 ? 's' : ''} here
						for the next run.
					</span>
				</li>
			{/if}
		</ul>
	{/if}
</div>
