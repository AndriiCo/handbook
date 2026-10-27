// Tiny WS client: topic envelopes + reconnect backoff + status.
// No dependencies. Lazy-imported only from (customer)/(admin) pages —
// never from the (public) entry chunk (see docs/specs/spa-separation.md §1.2).
//
// Envelope (server → client): { topic, type, ref, seq, payload }.
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
 * Open a topic socket. Status callback gets 'live' | 'reconnecting' | 'offline'.
 * Returns { close, status } — no Svelte dependency so admin/customer pages share it.
 * @param {string} url
 * @param {WsOptions} [options]
 * @returns {{ close: () => void, status: () => WsStatus }}
 */
export function createWsClient(url, { topics = [], onMessage, onStatus } = {}) {
	/** @type {WebSocket | null} */
	let socket = null;
	let attempt = 0;
	let closed = false;
	/** @type {ReturnType<typeof setTimeout> | null} */
	let timer = null;
	/** @type {WsStatus} */
	let current = 'offline';
	const wanted = topics.filter(isValidTopic);

	/** @param {WsStatus} next */
	function setStatus(next) {
		if (current === next) return;
		current = next;
		onStatus?.(next);
	}

	function connect() {
		if (closed) return;
		setStatus(attempt === 0 ? 'offline' : 'reconnecting');
		/** @type {WebSocket} */
		let ws;
		try {
			ws = new WebSocket(url);
		} catch {
			schedule();
			return;
		}
		socket = ws;
		ws.onopen = () => {
			attempt = 0;
			setStatus('live');
			for (const topic of wanted) ws.send(JSON.stringify({ type: 'subscribe', topic }));
		};
		ws.onmessage = (event) => {
			const msg = parseEnvelope(typeof event.data === 'string' ? event.data : '');
			if (msg) onMessage?.(msg);
		};
		ws.onclose = () => {
			socket = null;
			if (!closed) schedule();
		};
		ws.onerror = () => {
			// onclose follows; backoff is handled there.
		};
	}

	function schedule() {
		if (closed) return;
		setStatus('reconnecting');
		const wait = backoffMs(attempt);
		attempt += 1;
		timer = setTimeout(connect, wait);
	}

	connect();

	return {
		close() {
			closed = true;
			if (timer) clearTimeout(timer);
			try {
				socket?.close();
			} catch {
				// Already gone.
			}
			socket = null;
			setStatus('offline');
		},
		status() {
			return current;
		}
	};
}
