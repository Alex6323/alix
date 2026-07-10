// End-to-end smoke suite for the kids web client, run against the real
// `alix` binary (see ../playwright.config.ts) over the frozen fixture deck in
// ../fixtures/decks/animals/wild.txt. The fixture deck carries NO progress
// store (see ../fixtures/README.md) — every run starts from a deck nobody has
// reviewed yet, so the suite always exercises a real kid's first session, the
// never-seen (*acquire*) path included.
//
// This exists because two real bugs shipped past unit tests, code review, and
// a contract suite, and were only ever found by a human clicking:
//
//   1. The box screen POSTed a *workspace* name to /api/select, which 400s.
//      `api()` did `(await fetch()).json()`, so the empty error body made
//      `.json()` throw with no `.catch` — the button silently did nothing.
//   2. A never-seen (acquire) card skipped the attempt entirely, so the
//      depth a kid chose ("Tap the answer" vs "Say it yourself") changed
//      nothing.
//
// So every test here asserts the full chain: a click causes the expected
// request, the expected response, the expected screen — never just the
// screen. `pageErrors` (see helpers.ts) is an auto-fixture that fails any
// test which logged an uncaught page error or console.error.
//
// Not covered here: the "a wrong Recognize pick can only record failed" rule
// (fix c46dad5) needs a card that has already been seen once (acquired) AND
// is due again — the real acquire cooldown is a fixed 60s server-side
// constant, so reaching that state deterministically would mean either a
// real ~60s wait or committing pre-warmed progress state, and the latter is
// exactly what this fixture must not do (see fixtures/README.md). That
// specific bar-hiding rule is client-side JS with no other automated
// coverage right now — only the server's ChooseFeedbackDto.passed (the
// signal that rule reads) is covered, by the contract suite.
import { test, expect } from "./helpers";

// Tests share one running server and one review session on it (see
// `fullyParallel: false` / `workers: 1` in playwright.config.ts), so they run
// in file order and later tests may rely on earlier ones having navigated —
// each still starts from a fresh page load, though, so no test depends on a
// previous test's on-page state.

test("home lists the Animals box", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".box", { hasText: "Animals" })).toBeVisible();
});

test("a box drills into its decks, and a deck offers the two depth choices", async ({ page }) => {
  await page.goto("/");
  await page.locator(".box", { hasText: "Animals" }).click();

  const deckRow = page.locator(".deck-row", { hasText: "wild" });
  await expect(deckRow).toBeVisible();
  await expect(deckRow.evaluate((el) => el.tagName)).resolves.toBe("BUTTON");

  await deckRow.click();

  await expect(page.getByRole("button", { name: "Tap the answer" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Say it yourself" })).toBeVisible();
});

test('clicking "Tap the answer" selects the deck at recognize depth and shows a tappable question', async ({
  page,
}) => {
  await page.goto("/");
  await page.locator(".box", { hasText: "Animals" }).click();
  await page.locator(".deck-row", { hasText: "wild" }).click();

  const [request, response] = await Promise.all([
    page.waitForRequest((req) => req.url().includes("/api/select") && req.method() === "POST"),
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Tap the answer" }).click(),
  ]);

  // The user's own spec for this test: a click on "Tap the answer" MUST
  // result in this exact request reaching the server, and a real 200 back —
  // not just "the screen looks right afterwards".
  expect(request.postDataJSON()).toEqual(expect.objectContaining({ depth: "recognize" }));
  expect(response.status(), await response.text().catch(() => "")).toBe(200);

  const options = page.locator(".opt-btn");
  await expect(options.first()).toBeVisible();
  const count = await options.count();
  expect(count).toBeGreaterThan(1);
  for (let i = 0; i < count; i++) {
    await expect(options.nth(i)).toBeEnabled();
  }
});

test('clicking "Say it yourself" shows a reveal control, not options', async ({ page }) => {
  await page.goto("/");
  await page.locator(".box", { hasText: "Animals" }).click();
  await page.locator(".deck-row", { hasText: "wild" }).click();

  const [response] = await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Say it yourself" }).click(),
  ]);
  expect(response.status()).toBe(200);

  // The depth choice must actually change the presentation — bug #2 made it
  // a no-op. A Recall card self-grades after a reveal; it never taps options.
  await expect(page.getByRole("button", { name: "Show me" })).toBeVisible();
  await expect(page.locator(".opt-btn")).toHaveCount(0);
});

test("tapping an option on a never-seen card records the pick and offers only the ungraded next step", async ({
  page,
}) => {
  await page.goto("/");
  await page.locator(".box", { hasText: "Animals" }).click();
  await page.locator(".deck-row", { hasText: "wild" }).click();

  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Tap the answer" }).click(),
  ]);

  const firstFront = await page.locator(".rev-prompt").textContent();

  const options = page.locator(".opt-btn");
  await expect(options.first()).toBeVisible();

  const [chooseResponse] = await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose") && res.request().method() === "POST"),
    options.first().click(),
  ]);
  expect(chooseResponse.status()).toBe(200);

  // Exactly one option is ever marked correct, regardless of what was tapped
  // (the response body names it — see ChooseFeedbackDto.correct).
  await expect(page.locator(".opt-correct")).toHaveCount(1);

  // Bug #2's shape, pinned directly: a never-seen card is *attempted* (the
  // pick above), never skipped — but it's still ungraded on a first meeting.
  // Only the single acknowledge-and-move-on control appears, never a rate
  // bar (right or wrong pick alike — there is nothing to self-rate yet).
  await expect(page.getByRole("button", { name: "Got it! Next" })).toBeVisible();
  await expect(page.locator(".rate-got")).toHaveCount(0);
  await expect(page.locator(".rate-again")).toHaveCount(0);
  await expect(page.locator(".rate-quiet")).toHaveCount(0);

  const [acquireResponse] = await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/acquire") && res.request().method() === "POST"),
    page.getByRole("button", { name: "Got it! Next" }).click(),
  ]);
  expect(acquireResponse.status()).toBe(200);

  // The session actually moves on to the deck's other card, rather than
  // silently sitting on the same unanswered question.
  await expect(page.locator(".rev-prompt")).toBeVisible();
  await expect(page.locator(".rev-prompt")).not.toHaveText(firstFront ?? "");
});
