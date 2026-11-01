<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../../lib/components/brand/Heading.svelte';
	import { Skeleton } from '../../../../lib/components/ui/skeleton/index.js';

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const home = resolve('/(public)');

	let phase = $state<'verifying' | 'done' | 'expired'>('verifying');

	function tokenFromHash(): string | null {
		const hash = typeof location === 'undefined' ? '' : location.hash;
		const match = /[#&]t=([0-9a-fA-F]{64})\b/.exec(hash);
		return match ? match[1] : null;
	}

	onMount(async () => {
		// The mailed link carries the token in the fragment, so it is
		// never sent to the server in a GET — the page POSTs it.
		const token = tokenFromHash() ?? page.url.searchParams.get('token');
		if (!token) {
			phase = 'expired';
			return;
		}
		if (typeof history !== 'undefined') {
			history.replaceState(null, '', location.pathname);
		}
		try {
			const res = await fetch(`${apiRoot}/api/v1/notify/confirm`, {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				credentials: 'same-origin',
				body: JSON.stringify({ token })
			});
			phase = res.ok ? 'done' : 'expired';
		} catch {
			phase = 'expired';
		}
	});
</script>

<svelte:head>
	<title>Confirm updates — Apostille Handbook</title>
</svelte:head>

<div class="mx-auto w-full max-w-xl px-4 py-10 sm:px-6 xl:py-16">
	<Heading level={1}>Confirm email updates</Heading>
	{#if phase === 'verifying'}
		<div role="status" aria-label="Confirming subscription" class="mt-6 flex flex-col gap-2">
			<span class="sr-only">Confirming subscription</span>
			<Skeleton class="h-5 w-full" />
			<Skeleton class="h-5 w-2/3" />
		</div>
	{:else if phase === 'done'}
		<Notice
			variant="success"
			heading="You are following this office."
		>
			<p>
				You will get one email when staff accept a changed fact for it — at most one
				per hour. Every email carries a one-click unsubscribe link.
				<a href={home} class="underline underline-offset-4">Back to the authority index</a>.
			</p>
		</Notice>
	{:else}
		<Notice variant="warning" heading="This confirm link is invalid or expired.">
			<p>
				Links die 24 hours after they are sent and work only once. Follow the office
				again from its dossier page for a fresh link.
				<a href={home} class="underline underline-offset-4">Back to the authority index</a>.
			</p>
		</Notice>
	{/if}
</div>
