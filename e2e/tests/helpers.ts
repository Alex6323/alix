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
// `page.goto` defaults to `waitUntil: "load"`, which waits for EVERY subresource
// — including the kids page's five Baloo woff2 files. `alix` serves requests on
// one thread (`for request in server.incoming_requests()`, src/serve/mod.rs), so
// under a real browser's keep-alive connections on a loaded runner one of those
// parallel font requests can go unanswered, and `load` never fires. That is a
// real server property (tracked as {#server-subresource-stall}); it is not what
// these tests are about. They assert DOM and network behaviour, and every
// assertion below auto-waits — so wait for the DOM, not for webfonts.
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
