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

async function openWildCram(page: Parameters<typeof openApp>[0], depth: "Recall" | "Recognize") {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  await page.getByTitle("choose a depth").click();
  await page.getByRole("button", { name: /cram/i }).click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: new RegExp(`^${depth}`) }).click(),
  ]);
}

async function mockCompletedTutor(page: Parameters<typeof openApp>[0]) {
  await page.route("**/api/ask", (route) =>
    route.fulfill({
      json: {
        transcript: [{ q: "Why?", a: "Because the source says so." }],
        thinking: false,
        status: null,
        error: null,
        draft: null,
      },
    }),
  );
}

async function answerCurrentWildCard(page: Parameters<typeof openApp>[0]) {
  const reveal = page.getByRole("button", { name: "Reveal" });
  if (await reveal.isVisible()) {
    await reveal.click();
    return;
  }

  const front = await page.locator(".front-text").textContent();
  const answer = front?.includes("tallest") ? "Giraffe" : "Cheetah";
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose")),
    page.getByRole("button", { name: new RegExp(answer) }).click(),
  ]);
}

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

test("a task-list front renders as static checkboxes", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "fronts").click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Learn" }).click(),
  ]);

  await expect(page.locator(".region.q .checklist-row")).toHaveCount(2);
  await expect(page.locator(".region.q .checklist-box")).toHaveText(["☑", "☐"]);
});

test("revealed inline formatting renders as safe DOM elements", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);

  await expect(page.locator(".front-text")).toHaveText("Which animal is the tallest in the world?");
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose")),
    page.getByRole("button", { name: /Giraffe/ }).click(),
  ]);
  await expect(page.locator(".note .checklist-row")).toHaveCount(2);
  await expect(page.locator(".note .checklist-box")).toHaveText(["☑", "☐"]);
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/acquire")),
    page.getByRole("button", { name: "Seen" }).click(),
  ]);

  await expect(page.locator(".front-text")).toHaveText("What is the fastest animal on land?");
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose")),
    page.getByRole("button", { name: /Cheetah/ }).click(),
  ]);
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/acquire")),
    page.getByRole("button", { name: "Seen" }).click(),
  ]);

  await expect(page.getByText("session complete", { exact: true })).toBeVisible();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/deselect")),
    page.getByRole("button", { name: /^Leave/ }).click(),
  ]);

  await adultDeckRow(page, "wild").click();
  await page.getByTitle("choose a depth").click();
  await page.getByRole("button", { name: /cram/i }).click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);

  await expect(page.locator(".front-text")).toHaveText("Which animal is the tallest in the world?");
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/skip")),
    page.getByRole("button", { name: /^Skip/ }).click(),
  ]);
  await expect(page.locator(".front-text")).toHaveText("What is the fastest animal on land?");
  await page.getByRole("button", { name: "Reveal" }).click();

  const answer = page.locator(".reveal .answer");
  await expect(answer.locator("strong")).toHaveText("Cheetah");
  await expect(answer).toHaveText("Cheetah");
});

// The empty "Nothing due." summary (nothing reviewed or introduced) shows one
// quiet line saying when the next scheduled card comes due. The server-side
// production of `next_due_ms` on the done payload is covered by tests/api.rs;
// here the payload is mocked (as the tutor tests mock /api/ask) so the exact
// reviews==0/acquired==0 screen renders regardless of fixture scheduling, and
// the real embedded review.html JS is exercised against a fresh build.
test("an empty session says when the next card is due", async ({ page }) => {
  const done = {
    kind: "review",
    phase: "done",
    card: null,
    choices: null,
    choice_runs: null,
    keypoints: null,
    keypoint_runs: null,
    acquire: false,
    mode: "flip",
    depth: "recall",
    input: "type",
    remaining: 0,
    initial: 0,
    reviews: 0,
    passed: 0,
    failed: 0,
    acquired: 0,
    exam_due: [],
    can_restart: false,
    promotable: false,
    next_due_ms: Date.now() + 5 * 60 * 1000,
    label: "solo.md",
  };
  await page.route("**/api/state", (route) => route.fulfill({ json: done }));
  await openApp(page);

  await expect(page.getByRole("heading", { name: "Nothing due.", exact: true })).toBeVisible();
  await expect(page.locator(".summary .note")).toHaveText(/^Next due in \d+ min\.$/);
});

test("the tutor leave prompt keeps Enter for composing and Escape stays", async ({ page }) => {
  await openWildCram(page, "Recall");
  await answerCurrentWildCard(page);
  await mockCompletedTutor(page);
  await page.getByRole("button", { name: "Ask tutor" }).click();
  await expect(page.locator(".ask-q")).toHaveText("Why?");

  await page.getByRole("button", { name: /^Close/ }).click();
  const leave = page.getByRole("button", { name: /^Leave anyway/ });
  await expect(leave).toBeVisible();
  await expect(leave.locator(".k")).toHaveCount(0);

  const input = page.locator(".ask-input");
  await input.focus();
  await input.press("Enter");
  await expect(input).toHaveValue("\n");
  await expect(leave).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(leave).toHaveCount(0);
  await expect(page.locator(".ask-panel")).toBeVisible();
  await expect(page.getByRole("button", { name: /^Close/ })).toBeVisible();
});

test("leaving an unsaved tutor returns to its originating card without pulling state", async ({ page }) => {
  await openWildCram(page, "Recall");
  const firstState = await (await page.request.get("/api/state")).json();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/skip")),
    page.getByRole("button", { name: /^Skip/ }).click(),
  ]);
  const originFront = await page.locator(".front-text").textContent();
  await answerCurrentWildCard(page);
  await mockCompletedTutor(page);
  await page.getByRole("button", { name: "Ask tutor" }).click();
  await expect(page.locator(".ask-q")).toHaveText("Why?");
  await page.getByRole("button", { name: /^Close/ }).click();

  let statePulls = 0;
  await page.route("**/api/state", (route) => {
    statePulls += 1;
    return route.fulfill({ json: firstState });
  });
  await page.getByRole("button", { name: /^Leave anyway/ }).click();

  await expect(page.locator(".ask-panel")).toHaveCount(0);
  expect(statePulls).toBe(0);
  await expect(page.locator(".front-text")).toHaveText(originFront ?? "");
});

test("a revealed note matches its choice column width and text size", async ({ page }) => {
  await openWildCram(page, "Recognize");
  await answerCurrentWildCard(page);

  const choices = page.locator(".options");
  const note = page.locator(".note");
  await expect(note).toBeVisible();
  const choicesBox = await choices.boundingBox();
  const noteBox = await note.boundingBox();
  await page.screenshot({ path: "/tmp/alix-note-layout.png", fullPage: true });
  expect(choicesBox).not.toBeNull();
  expect(noteBox).not.toBeNull();
  expect(Math.abs((choicesBox?.width ?? 0) - (noteBox?.width ?? 0))).toBeLessThanOrEqual(1);

  const choicesFontSize = await choices.locator(".option").first().evaluate((node) => getComputedStyle(node).fontSize);
  const noteFontSize = await note.evaluate((node) => getComputedStyle(node).fontSize);
  expect(noteFontSize).toBe(choicesFontSize);
});

test("focusing a deck opens the drawer with its preamble, size and heatmap, no due count", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click(); // focuses the row → opens the drawer

  // A sibling row's drawer may still be animating closed as wild's opens, so
  // wait for a single stable drawer before asserting on it.
  await expect(page.locator(".drawer")).toHaveCount(1);
  const drawer = page.locator(".drawer");
  await expect(drawer.locator(".drawer-preamble")).toHaveText(/wild animals/i);
  // The progress funnel, top-right: both wild cards counted lib-side, and an
  // earlier test met (acquired) both, so the "seen" component appears while
  // "learned"/"retired" stay hidden at zero.
  await expect(drawer.locator(".drawer-size")).toHaveText("2 cards · 2 seen");
  await expect(drawer.locator(".crumb-cell")).toHaveCount(2); // one per stamped card
  // An earlier test met (acquired, ungraded) both wild cards, so each cell reads
  // as a dim "seen" cell rather than the never-met neutral one. Before the seen
  // tier existed, an acquired-but-ungraded card rendered identical to untouched.
  await expect(drawer.locator(".crumb-cell.seen")).toHaveCount(2);
  await expect(drawer.locator(".crumb-cell.empty")).toHaveCount(0);
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
