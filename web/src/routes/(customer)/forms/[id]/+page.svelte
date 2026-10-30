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
	<p class="text-sm text-ink-muted">
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

	<table class="my-4 max-w-[40rem] border-collapse">
		<tbody>
			{#each Object.entries(submission.values) as [key, value] (key)}
				<tr>
					<th class="border-b border-line px-3 py-2 text-left align-top" scope="row">{labelFor(key)}</th>
					<td class="border-b border-line px-3 py-2 text-left align-top">{displayValue(value)}</td>
				</tr>
			{/each}
		</tbody>
	</table>

	{#if submission.pdf_url}
		<p>
			<a
				class="inline-block min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 py-[0.6rem] font-bold text-accent-ink no-underline"
				href={submission.pdf_url}
				download
			>
				Download filled PDF
			</a>
		</p>
		<p class="text-sm text-ink-muted">sha256: <code class="font-mono break-all">{submission.pdf_sha256}</code></p>
		<p>Print it, sign in ink on the signature line, and mail it with your shipping labels.</p>
	{:else}
		<Notice variant="info" heading="No PDF yet.">
			<p>This submission is still a draft, or the PDF is rendering — this page refreshes itself.</p>
		</Notice>
	{/if}

	<h2>Something looks wrong?</h2>
	<label for="correction">Report it and staff will re-check the prefilled values.</label>
	<textarea
		class="my-2 block w-full max-w-[36rem] rounded-sm border border-line bg-surface px-[0.6rem] py-2 text-ink"
		id="correction"
		rows="3"
		maxlength="2000"
		bind:value={correctionNote}
	></textarea>
	<button
		class="inline-block min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 py-[0.6rem] font-bold text-accent-ink no-underline disabled:cursor-wait disabled:opacity-60"
		type="button"
		onclick={reportCorrection}
		disabled={busy}
	>
		{busy ? 'Sending…' : 'Send report'}
	</button>
	{#if correctionDone}
		<Notice variant="info" heading="Report sent."><p>{correctionDone}</p></Notice>
	{/if}
{/if}

