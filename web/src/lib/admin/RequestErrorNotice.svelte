<script lang="ts">
	import Notice from '../public-ui/Notice.svelte';

	// A refused admin request, triage-ready: the message, the
	// `x-request-id` the server logged it under (copyable, the same id
	// as the browser network tab's response header), and, for a stale
	// passkey step-up, one button that confirms and retries in place.
	let {
		heading = 'Save failed.',
		message,
		requestId = null,
		stepUp = false,
		busy = false,
		onConfirm
	}: {
		heading?: string;
		message: string;
		requestId?: string | null;
		stepUp?: boolean;
		busy?: boolean;
		onConfirm?: () => void;
	} = $props();

	let copied = $state(false);

	async function copy() {
		if (!requestId) return;
		try {
			await navigator.clipboard.writeText(requestId);
			copied = true;
			setTimeout(() => (copied = false), 2000);
		} catch {
			copied = false;
		}
	}
</script>

<Notice variant={stepUp ? 'warning' : 'error'} heading={stepUp ? 'Confirm it’s you.' : heading}>
	<p class="m-0 text-sm">{message}</p>
	{#if stepUp && onConfirm}
		<p class="mt-2 mb-0">
			<button
				type="button"
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 text-sm font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
				disabled={busy}
				onclick={onConfirm}
			>
				{busy ? 'Waiting for your passkey…' : 'Confirm with passkey and save'}
			</button>
		</p>
	{/if}
	{#if requestId}
		<p class="mt-2 mb-0 flex flex-wrap items-center gap-2 text-xs text-ink-muted">
			<span>Request ID</span>
			<code class="rounded-xs border border-line bg-surface px-1 font-mono break-all text-ink"
				>{requestId}</code
			>
			<button
				type="button"
				class="min-h-8 cursor-pointer rounded-xs border border-line px-2 text-ink hover:border-focus"
				onclick={copy}
				aria-label="Copy request ID"
			>
				{copied ? 'Copied' : 'Copy'}
			</button>
		</p>
	{/if}
</Notice>
