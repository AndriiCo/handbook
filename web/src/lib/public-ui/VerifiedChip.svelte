<script lang="ts">
	interface Props {
		tier: string;
		verified: string | null;
	}

	let { tier, verified }: Props = $props();
	const chipCls = (ok: boolean) =>
		`inline-block rounded-xs border px-2 py-[0.15rem] font-mono text-[0.8rem] font-semibold ${
			ok ? 'border-success bg-success-bg text-success' : 'border-line bg-surface-2 text-ink-muted'
		}`;
</script>

<span class={chipCls(verified !== null && verified !== '')} data-verified={verified ? 'yes' : 'no'}>
	{verified ? `Verified ${verified}` : tier}
</span>
