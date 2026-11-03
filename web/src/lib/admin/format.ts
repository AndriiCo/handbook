// Number, time and money formatters shared by the admin console.
// Pure functions: no DOM, no fetch.

const INT = new Intl.NumberFormat('en-US');
const COMPACT = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 });

export function fmtInt(n: number | null | undefined): string {
	return typeof n === 'number' && Number.isFinite(n) ? INT.format(n) : '—';
}

/** 412000 → "412K" (token counts). */
export function fmtCompact(n: number | null | undefined): string {
	return typeof n === 'number' && Number.isFinite(n) ? COMPACT.format(n) : '—';
}

/** 2900 → "2.9 s", 640 → "640 ms", null → "—". */
export function fmtMs(ms: number | null | undefined): string {
	if (typeof ms !== 'number' || !Number.isFinite(ms)) return '—';
	if (ms < 1000) return `${Math.round(ms)} ms`;
	return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`;
}

/** 0.612 → "61%". */
export function fmtPct(rate: number | null | undefined): string {
	if (typeof rate !== 'number' || !Number.isFinite(rate)) return '—';
	return `${Math.round(rate * 100)}%`;
}

export function fmtCents(cents: number | null | undefined, currency = 'usd'): string {
	if (typeof cents !== 'number' || !Number.isFinite(cents)) return '—';
	try {
		return new Intl.NumberFormat('en-US', {
			style: 'currency',
			currency: currency.toUpperCase()
		}).format(cents / 100);
	} catch {
		return `${(cents / 100).toFixed(2)} ${currency.toUpperCase()}`;
	}
}

/** Relative age of a unix-seconds stamp against `now` (unix seconds). */
export function ago(at: number, now: number): string {
	const secs = Math.max(0, Math.floor(now - at));
	if (secs < 45) return 'just now';
	const mins = Math.round(secs / 60);
	if (mins < 60) return `${mins} min ago`;
	const hours = Math.floor(mins / 60);
	if (hours < 48) return `${hours} h ago`;
	return `${Math.floor(hours / 24)} d ago`;
}

export function clock(at: number): string {
	return new Date(at * 1000).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
}

export function hourLabel(at: number): string {
	return new Date(at * 1000).toLocaleTimeString('en-US', { hour: 'numeric' });
}

export function dayLabel(at: number): string {
	return new Date(at * 1000).toLocaleDateString('en-US', { weekday: 'short' });
}

export function dateTime(at: number): string {
	return new Date(at * 1000).toLocaleString('en-US', {
		month: 'short',
		day: 'numeric',
		hour: 'numeric',
		minute: '2-digit'
	});
}

/** Display name for a Meta model id. */
export function modelName(id: string): string {
	if (id.startsWith('muse-spark')) return 'Muse Spark';
	if (id.startsWith('muse-wonder')) return 'Muse Wonder';
	if (id.startsWith('sam-')) return 'SAM';
	if (id.startsWith('muse-voice')) return 'Muse Voice';
	return id;
}
