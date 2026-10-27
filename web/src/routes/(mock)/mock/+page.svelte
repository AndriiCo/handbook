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
		Every page under <code>/mock</code> renders invented data from
		<code>$lib/mock/fixtures.ts</code> and makes zero backend calls. Layouts mirror the real
		routes so reviewers can click the full journey: search → dossier → login → order →
		uploads → precheck → admin review.
	</p>
</Notice>

<div class="cols">
	<section aria-labelledby="public">
		<h2 id="public">Public</h2>
		<ul>
			<li><a href={resolve('/(mock)/mock/search')}>Search</a> — query the fixture index</li>
			<li>
				<a href={resolve('/(mock)/mock/authority/[slug]', { slug: 'demo-secretary-north' })}
					>Authority dossier</a
				> — tabs, fee receipt, citations
			</li>
		</ul>
	</section>
	<section aria-labelledby="customer">
		<h2 id="customer">Customer</h2>
		<ul>
			<li><a href={resolve('/(mock)/mock/login')}>Login states</a> —Logged out, error, logged in</li>
			<li><a href={resolve('/(mock)/mock/orders')}>Orders</a> — status list</li>
			<li><a href={resolve('/(mock)/mock/order')}>Forms fill flow</a> — stepped new-order form</li>
			<li><a href={resolve('/(mock)/mock/uploads')}>Uploads</a> — file states</li>
			<li><a href={resolve('/(mock)/mock/precheck')}>Precheck</a> — pass / warn / fail checks</li>
			<li><a href={resolve('/(mock)/mock/billing')}>Billing</a> — invoice list</li>
		</ul>
	</section>
	<section aria-labelledby="admin">
		<h2 id="admin">Admin</h2>
		<ul>
			<li><a href={resolve('/(mock)/mock/admin')}>Dashboard</a> — queue counts</li>
			<li><a href={resolve('/(mock)/mock/admin/triage')}>Triage</a> — accept / reject with reasons</li>
			<li><a href={resolve('/(mock)/mock/admin/console')}>Console</a> — script / transcript / draft</li>
			<li><a href={resolve('/(mock)/mock/admin/config')}>Config</a> — typed keys, secret entry</li>
		</ul>
	</section>
</div>

<style>
	.cols {
		display: grid;
		gap: 1rem;
		padding-bottom: 2rem;
	}
	section {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.9rem 1rem;
	}
	h2 {
		font-size: 1rem;
		margin: 0 0 0.5rem;
	}
	ul {
		margin: 0;
		padding-left: 1.1rem;
	}
	li {
		margin: 0.3rem 0;
	}
	li a {
		font-weight: 700;
		min-height: 44px;
		display: inline-flex;
		align-items: center;
	}
	code {
		font-family: var(--font-mono);
		font-size: 0.85em;
	}
	@media (min-width: 720px) {
		.cols {
			grid-template-columns: repeat(3, 1fr);
		}
	}
</style>
