<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import {
		fetchCall,
		hangupCall,
		isLive,
		markQuestionAsked,
		quoteBody,
		quoteToDraft,
		recordCall
	} from '../../../../lib/admin/calls-client.js';

	// Live call view (voice-telnyx.md §5): status, questions-to-ask
	// panel, transcript. Polls the detail endpoint every 2 s while
	// the call is live (a WS push is a follow-up, spec OQ-6).
	const api = () => `/apostille/api/v1`;
	const id = $derived(page.params.id ?? '');

	interface Question {
		text: string;
		asked: boolean;
	}
	interface Segment {
		seq: number;
		speaker: string | null;
		text: string;
		at: number;
	}
	interface Detail {
		call_id: string;
		status: string;
		status_detail: string | null;
		authority: string | null;
		questions: Question[];
		transcript: Segment[];
		recording_status: string;
		ended_at: number | null;
	}

	let detail = $state<Detail | null>(null);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let actionError = $state<string | null>(null);
	let acting = $state(false);
	let timer: ReturnType<typeof setInterval> | null = null;

	// Quote-to-draft selection + form (voice-telnyx.md §7). Selection is
	// keyed by stable segment seq, so the 2 s poll does not clear it.
	let selected = $state<Set<number>>(new Set());
	let topic = $state('');
	let statement = $state('');
	let quoteResult = $state<string | null>(null);

	function toggle(seq: number) {
		if (selected.has(seq)) selected.delete(seq);
		else selected.add(seq);
	}

	async function quote() {
		quoteResult = null;
		const body = quoteBody([...selected], topic, statement);
		if (!body) {
			actionError = 'Pick at least one segment, a topic, and a 1–1024 char statement.';
			return;
		}
		await act('Quote to draft', async () => {
			const created = await quoteToDraft(api(), id, body);
			quoteResult = `Proposed fact #${created.fact_id} created (${created.topic}).`;
			selected = new Set();
			topic = '';
			statement = '';
		});
	}

	async function load() {
		try {
			detail = await fetchCall(api(), id);
			loadState = 'ready';
		} catch (err) {
			// Keep the last good render on poll failures; only the
			// first load fails the page.
			if (detail === null) {
				loadError = err instanceof Error ? err.message : 'Call load failed.';
				loadState = 'failed';
			}
		}
	}

	function armPolling() {
		if (timer !== null) return;
		timer = setInterval(() => {
			if (detail && !isLive(detail.status)) {
				if (timer !== null) clearInterval(timer);
				timer = null;
				return;
			}
			void load();
		}, 2000);
	}

	onMount(async () => {
		await load();
		armPolling();
	});

	onDestroy(() => {
		if (timer !== null) clearInterval(timer);
	});

	async function act(what: string, run: () => Promise<unknown>) {
		actionError = null;
		acting = true;
		try {
			await run();
			await load();
		} catch (err) {
			actionError = err instanceof Error ? err.message : `${what} failed.`;
		} finally {
			acting = false;
		}
	}
</script>

<svelte:head>
	<title>Call — Admin — Apostille Handbook</title>
</svelte:head>

<p><a href={resolve('/(admin)/calls')}>← All calls</a></p>

{#if loadState === 'loading'}
	<p>Loading…</p>
{:else if loadState === 'failed'}
	<p class="font-bold text-error" role="alert">{loadError}</p>
{:else if detail}
	<h1>Call — {detail.status}</h1>

	{#if detail.status_detail}
		<Notice variant="info" heading="Note."><p>{detail.status_detail}</p></Notice>
	{/if}

	<div class="grid gap-4 pb-8">
		<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="questions">
			<h2 class="m-0 mb-2 text-base" id="questions">Questions to ask</h2>
			{#if detail.questions.length === 0}
				<p class="text-ink-muted">No prepared questions for this call.</p>
			{:else}
				<ol>
					{#each detail.questions as question, idx (idx)}
						<li class="mb-2">
							<span class={question.asked ? 'text-ink-muted line-through' : ''}>{question.text}</span>
							{#if !question.asked}
								<button
									class="ml-2"
									type="button"
									disabled={acting}
									onclick={() => act('Mark asked', () => markQuestionAsked(api(), id, idx))}
								>
									Mark asked
								</button>
							{/if}
						</li>
					{/each}
				</ol>
			{/if}
		</section>

		<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="transcript">
			<h2 class="m-0 mb-2 text-base" id="transcript">Transcript</h2>
			{#if detail.transcript.length === 0}
				<p class="text-ink-muted">
					{isLive(detail.status) ? 'Listening — segments appear as they arrive.' : 'No transcript captured.'}
				</p>
			{:else}
				<ol class="m-0 grid list-none gap-2 p-0">
					{#each detail.transcript as segment (segment.seq)}
						<li class="pb-2 [border-bottom:1px_dotted_var(--border)]">
							<label>
								<input
									type="checkbox"
									checked={selected.has(segment.seq)}
									onchange={() => toggle(segment.seq)}
								/>
								<span class="text-[0.85rem] text-ink-muted">#{segment.seq}</span>
								{segment.text}
							</label>
						</li>
					{/each}
				</ol>
			{/if}
		</section>

		<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="quote">
			<h2 class="m-0 mb-2 text-base" id="quote">Quote to draft</h2>
			<p class="text-ink-muted">
				Quote the checked segments into one proposed fact. The fact lands in the
				standard approval queue — quoting never auto-accepts.
			</p>
			<p>
				<label>
					Topic
					<input type="text" bind:value={topic} placeholder="e.g. fees" autocomplete="off" />
				</label>
			</p>
			<p>
				<label>
					Statement
					<textarea bind:value={statement} rows="3" maxlength="1024" placeholder="One handbook sentence."></textarea>
				</label>
			</p>
			<p class="flex flex-wrap gap-2">
				<button class="min-h-11" type="button" disabled={acting || selected.size === 0} onclick={quote}>
					Quote {selected.size} segment{selected.size === 1 ? '' : 's'} to draft
				</button>
			</p>
			{#if quoteResult}
				<p class="font-bold text-[#1c7a2e]" role="status">{quoteResult}</p>
			{/if}
		</section>

		<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="controls">
			<h2 class="m-0 mb-2 text-base" id="controls">Controls</h2>
			<p class="text-ink-muted">Recording: {detail.recording_status}</p>
			{#if isLive(detail.status)}
				<p class="flex flex-wrap gap-2">
					<button
						class="min-h-11"
						type="button"
						disabled={acting}
						onclick={() => act('Record start', () => recordCall(api(), id, true))}
					>
						Start recording
					</button>
					<button
						class="min-h-11"
						type="button"
						disabled={acting}
						onclick={() => act('Record stop', () => recordCall(api(), id, false))}
					>
						Stop recording
					</button>
					<button
						class="min-h-11"
						type="button"
						disabled={acting}
						onclick={() => act('Hangup', () => hangupCall(api(), id))}
					>
						Hang up
					</button>
				</p>
			{:else}
				<p class="text-ink-muted">Call ended — controls disabled.</p>
			{/if}
			{#if actionError}
				<p class="font-bold text-error" role="alert">{actionError}</p>
			{/if}
		</section>
	</div>
{/if}
