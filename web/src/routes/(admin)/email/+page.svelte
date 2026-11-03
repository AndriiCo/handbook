<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { fade, fly } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import { toast } from 'svelte-sonner';
	import { resolve } from '$app/paths';
	import Heading from '#lib/components/brand/Heading.svelte';
	import Notice from '#lib/public-ui/Notice.svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import AuthorityPicker from '#lib/admin/AuthorityPicker.svelte';
	import QuestionMeta from '#lib/admin/QuestionMeta.svelte';
	import EmailStatusBadge from '#lib/admin/EmailStatusBadge.svelte';
	import { OpenQuestionsState } from '#lib/admin/first-source.svelte.js';
	import type { AuthorityOption } from '#lib/admin/authorities.js';
	import { ago, dateTime } from '#lib/admin/format.js';
	import {
		MAX_BODY,
		MAX_EMAIL_QUESTIONS,
		MAX_SUBJECT,
		OutreachError,
		TOP_N,
		addContact,
		discardEmail,
		draftBody,
		draftEmail,
		editProblem,
		emailContacts,
		fetchEmails,
		isDirty,
		modelLabel,
		parseEmailQuery,
		questionProblem,
		saveEmail,
		sendEmail,
		sentInLastDay,
		validEmail,
		type Contact,
		type EmailEdit,
		type OpenQuestion,
		type OutreachEmail,
		type QuestionSet
	} from '#lib/admin/outreach-client.js';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import SendIcon from '@lucide/svelte/icons/send';
	import SaveIcon from '@lucide/svelte/icons/save';
	import TrashIcon from '@lucide/svelte/icons/trash-2';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import MailPlusIcon from '@lucide/svelte/icons/mail-plus';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import XIcon from '@lucide/svelte/icons/x';

	// Email an office (First source). Muse Spark writes only the
	// subject, opening and closing; the server lists the ticked
	// questions word for word. A person reads the final text, edits
	// it, and presses Send behind a confirm that names the recipient
	// and subject — nothing leaves on its own. Live: an approval in
	// /review re-reads the open questions over `authority:{slug}`.
	// No polling.

	type Item = { text: string; q: OpenQuestion | null };

	const open = new OpenQuestionsState(onQuestions);

	let slug = $state('');
	let names = $state<Record<string, string>>({});
	let picker = $state<AuthorityPicker | null>(null);
	let reduceMotion = $state(false);
	let nowSec = $state(Math.floor(Date.now() / 1000));

	// Recipient
	let contacts = $state<Contact[]>([]);
	let recipient = $state('');
	let addOpen = $state(false);
	let newAddr = $state('');
	let newLabel = $state('');
	let adding = $state(false);
	let addError = $state<string | null>(null);

	// Questions
	let picked = $state<string[]>([]);
	let custom = $state<string[]>([]);
	let customText = $state('');
	let customError = $state<string | null>(null);
	let prefill: { slug: string; questions: string[] } | null = null;
	let seededFor = '';

	// Draft and the open email (the hero)
	let drafting = $state(false);
	let draftError = $state<string | null>(null);
	let current = $state<OutreachEmail | null>(null);
	let edit = $state<EmailEdit>({ to: '', subject: '', body: '' });
	let busy = $state<'save' | 'send' | 'discard' | null>(null);
	let editError = $state<string | null>(null);
	let errorTitle = $state('Not saved.');
	let confirmOpen = $state(false);
	let hero = $state<HTMLElement | null>(null);

	// Recent outreach
	let emails = $state<OutreachEmail[]>([]);
	let dailyCap = $state(20);
	let listState = $state<'loading' | 'ready' | 'failed'>('loading');
	let listError = $state<string | null>(null);

	const motion = $derived(reduceMotion ? 0 : 200);
	const nameOf = (s: string) => names[s] ?? (open.set && open.set.slug === s ? open.set.name : s);
	const items = $derived<Item[]>([
		...custom.map((text) => ({ text, q: null })),
		...open.questions.map((q) => ({ text: q.question, q }))
	]);
	const ordered = $derived(items.filter((i) => picked.includes(i.text)).map((i) => i.text));
	const atCap = $derived(picked.length >= MAX_EMAIL_QUESTIONS);
	const ready = $derived(draftBody(slug, recipient, ordered));
	const dirty = $derived(isDirty(current, edit));
	const editable = $derived(current?.status === 'draft');
	const sentToday = $derived(sentInLastDay(emails, Math.max(nowSec, ...emails.map((e) => e.sent_at ?? 0))));
	const openDraft = $derived(
		emails.find((e) => e.slug === slug && e.status === 'draft' && e.id !== current?.id) ?? null
	);
	const verbatim = $derived(
		(current?.questions ?? []).map((q) => ({ q, kept: edit.body.includes(q) }))
	);
	const keptCount = $derived(verbatim.filter((v) => v.kept).length);

	function syncUrl() {
		const params = new URLSearchParams();
		if (slug) params.set('authority', slug);
		if (current) params.set('email', String(current.id));
		const query = params.toString();
		history.replaceState(history.state, '', query ? `?${query}` : location.pathname);
	}

	function pick(next: string) {
		// The picker's bind:value already moved `slug`; the questions
		// state remembers which office it last read.
		slug = next;
		if (next === open.slug && open.phase !== 'failed') return;
		contacts = [];
		recipient = '';
		picked = [];
		custom = [];
		seededFor = '';
		addOpen = false;
		addError = null;
		draftError = null;
		open.select(next);
		syncUrl();
	}

	/** Each read of the open questions (first load and live refreshes). */
	function onQuestions(set: QuestionSet) {
		if (set.slug !== slug) return;
		const mail = emailContacts(set.contacts);
		contacts = mail;
		if (!recipient || !mail.some((c) => c.value === recipient)) recipient = mail[0]?.value ?? recipient;
		if (mail.length === 0) addOpen = true;
		const ranked = set.questions.map((q) => q.question);
		if (seededFor !== set.slug) {
			// First read for this office: the /calls hand-off wins,
			// else the strongest six are ticked.
			seededFor = set.slug;
			if (prefill && prefill.slug === set.slug && prefill.questions.length > 0) {
				custom = prefill.questions.filter((q) => !ranked.includes(q));
				picked = prefill.questions.slice(0, MAX_EMAIL_QUESTIONS);
			} else {
				picked = ranked.slice(0, TOP_N);
			}
			prefill = null;
			return;
		}
		// Live refresh: an answered question leaves the list and its tick.
		const keep = new Set([...custom, ...ranked]);
		const before = picked.length;
		picked = picked.filter((q) => keep.has(q));
		if (picked.length < before) {
			toast.info(`A fact was approved: ${before - picked.length} answered question${before - picked.length === 1 ? '' : 's'} left the email.`);
		}
	}

	function toggle(text: string, on: boolean) {
		if (on) {
			if (!picked.includes(text) && picked.length < MAX_EMAIL_QUESTIONS) picked = [...picked, text];
		} else picked = picked.filter((q) => q !== text);
	}

	function tickTop() {
		picked = items.slice(0, TOP_N).map((i) => i.text);
	}

	function addCustom(event: SubmitEvent) {
		event.preventDefault();
		const text = customText.trim();
		customError = questionProblem(text);
		if (customError) return;
		if (items.some((i) => i.text === text)) {
			customError = 'That question is already on the list.';
			return;
		}
		custom = [...custom, text];
		if (picked.length < MAX_EMAIL_QUESTIONS) picked = [...picked, text];
		customText = '';
	}

	async function saveAddress(event: SubmitEvent) {
		event.preventDefault();
		addError = null;
		const value = newAddr.trim().toLowerCase();
		if (!validEmail(value)) {
			addError = 'Enter one email address, like apostille@office.example.';
			return;
		}
		adding = true;
		try {
			const out = await addContact(slug, { kind: 'email', value, details: newLabel });
			contacts = emailContacts(out.contacts);
			recipient = value;
			newAddr = '';
			newLabel = '';
			addOpen = false;
			toast.success(`Saved ${value} to ${nameOf(slug)}'s contacts.`);
		} catch (err) {
			addError = err instanceof Error ? err.message : 'The address was not saved.';
		} finally {
			adding = false;
		}
	}

	async function openEmail(email: OutreachEmail, scroll = true) {
		// Switching emails saves the open draft's edits first.
		if (current && current.id !== email.id && dirty && editable && !(await save(true))) return;
		current = email;
		edit = { to: email.to, subject: email.subject, body: email.body };
		editError = null;
		syncUrl();
		if (scroll) revealHero();
	}

	/** Close the hero; unsaved edits are saved first (never lost). */
	async function closeEmail() {
		if (dirty && editable && !(await save(true))) return;
		current = null;
		editError = null;
		syncUrl();
	}

	function upsert(email: OutreachEmail) {
		emails = emails.some((e) => e.id === email.id)
			? emails.map((e) => (e.id === email.id ? email : e))
			: [email, ...emails];
		if (current?.id === email.id) current = email;
	}

	async function draft() {
		draftError = null;
		const check = draftBody(slug, recipient, ordered);
		if (!check.ok) {
			draftError = check.error;
			return;
		}
		drafting = true;
		current = null;
		syncUrl();
		revealHero();
		try {
			const email = await draftEmail(check.body);
			upsert(email);
			openEmail(email, false);
			nowSec = Math.floor(Date.now() / 1000);
			if (email.model === 'template') {
				toast.info(`Draft #${email.id} used the fixed template${email.note ? `: ${email.note}` : '.'}`);
			} else {
				toast.success(`Draft #${email.id} is ready. Read it before you send.`);
			}
		} catch (err) {
			draftError = err instanceof Error ? err.message : 'Drafting failed.';
			toast.error(draftError);
		} finally {
			drafting = false;
		}
	}

	/** Bring the hero into view only when it is off screen (phones). */
	function revealHero() {
		void tick().then(() => {
			const top = hero?.getBoundingClientRect().top ?? 0;
			if (top < 0 || top > window.innerHeight * 0.6) {
				hero?.scrollIntoView({ behavior: reduceMotion ? 'auto' : 'smooth', block: 'start' });
			}
		});
	}

	async function save(quiet = false): Promise<boolean> {
		if (!current) return false;
		errorTitle = 'Not saved.';
		editError = editProblem(edit);
		if (editError) return false;
		const before = current;
		const id = before.id;
		const optimistic = { ...before, to: edit.to.trim().toLowerCase(), subject: edit.subject.trim(), body: edit.body.trim() };
		busy = 'save';
		upsert(optimistic);
		try {
			const saved = await saveEmail(id, edit);
			upsert(saved);
			if (current?.id === id) edit = { to: saved.to, subject: saved.subject, body: saved.body };
			if (!quiet) toast.success(`Draft #${id} saved.`);
			return true;
		} catch (err) {
			upsert(before);
			editError = err instanceof Error ? err.message : 'Saving failed.';
			toast.error(editError);
			if (err instanceof OutreachError && err.status === 409) void loadList(true);
			return false;
		} finally {
			busy = null;
		}
	}

	function askSend() {
		if (!current) return;
		errorTitle = 'Not sent.';
		editError = editProblem(edit);
		if (editError) return;
		confirmOpen = true;
	}

	async function send() {
		if (!current) return;
		confirmOpen = false;
		if (dirty && !(await save(true))) return;
		const before = current;
		const id = before.id;
		busy = 'send';
		upsert({ ...before, status: 'sending' });
		try {
			const sent = await sendEmail(id);
			upsert(sent);
			nowSec = Math.floor(Date.now() / 1000);
			toast.success(`Sent to ${sent.to}. The office's reply threads back to this email.`);
		} catch (err) {
			const failedRow = err instanceof OutreachError ? err.email : null;
			upsert(failedRow ?? before);
			const msg = err instanceof Error ? err.message : 'Sending failed.';
			errorTitle = 'Not sent.';
			editError = failedRow ? null : msg;
			toast.error(
				err instanceof OutreachError && err.status === 503 ? `Sending is off on this host. ${msg}` : msg
			);
			if (err instanceof OutreachError && err.status === 409) void loadList(true);
		} finally {
			busy = null;
		}
	}

	async function discard() {
		if (!current) return;
		const before = current;
		const id = before.id;
		busy = 'discard';
		upsert({ ...before, status: 'discarded' });
		try {
			const gone = await discardEmail(id);
			upsert(gone);
			closeEmail();
			toast.success(`Draft #${id} discarded.`);
		} catch (err) {
			upsert(before);
			toast.error(err instanceof Error ? err.message : 'Discarding failed.');
		} finally {
			busy = null;
		}
	}

	async function loadList(quiet = false) {
		if (!quiet) {
			listState = 'loading';
			listError = null;
		}
		try {
			const out = await fetchEmails();
			emails = out.emails;
			dailyCap = out.dailyCap;
			nowSec = Math.floor(Date.now() / 1000);
			listState = 'ready';
			listError = null;
			if (current) {
				const fresh = emails.find((e) => e.id === current?.id);
				if (fresh && fresh.status !== current.status) openEmail(fresh, false);
			}
		} catch (err) {
			listError = err instanceof Error ? err.message : 'Outreach emails failed to load.';
			if (!quiet || listState !== 'ready') listState = 'failed';
		}
	}

	function onLoaded(rows: AuthorityOption[]) {
		names = Object.fromEntries(rows.map((r) => [r.slug, r.name]));
	}

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		const query = parseEmailQuery(location.search);
		const wantedId = Number(new URLSearchParams(location.search).get('email'));
		if (query.authority) {
			prefill = { slug: query.authority, questions: query.questions };
			pick(query.authority);
		}
		void loadList().then(() => {
			const email = emails.find((e) => e.id === wantedId);
			if (email) openEmail(email, false);
		});
		return () => open.dispose();
	});

	const fieldCls = 'h-9 text-base md:text-sm';
</script>

<svelte:head>
	<title>Email — Admin — Apostille Handbook</title>
</svelte:head>

<div class="mt-6 flex flex-col gap-6 pb-12">
	<header class="flex flex-wrap items-end justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<Heading level={1} eyebrow="First source">Email an office</Heading>
			<p class="m-0 max-w-prose text-sm text-ink-muted">
				Muse Spark writes the subject, opening and closing; your questions go in word for word.
				Nothing leaves until a person reads the final text and presses Send.
			</p>
		</div>
		{#if listState === 'ready'}
			<span
				class="inline-flex h-8 items-center gap-2 rounded-full border border-line px-3 text-sm text-ink-muted tabular-nums"
				title="Outreach emails sent in the last 24 hours, against the daily cap"
			>
				<SendIcon class="size-4" aria-hidden="true" />
				{sentToday} of {dailyCap} sent today
			</span>
		{/if}
	</header>

	<div class="grid items-start gap-6 xl:grid-cols-5">
		<!-- Compose: office, recipient, questions. -->
		<Card.Root class="min-w-0 xl:col-span-2">
			<Card.Header>
				<Card.Title><h2 class="m-0 text-h3 font-semibold">Compose</h2></Card.Title>
				<Card.Description class="text-ink-muted">Pick the office, who reads it, and what to ask.</Card.Description>
			</Card.Header>
			<Card.Content class="flex flex-col gap-6">
				<AuthorityPicker
					bind:this={picker}
					bind:value={slug}
					id="email-authority"
					label="Office"
					placeholder="Choose an office"
					showContacts
					{onLoaded}
					onChange={pick}
				/>

				{#if openDraft}
					<div
						class="flex flex-wrap items-center justify-between gap-2 rounded-md border border-changed bg-changed-bg px-3 py-2 text-sm text-changed"
						in:fade={{ duration: motion }}
					>
						<span>Draft #{openDraft.id} for this office is still open.</span>
						<Button variant="outline" size="sm" onclick={() => openEmail(openDraft)}>
							<PencilIcon aria-hidden="true" />
							Open it
						</Button>
					</div>
				{/if}

				{#if slug}
					<fieldset class="m-0 flex min-w-0 flex-col gap-2 border-0 p-0">
						<legend class="mb-2 p-0 text-xs font-medium text-ink-muted">To</legend>
						{#if open.phase === 'loading'}
							<Skeleton class="h-12 w-full rounded-md" />
						{:else}
							{#each contacts as c (c.id)}
								<label
									class="flex min-w-0 cursor-pointer items-center gap-3 rounded-md border border-line px-3 py-2 transition-colors duration-150 hover:bg-muted/50 has-checked:border-accent-strong has-checked:bg-surface"
								>
									<input
										type="radio"
										name="recipient"
										class="size-4 flex-none accent-accent-strong"
										value={c.value}
										bind:group={recipient}
									/>
									<span class="flex min-w-0 flex-col">
										<span class="font-mono text-xs break-all text-ink sm:text-sm">{c.value}</span>
										{#if c.details}<span class="truncate text-xs text-ink-muted">{c.details}</span>{/if}
									</span>
								</label>
							{/each}
							{#if contacts.length === 0 && open.phase === 'ready'}
								<p class="m-0 text-sm text-ink-muted">No email address on file for this office yet. Add the one its website publishes.</p>
							{/if}
							{#if addOpen}
								<form class="flex flex-col gap-2 rounded-md border border-dashed border-line p-3" onsubmit={saveAddress} in:fly={{ y: -4, duration: motion }}>
									<label class="flex flex-col gap-1" for="new-addr">
										<span class="text-xs font-medium text-ink-muted">Office email address</span>
										<Input id="new-addr" type="email" class={fieldCls} bind:value={newAddr} placeholder="apostille@office.example" autocomplete="off" />
									</label>
									<label class="flex flex-col gap-1" for="new-label">
										<span class="text-xs font-medium text-ink-muted">Label (optional)</span>
										<Input id="new-label" class={fieldCls} bind:value={newLabel} placeholder="Apostille desk" autocomplete="off" />
									</label>
									{#if addError}<p class="m-0 text-sm text-error" role="alert">{addError}</p>{/if}
									<div class="flex flex-wrap gap-2">
										<Button type="submit" variant="outline" disabled={adding}>
											{#if adding}<LoaderCircleIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<MailPlusIcon aria-hidden="true" />{/if}
											Save address
										</Button>
										{#if contacts.length > 0}
											<Button type="button" variant="ghost" onclick={() => (addOpen = false)}>Cancel</Button>
										{/if}
									</div>
								</form>
							{:else}
								<Button variant="ghost" class="self-start" onclick={() => (addOpen = true)}>
									<PlusIcon aria-hidden="true" />
									Add an address
								</Button>
							{/if}
						{/if}
					</fieldset>

					<fieldset class="m-0 flex min-w-0 flex-col gap-2 border-0 p-0">
						<legend class="mb-2 flex w-full items-baseline justify-between gap-2 p-0 text-xs font-medium text-ink-muted">
							Questions
							<span class={`font-mono tabular-nums ${atCap ? 'text-changed' : ''}`}>{picked.length} of {MAX_EMAIL_QUESTIONS} ticked</span>
						</legend>
						{#if open.phase === 'loading'}
							{#each [0, 1, 2] as i (i)}
								<div class="flex gap-3 rounded-md border border-line p-3">
									<Skeleton class="size-4 rounded-xs" />
									<div class="flex flex-1 flex-col gap-2">
										<Skeleton class="h-5 w-36 rounded-full" />
										<Skeleton class="h-4 w-full rounded-xs" />
									</div>
								</div>
							{/each}
						{:else if open.phase === 'failed'}
							<Notice variant="error" heading="Open questions could not load.">
								<p class="m-0">{open.error}</p>
								<p class="mt-3 mb-0"><Button variant="outline" onclick={() => open.reload()}>Try again</Button></p>
							</Notice>
						{:else}
							{#if items.length === 0}
								<p class="m-0 text-sm text-ink-muted">
									Nothing open for this office: every core topic has a current, accepted fact. Write your own question below.
								</p>
							{:else}
								<div class="flex flex-wrap gap-2">
									<Button variant="outline" size="sm" onclick={tickTop}>Tick the top {Math.min(TOP_N, items.length)}</Button>
									{#if picked.length > 0}
										<Button variant="ghost" size="sm" onclick={() => (picked = [])}>Clear ticks</Button>
									{/if}
									{#if open.live}
										<span class="ml-auto inline-flex items-center gap-1 text-xs text-ink-muted" title="Approvals in Review update this list live">
											<span class="size-2 rounded-full bg-verified" aria-hidden="true"></span>
											Live
										</span>
									{/if}
								</div>
							{/if}
							<ul class="m-0 flex list-none flex-col gap-2 p-0">
								{#each items as item (item.text)}
									{@const on = picked.includes(item.text)}
									<li animate:flip={{ duration: motion }} in:fly={{ y: -4, duration: motion }}>
										<label
											class={`flex min-w-0 items-start gap-3 rounded-md border border-line p-3 transition-colors duration-150 has-checked:border-accent-strong has-checked:bg-surface ${!on && atCap ? 'opacity-60' : 'cursor-pointer hover:bg-muted/50'}`}
										>
											<input
												type="checkbox"
												class="mt-0.5 size-4 flex-none accent-accent-strong"
												checked={on}
												disabled={!on && atCap}
												onchange={(e) => toggle(item.text, e.currentTarget.checked)}
											/>
											<span class="flex min-w-0 flex-1 flex-col gap-1.5">
												{#if item.q}
													<QuestionMeta question={item.q} />
												{:else}
													<span class="inline-flex h-5 w-fit items-center rounded-full border border-line bg-surface-2 px-2 text-xs font-medium text-ink">Your question</span>
												{/if}
												<span class="text-sm text-ink">{item.text}</span>
												{#if item.q?.why}<span class="text-xs text-ink-muted">{item.q.why}</span>{/if}
											</span>
										</label>
									</li>
								{/each}
							</ul>
							<form class="flex flex-col gap-1" onsubmit={addCustom}>
								<label class="text-xs font-medium text-ink-muted" for="custom-q">Add your own question</label>
								<div class="flex gap-2">
									<Input id="custom-q" class={fieldCls} bind:value={customText} placeholder="Is the request form still revision B?" autocomplete="off" />
									<Button type="submit" variant="outline" size="lg" aria-label="Add the question">
										<PlusIcon aria-hidden="true" />
										<span class="hidden sm:inline">Add</span>
									</Button>
								</div>
								{#if customError}<p class="m-0 text-sm text-error" role="alert">{customError}</p>{/if}
							</form>
						{/if}
					</fieldset>
				{:else}
					<p class="m-0 text-sm text-ink-muted">Its email contacts and open questions load here once you pick it.</p>
				{/if}
			</Card.Content>
		</Card.Root>

		<!-- The hero: the exact text a person approves. -->
		<section
			class={`min-w-0 scroll-mt-20 xl:col-span-3 ${current || drafting ? 'order-first xl:order-none' : ''}`}
			aria-labelledby="review-heading"
			bind:this={hero}
		>
			{#if drafting}
				<Card.Root class="shadow-md" aria-busy="true">
					<Card.Header>
						<Card.Title class="flex items-center gap-2">
							<h2 id="review-heading" class="m-0 text-h3 font-semibold">Drafting</h2>
							<LoaderCircleIcon class="size-4 text-accent-strong motion-safe:animate-spin" aria-hidden="true" />
						</Card.Title>
						<Card.Description class="text-ink-muted" role="status">
							Muse Spark is writing the subject, opening and closing for {nameOf(slug)}. Your {ordered.length}
							question{ordered.length === 1 ? '' : 's'} go in word for word. This usually takes a few seconds.
						</Card.Description>
					</Card.Header>
					<Card.Content class="flex flex-col gap-3">
						<Skeleton class="h-9 w-full rounded-sm" />
						<Skeleton class="h-9 w-full rounded-sm" />
						<div class="flex flex-col gap-2 rounded-lg border border-line p-4">
							<Skeleton class="h-4 w-1/3 rounded-xs" />
							<Skeleton class="h-4 w-full rounded-xs" />
							<Skeleton class="h-4 w-5/6 rounded-xs" />
							{#each ordered as q (q)}
								<p class="m-0 text-sm text-ink-muted">{q}</p>
							{/each}
							<Skeleton class="h-4 w-2/3 rounded-xs" />
						</div>
					</Card.Content>
				</Card.Root>
			{:else if current}
				{@const email = current}
				<div in:fly={{ y: 8, duration: motion }}>
					<Card.Root class="shadow-md">
						<Card.Header>
							<Card.Title class="flex flex-wrap items-center gap-2">
								<h2 id="review-heading" class="m-0 text-h3 font-semibold">
									{editable ? 'Review before sending' : `Email #${email.id}`}
								</h2>
								<EmailStatusBadge status={email.status} />
							</Card.Title>
							<Card.Description class="flex flex-wrap items-center gap-x-2 gap-y-1 text-ink-muted">
								{#if email.model === 'template'}
									<span class="inline-flex h-6 items-center gap-1.5 rounded-full border border-line bg-surface px-2 text-xs font-medium text-ink-muted" title={email.note}>
										<FileTextIcon class="size-3" aria-hidden="true" />
										Template
									</span>
								{:else}
									<span class="inline-flex h-6 items-center gap-1.5 rounded-full border border-line bg-surface-2 px-2 font-mono text-xs font-medium text-accent-strong" title="Muse Spark wrote the subject, opening and closing">
										<SparklesIcon class="size-3" aria-hidden="true" />
										{email.model}
									</span>
								{/if}
								<span>
									{editable ? `Draft #${email.id}` : modelLabel(email.model)} to {nameOf(email.slug)} · drafted
									<time datetime={new Date(email.drafted_at * 1000).toISOString()} title={dateTime(email.drafted_at)}>
										{ago(email.drafted_at, Math.max(nowSec, email.drafted_at))}
									</time>
									{#if email.sent_at}
										· sent <time datetime={new Date(email.sent_at * 1000).toISOString()}>{dateTime(email.sent_at)}</time>
									{/if}
								</span>
							</Card.Description>
							<Card.Action>
								<Button variant="ghost" size="icon-sm" aria-label="Close this email" onclick={closeEmail}>
									<XIcon aria-hidden="true" />
								</Button>
							</Card.Action>
						</Card.Header>
						<Card.Content class="flex flex-col gap-4">
							{#if email.model === 'template' && email.note}
								<Notice variant="info" heading="Fixed template, not a model draft.">
									<p class="m-0">{email.note}</p>
								</Notice>
							{/if}
							{#if email.status === 'failed'}
								<Notice variant="error" heading="The send failed.">
									<p class="m-0">{email.error || 'The mailer refused the message.'} Discard it and draft again.</p>
								</Notice>
							{/if}
							{#if email.status === 'replied' && email.reply_capture_id}
								<Notice variant="success" heading="The office replied.">
									<p class="m-0">
										The reply is capture #{email.reply_capture_id}.
										<a href={resolve('/(admin)/trace')}>Draft facts from it in Trace</a>.
									</p>
								</Notice>
							{/if}

							{#if editable}
								<div class="overflow-hidden rounded-lg border border-line bg-bg transition-shadow duration-150 focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50">
									<div class="flex flex-col border-b border-line px-4 pt-2 sm:flex-row sm:items-center sm:gap-3 sm:py-1">
										<label for="mail-to" class="flex-none text-xs font-medium text-ink-muted sm:w-14">To</label>
										<Input id="mail-to" type="email" class="h-9 border-0 px-0 font-mono text-sm shadow-none focus-visible:ring-0 dark:bg-transparent" bind:value={edit.to} autocomplete="off" />
									</div>
									<div class="flex flex-col border-b border-line px-4 pt-2 sm:flex-row sm:items-start sm:gap-3 sm:py-1">
										<label for="mail-subject" class="flex-none text-xs font-medium text-ink-muted sm:w-14 sm:pt-2.5">Subject</label>
										<Textarea
											id="mail-subject"
											rows={1}
											class="min-h-9 resize-none rounded-none border-0 px-0 py-2 text-base font-medium shadow-none focus-visible:ring-0 md:text-base dark:bg-transparent"
											bind:value={edit.subject}
											maxlength={MAX_SUBJECT}
											onkeydown={(e) => {
												if (e.key === 'Enter') e.preventDefault();
											}}
											oninput={() => (edit.subject = edit.subject.replace(/[\r\n]+/g, ' '))}
										/>
									</div>
									<label for="mail-body" class="sr-only">Body</label>
									<Textarea
										id="mail-body"
										class="min-h-96 rounded-none border-0 px-4 py-4 text-base leading-7 shadow-none focus-visible:ring-0 md:text-base dark:bg-transparent"
										bind:value={edit.body}
										maxlength={MAX_BODY}
									/>
								</div>
							{:else}
								<div class="overflow-hidden rounded-lg border border-line bg-bg">
									<dl class="m-0">
										<div class="flex flex-col gap-0.5 border-b border-line px-4 py-2 sm:flex-row sm:items-baseline sm:gap-3">
											<dt class="flex-none text-xs font-medium text-ink-muted sm:w-14">To</dt>
											<dd class="m-0 min-w-0 font-mono text-sm break-all">{email.to}</dd>
										</div>
										<div class="flex flex-col gap-0.5 border-b border-line px-4 py-2 sm:flex-row sm:items-baseline sm:gap-3">
											<dt class="flex-none text-xs font-medium text-ink-muted sm:w-14">Subject</dt>
											<dd class="m-0 min-w-0 font-medium">{email.subject}</dd>
										</div>
									</dl>
									<p class="m-0 px-4 py-4 text-base leading-7 whitespace-pre-wrap">{email.body}</p>
								</div>
							{/if}

							<div class="flex flex-col gap-2">
								<p class="m-0 flex flex-wrap items-center gap-2 text-xs text-ink-muted">
									{#if keptCount === verbatim.length}
										<CircleCheckIcon class="size-4 text-verified" aria-hidden="true" />
									{:else}
										<TriangleAlertIcon class="size-4 text-changed" aria-hidden="true" />
									{/if}
									{keptCount} of {verbatim.length} question{verbatim.length === 1 ? '' : 's'} still word for word in the body
									{#if editable}<span class="ml-auto font-mono tabular-nums">{edit.body.length} / {MAX_BODY}</span>{/if}
								</p>
								<ol class="m-0 flex list-none flex-col gap-1 p-0">
									{#each verbatim as v, i (v.q)}
										<li class="flex items-start gap-2 text-xs">
											<span class={`mt-0.5 size-2 flex-none rounded-full ${v.kept ? 'bg-verified' : 'bg-changed'}`} aria-hidden="true"></span>
											<span class={v.kept ? 'text-ink-muted' : 'text-changed'}>
												{i + 1}. {v.q}{#if !v.kept}<span class="font-medium"> — edited</span>{/if}
											</span>
										</li>
									{/each}
								</ol>
							</div>

							{#if editError}
								<Notice variant="error" heading={errorTitle}>
									<p class="m-0">{editError}</p>
								</Notice>
							{/if}
						</Card.Content>
						{#if editable || email.status === 'failed'}
							<Card.Footer class="flex flex-col-reverse gap-2 sm:flex-row sm:items-center sm:justify-between">
								<Button variant="destructive" size="lg" disabled={busy !== null} onclick={discard}>
									{#if busy === 'discard'}<LoaderCircleIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<TrashIcon aria-hidden="true" />{/if}
									Discard
								</Button>
								{#if editable}
									<div class="flex flex-col-reverse gap-2 sm:flex-row">
										<Button variant="outline" size="lg" disabled={!dirty || busy !== null} onclick={() => save()}>
											{#if busy === 'save'}<LoaderCircleIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<SaveIcon aria-hidden="true" />{/if}
											{dirty ? 'Save edits' : 'Saved'}
										</Button>
										<Button size="lg" disabled={busy !== null} onclick={askSend}>
											{#if busy === 'send'}<LoaderCircleIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<SendIcon aria-hidden="true" />{/if}
											Send…
										</Button>
									</div>
								{/if}
							</Card.Footer>
						{/if}
					</Card.Root>
				</div>
			{:else if slug && ordered.length > 0}
				<div class="flex flex-col gap-4 rounded-xl border border-dashed border-line bg-card p-6" in:fade={{ duration: motion }}>
					<div class="flex flex-col gap-1">
						<h2 id="review-heading" class="m-0 text-h3 font-semibold">Ready to draft</h2>
						<p class="m-0 max-w-prose text-sm text-ink-muted">
							To <span class="font-mono break-all">{recipient || 'an address you add'}</span> at {nameOf(slug)}. Muse Spark
							writes the subject, opening and closing around these {ordered.length} question{ordered.length === 1 ? '' : 's'},
							then the draft opens here for you to read and edit.
						</p>
					</div>
					<ol class="m-0 flex list-none flex-col gap-2 rounded-lg bg-surface p-4">
						{#each ordered as q, i (q)}
							<li class="flex gap-2 text-sm text-ink" animate:flip={{ duration: motion }}>
								<span class="w-5 flex-none text-right font-mono text-ink-muted tabular-nums">{i + 1}.</span>
								<span class="min-w-0">{q}</span>
							</li>
						{/each}
					</ol>
					{#if draftError}
						<Notice variant="error" heading="Not drafted.">
							<p class="m-0">{draftError}</p>
						</Notice>
					{/if}
					<div class="flex flex-wrap items-center gap-3">
						<Button size="lg" disabled={!ready.ok || drafting} onclick={draft}>
							<SparklesIcon aria-hidden="true" />
							Draft with Muse Spark
						</Button>
						{#if !ready.ok && open.phase === 'ready'}
							<span class="text-xs text-ink-muted">{ready.error}</span>
						{/if}
					</div>
				</div>
			{:else}
				<div class="flex flex-col items-center gap-3 rounded-xl border border-dashed border-line bg-card px-6 py-16 text-center">
					<SparklesIcon class="size-8 text-ink-muted" aria-hidden="true" />
					<h2 id="review-heading" class="m-0 text-h3 font-semibold">The draft opens here for review</h2>
					<p class="m-0 max-w-prose text-sm text-ink-muted">
						You read and edit the exact text the office receives: recipient, subject and every line of the
						body. Send asks once more, naming the recipient and subject.
					</p>
					{#if slug}
						<Button variant="outline" onclick={tickTop}>Tick the top questions</Button>
					{:else}
						<Button variant="outline" onclick={() => picker?.focus()}>Choose an office</Button>
					{/if}
				</div>
			{/if}
		</section>
	</div>

	<section class="flex flex-col gap-3" aria-labelledby="recent-emails">
		<h2 id="recent-emails" class="m-0 text-h3 font-semibold">Recent outreach</h2>
		{#if listState === 'loading'}
			<div class="overflow-hidden rounded-lg border border-line bg-card" role="status" aria-label="Loading outreach emails">
				<div class="flex flex-col divide-y divide-line">
					{#each [0, 1, 2] as i (i)}
						<div class="flex items-center gap-4 px-4 py-3">
							<Skeleton class="h-6 w-16 rounded-full" />
							<Skeleton class="h-4 flex-1 rounded-xs" />
							<Skeleton class="hidden h-4 w-40 rounded-xs md:block" />
						</div>
					{/each}
				</div>
			</div>
		{:else if listState === 'failed'}
			<Notice variant="error" heading="Outreach emails could not load.">
				<p class="m-0">{listError}</p>
				<p class="mt-3 mb-0"><Button variant="outline" onclick={() => loadList()}>Try again</Button></p>
			</Notice>
		{:else if emails.length === 0}
			<div class="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line bg-card px-6 py-10 text-center">
				<InboxIcon class="size-8 text-ink-muted" aria-hidden="true" />
				<p class="m-0 text-h3 font-semibold">No outreach yet</p>
				<p class="m-0 max-w-prose text-sm text-ink-muted">
					Every draft, send and reply lands here, newest first.
				</p>
				<Button variant="outline" onclick={() => picker?.focus()}>Draft the first email</Button>
			</div>
		{:else}
			<ul class="m-0 flex list-none flex-col divide-y divide-line overflow-hidden rounded-lg border border-line bg-card p-0">
				{#each emails as row (row.id)}
					<li animate:flip={{ duration: motion }} class={current?.id === row.id ? 'bg-muted' : ''}>
						<button
							type="button"
							class="flex w-full min-w-0 flex-wrap items-center gap-x-3 gap-y-1 px-4 py-3 text-left outline-none hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:ring-2 focus-visible:ring-focus"
							aria-current={current?.id === row.id ? 'true' : undefined}
							onclick={() => openEmail(row)}
						>
							<span class="w-24 flex-none"><EmailStatusBadge status={row.status} /></span>
							<span class="flex min-w-0 flex-1 basis-64 flex-col">
								<span class="truncate text-sm font-medium text-ink">{row.subject}</span>
								<span class="truncate text-xs text-ink-muted">
									{nameOf(row.slug)} · <span class="font-mono">{row.to}</span> · {row.questions.length} question{row.questions.length === 1 ? '' : 's'}
								</span>
							</span>
							<time class="text-xs text-ink-muted" datetime={new Date((row.sent_at ?? row.drafted_at) * 1000).toISOString()} title={dateTime(row.sent_at ?? row.drafted_at)}>
								{row.sent_at ? 'sent' : 'drafted'} {ago(row.sent_at ?? row.drafted_at, Math.max(nowSec, row.sent_at ?? row.drafted_at))}
							</time>
							<span class="inline-flex items-center gap-1 text-xs font-medium text-accent-strong">
								{#if row.status === 'draft'}
									<PencilIcon class="size-3.5" aria-hidden="true" />
									Edit
								{:else}
									<EyeIcon class="size-3.5" aria-hidden="true" />
									View
								{/if}
							</span>
						</button>
					</li>
				{/each}
			</ul>
			{#if listError}<p class="m-0 text-xs text-ink-muted" role="status">Last refresh failed: {listError}</p>{/if}
		{/if}
	</section>
</div>

<Dialog.Root bind:open={confirmOpen}>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title class="text-h3 font-semibold">Send this email?</Dialog.Title>
			<Dialog.Description class="text-sm text-ink-muted">
				It leaves from the Apostille Handbook mailer and cannot be recalled.
				{#if dirty}Your edits are saved first.{/if}
			</Dialog.Description>
		</Dialog.Header>
		<dl class="m-0 grid grid-cols-4 gap-x-3 gap-y-2 rounded-md border border-line bg-surface p-3 text-sm">
			<dt class="text-ink-muted">To</dt>
			<dd class="col-span-3 m-0 font-mono break-all">{edit.to.trim().toLowerCase()}</dd>
			<dt class="text-ink-muted">Subject</dt>
			<dd class="col-span-3 m-0 font-medium break-words">{edit.subject.trim()}</dd>
			<dt class="text-ink-muted">Office</dt>
			<dd class="col-span-3 m-0">{current ? nameOf(current.slug) : ''}</dd>
			<dt class="text-ink-muted">Today</dt>
			<dd class="col-span-3 m-0 tabular-nums">{sentToday} of {dailyCap} outreach emails sent</dd>
		</dl>
		<Dialog.Footer>
			<Button variant="outline" size="lg" onclick={() => (confirmOpen = false)}>Keep editing</Button>
			<Button size="lg" onclick={send}>
				<SendIcon aria-hidden="true" />
				Send now
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
