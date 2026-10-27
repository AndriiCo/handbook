<script lang="ts">
	import { resolve } from '$app/paths';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	const sections: { label: string; links: { href: string; label: string }[] }[] = [
		{
			label: 'Public',
			links: [
				{ href: resolve('/(mock)/mock/search'), label: 'Search' },
				{ href: resolve('/(mock)/mock/authority/[slug]', { slug: 'demo-secretary-north' }), label: 'Dossier' }
			]
		},
		{
			label: 'Customer',
			links: [
				{ href: resolve('/(mock)/mock/login'), label: 'Login' },
				{ href: resolve('/(mock)/mock/orders'), label: 'Orders' },
				{ href: resolve('/(mock)/mock/order'), label: 'New order' },
				{ href: resolve('/(mock)/mock/uploads'), label: 'Uploads' },
				{ href: resolve('/(mock)/mock/precheck'), label: 'Precheck' },
				{ href: resolve('/(mock)/mock/billing'), label: 'Billing' }
			]
		},
		{
			label: 'Admin',
			links: [
				{ href: resolve('/(mock)/mock/admin'), label: 'Dashboard' },
				{ href: resolve('/(mock)/mock/admin/triage'), label: 'Triage' },
				{ href: resolve('/(mock)/mock/admin/console'), label: 'Console' },
				{ href: resolve('/(mock)/mock/admin/config'), label: 'Config' }
			]
		}
	];
</script>

<p class="banner" role="note">
	<strong>Mock preview</strong> — fixture data only, nothing here touches the backend.
	<a href={resolve('/(mock)/mock')}>Mock index</a>
</p>

<nav class="mocknav" aria-label="Mock pages">
	{#each sections as section (section.label)}
		<span class="group">
			<span class="group-label">{section.label}</span>
			{#each section.links as link (link.href + link.label)}
				<a href={link.href}>{link.label}</a>
			{/each}
		</span>
	{/each}
</nav>

{@render children()}

<style>
	.banner {
		background: var(--warning-bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.6rem 1rem;
		font-size: 0.9rem;
	}
	.mocknav {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem 1.5rem;
		padding: 0.5rem 0 1rem;
		border-bottom: 1px solid var(--border);
		margin-bottom: 1rem;
	}
	.group {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.25rem 0.6rem;
	}
	.group-label {
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-muted);
	}
	.mocknav a {
		min-height: 44px;
		display: inline-flex;
		align-items: center;
	}
</style>
