// Shared test wiring for the kids-client smoke suite.
//
// The bug this whole suite exists to catch was a click that silently did
// nothing: a rejected fetch promise nobody awaited, with no visible trace
// except a browser console error. `pageErrors` is an auto-fixture — every
// test gets it before its body runs, and its teardown asserts nothing was
// logged. A test doesn't have to remember to opt in.
import { test as base, expect } from "@playwright/test";

type Fixtures = {
  pageErrors: string[];
};

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
