<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { subscribe, type BusMessage } from '#lib/live.svelte.js';
	import {
		CHANNEL_MAX,
		EvidenceError,
		FACT_KINDS,
		NOTE_KINDS,
		PUBLIC_NOTE_LINE,
		STANCES,
		SUMMARY_MAX,
		SUMMARY_MIN,
		anchorChoices,
		channelFor,
		charCount,
		createNote,
		defaultAnchor,
		fetchAcceptedFacts,
		fetchNotes,
		fromLocalInput,
		kindLabel,
		noteBody,
		noteProblem,
		notesForFact,
		preferredChoices,
		shorten,
		slotChoices,
		slotFor,
		slotLabel,
		stanceLabel,
		supportChoices,
		toLocalInput,
		withdrawBody,
		withdrawNote,
		type EvidenceNote,
		type NoteSubject,
		type PublishedFact
	} from './evidence-client.js';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import PlusIcon from '@lucide/svelte/icons/plus';

	// Considerations for the fact a /review drawer shows (plan §5.4):
	// the office's published staff notes about it, each with Withdraw,
	// and the "Add a consideration" form. A call, email or visit note is
	// about one accepted fact; an absence names a field; a decision picks
	// the preferred fact of a conflict and rests on published notes. The
	// server owns every guard and answers a refusal with a plain reason,
	// which shows inline. Lists re-read live on the office's channel
	// (`evidence_updated`, `fact_retired`). In the queue drawer the panel
	// starts closed and reads nothing until opened.
	let {
		subject,
		collapsible = false,
		onLive
	}: {
		subject: NoteSubject;
		/** Starts closed behind a "Considerations" toggle (the queue drawer). */
		collapsible?: boolean;
		/** Events on the office's channel, after the panel re-read its lists. */
		onLive?: (msg: BusMessage) => void;
	} = $props();

	const API = '/apostille/api/v1';
	const uid = $props.id();
	const STANCE_CLS: Record<string, string> = {
		confirms: 'border-verified text-verified',
		contradicts: 'border-error text-error',
		context: 'border-line text-ink-muted'
	};
	const CHANNEL_LABEL: Record<string, string> = {
		call: 'Phone line',
		email: 'Email address',
		visit: 'Where',
		absence: 'Pages checked (optional)'
	};
	const CHANNEL_HINT: Record<string, string> = {
		call: "Starts with the office's published number; add the menu option you used.",
		email: "The office's published address. A summary may name only this address.",
		visit: 'The counter or office, as the office names it.',
		absence: 'The office pages you read, by name.'
	};

	let open = $state(untrack(() => !collapsible));
	let notes = $state<EvidenceNote[]>([]);
	let notesState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let notesError = $state<string | null>(null);
	let facts = $state<PublishedFact[]>([]);
	let factsState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let factsError = $state<string | null>(null);

	let formOpen = $state(false);
	let kind = $state('call');
	let stance = $state('');
	let anchorId = $state<number | null>(null);
	let when = $state('');
	let latest = $state('');
	let channel = $state('');
	let channelEdited = $state(false);
	let summary = $state('');
	let fieldSlot = $state('');
	let preferredId = $state<number | null>(null);
	let supports = $state<number[]>([]);
	let saving = $state(false);
	let formError = $state<string | null>(null);
	let publishedId = $state<number | null>(null);

	let withdrawId = $state<number | null>(null);
	let withdrawReason = $state('');
	let withdrawing = $state(false);
	let withdrawError = $state<string | null>(null);

	const slug = $derived(subject.authority_slug);
	const subjectId = $derived(subject.id);
	const isAccepted = $derived(subject.status === 'accepted');
	const ownSlot = $derived(slotFor(subject.topic, subject.statement));
	// Only this office's rows: a drawer that moved to another office keeps
	// the last office's lists until the new read lands.
	const officeFacts = $derived(facts.filter((f) => f.authority_slug === slug));
	const shown = $derived(notesForFact(notes, subject));
	const factKind = $derived(FACT_KINDS.includes(kind));
	const anchors = $derived(anchorChoices(subject, officeFacts));
	const anchorFact = $derived(officeFacts.find((f) => f.id === anchorId) ?? null);
	const alternatives = $derived(preferredChoices(subject, officeFacts));
	const basis = $derived(supportChoices(notes, slug));
	const fields = $derived(slotChoices(ownSlot));
	const count = $derived(charCount(summary));
	const acceptedIds = $derived(new Set(officeFacts.map((f) => f.id)));
	const noteSlot = $derived.by(() => {
		if (kind === 'absence') return fieldSlot;
		if (kind === 'decision') {
			const preferred = alternatives.find((f) => f.id === preferredId);
			return preferred ? slotFor(preferred.topic, preferred.statement) : ownSlot;
		}
		if (anchorFact) return slotFor(anchorFact.topic, anchorFact.statement);
		return isAccepted && anchorId === subject.id ? ownSlot : '';
	});
	const input = $derived({
		slug,
		kind,
		stance,
		factId: factKind ? anchorId : null,
		slot: noteSlot,
		preferredFactId: kind === 'decision' ? preferredId : null,
		supports: kind === 'decision' ? supports : [],
		when: fromLocalInput(when),
		channel: kind === 'decision' ? '' : channel,
		summary
	});
	const problem = $derived(noteProblem(input, null));

	function resetForm() {
		formOpen = false;
		kind = 'call';
		stance = '';
		anchorId = null;
		when = '';
		channel = '';
		channelEdited = false;
		summary = '';
		fieldSlot = '';
		preferredId = null;
		supports = [];
		formError = null;
		publishedId = null;
		withdrawId = null;
		withdrawReason = '';
		withdrawError = null;
	}

	// Fill what the staff member has not set yet from the office's facts.
	function fillDefaults() {
		if (anchorId === null) anchorId = defaultAnchor(subject, officeFacts);
		if (!channelEdited) channel = channelFor(kind, officeFacts);
		if (!fieldSlot) fieldSlot = ownSlot;
		if (preferredId === null && alternatives.some((f) => f.id === subject.id)) preferredId = subject.id;
	}

	async function loadNotes(s: string, quiet = false) {
		if (!quiet) {
			notesState = 'loading';
			notesError = null;
		}
		try {
			const rows = await fetchNotes(API, s);
			if (s !== slug) return;
			notes = rows;
			notesState = 'ready';
			notesError = null;
		} catch (err) {
			if (s !== slug) return;
			notesError = err instanceof Error ? err.message : 'Staff notes failed to load.';
			if (!quiet || notesState !== 'ready') notesState = 'failed';
		}
	}

	async function loadFacts(s: string, quiet = false) {
		if (!quiet) {
			factsState = 'loading';
			factsError = null;
		}
		try {
			const rows = await fetchAcceptedFacts(API, s);
			if (s !== slug) return;
			facts = rows;
			factsState = 'ready';
			factsError = null;
			if (formOpen) fillDefaults();
		} catch (err) {
			if (s !== slug) return;
			factsError = err instanceof Error ? err.message : 'Accepted facts failed to load.';
			if (!quiet || factsState !== 'ready') factsState = 'failed';
		}
	}

	// Another fact in the drawer starts a clean form; the lists stay.
	$effect(() => {
		void subjectId;
		untrack(resetForm);
	});

	// Read the office's notes and accepted facts while the panel is open.
	$effect(() => {
		if (!open) return;
		const s = slug;
		untrack(() => {
			void loadNotes(s);
			void loadFacts(s);
		});
	});

	// Live: a note published or withdrawn, or a fact retired, by anyone.
	$effect(() => {
		if (!open) return;
		const s = slug;
		return subscribe(`authority:${s}`, (msg) => {
			if (msg.type === 'evidence_updated') {
				void loadNotes(s, true);
			} else if (msg.type === 'fact_retired') {
				void loadFacts(s, true);
				void loadNotes(s, true);
				onLive?.(msg);
			}
		});
	});

	async function openForm() {
		formOpen = true;
		formError = null;
		publishedId = null;
		const now = Math.floor(Date.now() / 1000);
		when = toLocalInput(now);
		latest = toLocalInput(now + 60);
		fillDefaults();
		await tick();
		document.getElementById(`${uid}-kind`)?.focus();
	}

	function setKind(value: string) {
		if (!NOTE_KINDS.some((k) => k.value === value)) return;
		kind = value;
		formError = null;
		if (!channelEdited) channel = channelFor(value, officeFacts);
	}

	function setAnchor(value: string) {
		const id = Number(value);
		anchorId = Number.isInteger(id) && id > 0 ? id : null;
	}

	function toggleSupport(id: number, on: boolean) {
		supports = on ? [...supports.filter((s) => s !== id), id] : supports.filter((s) => s !== id);
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (saving) return;
		const out = noteBody(input, Math.floor(Date.now() / 1000));
		if ('error' in out) {
			formError = out.error;
			return;
		}
		saving = true;
		formError = null;
		try {
			const note = await createNote(API, out.body);
			toast.success(`Note #${note.id} published on the ${slug} guide.`);
			resetForm();
			publishedId = note.id;
			await loadNotes(slug, true);
		} catch (err) {
			formError =
				err instanceof EvidenceError && err.reason
					? `Not published: ${err.reason}`
					: err instanceof Error
						? err.message
						: 'Publish failed.';
		} finally {
			saving = false;
		}
	}

	async function startWithdraw(id: number) {
		withdrawError = null;
		withdrawReason = '';
		withdrawId = withdrawId === id ? null : id;
		if (withdrawId === null) return;
		await tick();
		document.getElementById(`${uid}-withdraw-${id}`)?.focus();
	}

	async function withdraw(note: EvidenceNote) {
		const body = withdrawBody(withdrawReason);
		if (!body) {
			withdrawError = 'A reason is required (up to 1024 characters).';
			return;
		}
		withdrawing = true;
		withdrawError = null;
		try {
			await withdrawNote(API, note.id, body);
			toast.success(`Note #${note.id} withdrawn from the ${slug} guide.`);
			withdrawId = null;
			withdrawReason = '';
			notes = notes.filter((n) => n.id !== note.id);
			await loadNotes(slug, true);
		} catch (err) {
			withdrawError =
				err instanceof EvidenceError && err.reason
					? `Not withdrawn: ${err.reason}`
					: err instanceof Error
						? err.message
						: 'Withdraw failed.';
		} finally {
			withdrawing = false;
		}
	}

	function about(note: EvidenceNote): string {
		if (note.kind === 'decision') {
			const basisIds = note.supports.map((id) => `#${id}`).join(', ');
			return `Prefers fact #${note.preferred_fact_id ?? '?'} · rests on ${basisIds || 'no notes'}`;
		}
		if (note.fact_id === null) return `Field: ${slotLabel(note.slot)}`;
		const gone = factsState === 'ready' && !acceptedIds.has(note.fact_id);
		return gone
			? `About fact #${note.fact_id}, which is no longer accepted: the guide no longer shows this note.`
			: `About fact #${note.fact_id}${note.fact_id === subject.id ? ' (this fact)' : ''}`;
	}

	const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;
</script>

<section class="flex flex-col gap-3 border-t border-line pt-4" aria-labelledby={`${uid}-h`}>
	<div class="flex flex-wrap items-center justify-between gap-2">
		<h3 id={`${uid}-h`} class="m-0 text-sm font-semibold">
			{#if collapsible}
				<button
					type="button"
					class="inline-flex cursor-pointer items-center gap-1 rounded-sm text-left focus-visible:outline-2 focus-visible:outline-focus"
					aria-expanded={open}
					aria-controls={open ? `${uid}-body` : undefined}
					onclick={() => (open = !open)}
				>
					<ChevronRightIcon class={`size-4 flex-none transition-transform ${open ? 'rotate-90' : ''}`} aria-hidden="true" />
					Considerations
				</button>
			{:else}
				Considerations
			{/if}
		</h3>
		{#if open && notesState === 'ready'}
			<span class="text-xs text-ink-muted">{plural(shown.length, 'published note')}</span>
		{/if}
	</div>

	{#if !open}
		<p class="m-0 text-xs text-ink-muted">Staff notes on calls, emails, visits and decisions about this field.</p>
	{:else}
		<div id={`${uid}-body`} class="flex flex-col gap-3">
			{#if notesState === 'loading' || notesState === 'idle'}
				<Skeleton class="h-12 rounded-sm" />
			{:else if notesState === 'failed'}
				<div class="flex flex-wrap items-center gap-2" role="status">
					<p class="m-0 text-sm text-ink-muted">{notesError}</p>
					<Button variant="outline" size="sm" onclick={() => loadNotes(slug)}>Try again</Button>
				</div>
			{:else if shown.length === 0}
				<p class="m-0 text-sm text-ink-muted">No published notes for this fact yet.</p>
			{:else}
				<ul class="m-0 flex list-none flex-col p-0" aria-label="Published notes">
					{#each shown as note (note.id)}
						<li class="flex min-w-0 flex-col gap-1 border-t border-line py-2 text-sm first:border-t-0 first:pt-0">
							<p class="m-0 flex flex-wrap items-center gap-1.5">
								<Badge variant="secondary">{kindLabel(note.kind)}</Badge>
								<Badge variant="outline" class={STANCE_CLS[note.stance]}>{stanceLabel(note.stance)}</Badge>
								<span class="font-mono text-xs text-ink-muted">#{note.id} · {note.date}{note.created_by ? ` · ${note.created_by}` : ''}</span>
							</p>
							<p class="m-0 break-words">{note.summary}</p>
							{#if note.channel}
								<p class="m-0 text-xs break-words text-ink-muted">{note.channel}</p>
							{/if}
							<p class="m-0 text-xs text-ink-muted">{about(note)}</p>
							<div>
								<Button
									variant="ghost"
									size="xs"
									aria-expanded={withdrawId === note.id}
									onclick={() => startWithdraw(note.id)}
								>
									Withdraw…<span class="sr-only"> note #{note.id}</span>
								</Button>
							</div>
							{#if withdrawId === note.id}
								<div class="flex flex-col gap-2 rounded-sm border border-line p-3">
									<label class="flex flex-col gap-1 text-sm font-medium" for={`${uid}-withdraw-${note.id}`}>
										Withdraw reason (required)
										<Textarea id={`${uid}-withdraw-${note.id}`} rows={2} bind:value={withdrawReason} />
									</label>
									<p class="m-0 text-xs text-ink-muted">The note leaves the public guide; the reason is audited.</p>
									<div class="flex flex-wrap gap-2">
										<Button
											variant="destructive"
											size="sm"
											disabled={withdrawing || withdrawReason.trim().length === 0}
											onclick={() => withdraw(note)}
										>
											{withdrawing ? 'Withdrawing…' : `Withdraw note #${note.id}`}
										</Button>
										<Button variant="ghost" size="sm" disabled={withdrawing} onclick={() => startWithdraw(note.id)}>Cancel</Button>
									</div>
									{#if withdrawError}
										<p class="m-0 text-sm font-semibold text-error" role="alert">{withdrawError}</p>
									{/if}
								</div>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}

			{#if publishedId !== null}
				<p class="m-0 text-sm font-semibold text-verified" role="status">Note #{publishedId} published.</p>
			{/if}

			{#if !formOpen}
				<div>
					<Button variant="outline" size="sm" aria-expanded={false} onclick={openForm}>
						<PlusIcon aria-hidden="true" />Add a consideration
					</Button>
				</div>
			{:else}
				<p id={`${uid}-line`} class="m-0 rounded-sm bg-surface px-3 py-2 text-xs">{PUBLIC_NOTE_LINE}</p>
				<form
					class="flex min-w-0 flex-col gap-3 rounded-md border border-line p-3"
					aria-label="Add a consideration"
					aria-describedby={`${uid}-line`}
					novalidate
					onsubmit={submit}
				>
					<div class="flex min-w-0 flex-col gap-3">
						<div class="flex min-w-0 flex-col gap-1">
							<span id={`${uid}-kind-label`} class="text-xs font-medium text-ink-muted">Kind</span>
							<Select.Root type="single" value={kind} onValueChange={setKind} disabled={saving}>
								<Select.Trigger id={`${uid}-kind`} class="h-9 w-full" aria-labelledby={`${uid}-kind-label ${uid}-kind`}>
									<span class="min-w-0 truncate">{kindLabel(kind)}</span>
								</Select.Trigger>
								<Select.Content>
									{#each NOTE_KINDS as choice (choice.value)}
										<Select.Item value={choice.value} label={choice.label} />
									{/each}
								</Select.Content>
							</Select.Root>
						</div>
						<div class="flex min-w-0 flex-col gap-1">
							<label for={`${uid}-when`} class="text-xs font-medium text-ink-muted">Date and time</label>
							<Input id={`${uid}-when`} type="datetime-local" class="h-9" max={latest} bind:value={when} disabled={saving} />
						</div>
					</div>

					<fieldset class="m-0 flex min-w-0 flex-col gap-1 border-0 p-0">
						<legend class="mb-1 text-xs font-medium text-ink-muted">Stance</legend>
						<div class="flex flex-wrap gap-x-4 gap-y-1 text-sm">
							{#each STANCES as choice (choice.value)}
								<label class="flex items-center gap-2">
									<input
										type="radio"
										class="accent-accent-strong dark:scheme-dark"
										name={`${uid}-stance`}
										value={choice.value}
										bind:group={stance}
										disabled={saving}
									/>
									{choice.label}
								</label>
							{/each}
						</div>
					</fieldset>

					{#if factKind}
						{#if isAccepted}
							<p class="m-0 flex flex-col gap-1 text-sm">
								<span class="text-xs font-medium text-ink-muted">About</span>
								<span>This fact (#{subject.id})</span>
							</p>
						{:else}
							<div class="flex min-w-0 flex-col gap-1">
								<span id={`${uid}-about-label`} class="text-xs font-medium text-ink-muted">About this accepted fact</span>
								{#if factsState === 'loading' || factsState === 'idle'}
									<Skeleton class="h-9 w-full rounded-md" />
								{:else if factsState === 'failed'}
									<div class="flex flex-wrap items-center gap-2" role="status">
										<p class="m-0 text-sm text-ink-muted">{factsError}</p>
										<Button variant="outline" size="sm" onclick={() => loadFacts(slug)}>Try again</Button>
									</div>
								{:else if anchors.length === 0}
									<p class="m-0 text-sm text-ink-muted">
										This office has no accepted facts yet. A call, email or visit note is about one of
										them: accept one first, or record what the pages do not say.
									</p>
								{:else}
									<Select.Root type="single" value={anchorId === null ? '' : String(anchorId)} onValueChange={setAnchor} disabled={saving}>
										<Select.Trigger id={`${uid}-about`} class="h-9 w-full" aria-labelledby={`${uid}-about-label ${uid}-about`}>
											<span class="min-w-0 truncate">{anchorFact ? `#${anchorFact.id} ${shorten(anchorFact.statement, 60)}` : 'Choose an accepted fact'}</span>
										</Select.Trigger>
										<Select.Content>
											{#each anchors as fact (fact.id)}
												<Select.Item value={String(fact.id)} label={`#${fact.id} ${shorten(fact.statement, 56)}`} />
											{/each}
										</Select.Content>
									</Select.Root>
								{/if}
							</div>
						{/if}
					{:else if kind === 'absence'}
						<div class="flex min-w-0 flex-col gap-1">
							<span id={`${uid}-field-label`} class="text-xs font-medium text-ink-muted">Field the pages do not state</span>
							<Select.Root type="single" value={fieldSlot} onValueChange={(v) => (fieldSlot = v)} disabled={saving}>
								<Select.Trigger id={`${uid}-field`} class="h-9 w-full" aria-labelledby={`${uid}-field-label ${uid}-field`}>
									<span class="min-w-0 truncate">{fieldSlot ? slotLabel(fieldSlot) : 'Choose a field'}</span>
								</Select.Trigger>
								<Select.Content>
									{#each fields as choice (choice.value)}
										<Select.Item value={choice.value} label={choice.label} />
									{/each}
								</Select.Content>
							</Select.Root>
						</div>
					{:else}
						<fieldset class="m-0 flex min-w-0 flex-col gap-1 border-0 p-0">
							<legend class="mb-1 text-xs font-medium text-ink-muted">Preferred fact</legend>
							{#if factsState === 'loading' || factsState === 'idle'}
								<Skeleton class="h-9 w-full rounded-md" />
							{:else if factsState === 'failed'}
								<p class="m-0 text-sm text-ink-muted" role="status">{factsError}</p>
							{:else if alternatives.length === 0}
								<p class="m-0 text-sm text-ink-muted">
									No published facts disagree here, so there is nothing to decide.
								</p>
							{:else}
								{#each alternatives as fact (fact.id)}
									<label class="flex min-w-0 items-start gap-2 text-sm">
										<input
											type="radio"
											class="mt-1 accent-accent-strong dark:scheme-dark"
											name={`${uid}-preferred`}
											value={fact.id}
											bind:group={preferredId}
											disabled={saving}
										/>
										<span class="min-w-0 break-words"><span class="font-mono text-xs text-ink-muted">#{fact.id}</span> {fact.statement}</span>
									</label>
								{/each}
							{/if}
						</fieldset>
						<fieldset class="m-0 flex min-w-0 flex-col gap-1 border-0 p-0">
							<legend class="mb-1 text-xs font-medium text-ink-muted">Rests on these published notes</legend>
							{#if basis.length === 0}
								<p class="m-0 text-sm text-ink-muted">
									No published call, email, visit or absence notes for this office yet. Record what
									the office said first.
								</p>
							{:else}
								{#each basis as note (note.id)}
									<label class="flex min-w-0 items-start gap-2 text-sm">
										<input
											type="checkbox"
											class="mt-1 accent-accent-strong dark:scheme-dark"
											checked={supports.includes(note.id)}
											onchange={(e) => toggleSupport(note.id, e.currentTarget.checked)}
											disabled={saving}
										/>
										<span class="min-w-0 break-words">
											<span class="font-mono text-xs text-ink-muted">#{note.id} · {kindLabel(note.kind)} · {note.date}</span>
											{shorten(note.summary, 120)}
										</span>
									</label>
								{/each}
							{/if}
						</fieldset>
					{/if}

					{#if kind !== 'decision'}
						<div class="flex min-w-0 flex-col gap-1">
							<label for={`${uid}-channel`} class="text-xs font-medium text-ink-muted">{CHANNEL_LABEL[kind]}</label>
							<Input
								id={`${uid}-channel`}
								class="h-9"
								bind:value={channel}
								oninput={() => (channelEdited = true)}
								autocomplete="off"
								aria-describedby={`${uid}-channel-hint`}
								disabled={saving}
							/>
							<p id={`${uid}-channel-hint`} class={`m-0 text-xs ${charCount(channel) > CHANNEL_MAX ? 'text-error' : 'text-ink-muted'}`}>
								{CHANNEL_HINT[kind]}
							</p>
						</div>
					{/if}

					<div class="flex min-w-0 flex-col gap-1">
						<label for={`${uid}-summary`} class="text-xs font-medium text-ink-muted">Summary</label>
						<Textarea id={`${uid}-summary`} rows={3} bind:value={summary} aria-describedby={`${uid}-count`} disabled={saving} />
						<p id={`${uid}-count`} class={`m-0 text-xs ${count > SUMMARY_MAX ? 'text-error' : 'text-ink-muted'}`}>
							{count}/{SUMMARY_MAX} characters{count < SUMMARY_MIN ? ` · at least ${SUMMARY_MIN}` : ''}
						</p>
					</div>

					<p class="m-0 text-xs text-ink-muted">
						Field on the guide: {noteSlot ? slotLabel(noteSlot) : 'none yet'}
					</p>

					<div class="flex flex-wrap items-center gap-2">
						<Button type="submit" disabled={saving || problem !== null}>
							{saving ? 'Publishing…' : 'Publish note'}
						</Button>
						<Button type="button" variant="ghost" disabled={saving} onclick={() => (formOpen = false)}>Cancel</Button>
					</div>
					{#if formError}
						<p class="m-0 text-sm font-semibold text-error" role="alert">{formError}</p>
					{:else if problem}
						<p class="m-0 text-xs text-ink-muted">{problem}</p>
					{/if}
				</form>
			{/if}
		</div>
	{/if}
</section>
