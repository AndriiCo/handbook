<script lang="ts">
	import type { Snippet } from 'svelte';
	import InfoIcon from '@lucide/svelte/icons/info';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import OctagonXIcon from '@lucide/svelte/icons/octagon-x';

	interface Props {
		variant: 'info' | 'success' | 'warning' | 'error';
		heading?: string;
		children: Snippet;
	}

	let { variant, heading, children }: Props = $props();
	const icons = {
		info: InfoIcon,
		success: CircleCheckIcon,
		warning: TriangleAlertIcon,
		error: OctagonXIcon
	};
	const Icon = $derived(icons[variant]);
	// Full class strings per variant (never stacked): exactly one
	// background wins on each element.
	const boxCls = (v: Props['variant']) =>
		`flex items-start gap-3 rounded-sm border border-line px-4 py-3 ${
			v === 'success'
				? 'bg-success-bg'
				: v === 'warning'
					? 'bg-warning-bg'
					: v === 'error'
						? 'bg-error-bg'
						: 'bg-surface'
		}`;
	const iconCls = (v: Props['variant']) =>
		`inline-flex size-6 flex-none items-center justify-center rounded-xs text-accent-ink ${
			v === 'success' ? 'bg-success' : v === 'warning' ? 'bg-warning' : v === 'error' ? 'bg-error' : 'bg-accent-fill'
		}`;
</script>

<div class={boxCls(variant)} role={variant === 'error' ? 'alert' : 'status'}>
	<span class={iconCls(variant)} aria-hidden="true"><Icon class="size-4" /></span>
	<div>
		{#if heading}<p class="m-0 mb-1 font-bold">{heading}</p>{/if}
		{@render children()}
	</div>
</div>

<style>
	/* Slotted-child resets stay global CSS: the children render from
	   the CALLER's scope, so scoped utilities cannot reach them. */
	div :global(p:last-child) {
		margin-bottom: 0;
	}
	div :global(p:first-child) {
		margin-top: 0;
	}
</style>
