// Staff browser phone (voice-telnyx.md §12): staff talk on calls
// through this tab. The page logs in to Telnyx WebRTC with a login
// token from `POST /calls/phone/token`; a dial makes Telnyx ring this
// phone, it answers, and the server then dials the office and joins
// the two legs. One instance per tab (module singleton), so a call
// survives the move from /calls to /calls/[id]. Admin-only.
import type { Call, INotification, TelnyxRTC } from '@telnyx/webrtc';
import { fetchPhoneToken } from './calls-client.js';

export type PhonePhase = 'off' | 'connecting' | 'ready' | 'in_call' | 'error';

/** How long a dial may wait for Telnyx to ring this phone. */
const EXPECT_MS = 45_000;
/** How long the SDK may take to log in before we give up. */
const READY_MS = 20_000;

class BrowserPhone {
	phase = $state<PhonePhase>('off');
	error = $state<string | null>(null);
	muted = $state(false);
	/** Our call id whose browser leg this phone answered. */
	callId = $state<string | null>(null);

	#client: TelnyxRTC | null = null;
	#call: Call | null = null;
	#ringing: Call | null = null;
	#expecting: { callId: string; until: number } | null = null;
	#ready: Promise<void> | null = null;
	#audio: HTMLAudioElement | null = null;

	/**
	 * Allow the microphone, fetch a login token and log in. Resolves
	 * once Telnyx says ready; safe to call again (one login at a time).
	 * Call it from a click: browsers only prompt for the microphone
	 * and start audio after a user gesture.
	 */
	connect(apiBase: string): Promise<void> {
		if (this.phase === 'ready' || this.phase === 'in_call') return Promise.resolve();
		if (this.#ready) return this.#ready;
		this.phase = 'connecting';
		this.error = null;
		this.#ready = this.#login(apiBase).then(
			() => {
				this.#ready = null;
			},
			(err: unknown) => {
				this.#ready = null;
				this.#teardown();
				this.phase = 'error';
				this.error = err instanceof Error ? err.message : 'The browser phone could not connect.';
				throw err;
			}
		);
		return this.#ready;
	}

	async #login(apiBase: string) {
		await this.#allowMicrophone();
		const token = await fetchPhoneToken(apiBase);
		const { TelnyxRTC } = await import('@telnyx/webrtc');
		const client = new TelnyxRTC({ login_token: token });
		client.remoteElement = this.#remoteAudio();
		this.#client = client;
		await new Promise<void>((resolve, reject) => {
			const timer = setTimeout(
				() => reject(new Error('Telnyx did not log the browser phone in within 20 s.')),
				READY_MS
			);
			client.on('telnyx.ready', () => {
				clearTimeout(timer);
				this.phase = 'ready';
				resolve();
			});
			client.on('telnyx.error', (event: { error?: { message?: string } }) => {
				clearTimeout(timer);
				const message = event?.error?.message ?? 'Telnyx refused the browser phone.';
				if (this.phase === 'connecting') reject(new Error(message));
				else {
					this.phase = 'error';
					this.error = message;
				}
			});
			client.on('telnyx.socket.close', () => {
				if (this.phase === 'ready' || this.phase === 'in_call') {
					this.#teardown();
					this.phase = 'off';
					this.error = 'The browser phone disconnected. Connect it again to dial.';
				}
			});
			client.on('telnyx.notification', (n: INotification) => this.#onNotification(n));
			void client.connect();
		});
	}

	async #allowMicrophone() {
		if (!navigator.mediaDevices?.getUserMedia) {
			throw new Error('This browser cannot use a microphone here (it needs HTTPS and WebRTC).');
		}
		try {
			const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
			for (const track of stream.getTracks()) track.stop();
		} catch {
			throw new Error('Microphone blocked: allow it for this site to talk on calls.');
		}
	}

	#remoteAudio(): HTMLAudioElement {
		if (!this.#audio) {
			const audio = document.createElement('audio');
			audio.autoplay = true;
			audio.hidden = true;
			document.body.append(audio);
			this.#audio = audio;
		}
		return this.#audio;
	}

	/** A dial for `callId` returned: answer its ringing leg (now or when it rings). */
	expect(callId: string) {
		this.#expecting = { callId, until: Date.now() + EXPECT_MS };
		this.#tryAnswer();
	}

	#onNotification(n: INotification) {
		const call = n.call;
		if (n.type !== 'callUpdate' || !call) return;
		if (call.direction === 'inbound' && call.state === 'ringing') {
			this.#ringing = call;
			this.#tryAnswer();
			// Telnyx can ring before the dial response lands; an
			// incoming call nobody dialed for is refused.
			setTimeout(() => {
				if (this.#ringing === call) {
					this.#ringing = null;
					void call.hangup();
				}
			}, 10_000);
		} else if (call.state === 'active' && call === this.#call) {
			this.phase = 'in_call';
		} else if ((call.state === 'hangup' || call.state === 'destroy') && call === this.#call) {
			this.#call = null;
			this.callId = null;
			this.muted = false;
			if (this.phase === 'in_call') this.phase = 'ready';
		}
	}

	#tryAnswer() {
		const call = this.#ringing;
		const expecting = this.#expecting;
		if (!call || !expecting) return;
		this.#ringing = null;
		this.#expecting = null;
		if (Date.now() > expecting.until) {
			void call.hangup();
			return;
		}
		this.#call = call;
		this.callId = expecting.callId;
		this.phase = 'in_call';
		void call.answer();
	}

	/** Leave the call from this side (the server hangs up both legs too). */
	hangup() {
		void this.#call?.hangup();
	}

	toggleMute() {
		if (!this.#call) return;
		if (this.muted) this.#call.unmuteAudio();
		else this.#call.muteAudio();
		this.muted = !this.muted;
	}

	/** True while leaving the page would drop a live call. */
	get busy() {
		return this.phase === 'in_call';
	}

	#teardown() {
		const client = this.#client;
		this.#client = null;
		this.#call = null;
		this.#ringing = null;
		this.callId = null;
		this.muted = false;
		if (client) void client.disconnect();
	}
}

export const browserPhone = new BrowserPhone();

if (typeof window !== 'undefined') {
	// A reload or tab close drops the WebRTC session and with it the
	// staff side of a live call: ask first.
	window.addEventListener('beforeunload', (event) => {
		if (browserPhone.busy) event.preventDefault();
	});
}
