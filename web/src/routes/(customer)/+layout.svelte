<script lang="ts">
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();
</script>

<!-- Customer account navigation lives in the header account menu
     (Orders, New order, Billing, Security, Sign out). No sub-nav strip. -->
{@render children()}
