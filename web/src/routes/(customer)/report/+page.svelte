<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import ReportForm from '../../../lib/public-ui/ReportForm.svelte';

	// Walk-in report form (SA-21) on its own page: the no-JS target of
	// a dossier's "Report a change" link (with JS it opens docked beside
	// the dossier instead) and the place to report an office by slug.
	// The form itself (immediate photo uploads, the draft kept on this
	// device until sent) is lib/public-ui/ReportForm.svelte.

	const apiBase = resolve('/(public)').replace(/\/$/, '');
	const destination = (page.url.searchParams.get('destination') ?? '').trim() || null;
</script>

<svelte:head>
	<title>Report a change — Apostille Handbook</title>
</svelte:head>

<Heading level={1}>Report a change</Heading>
<p class="mt-2 max-w-[40rem] text-ink-muted">
	Spotted a changed fee, address, or step at an issuing office? File a walk-in report. Photos are
	checked on your device and stripped of camera metadata on our end; your note stays private while
	personal details are removed.
</p>

<div class="mt-6 max-w-[40rem]">
	<ReportForm slug={destination} {apiBase} loginHref={resolve('/(customer)/login')} />
</div>
