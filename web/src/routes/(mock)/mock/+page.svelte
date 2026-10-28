<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
</script>

<svelte:head>
	<title>Mock index — Apostille Handbook</title>
</svelte:head>

<h1>Clickable mock</h1>

<Notice variant="info" heading="Fixture data only.">
	<p>
		Every page under <code class="font-mono text-[0.85em]">/mock</code> renders invented data from
		<code class="font-mono text-[0.85em]">$lib/mock/fixtures.ts</code> and makes zero backend calls.
		Layouts mirror the real routes so reviewers can click the full journey: search → dossier →
		login → order → uploads → precheck → admin review.
	</p>
</Notice>

<div class="grid gap-4 pb-8 md:grid-cols-3">
	<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="public">
		<h2 class="m-0 mb-2 text-base" id="public">Public</h2>
		<ul class="m-0 pl-[1.1rem]">
			<li class="my-[0.3rem]">
				<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/search')}
					>Search</a
				> — query the fixture index
			</li>
			<li class="my-[0.3rem]">
				<a
					class="inline-flex min-h-11 items-center font-bold"
					href={resolve('/(mock)/mock/authority/[slug]', { slug: 'demo-secretary-north' })}
					>Authority dossier</a
				> — tabs, fee receipt, citations
			</li>
		</ul>
	</section>
	<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="customer">
		<h2 class="m-0 mb-2 text-base" id="customer">Customer</h2>
		<ul class="m-0 pl-[1.1rem]">
			<li class="my-[0.3rem]">
				<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/login')}
					>Login states</a
				> —Logged out, error, logged in
			</li>
			<li class="my-[0.3rem]">
				<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/orders')}
					>Orders</a
				> — status list
			</li>
			<li class="my-[0.3rem]">
				<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/order')}
					>Forms fill flow</a
				> — stepped new-order form
			</li>
			<li class="my-[0.3rem]">
				<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/uploads')}
					>Uploads</a
				> — file states
			</li>
			<li class="my-[0.3rem]">
				<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/precheck')}
					>Precheck</a
				> — pass / warn / fail checks
			</li>
			<li class="my-[0.3rem]">
				<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/billing')}
					>Billing</a
				> — invoice list
			</li>
		</ul>
	</section>
	<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="admin">
		<h2 class="m-0 mb-2 text-base" id="admin">Admin</h2>
		<ul class="m-0 pl-[1.1rem]">
			<li class="my-[0.3rem]">
				<a class="inline-flex min-h-11 items-center font-bold" href={resolve('/(mock)/mock/admin')}
					>Dashboard</a
				> — queue counts
			</li>
			<li class="my-[0.3rem]">
				<a
					class="inline-flex min-h-11 items-center font-bold"
					href={resolve('/(mock)/mock/admin/triage')}>Triage</a
				> — accept / reject with reasons
			</li>
			<li class="my-[0.3rem]">
				<a
					class="inline-flex min-h-11 items-center font-bold"
					href={resolve('/(mock)/mock/admin/console')}>Console</a
				> — script / transcript / draft
			</li>
			<li class="my-[0.3rem]">
				<a
					class="inline-flex min-h-11 items-center font-bold"
					href={resolve('/(mock)/mock/admin/config')}>Config</a
				> — typed keys, secret entry
			</li>
		</ul>
	</section>
</div>
