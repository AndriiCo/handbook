// Tiny WS client: topic envelopes + reconnect backoff + status.
// No dependencies. Lazy-imported only from (customer)/(admin) pages —
// never from the (public) entry chunk (see docs/specs/spa-separation.md §1.2).
//
// Protocol: one socket per topic, `?topic=` set at upgrade (the server
// authorizes the topic during the handshake and ignores client frames).
// Envelope (server → client): { topic, type, ... }.
// Topics: authority:{slug} | call:{id} | submission:{id} | shipment:{id} | billing:{id}.

/** @typedef {'live' | 'reconnecting' | 'offline'} WsStatus */
/** @typedef {{ topic: string, type: string, seq?: number, payload?: unknown }} Envelope */
/**
 * @typedef {object} WsOptions
 * @property {string[]} [topics]
 * @property {(msg: Envelope) => void} [onMessage]
 * @property {(status: WsStatus) => void} [onStatus]
 */

/** True when the client asked to skip animated inserts. */
export function prefersReducedMotion() {
	if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false;
	return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

const TOPIC_RE = /^(authority|call|submission|shipment|billing):[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;

/**
 * Validate a topic string before subscribing.
 * @param {unknown} topic
 * @returns {topic is string}
 */
export function isValidTopic(topic) {
	return typeof topic === 'string' && TOPIC_RE.test(topic);
}

/**
 * Exponential backoff with jitter: 500ms × 2^attempt, capped at 30s. Pure (tested).
 * @param {number} attempt
 * @returns {number}
 */
export function backoffMs(attempt) {
	const base = Math.min(500 * 2 ** Math.max(0, attempt), 30_000);
	return Math.round(base * (0.8 + Math.random() * 0.4));
}

/**
 * Parse one inbound frame. Returns null for anything malformed. Pure (tested).
 * @param {string} raw
 * @returns {Envelope | null}
 */
export function parseEnvelope(raw) {
	/** @type {any} */
	let msg;
	try {
		msg = JSON.parse(raw);
	} catch {
		return null;
	}
	if (!msg || typeof msg !== 'object') return null;
	if (typeof msg.topic !== 'string' || typeof msg.type !== 'string') return null;
	if (!isValidTopic(msg.topic)) return null;
	return msg;
}

/**
 * Subscription URL for one topic. Pure (tested).
 * @param {string} url
 * @param {string} topic
 * @returns {string}
 */
export function topicUrl(url, topic) {
	const u = new URL(url);
	u.searchParams.set('topic', topic);
	return u.toString();
}

/**
 * Open one socket per topic. Status is aggregate: 'live' when any
 * socket is open, 'reconnecting' while retrying, 'offline' when
 * closed or when no valid topic was given. No Svelte dependency so
 * admin/customer pages share it.
 * @param {string} url
 * @param {WsOptions} [options]
 * @returns {{ close: () => void, status: () => WsStatus }}
 */
export function createWsClient(url, { topics = [], onMessage, onStatus } = {}) {
	const wanted = [...new Set(topics.filter(isValidTopic))];
	let closed = false;
	/** @type {WsStatus} */
	let current = 'offline';
	/** @type {Set<string>} */
	const live = new Set();
	/** @type {Map<string, { socket: WebSocket | null, timer: ReturnType<typeof setTimeout> | null, attempt: number }>} */
	const conns = new Map();

	/** @param {WsStatus} next */
	function setStatus(next) {
		if (current === next) return;
		current = next;
		onStatus?.(next);
	}

	function aggregate() {
		if (closed || conns.size === 0) setStatus('offline');
		else if (live.size > 0) setStatus('live');
		else setStatus('reconnecting');
	}

	/** @param {string} topic */
	function connect(topic) {
		const conn = conns.get(topic);
		if (!conn || closed) return;
		/** @type {WebSocket} */
		let ws;
		try {
			ws = new WebSocket(topicUrl(url, topic));
		} catch {
			schedule(topic);
			return;
		}
		conn.socket = ws;
		ws.onopen = () => {
			conn.attempt = 0;
			live.add(topic);
			aggregate();
		};
		ws.onmessage = (event) => {
			const msg = parseEnvelope(typeof event.data === 'string' ? event.data : '');
			if (msg) onMessage?.(msg);
		};
		ws.onclose = () => {
			conn.socket = null;
			live.delete(topic);
			aggregate();
			if (!closed) schedule(topic);
		};
		ws.onerror = () => {
			// onclose follows; backoff is handled there.
		};
	}

	/** @param {string} topic */
	function schedule(topic) {
		const conn = conns.get(topic);
		if (!conn || closed) return;
		aggregate();
		const wait = backoffMs(conn.attempt);
		conn.attempt += 1;
		conn.timer = setTimeout(() => connect(topic), wait);
	}

	for (const topic of wanted) {
		conns.set(topic, { socket: null, timer: null, attempt: 0 });
		connect(topic);
	}
	if (wanted.length === 0) {
		// No subscription possible: report offline so pages don't hang
		// on "connecting".
		queueMicrotask(() => {
			if (!closed) onStatus?.('offline');
		});
	}

	return {
		close() {
			closed = true;
			for (const conn of conns.values()) {
				if (conn.timer) clearTimeout(conn.timer);
				try {
					conn.socket?.close();
				} catch {
					// Already gone.
				}
				conn.socket = null;
			}
			setStatus('offline');
		},
		status() {
			return current;
		}
	};
}
