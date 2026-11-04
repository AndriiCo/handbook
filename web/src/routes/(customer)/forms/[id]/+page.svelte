<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { FORMS_COPY } from '../../../../lib/customer/forms-copy.js';
	import { INTL_LOCALE } from '../../../../lib/customer/orders-copy.js';
	import { site } from '../../../../lib/locale.svelte.js';

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

	// Page words follow the site language; messages are kept as
	// functions of the copy so a language switch re-words them too.
	const c = $derived(FORMS_COPY[site.locale]);
	type Msg = (copy: typeof c) => string;

	const id = $derived(page.params.id ?? '');
	let submission = $state<Submission | null>(null);
	let labels = $state<Record<string, string>>({});
	let error = $state<Msg | null>(null);
	let live = $state<'connecting' | 'live' | 'reconnecting' | 'offline'>('connecting');
	let correctionNote = $state('');
	let correctionDone = $state<Msg | null>(null);
	let busy = $state(false);
	let closeWs: (() => void) | null = null;

	function labelFor(key: string): string {
		return labels[key] ?? key;
	}

	function displayValue(value: unknown): string {
		if (value === undefined || value === null || value === '') return '—';
		if (typeof value === 'boolean') return value ? c.yesValue : c.noValue;
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
				error = (k) => k.submissionNotFound;
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
			error = (k) => k.submissionUnreachable;
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
			correctionDone = (k) => k.submissionThanks;
			correctionNote = '';
		} catch {
			correctionDone = (k) => k.correctionFailed;
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
	<title>{c.submission(id)} — Apostille Handbook</title>
</svelte:head>

<h1>{c.submission(id)}</h1>

{#if error}
	<Notice variant="error" heading={c.cannotLoadSubmission}><p>{error(c)}</p></Notice>
{:else if !submission}
	<p>{c.loading}</p>
{:else}
	<p class="text-sm text-ink-muted">
		{submission.code} v{submission.template_version} · {submission.status}
		{#if submission.submitted_at}
			· {c.submittedAt(new Date(submission.submitted_at * 1000).toLocaleString(INTL_LOCALE[site.locale]))}
		{/if}
		· {c.updates(c.liveState[live])}
	</p>

	{#if submission.replica_notice}
		<Notice variant="info" heading={c.aboutPrintout}>
			<p>{submission.replica_notice}</p>
		</Notice>
	{/if}
	{#if submission.customer_override}
		<Notice variant="info" heading={c.overrideOnFile}>
			<p>{c.overrideOnFileBody}</p>
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
				{c.downloadPdf}
			</a>
		</p>
		<p class="text-sm text-ink-muted">sha256: <code class="font-mono break-all">{submission.pdf_sha256}</code></p>
		<p>{c.printSign}</p>
	{:else}
		<Notice variant="info" heading={c.noPdf}>
			<p>{c.noPdfBody}</p>
		</Notice>
	{/if}

	<h2>{c.somethingWrong}</h2>
	<label for="correction">{c.reportPrompt}</label>
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
		{busy ? c.sending : c.sendReport}
	</button>
	{#if correctionDone}
		<Notice variant="info" heading={c.reportSent}><p>{correctionDone(c)}</p></Notice>
	{/if}
{/if}

