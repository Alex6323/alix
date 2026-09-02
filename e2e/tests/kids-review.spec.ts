// End-to-end smoke suite for the kids web client, run against the real
// `alix` binary (see ../playwright.config.ts) over the frozen fixture deck in
// ../fixtures/decks/animals/decks/wild.md. The fixture deck carries NO progress
// store (see ../fixtures/README.md) — every run starts from a deck nobody has
// reviewed yet, so the suite always exercises a real kid's first session, the
// never-seen (*introduction*) path included.
//
// This exists because two real bugs shipped past unit tests, code review, and
// a contract suite, and were only ever found by a human clicking:
//
//   1. The box screen POSTed a *workspace* name to /api/select, which 400s.
//      `api()` did `(await fetch()).json()`, so the empty error body made
//      `.json()` throw with no `.catch` — the button silently did nothing.
//   2. A never-seen (introduction) card skipped the attempt entirely, so the
//      depth a kid chose ("Tap the answer" vs "Say it yourself") changed
//      nothing.
//
// So every test here asserts the full chain: a click causes the expected
// request, the expected response, the expected screen — never just the
// screen. `pageErrors` (see helpers.ts) is an auto-fixture that fails any
// test which logged an uncaught page error or console.error.
import { test, expect } from "./helpers";
import { kidsDeckRow, openApp } from "./helpers";

// Tests share one running server and one review session on it (see
// `fullyParallel: false` / `workers: 1` in playwright.config.ts), so they run
// in file order and later tests may rely on earlier ones having navigated —
// each still starts from a fresh page load, though, so no test depends on a
// previous test's on-page state.

test("home lists the Animals box", async ({ page }) => {
  await openApp(page);
  await expect(page.locator(".box", { hasText: "Animals" })).toBeVisible();

  const settings = page.getByRole("button", { name: "Settings" });
  await settings.click();
  await expect(page.getByRole("menu", { name: "Colours" })).toBeVisible();
  await expect(settings).toHaveAttribute("aria-expanded", "true");

  await page.getByRole("button", { name: "Ocean" }).click();
  await expect(page.getByRole("menu", { name: "Colours" })).toBeHidden();
  await expect(settings).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(":root")).toHaveCSS("--accent", "#0fa8b4");

  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(page.locator(":root")).toHaveCSS("--accent", "#0fa8b4");
});

test("a box drills into its decks, and a deck offers the two depth choices", async ({ page }) => {
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();

  const deckRow = kidsDeckRow(page, "wild");
  await expect(deckRow).toBeVisible();
  await expect(deckRow.evaluate((el) => el.tagName)).resolves.toBe("BUTTON");

  await deckRow.click();

  await expect(page.getByRole("button", { name: "Tap the answer" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Say it yourself" })).toBeVisible();
});

test('clicking "Tap the answer" selects the deck at recognize depth and shows a tappable question', async ({
  page,
}) => {
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "wild").click();

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
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "wild").click();

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

test("a task-list front renders as static checkboxes", async ({ page }) => {
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "fronts").click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Say it yourself" }).click(),
  ]);

  await expect(page.locator(".rev-prompt .checklist-row")).toHaveCount(2);
  await expect(page.locator(".rev-prompt .checklist-box")).toHaveText(["☑", "☐"]);
});

test("tapping an option on a never-seen card records the pick and offers only the ungraded next step", async ({
  page,
}) => {
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "wild").click();

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
  await expect(page.locator(".rev-why .checklist-row")).toHaveCount(2);
  await expect(page.locator(".rev-why .checklist-box")).toHaveText(["☑", "☐"]);

  // Bug #2's shape, pinned directly: a never-seen card is *attempted* (the
  // pick above), never skipped — but it's still ungraded on a first meeting.
  // Only the single acknowledge-and-move-on control appears, never a rate
  // bar (right or wrong pick alike — there is nothing to self-rate yet).
  await expect(page.getByRole("button", { name: "Got it! Next" })).toBeVisible();
  await expect(page.locator(".rate-got")).toHaveCount(0);
  await expect(page.locator(".rate-again")).toHaveCount(0);
  await expect(page.locator(".rate-quiet")).toHaveCount(0);

  const [introduceResponse] = await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/introduce") && res.request().method() === "POST"),
    page.getByRole("button", { name: "Got it! Next" }).click(),
  ]);
  expect(introduceResponse.status()).toBe(200);

  // The session actually moves on to the deck's other card, rather than
  // silently sitting on the same unanswered question.
  await expect(page.locator(".rev-prompt")).toBeVisible();
  await expect(page.locator(".rev-prompt")).not.toHaveText(firstFront ?? "");
});

test("a resynced study response does not close the kids tutor or lose its transcript", async ({
  page,
  pageErrors,
}) => {
  await page.route("**/api/ask", (route) => route.fulfill({
    json: {
      transcript: [{ q: "Why?", a: "Because this card says so." }],
      thinking: false,
      status: null,
      error: null,
    },
  }));
  await page.route("**/api/choose", async (route) => {
    const headers = await route.request().allHeaders();
    await route.continue({
      headers: { ...headers, "x-alix-study-revision": "0" },
    });
  });

  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "wild").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: "Tap the answer" }).click(),
  ]);

  await page.getByRole("button", { name: "Ask Alix" }).click();
  const tutor = page.locator("#askOverlay");
  await expect(tutor).toBeVisible();
  await expect(tutor.locator(".ask-bubble")).toHaveText([
    "Why?",
    "Because this card says so.",
  ]);

  const resynced = page.waitForResponse((response) =>
    response.url().includes("/api/state") && response.request().method() === "GET"
  );
  const rejected = page.waitForResponse((response) =>
    response.url().includes("/api/choose") && response.request().method() === "POST"
  );
  const previousPrompt = await page.locator(".rev-prompt").elementHandle();
  expect(previousPrompt).not.toBeNull();
  await page.locator(".opt-btn").first().evaluate((element: HTMLButtonElement) => element.click());
  expect((await rejected).status()).toBe(409);
  expect((await resynced).status()).toBe(200);
  await previousPrompt!.waitForElementState("hidden");

  await expect(tutor).toBeVisible();
  await expect(tutor.locator(".ask-bubble")).toHaveText([
    "Why?",
    "Because this card says so.",
  ]);

  const expected = pageErrors.filter((entry) => entry.includes("status of 409"));
  expect(expected).toHaveLength(1);
  pageErrors.splice(pageErrors.indexOf(expected[0]), 1);
});

test("the kids client never shows a card's section, and offers no way to ask for it", async ({ page }) => {
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "sectioned").click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Say it yourself" }).click(),
  ]);

  // The section rides every card on the wire (CardDto.section_context), so
  // "the kids client does not show it" is a claim about this client's DOM,
  // not about what the server sent.
  await expect(page.locator("body")).toContainText("sunlight zone");
  await expect(page.locator("body")).not.toContainText("Ocean depths");
  await expect(page.locator("body")).not.toContainText("Sunlight reaches only the top layer.");
  await expect(page.locator(".context.section")).toHaveCount(0);

  // And the adult toggle's key does nothing here: no aid to reveal.
  await page.keyboard.press("c");
  await expect(page.locator("body")).not.toContainText("Ocean depths");
});
