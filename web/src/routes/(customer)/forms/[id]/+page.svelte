<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';

	// Submission review screen (P2-U1 steps 5-8): field summary,
	// replica notice, override flag, PDF download. Subscribes to
	// submission:{id} so pdf.ready refreshes the page live.
	// Same dual-build API root derivation as the fill page.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;
	const wsUrl = () =>
		`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}${apiRoot}/ws/bus`;

	interface Submission {
		id: number;
		form_template_id: number;
		authority: string;
		code: string;
		template_version: number;
		values: Record<string, unknown>;
		customer_override: boolean;
		replica_unconfirmed: boolean;
		replica_notice: string | null;
		pdf_sha256: string;
		pdf_url: string | null;
		status: string;
		submitted_at: number | null;
	}

	interface Label {
		key: string;
		label: string;
	}

	const id = $derived(page.params.id ?? '');
	let submission = $state<Submission | null>(null);
	let labels = $state<Record<string, string>>({});
	let error = $state<string | null>(null);
	let live = $state<'connecting' | 'live' | 'reconnecting' | 'offline'>('connecting');
	let correctionNote = $state('');
	let correctionDone = $state<string | null>(null);
	let busy = $state(false);
	let closeWs: (() => void) | null = null;

	function labelFor(key: string): string {
		return labels[key] ?? key;
	}

	function displayValue(value: unknown): string {
		if (value === undefined || value === null || value === '') return '—';
		if (typeof value === 'boolean') return value ? 'yes' : 'no';
		if (typeof value === 'object') {
			return Object.entries(value as Record<string, unknown>)
				.map(([k, v]) => `${k}: ${v}`)
				.join('; ');
		}
		return String(value);
	}

	async function load() {
		error = null;
		try {
			const res = await fetch(api(`/p2/submissions/${id}`), { credentials: 'same-origin' });
			if (res.status === 404) {
				error = 'Submission not found. It may belong to another account.';
				return;
			}
			if (!res.ok) throw new Error(String(res.status));
			submission = (await res.json()) as Submission;
			const formRes = await fetch(api(`/p2/forms/${submission.authority}/${submission.code}`), {
				credentials: 'same-origin'
			});
			if (formRes.ok) {
				const form = await formRes.json();
				const map: Record<string, string> = {};
				for (const f of (form.fields ?? []) as Label[]) map[f.key] = f.label;
				labels = map;
			}
		} catch {
			error = 'Submission service unreachable — is the API server running?';
		}
	}

	async function reportCorrection() {
		if (!submission || correctionNote.trim() === '') return;
		correctionDone = null;
		busy = true;
		try {
			const res = await fetch(api('/p2/corrections'), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					submission_id: submission.id,
					topic: 'general',
					note: correctionNote.trim()
				})
			});
			if (!res.ok) throw new Error(String(res.status));
			correctionDone = 'Thanks — staff will re-check this submission.';
			correctionNote = '';
		} catch {
			correctionDone = 'Could not send the report. Please try again.';
		} finally {
			busy = false;
		}
	}

	onMount(() => {
		let cancelled = false;
		load();
		(async () => {
			try {
				const { createWsClient } = await import('../../../../lib/ws-client.js');
				if (cancelled) return;
				const client = createWsClient(wsUrl(), {
					topics: [`submission:${id}`],
					onStatus: (next: typeof live) => {
						live = next;
					},
					onMessage: (msg: { type: string }) => {
						if (msg.type === 'pdf.ready' || msg.type === 'form.validated') load();
					}
				});
				closeWs = () => client.close();
			} catch {
				if (!cancelled) live = 'offline';
			}
		})();
		return () => {
			cancelled = true;
			closeWs?.();
		};
	});
</script>

<svelte:head>
	<title>Submission #{id} — Apostille Handbook</title>
</svelte:head>

<h1>Submission #{id}</h1>

{#if error}
	<Notice variant="error" heading="Cannot load this submission."><p>{error}</p></Notice>
{:else if !submission}
	<p>Loading…</p>
{:else}
	<p class="meta">
		{submission.code} v{submission.template_version} · {submission.status}
		{#if submission.submitted_at}
			· submitted {new Date(submission.submitted_at * 1000).toLocaleString()}
		{/if}
		· updates: {live}
	</p>

	{#if submission.replica_notice}
		<Notice variant="info" heading="About your printout.">
			<p>{submission.replica_notice}</p>
		</Notice>
	{/if}
	{#if submission.customer_override}
		<Notice variant="info" heading="Prefill override on file.">
			<p>You typed over a prefilled value; staff can see the flag.</p>
		</Notice>
	{/if}

	<table>
		<tbody>
			{#each Object.entries(submission.values) as [key, value] (key)}
				<tr>
					<th scope="row">{labelFor(key)}</th>
					<td>{displayValue(value)}</td>
				</tr>
			{/each}
		</tbody>
	</table>

	{#if submission.pdf_url}
		<p>
			<a class="button" href={submission.pdf_url} download>
				Download filled PDF
			</a>
		</p>
		<p class="meta">sha256: <code>{submission.pdf_sha256}</code></p>
		<p>Print it, sign in ink on the signature line, and mail it with your shipping labels.</p>
	{:else}
		<Notice variant="info" heading="No PDF yet.">
			<p>This submission is still a draft, or the PDF is rendering — this page refreshes itself.</p>
		</Notice>
	{/if}

	<h2>Something looks wrong?</h2>
	<label for="correction">Report it and staff will re-check the prefilled values.</label>
	<textarea id="correction" rows="3" maxlength="2000" bind:value={correctionNote}></textarea>
	<button type="button" onclick={reportCorrection} disabled={busy}>
		{busy ? 'Sending…' : 'Send report'}
	</button>
	{#if correctionDone}
		<Notice variant="info" heading="Report sent."><p>{correctionDone}</p></Notice>
	{/if}
{/if}

<style>
	.meta {
		color: var(--text-muted);
		font-size: 0.875rem;
	}
	.meta code {
		font-family: var(--font-mono);
		word-break: break-all;
	}
	table {
		border-collapse: collapse;
		max-width: 40rem;
		margin: 1rem 0;
	}
	th,
	td {
		border-bottom: 1px solid var(--border);
		padding: 0.5rem 0.75rem;
		text-align: left;
		vertical-align: top;
	}
	textarea {
		display: block;
		font: inherit;
		width: 100%;
		max-width: 36rem;
		padding: 0.5rem 0.6rem;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		background: var(--surface);
		color: var(--text);
		margin: 0.5rem 0;
	}
	button,
	.button {
		display: inline-block;
		min-height: 44px;
		padding: 0.6rem 1rem;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
		text-decoration: none;
	}
	button:disabled {
		opacity: 0.6;
		cursor: wait;
	}
</style>
