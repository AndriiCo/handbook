<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../../lib/components/brand/Heading.svelte';
	import { Input } from '../../../../lib/components/ui/input/index.js';
	import { Button } from '../../../../lib/components/ui/button/index.js';

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const home = resolve('/(public)');

	let phase = $state<'form' | 'confirm' | 'done' | 'invalid'>('form');
	let token = $state('');
	let pasted = $state('');
	let busy = $state(false);
	let failed = $state(false);

	async function send(raw: string): Promise<'ok' | 'gone' | 'error'> {
		try {
			const res = await fetch(`${apiRoot}/api/v1/notify/unsubscribe`, {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				credentials: 'same-origin',
				body: JSON.stringify({ token: raw })
			});
			if (res.ok) return 'ok';
			if (res.status === 410) return 'gone';
			return 'error';
		} catch {
			return 'error';
		}
	}

	onMount(() => {
		// One-click footer links land here with ?token= (or ?t=).
		// Nothing is sent until the button below is pressed.
		const found = page.url.searchParams.get('token') ?? page.url.searchParams.get('t') ?? '';
		if (/^[0-9a-fA-F]{64}$/.test(found)) {
			token = found;
			phase = 'confirm';
		}
	});

	async function submit(event: Event) {
		event.preventDefault();
		if (busy) return;
		busy = true;
		failed = false;
		try {
			const raw = phase === 'confirm' ? token : pasted.trim();
			const outcome = await send(raw);
			if (outcome === 'ok') phase = 'done';
			else if (outcome === 'gone') phase = 'invalid';
			else failed = true;
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>Unsubscribe — Apostille Handbook</title>
</svelte:head>

<div class="mx-auto w-full max-w-xl px-4 py-10 sm:px-6 xl:py-16">
	<Heading level={1}>Unsubscribe from updates</Heading>
	{#if phase === 'done'}
		<Notice variant="success" heading="Unsubscribed.">
			<p>
				No more update emails for this office. The address stays on file only as a
				hash so a later resubscribe is instant — write to us if you want it fully
				erased. <a href={home} class="underline underline-offset-4">Back to the authority index</a>.
			</p>
		</Notice>
	{:else if phase === 'invalid'}
		<Notice variant="warning" heading="This unsubscribe link is invalid or expired.">
			<p>
				Open the one-click link from a recent update email, or follow the office
				again from its dossier page for a fresh link.
				<a href={home} class="underline underline-offset-4">Back to the authority index</a>.
			</p>
		</Notice>
	{:else if phase === 'confirm'}
		<p class="mt-6 mb-0 text-ink-muted">
			Press the button below to stop update emails for this subscription.
		</p>
		<form class="mt-4 flex flex-col gap-3 sm:flex-row" onsubmit={submit}>
			<Button type="submit" disabled={busy} class="min-h-11 sm:w-auto">
				{busy ? 'Working…' : 'Unsubscribe'}
			</Button>
		</form>
		{#if failed}
			<p class="mt-3 mb-0 text-ink-muted">Something went wrong — please try again.</p>
		{/if}
	{:else}
		<p class="mt-6 mb-0 text-ink-muted">
			Paste the token from your email footer, or open the one-click link from any
			update email.
		</p>
		<form class="mt-4 flex flex-col gap-3 sm:flex-row" onsubmit={submit}>
			<div class="min-w-0 flex-1">
				<label for="unsub-token" class="sr-only">Unsubscribe token</label>
				<Input
					id="unsub-token"
					name="token"
					autocomplete="off"
					placeholder="64-character token"
					bind:value={pasted}
					disabled={busy}
					class="min-h-11 font-mono"
				/>
			</div>
			<Button type="submit" disabled={busy} class="min-h-11 sm:w-auto">
				{busy ? 'Working…' : 'Unsubscribe'}
			</Button>
		</form>
		{#if failed}
			<p class="mt-3 mb-0 text-ink-muted">Something went wrong — please try again.</p>
		{/if}
	{/if}
</div>
