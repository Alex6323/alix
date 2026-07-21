// End-to-end smoke suite for the ADULT web client (assets/web/review.html),
// run against the real `alix` binary (see ../playwright.config.ts) over the
// same frozen fixture workspace the kids suite uses
// (../fixtures/decks/animals/). See kids-review.spec.ts for the bug class
// this exists to catch (a click that never reaches the server, or reaches it
// with the wrong data) and `pageErrors` (helpers.ts) for the auto-fixture
// that fails any test which logged an uncaught page error or console.error.
//
// Unlike the kids client, the adult client resumes whatever session the
// server still has selected on page load (`load()` calls GET /api/state and
// renders wherever it left off), so a previous test's unfinished review
// would otherwise leak into the next one. `beforeEach` forces a clean slate.
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test("the picker lists the fixture workspace and its decks", async ({ page }) => {
  const animals = adultDeckRow(page, "Animals");
  await expect(animals).toBeVisible();
  await animals.click();

  await expect(adultDeckRow(page, "wild")).toBeVisible();
  await expect(adultDeckRow(page, "cats")).toBeVisible();
});

test("clicking a deck row fires POST /api/select, and a card front renders", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click(); // focuses the row; doesn't launch it yet

  const [request, response] = await Promise.all([
    page.waitForRequest((req) => req.url().includes("/api/select") && req.method() === "POST"),
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Learn" }).click(),
  ]);

  expect(request.postDataJSON()).toEqual(expect.objectContaining({ deck: "animals/wild.md" }));
  expect(response.status(), await response.text().catch(() => "")).toBe(200);

  await expect(page.locator(".front-text")).toBeVisible();
  // The header carries the one in-session readout: the "N left" token.
  await expect(page.locator("#hist .left-token")).toHaveText(/^\d+ left$/);
});

test("focusing a deck opens the drawer with its preamble and heatmap, no due count", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click(); // focuses the row → opens the drawer

  // A sibling row's drawer may still be animating closed as wild's opens, so
  // wait for a single stable drawer before asserting on it.
  await expect(page.locator(".drawer")).toHaveCount(1);
  const drawer = page.locator(".drawer");
  await expect(drawer.locator(".drawer-preamble")).toHaveText(/wild animals/i);
  await expect(drawer.locator(".crumb-cell")).toHaveCount(2); // one per stamped card
  await expect(page.locator(".drawer-due")).toHaveCount(0); // the old due count is gone
  await drawer.screenshot({ path: "/tmp/claude-1000/-home-me-dev-developer-alex6323-projects-flashcard2-claude-agent-2/ea6ad9c5-47cc-4ff1-9a0d-b19dd66cad08/scratchpad/drawer.png" });
});

test("the ☰ menu opens without error", async ({ page }) => {
  await page.locator("#kebab").click();
  await expect(page.locator("#menu")).toHaveClass(/open/);
  await expect(page.locator("#mAdd")).toBeVisible(); // a picker-context item, since nothing is selected
  await page.locator("#kebab").click(); // close it again
});

// KNOWN GAP — reported as skipped on every run, deliberately.
//
// The fixture ships no progress store, so every card is never-seen (`acquire`)
// and the adult app posts /api/acquire, never /api/grade. Reaching a genuinely
// graded card needs one past the server's acquire cooldown (5 min default; a
// sleep or a committed pre-warmed store are both banned — see ../README.md,
// "fixture contract").
//
// Two leads worth verifying: (a) since 2026-07-14 the cooldown is configurable
// (`[review] acquire_cooldown`, "0" = none) — a fixture config with a zero
// cooldown would make graded cards reachable in one run. (b) `POST /api/select
// {cram: true}` is documented to queue cards that are not due; if it bypasses
// the cooldown for an already-acquired card, this test becomes cheap. Verify
// either with curl before writing the test — do not assume.
//
// The same gap blocks the kids honest-grading rule (a wrong Recognize pick must
// only ever record `failed`). Neither has automated coverage today.
test.fixme("grading fires POST /api/grade and advances the session", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Learn" }).click(),
  ]);
  await expect(page.locator(".front-text")).toBeVisible();
  const firstFront = await page.locator(".front-text").textContent();

  // Reveal key (default Space), then a grade key (default "n" = passed) —
  // see [keys.review] / Bindings::default in src/config.rs.
  await page.keyboard.press("Space");
  const [response] = await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/grade") && res.request().method() === "POST"),
    page.keyboard.press("n"),
  ]);
  expect(response.status()).toBe(200);

  await expect(page.locator(".front-text")).not.toHaveText(firstFront ?? "");
});
