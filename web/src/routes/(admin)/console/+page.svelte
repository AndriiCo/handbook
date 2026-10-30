<script lang="ts">
	import Notice from '../../../lib/public-ui/Notice.svelte';

	// Call console shell: three-pane per site-theme §1.2 (script /
	// transcript / fact-draft). Live realtime is a Week 4 cut — this
	// slice keeps file-upload transcript → proposed facts only.
	let draft = $state('');
	let saved = $state(false);

	function saveDraft(event: SubmitEvent) {
		event.preventDefault();
		saved = draft.trim().length > 0;
	}
</script>

<svelte:head>
	<title>Call console — Admin — Apostille Handbook</title>
</svelte:head>

<h1>Call console</h1>

<Notice variant="info" heading="No active calls.">
	<p>
		Live realtime lands Week 4. This slice drafts facts from uploaded
		transcripts; paste or type below.
	</p>
</Notice>

<div class="grid gap-4 pb-8 lg:grid-cols-3">
	<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="script">
		<h2 class="m-0 mb-2 text-base" id="script">Script</h2>
		<ol>
			<li>Confirm the authority name and office you reached.</li>
			<li>Ask for the current fee, form revision, and mailing address.</li>
			<li>Read back every fact; note the speaker's name and the time.</li>
		</ol>
	</section>
	<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="transcript">
		<h2 class="m-0 mb-2 text-base" id="transcript">Transcript</h2>
		<p class="text-ink-muted">No transcript loaded. Upload handling lands with the ingest lane.</p>
	</section>
	<section class="rounded-sm border border-line px-4 py-[0.9rem]" aria-labelledby="fact-draft">
		<h2 class="m-0 mb-2 text-base" id="fact-draft">Fact draft</h2>
		<form onsubmit={saveDraft}>
			<label class="flex flex-col gap-[0.35rem] font-bold" for="draft">
				Proposed fact (with citation)
				<textarea
					class="resize-y rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-normal text-ink"
					id="draft"
					rows="5"
					bind:value={draft}
					placeholder="e.g. Fee is USD $8.00 — per clerk J. Doe, 2026-10-27 10:02 PT"
				></textarea>
			</label>
			{#if saved}
				<p class="rounded-sm border border-line bg-success-bg px-[0.8rem] py-[0.6rem]" role="status">
					Draft kept on this page — queue submit is not wired yet.
				</p>
			{/if}
			<p>
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink"
					type="submit">Keep draft</button
				>
			</p>
		</form>
	</section>
</div>
