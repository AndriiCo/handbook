<script lang="ts">
	import { statusLabel, statusTone } from './outreach-client.js';

	// Outreach email status pill: green once it left (sent, replied),
	// amber while a human still has to act (draft), red on a failed
	// send, neutral otherwise.
	let { status }: { status: string } = $props();

	const TONE = {
		done: 'border-verified bg-verified-bg text-verified',
		active: 'border-line bg-surface-2 text-accent-strong',
		attention: 'border-changed bg-changed-bg text-changed',
		error: 'border-error bg-error-bg text-error',
		muted: 'border-line bg-surface text-ink-muted'
	} as const;
</script>

<span
	class={`inline-flex h-6 items-center gap-1.5 rounded-full border px-2 text-xs font-medium whitespace-nowrap transition-colors duration-200 ${TONE[statusTone(status)]}`}
	data-status={status}
>
	<span aria-hidden="true" class={`size-1.5 rounded-full bg-current ${status === 'sending' ? 'motion-safe:animate-pulse' : ''}`}></span>
	{statusLabel(status)}
</span>
