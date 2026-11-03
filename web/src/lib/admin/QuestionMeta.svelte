<script lang="ts">
	import { PRIORITY_LABEL, reasonLabel, topicLabel, type OpenQuestion } from './outreach-client.js';

	// One open question's labels: priority pill (amber = high), topic,
	// and why it is on the list. Shared by /calls and /email.
	let { question }: { question: OpenQuestion } = $props();

	const TONE: Record<OpenQuestion['priority'], string> = {
		high: 'border-changed bg-changed-bg text-changed',
		medium: 'border-line bg-surface-2 text-accent-strong',
		low: 'border-line bg-surface text-ink-muted'
	};
</script>

<span class="flex min-w-0 flex-wrap items-center gap-1.5">
	<span
		class={`inline-flex h-5 items-center gap-1 rounded-full border px-2 text-xs font-medium whitespace-nowrap ${TONE[question.priority]}`}
		data-priority={question.priority}
	>
		<span aria-hidden="true" class="size-1.5 rounded-full bg-current"></span>
		{PRIORITY_LABEL[question.priority]}<span class="sr-only"> priority</span>
	</span>
	<span class="inline-flex h-5 items-center rounded-full border border-line px-2 font-mono text-xs text-ink">
		{topicLabel(question.topic)}
	</span>
	{#if question.reason}
		<span class="text-xs text-ink-muted">{reasonLabel(question.reason)}</span>
	{/if}
</span>
