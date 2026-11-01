// Live bus client for GP screens and the admin console: one shared
// WebSocket per topic, refcounted across subscribers. The bus takes
// one `?topic=` per connection; anonymous callers may subscribe to
// `authority:{slug}` only (server-enforced in ws_bus + auth::ws).
// Anything else needs a session cookie, same-origin. No polling on
// GP screens.
//
// Drops reconnect with jittered exponential back-off and resubscribe:
// handlers stay registered across retries, so a `/review` approval
// that lands mid-blink still arrives. The server re-sends its hello
// on every connect (callers already ignore `type: 'subscribed'`).
// The server's 30 s ping is answered by the browser itself — no
// client heartbeat needed.
//
// The API mount is `/apostille` on BOTH hosts (public base IS
// `/apostille`; the admin host serves its API under `/apostille`
// too), so the socket always targets this host's `/apostille/ws/bus`
// — never `$app/paths`, whose base is '' in the admin build.

export type BusMessage = {
	type?: string;
	[key: string]: unknown;
};

type Handler = (msg: BusMessage) => void;

type Entry = {
	ws: WebSocket | null;
	handlers: Set<Handler>;
	attempt: number;
	timer: ReturnType<typeof setTimeout> | null;
};

const sockets = new Map<string, Entry>();

const BACKOFF_BASE_MS = 1000;
const BACKOFF_CAP_MS = 30000;

/** Jittered exponential back-off (same shape as ws-client.js). */
export function backoffMs(attempt: number): number {
	const base = Math.min(BACKOFF_BASE_MS * 2 ** Math.max(0, attempt), BACKOFF_CAP_MS);
	return base * (0.75 + Math.random() * 0.5);
}

function busUrl(topic: string): string {
	const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
	return `${scheme}://${window.location.host}/apostille/ws/bus?topic=${encodeURIComponent(topic)}`;
}

function connect(topic: string, entry: Entry): void {
	let ws: WebSocket;
	try {
		ws = new WebSocket(busUrl(topic));
	} catch {
		schedule(topic, entry);
		return;
	}
	entry.ws = ws;
	ws.addEventListener('message', (ev) => {
		let msg: BusMessage;
		try {
			msg = JSON.parse(String(ev.data));
		} catch {
			return;
		}
		for (const h of entry.handlers) {
			try {
				h(msg);
			} catch {
				// One bad subscriber never breaks the others.
			}
		}
	});
	ws.addEventListener('open', () => {
		entry.attempt = 0;
	});
	ws.addEventListener('close', () => {
		if (entry.ws !== ws) return;
		entry.ws = null;
		// Explicit unsubscribe cleared the entry or emptied the
		// handlers: stay down. Otherwise the drop was network —
		// retry with the handlers still registered.
		if (sockets.get(topic) === entry && entry.handlers.size > 0) schedule(topic, entry);
		else if (sockets.get(topic) === entry) sockets.delete(topic);
	});
}

function schedule(topic: string, entry: Entry): void {
	if (entry.timer !== null) return;
	const wait = backoffMs(entry.attempt);
	entry.attempt += 1;
	entry.timer = setTimeout(() => {
		entry.timer = null;
		if (sockets.get(topic) === entry && entry.handlers.size > 0) connect(topic, entry);
	}, wait);
}

/**
 * Subscribe to a bus topic. Returns an unsubscribe function. Shared
 * sockets close when the last subscriber leaves; unexpected drops
 * reconnect on their own. Server-side calls are a no-op
 * unsubscribe (no WebSocket during SSR).
 */
export function subscribe(topic: string, onMessage: Handler): () => void {
	if (typeof window === 'undefined') return () => {};
	let entry = sockets.get(topic);
	if (!entry) {
		entry = { ws: null, handlers: new Set(), attempt: 0, timer: null };
		sockets.set(topic, entry);
		connect(topic, entry);
	}
	const current = entry;
	current.handlers.add(onMessage);
	let gone = false;
	return () => {
		if (gone) return;
		gone = true;
		current.handlers.delete(onMessage);
		if (current.handlers.size === 0) {
			if (current.timer !== null) {
				clearTimeout(current.timer);
				current.timer = null;
			}
			if (sockets.get(topic) === current) sockets.delete(topic);
			try {
				current.ws?.close();
			} catch {
				// Already gone; the close listener cleans up.
			}
			current.ws = null;
		}
	};
}
