import { defineConfig } from '@playwright/test';

// Browser suite for the built SPAs (see e2e/README.md). No
// `webServer` block: specs start their own static server on
// port 0 (repo rule) via e2e/helpers/serve.mjs and drive the
// fresh `build`/`build-admin` output `cargo xtask ci` produces
// just before this gate.
export default defineConfig({
	testDir: './e2e',
	timeout: 30_000,
	use: { headless: true },
	projects: [
		{
			name: 'chromium',
			// PW_CHANNEL=chrome runs against system Chrome instead
			// of the bundled build (local iteration only; CI uses
			// the pinned `npx playwright install chromium`).
			use: { channel: process.env.PW_CHANNEL || undefined }
		}
	]
});
