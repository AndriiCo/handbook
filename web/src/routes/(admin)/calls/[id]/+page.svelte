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
	<p class="error" role="alert">{loadError}</p>
{:else if detail}
	<h1>Call — {detail.status}</h1>

	{#if detail.status_detail}
		<Notice variant="info" heading="Note."><p>{detail.status_detail}</p></Notice>
	{/if}

	<div class="panes">
		<section aria-labelledby="questions">
			<h2 id="questions">Questions to ask</h2>
			{#if detail.questions.length === 0}
				<p class="muted">No prepared questions for this call.</p>
			{:else}
				<ol class="qlist">
					{#each detail.questions as question, idx (idx)}
						<li class:asked={question.asked}>
							<span>{question.text}</span>
							{#if !question.asked}
								<button
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

		<section aria-labelledby="transcript">
			<h2 id="transcript">Transcript</h2>
			{#if detail.transcript.length === 0}
				<p class="muted">
					{isLive(detail.status) ? 'Listening — segments appear as they arrive.' : 'No transcript captured.'}
				</p>
			{:else}
				<ol class="segments">
					{#each detail.transcript as segment (segment.seq)}
						<li>{segment.text}</li>
					{/each}
				</ol>
			{/if}
		</section>

		<section aria-labelledby="controls">
			<h2 id="controls">Controls</h2>
			<p class="muted">Recording: {detail.recording_status}</p>
			{#if isLive(detail.status)}
				<p class="buttons">
					<button
						type="button"
						disabled={acting}
						onclick={() => act('Record start', () => recordCall(api(), id, true))}
					>
						Start recording
					</button>
					<button
						type="button"
						disabled={acting}
						onclick={() => act('Record stop', () => recordCall(api(), id, false))}
					>
						Stop recording
					</button>
					<button
						type="button"
						disabled={acting}
						onclick={() => act('Hangup', () => hangupCall(api(), id))}
					>
						Hang up
					</button>
				</p>
			{:else}
				<p class="muted">Call ended — controls disabled.</p>
			{/if}
			{#if actionError}
				<p class="error" role="alert">{actionError}</p>
			{/if}
		</section>
	</div>
{/if}

<style>
	.panes {
		display: grid;
		gap: 1rem;
		padding-bottom: 2rem;
	}
	.panes section {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.9rem 1rem;
	}
	.panes h2 {
		font-size: 1rem;
		margin: 0 0 0.5rem;
	}
	.muted {
		color: var(--text-muted);
	}
	.error {
		color: var(--error, #b00020);
		font-weight: 700;
	}
	.asked span {
		text-decoration: line-through;
		color: var(--text-muted);
	}
	.qlist li {
		margin-bottom: 0.5rem;
	}
	.qlist button {
		margin-left: 0.5rem;
	}
	.segments {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 0.5rem;
	}
	.segments li {
		border-bottom: 1px dotted var(--border);
		padding-bottom: 0.5rem;
	}
	.buttons {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
	.buttons button {
		min-height: 44px;
	}
</style>
