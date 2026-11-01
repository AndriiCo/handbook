<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../../lib/components/brand/Heading.svelte';
	import { Input } from '../../../../lib/components/ui/input/index.js';
	import { Button } from '../../../../lib/components/ui/button/index.js';
	import { Skeleton } from '../../../../lib/components/ui/skeleton/index.js';

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const home = resolve('/(public)');

	let phase = $state<'working' | 'done' | 'form'>('working');
	let pasted = $state('');
	let busy = $state(false);

	async function send(token: string): Promise<boolean> {
		try {
			const res = await fetch(`${apiRoot}/api/v1/notify/unsubscribe`, {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				credentials: 'same-origin',
				body: JSON.stringify({ token })
			});
			return res.ok;
		} catch {
			return false;
		}
	}

	onMount(async () => {
		// One-click footer links land here with ?t=; without one we
		// show the form (a pasted token works the same way).
		const token = page.url.searchParams.get('t') ?? page.url.searchParams.get('token');
		if (!token) {
			phase = 'form';
			return;
		}
		await send(token);
		phase = 'done';
	});

	async function submit(event: Event) {
		event.preventDefault();
		if (busy) return;
		busy = true;
		try {
			if (await send(pasted.trim())) phase = 'done';
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
	{#if phase === 'working'}
		<div role="status" aria-label="Unsubscribing" class="mt-6 flex flex-col gap-2">
			<span class="sr-only">Unsubscribing</span>
			<Skeleton class="h-5 w-full" />
			<Skeleton class="h-5 w-1/2" />
		</div>
	{:else if phase === 'done'}
		<Notice variant="success" heading="Unsubscribed.">
			<p>
				No more update emails for this office. The address stays on file only as a
				hash so a later resubscribe is instant — write to us if you want it fully
				erased. <a href={home} class="underline underline-offset-4">Back to the authority index</a>.
			</p>
		</Notice>
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
	{/if}
</div>
