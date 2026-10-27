<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		variant: 'info' | 'success' | 'warning' | 'error';
		heading?: string;
		children: Snippet;
	}

	let { variant, heading, children }: Props = $props();
	const icons: Record<Props['variant'], string> = {
		info: 'i',
		success: '✓',
		warning: '!',
		error: '×'
	};
</script>

<div class="notice {variant}" role={variant === 'error' ? 'alert' : 'status'}>
	<span class="icon" aria-hidden="true">{icons[variant]}</span>
	<div>
		{#if heading}<p class="heading">{heading}</p>{/if}
		{@render children()}
	</div>
</div>

<style>
	.notice {
		display: flex;
		gap: 0.75rem;
		align-items: flex-start;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
		background: var(--surface);
	}
	.notice.success {
		background: var(--success-bg);
	}
	.notice.warning {
		background: var(--warning-bg);
	}
	.notice.error {
		background: var(--error-bg);
	}
	.icon {
		flex: none;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 1.5rem;
		height: 1.5rem;
		border-radius: var(--radius-xs);
		font-weight: 700;
		font-family: var(--font-mono);
		color: var(--accent-ink);
		background: var(--accent);
	}
	.success .icon {
		background: var(--success);
	}
	.warning .icon {
		background: var(--warning);
	}
	.error .icon {
		background: var(--error);
	}
	.heading {
		margin: 0 0 0.25rem;
		font-weight: 700;
	}
	.notice :global(p:last-child) {
		margin-bottom: 0;
	}
	.notice :global(p:first-child) {
		margin-top: 0;
	}
</style>
