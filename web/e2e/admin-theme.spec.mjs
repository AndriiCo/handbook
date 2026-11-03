// Staff pages built on raw Tailwind greys rendered light text on light
// panels under the dark theme (the Packs load log read white on white).
// Packs and Labels now use theme tokens: every text node keeps WCAG AA
// contrast against its painted background in both themes.
import { expect, test } from "@playwright/test";
import { serveDir } from "./helpers/serve.mjs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));

// Worst text/background contrast over the elements matched by `selector`.
async function worstContrast(page, selector) {
  return page.evaluate((sel) => {
    const rgb = (c) => (c.match(/[\d.]+/g) || []).slice(0, 4).map(Number);
    const lum = ([r, g, b]) =>
      [r, g, b]
        .map((v) => v / 255)
        .map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
        .reduce((a, v, i) => a + v * [0.2126, 0.7152, 0.0722][i], 0);
    const background = (el) => {
      for (let n = el; n; n = n.parentElement) {
        const c = rgb(getComputedStyle(n).backgroundColor);
        if (c.length && (c[3] === undefined || c[3] > 0.5)) return c;
      }
      return rgb(getComputedStyle(document.body).backgroundColor);
    };
    let worst = Infinity;
    let where = "";
    for (const el of document.querySelectorAll(sel)) {
      if (!el.textContent.trim()) continue;
      const fg = lum(rgb(getComputedStyle(el).color));
      const bg = lum(background(el));
      const ratio = (Math.max(fg, bg) + 0.05) / (Math.min(fg, bg) + 0.05);
      if (ratio < worst) {
        worst = ratio;
        where = `${el.tagName} "${el.textContent.trim().slice(0, 40)}"`;
      }
    }
    return { worst, where };
  }, selector);
}

for (const theme of ["light", "dark"]) {
  test(`packs load result stays readable in the ${theme} theme`, async ({
    page,
  }) => {
    const srv = await serveDir(join(HERE, "..", "build-admin"));
    try {
      await page.addInitScript((t) => localStorage.setItem("theme", t), theme);
      await page.route("**/apostille/api/v1/setup/status", (route) =>
        route.fulfill({ json: { setup_required: false } }),
      );
      await page.route("**/apostille/api/v1/admin/packs", (route) =>
        route.fulfill({
          json: {
            sets: [
              { set: "curated", packs: 9 },
              { set: "world", packs: 130 },
              { set: "world-nonparty", packs: 0 },
            ],
            authorities: 3,
            running: false,
          },
        }),
      );
      await page.route("**/apostille/api/v1/admin/packs/load", (route) =>
        route.fulfill({
          json: {
            ok: true,
            set: "world",
            before: 15,
            after: 510,
            stdout:
              "AD: +1 authorities +1 domains +1 sources +3 tasks\npacks-load: 130 pack(s)\n",
            stderr: "",
          },
        }),
      );
      await page.goto(`${srv.url}/packs`);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.getByRole("button", { name: "Load" }).nth(1).click();
      const result = page.getByRole("region", { name: "Load result" });
      await expect(result).toContainText("packs-load: 130 pack(s)");
      const { worst, where } = await worstContrast(
        page,
        ["h1", "h2", "p", "pre", "button"]
          .map((t) => `main.max-w-2xl ${t}`)
          .join(", "),
      );
      expect(worst, `lowest contrast at ${where}`).toBeGreaterThanOrEqual(4.5);
    } finally {
      await srv.close();
    }
  });

  test(`labels form stays readable in the ${theme} theme`, async ({ page }) => {
    const srv = await serveDir(join(HERE, "..", "build-admin"));
    try {
      await page.addInitScript((t) => localStorage.setItem("theme", t), theme);
      await page.route("**/apostille/api/v1/setup/status", (route) =>
        route.fulfill({ json: { setup_required: false } }),
      );
      await page.route("**/apostille/api/v1/fedex-live/caps", (route) =>
        route.fulfill({
          json: { enabled: false, daily_cap: 2, bought_today: 0, max_usd: 60 },
        }),
      );
      await page.goto(`${srv.url}/labels`);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.getByText("Live buys: off")).toBeVisible();
      const { worst, where } = await worstContrast(
        page,
        ["h1", "h2", "p", "li", "label", "button"]
          .map((t) => `main.max-w-2xl ${t}`)
          .join(", "),
      );
      expect(worst, `lowest contrast at ${where}`).toBeGreaterThanOrEqual(4.5);
    } finally {
      await srv.close();
    }
  });
}
