<script lang="ts">
	interface Props {
		tier: string;
		verified: string | null;
	}

	let { tier, verified }: Props = $props();
</script>

<span class="chip" data-verified={verified ? 'yes' : 'no'}>
	{verified ? `Verified ${verified}` : tier}
</span>

<style>
	.chip {
		display: inline-block;
		border-radius: var(--radius-xs);
		padding: 0.15rem 0.5rem;
		font-family: var(--font-mono);
		font-size: 0.8rem;
		font-weight: 600;
		background: var(--surface-2);
		color: var(--text-muted);
		border: 1px solid var(--border);
	}
	.chip[data-verified='yes'] {
		background: var(--success-bg);
		color: var(--success);
		border-color: var(--success);
	}
</style>
