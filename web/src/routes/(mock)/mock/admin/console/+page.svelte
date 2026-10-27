<script lang="ts">
	import Notice from '../../../../../lib/public-ui/Notice.svelte';
	import { CONSOLE_TRANSCRIPT } from '../../../../../lib/mock/fixtures';

	let draft = $state('');
	let saved = $state(false);

	function saveDraft(event: SubmitEvent) {
		event.preventDefault();
		saved = draft.trim().length > 0;
	}
</script>

<svelte:head>
	<title>Mock call console — Apostille Handbook</title>
</svelte:head>

<h1>Call console</h1>

<Notice variant="info" heading="Demo call loaded.">
	<p>Fixture transcript below; the draft box keeps text on this page only.</p>
</Notice>

<div class="panes">
	<section aria-labelledby="script">
		<h2 id="script">Script</h2>
		<ol>
			<li>Confirm the authority name and office you reached.</li>
			<li>Ask for the current fee, form revision, and mailing address.</li>
			<li>Read back every fact; note the speaker's name and the time.</li>
		</ol>
	</section>
	<section aria-labelledby="transcript">
		<h2 id="transcript">Transcript</h2>
		<ol class="lines">
			{#each CONSOLE_TRANSCRIPT as turn, i (i)}
				<li><strong>{turn.speaker}:</strong> {turn.line}</li>
			{/each}
		</ol>
	</section>
	<section aria-labelledby="fact-draft">
		<h2 id="fact-draft">Fact draft</h2>
		<form onsubmit={saveDraft}>
			<label class="stack" for="draft">
				Proposed fact (with citation)
				<textarea
					id="draft"
					rows="5"
					bind:value={draft}
					placeholder="e.g. Fee is USD $8.00 — per clerk J. Doe, 2026-10-27 10:02 PT"
				></textarea>
			</label>
			{#if saved}
				<p class="saved" role="status">Draft kept on this page — queue submit is not wired yet.</p>
			{/if}
			<p><button type="submit">Keep draft</button></p>
		</form>
	</section>
</div>

<style>
	.panes {
		display: grid;
		gap: 1rem;
		padding-bottom: 2rem;
	}
	.panes section {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.9rem 1rem;
	}
	.panes h2 {
		font-size: 1rem;
		margin: 0 0 0.5rem;
	}
	.lines {
		margin: 0;
		padding-left: 1.1rem;
		display: grid;
		gap: 0.4rem;
		font-size: 0.9rem;
	}
	.stack {
		display: flex;
		flex-direction: column;
		gap: 0.35rem;
		font-weight: 700;
	}
	textarea {
		font: inherit;
		font-weight: 400;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
		padding: 0.6rem 0.8rem;
		resize: vertical;
	}
	.saved {
		background: var(--success-bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.6rem 0.8rem;
	}
	button {
		min-height: 44px;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
		padding: 0 1.25rem;
	}
	:root[data-theme='dark'] button {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	@media (min-width: 1080px) {
		.panes {
			grid-template-columns: 1fr 1fr 1fr;
		}
	}
</style>
