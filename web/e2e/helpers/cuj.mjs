// Critical-User-Journey action counter. Every interaction a spec
// performs goes through here so journeys assert interaction
// budgets: `actions` (clicks, checks, submits, navigations) and
// `inputs` (text fills, counted separately — typing is
// irreducible, but prefill and option-picking reduce it).
// Budgets live in the specs and ratchet down: a UI change that
// adds steps fails CI until its budget line is consciously
// raised with a recorded reason.

/**
 * @param {import('@playwright/test').Page} page
 */
export function cuj(page) {
	const counts = { actions: 0, inputs: 0 };
	return {
		counts,
		async goto(url) {
			counts.actions++;
			await page.goto(url);
		},
		async click(locator) {
			counts.actions++;
			await locator.click();
		},
		async check(locator) {
			counts.actions++;
			await locator.check();
		},
		async select(locator, value) {
			counts.actions++;
			await locator.selectOption(value);
		},
		async fill(locator, value) {
			counts.inputs++;
			await locator.fill(value);
		},
		summary() {
			return { ...counts };
		}
	};
}
