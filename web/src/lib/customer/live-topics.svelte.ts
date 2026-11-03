// Cabinet live wiring: one subscription per server-authored topic
// (`billing:{customer_ref}`, `shipment:{id}`) through the shared bus
// store. Events carry ids only, so callers re-fetch the API; bursts
// coalesce into one refetch. No polling.
import { subscribe, type BusMessage } from '../live.svelte';
import { isCabinetTopic } from './orders';

export function liveTopics(
	topics: () => string[],
	onEvent: (topic: string, msg: BusMessage) => void
) {
	let live = $state(false);
	let lastAt = $state<number | null>(null);
	// A string key: a re-fetched order with the same legs keeps its
	// sockets (no unsubscribe/resubscribe churn on every refresh).
	const key = $derived([...new Set(topics().filter(isCabinetTopic))].sort().join(' '));

	$effect(() => {
		const wanted = key ? key.split(' ') : [];
		const offs = wanted.map((topic) =>
			subscribe(topic, (msg) => {
				live = true;
				if (msg.type === 'subscribed') return;
				lastAt = Date.now();
				onEvent(topic, msg);
			})
		);
		return () => {
			for (const off of offs) off();
		};
	});

	return {
		get live() {
			return live;
		},
		get lastAt() {
			return lastAt;
		}
	};
}

/** Trailing debounce: a burst of ids-only events → one re-read. */
export function coalesce(fn: () => void, ms = 250): () => void {
	let timer: ReturnType<typeof setTimeout> | null = null;
	return () => {
		if (timer !== null) clearTimeout(timer);
		timer = setTimeout(() => {
			timer = null;
			fn();
		}, ms);
	};
}
