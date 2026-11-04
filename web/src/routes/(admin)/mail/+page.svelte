<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '#lib/public-ui/Notice.svelte';
	import { getJson, errorText } from '#lib/admin/admin-api.js';
	import { ago, dateTime } from '#lib/admin/format.js';

	// Mail health (E-5): the Gmail poll that turns office notices and
	// replies into captures, and the outgoing budget. The same freshness
	// rule drives `/readyz`: a clean poll within the window is healthy;
	// an error or a quiet poll is not; no poll row means mail is off.
	// Admin build only; the staff API keeps the /apostille prefix.

	interface MailStatus {
		state: 'disabled' | 'ok' | 'failing' | 'stale';
		mailbox: string;
		last_poll_at: number;
		last_error: string;
		max_age_secs: number;
		now: number;
		sends_today: number;
		send_cap: number;
		suppressed: number;
	}

	let status = $state<MailStatus | null>(null);
	let loadError = $state<string | null>(null);
	let loading = $state(false);

	async function refresh() {
		loading = true;
		loadError = null;
		try {
			status = await getJson<MailStatus>('/admin/mail', 'Mail status');
		} catch (err) {
			loadError = errorText(err, 'Mail status request failed.');
		} finally {
			loading = false;
		}
	}

	onMount(refresh);

	const HEADING: Record<MailStatus['state'], string> = {
		ok: 'Polling',
		failing: 'Last poll failed',
		stale: 'Poll has stopped',
		disabled: 'Mail poll is off'
	};
	const VARIANT: Record<MailStatus['state'], 'success' | 'error' | 'warning' | 'info'> = {
		ok: 'success',
		failing: 'error',
		stale: 'warning',
		disabled: 'info'
	};
	const windowMin = $derived(status ? Math.round(status.max_age_secs / 60) : 45);
</script>

<svelte:head>
	<title>Mail — staff</title>
</svelte:head>

<main class="mx-auto max-w-2xl space-y-6 p-6">
	<div class="flex items-center justify-between gap-3">
		<h1 class="text-xl font-bold">Mail</h1>
		<button
			type="button"
			class="min-h-11 rounded border border-line px-4 py-2 disabled:opacity-50"
			disabled={loading}
			onclick={refresh}
		>
			{loading ? 'Checking…' : 'Check again'}
		</button>
	</div>
	<p class="text-sm">
		The Gmail poll reads the handbook mailbox every 15 minutes. Office notices become captures for
		the instruction sweep; replies to outreach attach to the email they answer; bounces and
		complaints stop further mail to that address.
	</p>

	{#if loadError}
		<Notice variant="error" heading="Mail status unavailable"><p>{loadError}</p></Notice>
	{:else if status === null}
		<div role="status" aria-label="Working" class="space-y-2">
			<div class="h-4 animate-pulse rounded bg-surface-2"></div>
			<div class="h-4 w-2/3 animate-pulse rounded bg-surface-2"></div>
		</div>
	{:else}
		<Notice variant={VARIANT[status.state]} heading={HEADING[status.state]}>
			{#if status.state === 'ok'}
				<p>
					Last poll <time datetime={new Date(status.last_poll_at * 1000).toISOString()} title={dateTime(status.last_poll_at)}
						>{ago(status.last_poll_at, Math.max(status.now, status.last_poll_at))}</time
					>, with no error.
				</p>
			{:else if status.state === 'failing'}
				<p>The most recent run reported:</p>
				<pre class="mt-2 overflow-auto rounded border border-line bg-surface p-2 text-xs whitespace-pre-wrap">{status.last_error}</pre>
				<p class="mt-2">
					An expired or revoked Gmail grant reads <code>invalid_grant</code>: repeat the one-time
					consent and seal the new refresh token in Config.
				</p>
			{:else if status.state === 'stale'}
				<p>
					No poll in the last {windowMin} minutes{#if status.last_poll_at > 0}; the last one finished
						{dateTime(status.last_poll_at)}{/if}. Check that <code>apostille-mail.timer</code> is
					enabled on the server.
				</p>
			{:else}
				<p>
					The poll has never run here. Seal <code>mail.client_id</code>,
					<code>mail.client_secret</code> and <code>mail.refresh_token</code> in Config, then enable
					<code>apostille-mail.timer</code>.
				</p>
			{/if}
		</Notice>

		<dl class="grid grid-cols-1 gap-3 sm:grid-cols-2">
			<div class="rounded border border-line p-4">
				<dt class="text-sm text-ink-muted">Mailbox polled</dt>
				<dd class="m-0 font-medium break-all">{status.mailbox || '—'}</dd>
			</div>
			<div class="rounded border border-line p-4">
				<dt class="text-sm text-ink-muted">Last completed poll</dt>
				<dd class="m-0 font-medium">
					{status.last_poll_at > 0 ? dateTime(status.last_poll_at) : '—'}
				</dd>
			</div>
			<div class="rounded border border-line p-4">
				<dt class="text-sm text-ink-muted">Sent in the last 24 hours</dt>
				<dd class="m-0 font-medium">{status.sends_today} of {status.send_cap}</dd>
			</div>
			<div class="rounded border border-line p-4">
				<dt class="text-sm text-ink-muted">Addresses stopped (bounces, complaints)</dt>
				<dd class="m-0 font-medium">{status.suppressed}</dd>
			</div>
		</dl>
	{/if}
</main>
