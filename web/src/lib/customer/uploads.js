// Customer-only upload helpers. Pure functions (tested indirectly via
// page behavior); never imported from (public) routes.

const MAX_FILES = 10;
const MAX_BYTES = 25 * 1024 * 1024; // 25 MB per file

/**
 * Human file size, e.g. 1536 → "1.5 KB".
 * @param {number} bytes
 * @returns {string}
 */
export function formatBytes(bytes) {
	if (!Number.isFinite(bytes) || bytes < 0) return '—';
	if (bytes < 1024) return `${bytes} B`;
	const units = ['KB', 'MB', 'GB'];
	let value = bytes / 1024;
	let unit = 0;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit += 1;
	}
	return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

/**
 * Validate a FileList/File[] before upload. Returns { ok, errors }.
 * @param {ArrayLike<File> | null | undefined} files
 * @returns {{ ok: boolean, errors: string[] }}
 */
export function validateFiles(files) {
	const list = Array.from(files ?? []);
	const errors = [];
	if (list.length === 0) errors.push('Choose at least one file.');
	if (list.length > MAX_FILES) errors.push(`At most ${MAX_FILES} files per upload.`);
	for (const file of list) {
		if (file.size > MAX_BYTES) errors.push(`“${file.name}” is over ${formatBytes(MAX_BYTES)}.`);
	}
	return { ok: errors.length === 0, errors };
}
