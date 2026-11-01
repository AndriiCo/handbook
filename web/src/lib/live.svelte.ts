// Live bus client for GP screens and the admin console: one shared
// WebSocket per topic, refcounted across subscribers. The bus takes
// one `?topic=` per connection; anonymous callers may subscribe to
// `authority:{slug}` only (server-enforced in ws_bus + auth::ws).
// Anything else needs a session cookie, same-origin. No polling on
// GP screens.
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

const sockets = new Map<string, { ws: WebSocket; handlers: Set<Handler> }>();

function busUrl(topic: string): string {
	const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
	return `${scheme}://${window.location.host}/apostille/ws/bus?topic=${encodeURIComponent(topic)}`;
}

/**
 * Subscribe to a bus topic. Returns an unsubscribe function. Shared
 * sockets close when the last subscriber leaves. Server-side calls
 * are a no-op unsubscribe (no WebSocket during SSR).
 */
export function subscribe(topic: string, onMessage: Handler): () => void {
	if (typeof window === 'undefined') return () => {};
	let entry = sockets.get(topic);
	if (!entry) {
		const ws = new WebSocket(busUrl(topic));
		const handlers = new Set<Handler>();
		entry = { ws, handlers };
		sockets.set(topic, entry);
		ws.addEventListener('message', (ev) => {
			let msg: BusMessage;
			try {
				msg = JSON.parse(String(ev.data));
			} catch {
				return;
			}
			for (const h of handlers) {
				try {
					h(msg);
				} catch {
					// One bad subscriber never breaks the others.
				}
			}
		});
		const drop = () => {
			if (sockets.get(topic)?.ws === ws) sockets.delete(topic);
		};
		ws.addEventListener('close', drop);
		ws.addEventListener('error', drop);
	}
	const current = entry;
	current.handlers.add(onMessage);
	let gone = false;
	return () => {
		if (gone) return;
		gone = true;
		current.handlers.delete(onMessage);
		if (current.handlers.size === 0) {
			if (sockets.get(topic)?.ws === current.ws) sockets.delete(topic);
			try {
				current.ws.close();
			} catch {
				// Already gone; the close listener cleans up.
			}
		}
	};
}
