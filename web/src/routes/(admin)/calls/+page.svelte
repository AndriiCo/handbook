<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import {
		defaultScript,
		dialBody,
		dialCall,
		e164Valid,
		fetchCalls
	} from '../../../lib/admin/calls-client.js';

	interface CallSummary {
		call_id: string;
		status: string;
		authority: string | null;
		started_at: number;
		ended_at: number | null;
		recording_status: string;
		questions_asked: number;
		questions_total: number;
	}

	// Voice calls (voice-telnyx.md §7): dial form + recent calls.
	// The admin API keeps the /apostille prefix on the admin origin,
	// so the root is literal (same as the config page).
	const api = () => `/apostille/api/v1`;

	let calls = $state<CallSummary[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);

	let to = $state('');
	let authority = $state('');
	let questionsText = $state('');
	let consent = $state(false);
	let dialing = $state(false);
	let dialError = $state<string | null>(null);

	async function load() {
		loadState = 'loading';
		loadError = null;
		try {
			calls = await fetchCalls(api());
			loadState = 'ready';
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Call list failed.';
			loadState = 'failed';
		}
	}

	onMount(load);

	function questions(): string[] {
		return questionsText
			.split('\n')
			.map((line) => line.trim())
			.filter((line) => line.length > 0);
	}

	function loadScript() {
		const have = new Set(questions());
		const missing = defaultScript().filter((line) => !have.has(line));
		if (missing.length === 0) return;
		questionsText = questionsText.trimEnd();
		questionsText += (questionsText.length > 0 ? '\n' : '') + missing.join('\n');
	}

	async function dial(event: SubmitEvent) {
		event.preventDefault();
		dialError = null;
		const body = dialBody(to.trim(), authority, questions(), consent);
		if (!body) {
			dialError = !e164Valid(to.trim())
				? 'Enter the office number as E.164 (e.g. +13605550100).'
				: !consent
					? 'Confirm you will announce the recording to the callee.'
					: 'Check the questions (1–20 lines, each 1–280 chars).';
			return;
		}
		dialing = true;
		try {
			const out = await dialCall(api(), body);
			await goto(`${resolve('/(admin)/calls')}/${encodeURIComponent(out.call_id)}`);
		} catch (err) {
			dialError = err instanceof Error ? err.message : 'Dial failed.';
		} finally {
			dialing = false;
		}
	}
</script>

<svelte:head>
	<title>Voice calls — Admin — Apostille Handbook</title>
</svelte:head>

<h1>Voice calls</h1>

<Notice variant="warning" heading="All-party consent.">
	<p>
		Washington requires every party's consent to record. Announce the recording when the
		callee answers, before asking anything. Dialing is staff-only and audited.
	</p>
</Notice>

<section aria-labelledby="dial">
	<h2 id="dial">Dial an office</h2>
	<form onsubmit={dial}>
		<p>
			<label class="stack" for="to">
				Office number (E.164)
				<input id="to" type="tel" bind:value={to} placeholder="+13605550100" autocomplete="off" />
			</label>
		</p>
		<p>
			<label class="stack" for="authority">
				Authority slug (optional)
				<input id="authority" type="text" bind:value={authority} placeholder="usa-wa-sos" autocomplete="off" />
			</label>
		</p>
		<p>
			<label class="stack" for="questions">
				Questions to ask (one per line, up to 20)
				<textarea
					id="questions"
					rows="4"
					bind:value={questionsText}
					placeholder={'Current apostille fee?\nForm revision and where to mail?'}
				></textarea>
			</label>
		</p>
		<p>
			<button type="button" onclick={loadScript}>Load default script</button>
		</p>
		<p>
			<label class="consent" for="consent">
				<input id="consent" type="checkbox" bind:checked={consent} />
				I will announce that this call is recorded before asking anything.
			</label>
		</p>
		{#if dialError}
			<p class="error" role="alert">{dialError}</p>
		{/if}
		<p><button type="submit" disabled={dialing}>{dialing ? 'Dialing…' : 'Dial'}</button></p>
	</form>
</section>

<section aria-labelledby="recent">
	<h2 id="recent">Recent calls</h2>
	{#if loadState === 'loading'}
		<p>Loading…</p>
	{:else if loadState === 'failed'}
		<p class="error" role="alert">{loadError}</p>
		<p><button type="button" onclick={load}>Retry</button></p>
	{:else if calls.length === 0}
		<p class="muted">No calls yet.</p>
	{:else}
		<ul class="ledger">
			{#each calls as call (call.call_id)}
				<li>
					<a href={`${resolve('/(admin)/calls')}/${encodeURIComponent(call.call_id)}`}>
						{call.authority ?? 'call'} — {call.status}
					</a>
					<span class="kind">
						{call.questions_asked}/{call.questions_total} asked · recording {call.recording_status}
					</span>
				</li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	.stack {
		display: grid;
		gap: 0.25rem;
		max-width: 36rem;
		font-weight: 700;
	}
	.stack input,
	.stack textarea {
		font-weight: 400;
		padding: 0.5rem;
	}
	.consent {
		display: flex;
		gap: 0.5rem;
		align-items: flex-start;
		max-width: 36rem;
	}
	.error {
		color: var(--error, #b00020);
		font-weight: 700;
	}
	.muted {
		color: var(--text-muted);
	}
	.ledger {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0 0 2rem;
	}
	.ledger li {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.25rem 0.75rem;
		padding: 0.7rem 0;
		border-bottom: 1px solid var(--border);
	}
	.ledger a {
		font-weight: 700;
		min-height: 44px;
		display: inline-flex;
		align-items: center;
	}
	.kind {
		color: var(--text-muted);
		font-size: 0.875rem;
	}
</style>
