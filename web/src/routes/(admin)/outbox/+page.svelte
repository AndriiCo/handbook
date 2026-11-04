<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '#lib/public-ui/Notice.svelte';
	import { fetchOutbox, KIND_LABEL } from '#lib/admin/fake-client.js';
	import { dateTime } from '#lib/admin/format.js';

	// W-3 fake outbox (`mailer.mode = fake`): every email the server
	// would have sent — sign-in links, follower digests, outreach — kept
	// in the store instead. Nothing here was sent. "Reply" opens Compose
	// as the recipient, threaded on our Message-ID, while the fake inbox
	// is on too. Admin build only.

	let box = $state<Awaited<ReturnType<typeof fetchOutbox>> | null>(null);
	let loadError = $state<string | null>(null);
	let loading = $state(false);

	async function refresh() {
		loading = true;
		loadError = null;
		try {
			box = await fetchOutbox();
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'The outbox request failed.';
		} finally {
			loading = false;
		}
	}

	onMount(refresh);
</script>

<svelte:head>
	<title>Outbox — staff</title>
</svelte:head>

<main class="mx-auto max-w-3xl space-y-6 p-6">
	<div class="flex items-center justify-between gap-3">
		<h1 class="text-xl font-bold">Outbox</h1>
		<button
			type="button"
			class="min-h-11 rounded border border-line px-4 py-2 disabled:opacity-50"
			disabled={loading}
			onclick={refresh}
		>
			{loading ? 'Loading…' : 'Refresh'}
		</button>
	</div>
	<p class="text-sm">
		The fake outbox keeps every email this server would have sent: sign-in links, follower digests
		and outreach to offices. <strong>Nothing here was sent.</strong> Open a sign-in link from here to
		sign in as that customer.
	</p>

	{#if loadError}
		<Notice variant="error" heading="Outbox unavailable"><p>{loadError}</p></Notice>
	{:else if box === null}
		<div role="status" aria-label="Working" class="space-y-2">
			<div class="h-4 animate-pulse rounded bg-surface-2"></div>
			<div class="h-4 w-2/3 animate-pulse rounded bg-surface-2"></div>
		</div>
	{:else}
		{#if !box.outbox}
			<Notice variant="info" heading="Outgoing email is not in fake mode">
				<p>
					Mail goes out through <code>mailer.provider</code>. Set <code>mailer.mode</code> to
					<code>fake</code> in Config and restart to keep it here instead (loopback and staging only).
				</p>
			</Notice>
		{/if}
		{#if box.messages.length === 0}
			<p class="text-sm text-ink-muted">No kept mail yet.</p>
		{:else}
			<ul class="m-0 list-none space-y-3 p-0" aria-label="Kept messages">
				{#each box.messages as message (message.id)}
					<li class="rounded border border-line p-4" data-testid="outbox-message">
						<div class="flex flex-wrap items-baseline justify-between gap-2">
							<h2 class="m-0 text-base font-semibold break-words">{message.subject || '(no subject)'}</h2>
							<span class="rounded bg-surface-2 px-2 py-0.5 text-xs">{KIND_LABEL[message.kind] ?? message.kind}</span>
						</div>
						<p class="m-0 mt-1 text-sm text-ink-muted">
							To <span class="break-all">{message.to}</span> · {dateTime(message.at)} ·
							<strong class="text-changed">never sent</strong>
						</p>
						{#if Object.keys(message.headers).length > 0}
							<dl class="m-0 mt-2 grid grid-cols-[auto_1fr] gap-x-3 text-xs">
								{#each Object.entries(message.headers) as [name, value] (name)}
									<dt class="text-ink-muted">{name}</dt>
									<dd class="m-0 font-mono break-all">{value}</dd>
								{/each}
							</dl>
						{/if}
						<pre class="mt-2 overflow-auto rounded border border-line bg-surface p-2 text-xs break-all whitespace-pre-wrap">{message.body}</pre>
						{#if box.inbox}
							<a
								class="mt-2 inline-flex min-h-11 items-center rounded border border-line px-4 py-2 text-sm"
								href={`${resolve('/(admin)/compose')}?reply=${message.id}`}
							>
								Reply as {message.to}
							</a>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	{/if}
</main>
