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

<p class="rounded-sm border border-line bg-warning-bg px-4 py-[0.6rem] text-[0.9rem]" role="note">
	<strong>Mock preview</strong> — fixture data only, nothing here touches the backend.
	<a href={resolve('/(mock)/mock')}>Mock index</a>
</p>

<nav
	class="mb-4 flex flex-wrap gap-x-6 gap-y-2 border-b border-line px-0 pt-2 pb-4"
	aria-label="Mock pages"
>
	{#each sections as section (section.label)}
		<span class="flex flex-wrap items-baseline gap-x-[0.6rem] gap-y-1">
			<span class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">{section.label}</span>
			{#each section.links as link (link.href + link.label)}
				<a class="inline-flex min-h-11 items-center" href={link.href}>{link.label}</a>
			{/each}
		</span>
	{/each}
</nav>

{@render children()}
