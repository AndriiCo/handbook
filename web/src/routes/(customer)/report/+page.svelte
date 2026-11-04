<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import ReportForm from '../../../lib/public-ui/ReportForm.svelte';
	import { site } from '../../../lib/locale.svelte.js';
	import { REPORT_COPY } from '../../../lib/public-ui/report-copy.js';

	// Walk-in report form (SA-21) on its own page: the no-JS target of
	// a dossier's "Report a change" link (with JS it opens docked beside
	// the dossier instead) and the place to report an office by slug.
	// The form itself (immediate photo uploads, the draft kept on this
	// device until sent) is lib/public-ui/ReportForm.svelte.

	const apiBase = resolve('/(public)').replace(/\/$/, '');
	const c = $derived(REPORT_COPY[site.locale]);
	const destination = (page.url.searchParams.get('destination') ?? '').trim() || null;
</script>

<svelte:head>
	<title>{c.title} — Apostille Handbook</title>
</svelte:head>

<Heading level={1}>{c.title}</Heading>
<p class="mt-2 max-w-[40rem] text-ink-muted">{c.intro}</p>

<div class="mt-6 max-w-[40rem]">
	<ReportForm slug={destination} {apiBase} loginHref={resolve('/(customer)/login')} />
</div>
