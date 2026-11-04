<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../../lib/components/brand/Heading.svelte';
	import { Skeleton } from '../../../../lib/components/ui/skeleton/index.js';
	import { NOTIFY_COPY } from '../../../../lib/public-ui/notify-copy.js';
	import { site } from '../../../../lib/locale.svelte.js';

	const c = $derived(NOTIFY_COPY[site.locale]);

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
	<title>{c.confirmTitle}</title>
</svelte:head>

<div class="mx-auto w-full max-w-xl px-4 py-10 sm:px-6 xl:py-16">
	<Heading level={1}>{c.confirmHeading}</Heading>
	{#if phase === 'verifying'}
		<div role="status" aria-label={c.confirming} class="mt-6 flex flex-col gap-2">
			<span class="sr-only">{c.confirming}</span>
			<Skeleton class="h-5 w-full" />
			<Skeleton class="h-5 w-2/3" />
		</div>
	{:else if phase === 'done'}
		<Notice variant="success" heading={c.followingHeading}>
			<p>
				{c.followingBody}
				<a href={home} class="underline underline-offset-4">{c.backToIndex}</a>.
			</p>
		</Notice>
	{:else}
		<Notice variant="warning" heading={c.expiredHeading}>
			<p>
				{c.expiredBody}
				<a href={home} class="underline underline-offset-4">{c.backToIndex}</a>.
			</p>
		</Notice>
	{/if}
</div>
