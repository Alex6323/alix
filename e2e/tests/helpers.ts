// Shared test wiring for the kids-client smoke suite.
//
// The bug this whole suite exists to catch was a click that silently did
// nothing: a rejected fetch promise nobody awaited, with no visible trace
// except a browser console error. `pageErrors` is an auto-fixture — every
// test gets it before its body runs, and its teardown asserts nothing was
// logged. A test doesn't have to remember to opt in.
import { test as base, expect, type Locator, type Page } from "@playwright/test";

type Fixtures = {
  pageErrors: string[];
};

// A kids `.deck-row` also carries a mastery pill and a `›` chevron, so
// matching the row's whole text loosely (`hasText: "wild"`) stays unique only
// by accident — a second deck whose name happens to contain the same
// substring would silently match too. Target the exact label instead.
// Open the app under test.
//
// `page.goto` defaults to `waitUntil: "load"`, which waits for EVERY subresource.
// The kids page requests eight in parallel (`/`, `/api/decks`, `/alix-logo.js`,
// five Baloo woff2). On the first CI run, four navigations hit the 60s timeout;
// the trace showed seven of those eight answered and one font with no response
// recorded at the moment Playwright gave up.
//
// There is no evidence of a server bug. That was the FIRST ever CI run, with cold
// caches (a 114MB browser download and a full cargo compile); the very next run —
// one second later, warm, and still using `waitUntil: "load"` — passed. So the
// failure did not depend on the code, and this change did not fix it. See
// {#server-subresource-stall}.
//
// The change stands on its own: these tests assert DOM and network behaviour and
// every assertion auto-waits, so they must not gate on webfont delivery. That
// removes a cold-start flake source. It is not a fix for anything.
export async function openApp(page: Page): Promise<void> {
  await page.goto("/", { waitUntil: "domcontentloaded" });
}

export function kidsDeckRow(page: Page, name: string): Locator {
  return page.locator(".deck-row").filter({ has: page.locator(".deck-label", { hasText: name, exact: true }) });
}

// Same idea for the adult picker's `.deckrow` (name in a `.name` span,
// plus optional badges/meta after it).
export function adultDeckRow(page: Page, name: string): Locator {
  return page.locator(".deckrow").filter({ has: page.locator(".name", { hasText: name, exact: true }) });
}

export const test = base.extend<Fixtures>({
  // eslint-disable-next-line no-empty-pattern
  pageErrors: [
    async ({ page }, use) => {
      const errors: string[] = [];
      page.on("pageerror", (err) => errors.push(`pageerror: ${err.message}`));
      page.on("console", (msg) => {
        if (msg.type() === "error") errors.push(`console.error: ${msg.text()}`);
      });

      await use(errors);

      expect(errors, `unexpected page/console errors:\n${errors.join("\n")}`).toEqual([]);
    },
    { auto: true },
  ],
});

export { expect };
