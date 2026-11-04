<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '#lib/public-ui/Notice.svelte';
	import { bytesToBase64, composeMail, composeProblem, fetchOutbox, replyDraft } from '#lib/admin/fake-client.js';

	// W-3 fake inbox (`mail.mode = fake`): write the mail an office or an
	// authority would send — a reply to outreach, a notice — and pick the
	// DMARC result Gmail would stamp. The server captures it through the
	// Gmail poll's own step, so DMARC decides the trust exactly as for
	// real mail, and the instruction sweep turns it into facts for Review.
	// `?reply=<outbox id>` prefills an answer to a kept message.

	let inbox = $state<boolean | null>(null);
	let loadError = $state<string | null>(null);
	let from = $state('');
	let subject = $state('');
	let body = $state('');
	let inReplyTo = $state('');
	let dmarc = $state<'pass' | 'fail'>('pass');
	let files = $state<FileList | null>(null);
	let sending = $state(false);
	let problem = $state<string | null>(null);
	let result = $state<Record<string, unknown> | null>(null);

	onMount(async () => {
		try {
			const box = await fetchOutbox();
			inbox = box.inbox;
			const reply = Number(new URL(window.location.href).searchParams.get('reply'));
			const message = box.messages.find((m) => m.id === reply);
			if (message) {
				const draft = replyDraft(message);
				from = draft.from;
				subject = draft.subject;
				inReplyTo = draft.in_reply_to;
			}
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'The fake inbox status request failed.';
		}
	});

	async function deliver(event: SubmitEvent) {
		event.preventDefault();
		result = null;
		problem = composeProblem({ from, subject, body, in_reply_to: inReplyTo, dmarc });
		if (problem) return;
		sending = true;
		try {
			const attachments = [];
			for (const file of Array.from(files ?? [])) {
				attachments.push({
					name: file.name,
					content_type: file.type || 'application/octet-stream',
					data_base64: bytesToBase64(new Uint8Array(await file.arrayBuffer()))
				});
			}
			result = await composeMail({ from, subject, body, in_reply_to: inReplyTo, dmarc, attachments });
		} catch (err) {
			problem = err instanceof Error ? err.message : 'Delivering failed.';
		} finally {
			sending = false;
		}
	}

	const trustText = (r: Record<string, unknown>) =>
		r.verified_by === 'none'
			? `${r.trust} trust (not authenticated: DMARC fail or an unknown domain)`
			: `${r.trust} trust (authenticated by ${r.verified_by})`;
</script>

<svelte:head>
	<title>Compose — staff</title>
</svelte:head>

<main class="mx-auto max-w-2xl space-y-6 p-6">
	<h1 class="text-xl font-bold">Compose</h1>
	<p class="text-sm">
		The fake inbox: write a reply or an authority notice as the sender would, and choose what Gmail's
		DMARC check would say. It is captured like polled mail — an authenticated official domain earns
		official trust, a DMARC fail stays community — and the instruction sweep drafts facts from it for
		Review. No mail server is involved.
	</p>

	{#if loadError}
		<Notice variant="error" heading="Fake inbox unavailable"><p>{loadError}</p></Notice>
	{:else if inbox === false}
		<Notice variant="info" heading="Incoming email is not in fake mode">
			<p>
				Mail arrives through the Gmail poll. Set <code>mail.mode</code> to <code>fake</code> in Config and
				restart to compose it here instead (loopback and staging only).
			</p>
		</Notice>
	{/if}

	<form class="space-y-4" onsubmit={deliver} aria-label="Compose a message">
		<label class="block space-y-1">
			<span class="text-sm font-medium">From</span>
			<input class="min-h-11 w-full rounded border border-line bg-bg px-3" type="email" bind:value={from} placeholder="ops@example.gov" autocomplete="off" />
		</label>
		<label class="block space-y-1">
			<span class="text-sm font-medium">Subject</span>
			<input class="min-h-11 w-full rounded border border-line bg-bg px-3" bind:value={subject} />
		</label>
		<label class="block space-y-1">
			<span class="text-sm font-medium">Message</span>
			<textarea class="min-h-40 w-full rounded border border-line bg-bg p-3" bind:value={body}></textarea>
		</label>
		<div class="space-y-1">
			<label class="block space-y-1">
				<span class="text-sm font-medium">In reply to (optional)</span>
				<input
					class="min-h-11 w-full rounded border border-line bg-bg px-3 font-mono text-sm"
					bind:value={inReplyTo}
					placeholder="<message-id>"
					aria-describedby="reply-help"
				/>
			</label>
			<p id="reply-help" class="m-0 text-xs text-ink-muted">
				A Message-ID from the Outbox threads this reply to the outreach email it answers.
			</p>
		</div>
		<fieldset class="space-y-1">
			<legend class="text-sm font-medium">DMARC result for the From domain</legend>
			<label class="mr-4 inline-flex min-h-11 items-center gap-2">
				<input type="radio" name="dmarc" value="pass" bind:group={dmarc} /> Pass
			</label>
			<label class="inline-flex min-h-11 items-center gap-2">
				<input type="radio" name="dmarc" value="fail" bind:group={dmarc} /> Fail
			</label>
		</fieldset>
		<label class="block space-y-1">
			<span class="text-sm font-medium">Attachments (optional)</span>
			<input class="block min-h-11 w-full text-sm" type="file" multiple bind:files />
		</label>
		{#if problem}
			<Notice variant="error" heading="Not delivered"><p>{problem}</p></Notice>
		{/if}
		<button
			type="submit"
			class="min-h-11 rounded border border-line bg-surface-2 px-4 py-2 font-medium disabled:opacity-50"
			disabled={sending || inbox !== true}
		>
			{sending ? 'Delivering…' : 'Deliver to the inbox'}
		</button>
	</form>

	{#if result}
		{#if result.captured}
			<Notice variant="success" heading={`Captured as capture #${result.capture_id}`}>
				<p data-testid="compose-result">
					Filed under {result.authority ?? 'the mail inbox'} with {trustText(result)}.
					{#if typeof result.outreach_email === 'number'}Threaded to outreach email #{result.outreach_email}.{/if}
				</p>
				<p class="mt-2">
					The next instruction sweep drafts facts from it; they wait in
					<a href={resolve('/(admin)/review')}>Review</a>.
				</p>
			</Notice>
		{:else if result.duplicate}
			<Notice variant="info" heading="Already captured"><p>This exact message was delivered before.</p></Notice>
		{:else}
			<Notice variant="info" heading="Handled as a bounce or complaint">
				<p>{result.suppressed} address(es) will get no further mail.</p>
			</Notice>
		{/if}
	{/if}
</main>
