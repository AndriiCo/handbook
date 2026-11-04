// Customer-only upload helpers. Pure functions (tested indirectly via
// page behavior); never imported from (public) routes.

const MAX_FILES = 10;
const MAX_BYTES = 25 * 1024 * 1024; // 25 MB per file

/** File-check messages per site language (lib/i18n.ts LOCALES). */
const CHECK_COPY = {
	en: {
		none: 'Choose at least one file.',
		tooMany: (/** @type {number} */ n) => `At most ${n} files per upload.`,
		tooBig: (/** @type {string} */ name, /** @type {string} */ size) => `“${name}” is over ${size}.`
	},
	es: {
		none: 'Elija al menos un archivo.',
		tooMany: (/** @type {number} */ n) => `Como máximo ${n} archivos por subida.`,
		tooBig: (/** @type {string} */ name, /** @type {string} */ size) => `«${name}» supera los ${size}.`
	},
	fr: {
		none: 'Choisissez au moins un fichier.',
		tooMany: (/** @type {number} */ n) => `${n} fichiers au maximum par envoi.`,
		tooBig: (/** @type {string} */ name, /** @type {string} */ size) => `« ${name} » dépasse ${size}.`
	}
};

/** French writes octets (Ko, Mo, Go); English and Spanish write bytes. */
const UNITS = { en: ['B', 'KB', 'MB', 'GB'], es: ['B', 'KB', 'MB', 'GB'], fr: ['o', 'Ko', 'Mo', 'Go'] };

/**
 * Human file size, e.g. 1536 → "1.5 KB" ("1,5 Ko" in French).
 * @param {number} bytes
 * @param {'en' | 'es' | 'fr'} [locale]
 * @returns {string}
 */
export function formatBytes(bytes, locale = 'en') {
	if (!Number.isFinite(bytes) || bytes < 0) return '—';
	const units = UNITS[locale] ?? UNITS.en;
	if (bytes < 1024) return `${bytes} ${units[0]}`;
	let value = bytes / 1024;
	let unit = 1;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit += 1;
	}
	const digits = value >= 100 ? 0 : 1;
	const number =
		locale === 'en'
			? value >= 100
				? String(Math.round(value))
				: value.toFixed(1)
			: new Intl.NumberFormat(locale, {
					minimumFractionDigits: digits,
					maximumFractionDigits: digits
				}).format(value);
	return `${number} ${units[unit]}`;
}

/**
 * Validate a FileList/File[] before upload. Returns { ok, errors }.
 * @param {ArrayLike<File> | null | undefined} files
 * @param {'en' | 'es' | 'fr'} [locale] language of the messages
 * @returns {{ ok: boolean, errors: string[] }}
 */
export function validateFiles(files, locale = 'en') {
	const list = Array.from(files ?? []);
	const copy = CHECK_COPY[locale] ?? CHECK_COPY.en;
	const errors = [];
	if (list.length === 0) errors.push(copy.none);
	if (list.length > MAX_FILES) errors.push(copy.tooMany(MAX_FILES));
	for (const file of list) {
		if (file.size > MAX_BYTES) errors.push(copy.tooBig(file.name, formatBytes(MAX_BYTES, locale)));
	}
	return { ok: errors.length === 0, errors };
}
