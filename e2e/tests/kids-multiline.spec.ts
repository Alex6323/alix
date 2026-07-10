// Regression coverage for a third shipped bug: the kids client's
// `answerFill` used to flatten a multi-line answer into one run-on string
// via `back.join(" ")` (fixed in 1129c04), turning an ordered sequence into
// nonsense. Nothing else in this suite exercises a multi-line answer —
// ../fixtures/decks/animals/wild.txt's two cards are both single-line.
//
// This deck (cats.txt) exists solely for this test, in its own file, so it
// never touches wild.txt's card ids (which key the committed augment.json —
// see ../README.md).
import { test, expect } from "./helpers";
import { kidsDeckRow, openApp } from "./helpers";

test("a multi-line answer renders as separate lines, not one joined string", async ({ page }) => {
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "cats").click();

  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Say it yourself" }).click(),
  ]);

  await page.getByRole("button", { name: "Show me" }).click();

  const lines = page.locator(".rev-answer-fill");
  await expect(lines).toHaveCount(2);
  await expect(lines.nth(0)).toHaveText("Lion");
  await expect(lines.nth(1)).toHaveText("Tiger");

  // Pins the regression directly: joining would produce ONE element reading
  // "Lion Tiger" instead of two separate ones.
  await expect(page.locator(".rev-answer-fill", { hasText: "Lion Tiger" })).toHaveCount(0);
});
