// End-to-end smoke suite for the ADULT web client (web/alix/review.html),
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

  // The front and its options render together, but a caller arriving right
  // after a transition response can read the OLD front while the new card is
  // being painted (the answer for the stale front then never appears, and the
  // click retries forever on a detached button). Wait until the front and the
  // visible options agree before answering.
  let answer = "";
  await expect(async () => {
    const front = await page.locator(".front-text").textContent();
    answer = front?.includes("tallest") ? "Giraffe" : "Cheetah";
    await expect(page.getByRole("button", { name: new RegExp(answer) })).toBeVisible({
      visible: true,
    });
  }).toPass({ timeout: 10_000 });
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose")),
    page.getByRole("button", { name: new RegExp(answer) }).click(),
  ]);
}

function longContentState({
  answerLines,
  choices = null,
  note = [],
  citations = [],
  front = "What should remain visible when an authored answer is long?",
  studyRevision = 1,
}: {
  answerLines: string[];
  choices?: string[] | null;
  note?: Array<{ kind: string; text: string }>;
  citations?: Array<{
    locator: string;
    excerpt: { path: string; lines: Array<{ n: number; text: string }>; truncated: boolean };
    error: null;
  }>;
  front?: string;
  studyRevision?: number;
}) {
  return {
    kind: "review",
    study_revision: studyRevision,
    phase: "review",
    card: {
      front,
      front_runs: [{ text: front }],
      front_units: null,
      context: [],
      context_runs: [],
      back: answerLines,
      back_runs: answerLines.map((text) => [{ text }]),
      back_units: answerLines.map((text) => ({ kind: "sentence", text })),
      reshaped: false,
      note,
      images: [],
      images_back: [],
      citations,
      crumb: null,
    },
    choices,
    choice_runs: choices?.map((text) => [{ text }]) ?? null,
    keypoints: null,
    keypoint_runs: null,
    introducing: false,
    mode: choices ? "choice" : "flip",
    depth: "recall",
    input: "type",
    remaining: 1,
    initial: 1,
    reviews: 0,
    passed: 0,
    failed: 0,
    introduced: 0,
    exam_due: [],
    can_restart: false,
    promotable: false,
    next_due_ms: null,
    due_left: 0,
    new_left: 0,
    label: "long-content.md",
    save_error: null,
  };
}

test("the picker lists the fixture workspace and its decks", async ({ page }) => {
  const animals = adultDeckRow(page, "Animals");
  await expect(animals).toBeVisible();
  await animals.click();

  await expect(adultDeckRow(page, "wild")).toBeVisible();
  await expect(adultDeckRow(page, "cats")).toBeVisible();
});

test("the exam chip stays put when focus moves between ready and locked exams", async ({ page }) => {
  // A normal workspace can contain both an exam a learner may take early and
  // one that is still locked. Pin every footer-relevant field to the same
  // value except `examable`, so moving between the rows isolates the x/lock
  // hint swap that used to resize the centered action group.
  await page.route("**/api/decks", async (route) => {
    const response = await route.fetch();
    const catalog = await response.json();
    const rows = [
      ...catalog.recent,
      ...catalog.workspaces.flatMap((row) => row.members),
      ...catalog.folders.flatMap((row) => row.members),
    ];
    const wild = rows.find((row) => row.name.endsWith("wild.md"));
    const cats = rows.find((row) => row.name.endsWith("cats.md"));
    const shared = {
      state: "learning",
      has_exam: true,
      is_trace: false,
      reviewable: true,
      reviewable_recognize: true,
      reviewable_recall: true,
      reviewable_reconstruct: false,
      last_depth: null,
    };
    Object.assign(wild, shared, { examable: true });
    Object.assign(cats, shared, { examable: false });
    await route.fulfill({ response, json: catalog });
  });

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();

  const exam = page.locator("#legend .chip").filter({ hasText: "Take exam" });
  const browse = page.getByRole("button", { name: "Browse" });

  await adultDeckRow(page, "wild").click();
  await expect(exam.locator(".k")).toHaveText("x");
  const readyExam = await exam.boundingBox();
  const readyBrowse = await browse.boundingBox();

  await adultDeckRow(page, "cats").click();
  await expect(exam.locator(".k")).toHaveText("🔒");
  const lockedExam = await exam.boundingBox();
  const lockedBrowse = await browse.boundingBox();

  expect(readyExam).not.toBeNull();
  expect(readyBrowse).not.toBeNull();
  expect(lockedExam).not.toBeNull();
  expect(lockedBrowse).not.toBeNull();
  expect(lockedExam!.width).toBeCloseTo(readyExam!.width, 1);
  expect(lockedBrowse!.x).toBeCloseTo(readyBrowse!.x, 1);
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

test("locking in a choice redraws only the answer, not the question", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);
  await expect(page.locator(".front-text")).toHaveText("Which animal is the tallest in the world?");

  // Mark the live question node. If answering rebuilds the card the marker is
  // gone with it, which is the flicker: the question is destroyed and recreated
  // even though only the answer changed.
  await page.locator(".region.q").evaluate((node) => { node.dataset.probe = "kept"; });

  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose")),
    page.getByRole("button", { name: /Giraffe/ }).click(),
  ]);

  await expect(page.locator(".option.correct")).toHaveCount(1); // the answer did update
  await expect(
    page.locator('.region.q[data-probe="kept"]'),
    "the question region was rebuilt when only the answer changed"
  ).toHaveCount(1);
});

test("revealed inline formatting renders as safe DOM elements", async ({ page }) => {
  // Isolation: earlier tests answered and graded wild's cards, which
  // reshapes this session's queue, so start from a clean wild.
  const resetResponse = await page.request.post("/api/reset", { data: { deck: "animals/wild.md" } });
  expect(resetResponse.ok(), "the isolation reset must land").toBeTruthy();
  // Reload after the out-of-band reset: the picker listing gates the row
  // actions and was fetched before the reset landed.
  await openApp(page);
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
    page.waitForResponse((res) => res.url().includes("/api/introduce")),
    page.getByRole("button", { name: "Seen" }).click(),
  ]);

  await expect(page.locator(".front-text")).toHaveText("What is the fastest animal on land?");
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose")),
    page.getByRole("button", { name: /Cheetah/ }).click(),
  ]);
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/introduce")),
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

test("long answer variants start at the first line and show scroll hints", async ({ page }) => {
  const answerLines = Array.from(
    { length: 36 },
    (_, index) => `Answer line ${index + 1} stays reachable in authored study material.`,
  );
  const sourceLines = Array.from(
    { length: 48 },
    (_, index) => ({ n: index + 1, text: `source line ${index + 1}: reachable evidence` }),
  );
  const longAnswerState = longContentState({
    answerLines,
    front: "This long card follows a short card.",
    studyRevision: 2,
  });
  const variants = [
    {
      name: "long answer after a short card",
      state: longContentState({
        answerLines: ["Short answer."],
        front: "This short card comes first.",
      }),
      firstContent: ".reveal .answer",
      open: async () => {
        await page.getByRole("button", { name: "Reveal" }).click();
        await expect(page.locator(".region.a")).toHaveClass(/balanced/);
        await page.route("**/api/grade", (route) => route.fulfill({ json: longAnswerState }));
        await page.getByRole("button", { name: "Got it" }).click();
        await expect(page.locator(".front-text")).toHaveText("This long card follows a short card.");
        await page.getByRole("button", { name: "Reveal" }).click();
      },
    },
    {
      name: "authored choices",
      state: longContentState({
        answerLines: ["The first choice is correct."],
        choices: Array.from(
          { length: 18 },
          (_, index) => `Choice ${index + 1} is a deliberately substantial authored option.`,
        ),
      }),
      firstContent: ".option",
      open: async () => {},
    },
    {
      name: "answer with note",
      state: longContentState({
        answerLines,
        note: [{ kind: "sentence", text: "The note remains pinned below the scrollable answer." }],
      }),
      firstContent: ".reveal .answer",
      open: async () => page.getByRole("button", { name: "Reveal" }).click(),
    },
    {
      name: "source panel",
      state: longContentState({
        answerLines: ["A short answer that starts centered."],
        citations: [{
          locator: "source.txt:1-48",
          excerpt: { path: "source.txt", lines: sourceLines, truncated: false },
          error: null,
        }],
      }),
      firstContent: ".source-line",
      open: async () => {
        await page.getByRole("button", { name: "Reveal" }).click();
        await expect(page.locator(".region.a")).toHaveClass(/balanced/);
        await page.keyboard.press("s");
      },
    },
  ];

  for (const variant of variants) {
    await page.unroute("**/api/state");
    await page.unroute("**/api/grade");
    await page.route("**/api/state", (route) => route.fulfill({ json: variant.state }));
    await openApp(page);
    await variant.open();

    const answer = page.locator(".region.a");
    const below = page.locator(".answer-hint:not(.top)");
    const above = page.locator(".answer-hint.top");
    await expect(answer, variant.name).toHaveClass(/filled/);
    expect(await answer.evaluate((element) => ({
      scrollTop: element.scrollTop,
      overflows: element.scrollHeight > element.clientHeight + 2,
    })), variant.name).toEqual({ scrollTop: 0, overflows: true });
    const answerBox = await answer.boundingBox();
    const firstContentBox = await answer.locator(variant.firstContent).first().boundingBox();
    expect(answerBox, variant.name).not.toBeNull();
    expect(firstContentBox, variant.name).not.toBeNull();
    expect(firstContentBox?.y ?? -1, variant.name).toBeGreaterThanOrEqual((answerBox?.y ?? 0) - 1);
    await expect(below, variant.name).toHaveClass(/show/);
    await expect(above, variant.name).not.toHaveClass(/show/);
    await expect(page.locator(".region.q"), variant.name).toBeVisible();
    await expect(page.locator(".legend"), variant.name).toBeVisible();

    await answer.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
      element.dispatchEvent(new Event("scroll"));
    });
    await expect(above, variant.name).toHaveClass(/show/);
  }
});

test("a long introduction note shows overflow hints at both edges", async ({ page }) => {
  const state = {
    ...longContentState({
      answerLines: ["The revealed answer remains above the note."],
      note: Array.from(
        { length: 16 },
        (_, index) => ({
          kind: "sentence",
          text: `Note paragraph ${index + 1} remains reachable in the authored explanation.`,
        }),
      ),
    }),
    introducing: true,
  };
  await page.unroute("**/api/state");
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  await openApp(page);
  await page.getByRole("button", { name: "Reveal" }).click();

  const note = page.locator("#noteRegion");
  const below = page.locator(".note-hint:not(.top)");
  const above = page.locator(".note-hint.top");
  await expect(page.getByRole("button", { name: "Seen" })).toBeVisible();
  expect(await note.evaluate((element) => ({
    scrollTop: element.scrollTop,
    overflows: element.scrollHeight > element.clientHeight + 2,
  }))).toEqual({ scrollTop: 0, overflows: true });
  await expect(note).toHaveClass(/fade-bottom/);
  await expect(below).toHaveClass(/show/);
  await expect(above).not.toHaveClass(/show/);

  await note.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
    element.dispatchEvent(new Event("scroll"));
  });
  await expect(note).toHaveClass(/fade-top/);
  await expect(note).not.toHaveClass(/fade-bottom/);
  await expect(above).toHaveClass(/show/);
  await expect(below).not.toHaveClass(/show/);
});

test("a tall review window keeps the card close to both shell edges", async ({ page }) => {
  await page.setViewportSize({ width: 1800, height: 1200 });
  const state = {
    ...longContentState({
      answerLines: ["The card should use the height available between shell bars."],
      front: "Where should the study card sit in a tall review window?",
      note: Array.from(
        { length: 5 },
        (_, index) => ({
          kind: "sentence",
          text: `Authored explanation paragraph ${index + 1}.`,
        }),
      ),
    }),
    introducing: true,
  };
  await page.unroute("**/api/state");
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  await openApp(page);
  await page.getByRole("button", { name: "Reveal" }).click();
  await expect(page.getByRole("button", { name: "Seen" })).toBeVisible();

  const gaps = await page.evaluate(() => {
    const deck = document.querySelector(".bar .deck");
    const mode = document.querySelector(".mode-tag");
    const note = document.querySelector("#noteRegion");
    const footer = document.querySelector(".legend");
    if (![deck, mode, note, footer].every((node) => node instanceof HTMLElement)) {
      throw new Error("missing review shell geometry");
    }
    const deckBox = deck.getBoundingClientRect();
    const modeBox = mode.getBoundingClientRect();
    const noteBox = note.getBoundingClientRect();
    const footerBox = footer.getBoundingClientRect();
    return {
      top: modeBox.top - deckBox.bottom,
      bottom: footerBox.top - noteBox.bottom,
    };
  });
  expect(
    { topFits: gaps.top <= 72, bottomFits: gaps.bottom <= 48 },
    `mode gap ${gaps.top}px must be <= 72px; note gap ${gaps.bottom}px must be <= 48px`,
  ).toEqual({ topFits: true, bottomFits: true });
});

test("keyboard-focused choices stay clear of both overflow hints", async ({ page }) => {
  const choices = Array.from(
    { length: 18 },
    (_, index) => `Choice ${index + 1} is a deliberately substantial authored option.`,
  );
  await page.unroute("**/api/state");
  await page.route("**/api/state", (route) => route.fulfill({
    json: longContentState({
      answerLines: ["The first choice is correct."],
      choices,
    }),
  }));
  await openApp(page);

  await expect(page.locator(".option")).toHaveCount(18);
  await expect(page.locator(".option.focused .num")).toHaveText("1");
  for (let step = 0; step < 10; step += 1) await page.keyboard.press("ArrowDown");
  for (let step = 0; step < 6; step += 1) await page.keyboard.press("ArrowUp");

  const answer = page.locator(".region.a");
  const focused = page.locator(".option.focused");
  const above = page.locator(".answer-hint.top");
  await expect(focused.locator(".num")).toHaveText("5");
  await expect(answer).toHaveClass(/fade-top/);
  await expect(above).toHaveClass(/show/);
  const upperEdge = await focused.evaluate((element) => {
    const focusedBox = element.getBoundingClientRect();
    const hint = document.querySelector(".answer-hint.top");
    if (!(hint instanceof HTMLElement)) throw new Error("missing upper overflow hint");
    return {
      focusedTop: focusedBox.top,
      safeTop: hint.getBoundingClientRect().bottom,
    };
  });
  expect(
    upperEdge.focusedTop,
    `focused row top ${upperEdge.focusedTop} must clear hint bottom ${upperEdge.safeTop}`,
  ).toBeGreaterThanOrEqual(upperEdge.safeTop);

  for (let step = 0; step < 6; step += 1) await page.keyboard.press("ArrowDown");
  const below = page.locator(".answer-hint:not(.top)");
  await expect(focused.locator(".num")).toHaveText("11");
  await expect(answer).toHaveClass(/fade-bottom/);
  await expect(below).toHaveClass(/show/);
  const lowerEdge = await focused.evaluate((element) => {
    const focusedBox = element.getBoundingClientRect();
    const hint = document.querySelector(".answer-hint:not(.top)");
    if (!(hint instanceof HTMLElement)) throw new Error("missing lower overflow hint");
    return {
      focusedBottom: focusedBox.bottom,
      safeBottom: hint.getBoundingClientRect().top,
    };
  });
  expect(
    lowerEdge.focusedBottom,
    `focused row bottom ${lowerEdge.focusedBottom} must clear hint top ${lowerEdge.safeBottom}`,
  ).toBeLessThanOrEqual(lowerEdge.safeBottom);
});

test("a source answer and its note have a visible separator", async ({ page }, testInfo) => {
  const sourceLines = Array.from(
    { length: 24 },
    (_, index) => ({ n: index + 1, text: `source line ${index + 1}: evidence for the answer` }),
  );
  const state = longContentState({
    answerLines: ["The source establishes the expected answer."],
    citations: [{
      locator: "source.rs:1-24",
      excerpt: { path: "source.rs", lines: sourceLines, truncated: false },
      error: null,
    }],
    note: [
      { kind: "sentence", text: "This note explains why the evidence matters." },
      { kind: "sentence", text: "It must read as a separate section from the source answer." },
    ],
  });
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  await openApp(page);
  await page.getByRole("button", { name: "Reveal" }).click();
  await page.keyboard.press("s");

  const card = page.locator("#card");
  await expect(card.locator(".source-excerpt")).toBeVisible();
  await expect(card.locator(".note")).toBeVisible();
  const divider = card.locator("#noteDivider");
  const cardBox = await card.boundingBox();
  const dividerBox = await divider.boundingBox();
  expect(cardBox).not.toBeNull();
  expect(dividerBox).not.toBeNull();
  const screenshot = await card.screenshot({ path: testInfo.outputPath("source-note-separator.png") });
  const contrast = await page.evaluate(async ({ png, x0, x1, y }) => {
    const image = new Image();
    image.src = `data:image/png;base64,${png}`;
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) return 0;
    context.drawImage(image, 0, 0);
    const start = Math.round(x0 * canvas.width);
    const end = Math.round(x1 * canvas.width);
    const center = Math.round(y * canvas.height);
    const average = (row: number) => {
      const pixels = context.getImageData(start, row, end - start, 1).data;
      const color = [0, 0, 0];
      for (let index = 0; index < pixels.length; index += 4) {
        color[0] += pixels[index];
        color[1] += pixels[index + 1];
        color[2] += pixels[index + 2];
      }
      return color.map((value) => value / (pixels.length / 4));
    };
    const above = average(center - 4);
    const below = average(center + 4);
    const background = above.map((value, index) => (value + below[index]) / 2);
    return Math.max(...[-1, 0, 1].map((offset) => {
      const band = average(center + offset);
      return Math.hypot(...band.map((value, index) => value - background[index]));
    }));
  }, {
    png: screenshot.toString("base64"),
    x0: ((dividerBox?.x ?? 0) - (cardBox?.x ?? 0) + (dividerBox?.width ?? 0) * 0.25) / (cardBox?.width ?? 1),
    x1: ((dividerBox?.x ?? 0) - (cardBox?.x ?? 0) + (dividerBox?.width ?? 0) * 0.75) / (cardBox?.width ?? 1),
    y: ((dividerBox?.y ?? 0) - (cardBox?.y ?? 0) + (dividerBox?.height ?? 0) / 2) / (cardBox?.height ?? 1),
  });
  expect(contrast, "the rendered separator must visibly contrast with the card background").toBeGreaterThan(50);
});

test("line reveal replaces a complete Mermaid fence with its rendered diagram", async ({ page }) => {
  const back = ["```mermaid", "flowchart LR", " A-->B", "```"];
  const original = longContentState({ answerLines: back });
  const state = {
    ...original,
    mode: "line",
    card: {
      ...original.card,
      back_units: [{
        kind: "diagram",
        src: "/img/0123456789abcdef",
        width: 188,
        height: 114,
        alt: "flowchart LR\n A-->B",
      }],
    },
  };
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  // The fabricated key must serve bytes once the swap works, or the img's
  // fetch 404s and trips the no-console-errors harness invariant.
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABXvMqOgAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/img/0123456789abcdef", (route) =>
    route.fulfill({ body: png, contentType: "image/png" }),
  );
  await openApp(page);

  const reveal = page.getByRole("button", { name: /^Reveal/ });
  await reveal.click();
  await reveal.click();
  await reveal.click();
  await expect(page.locator(".region.a img.diagram")).toHaveCount(0);

  await reveal.click();
  const diagram = page.locator(".region.a img.diagram");
  await expect(diagram).toHaveAttribute("src", "/img/0123456789abcdef");
  await expect(diagram).toHaveAttribute("alt", "flowchart LR\n A-->B");
});

test("a load warning surfaces once as a notice at session open", async ({ page }) => {
  const original = longContentState({ answerLines: ["Answer"] });
  const state = {
    ...original,
    load_warnings: [
      "1 frozen diagram(s) did not resolve and fall back to source; run `alix doctor` for details",
    ],
  };
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  await openApp(page);

  const notice = page.locator("#notice");
  await expect(notice).toHaveClass(/show/);
  await expect(notice).toContainText("did not resolve");
});

test("a masked context diagram lifts its asked mask and swaps alt on reveal", async ({ page }) => {
  // A raster-sized fixture: mask geometry clips against naturalWidth, so a
  // 1x1 stand-in would hide every region as out-of-source.
  const rasterPng = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAXgAAADkCAIAAAAdLKh9AAACMUlEQVR42u3UIQEAAADCMPonJQYpcFuEi6cAZ5EAMBrAaACMBjAawGgAjAYwGgCjAYwGMBoAowGMBjAaAKMBjAbAaACjAYwGwGgAowGMBsBoAKMBMBrAaACjATAawGgAowEwGsBoAIwGMBrAaACMBjAawGgAjAYwGgCjAYwGMBoAowGMBjAaAKMBjAbAaACjAYwGwGgAowGMBsBoAKMBMBrAaACjATAawGgAowEwGsBoAIwGMBrAaACMBjAawGgAjAYwGgCjAYwGMBoAowGMBjAaAKMBjAbAaACjAYwGwGgAowGMBsBoAKMBMBrAaACjATAawGgAowEwGsBoAIwGMBrAaACMBjAawGgAjAYwGgCjAYwGMBoAowGMBjAaAKMBjAbAaACjAYwGwGgAowGMBsBoAKMBMBrAaACjATAawGgAowEwGsBoAIwGMBrAaACMBjAaAKMBjAYwGgCjAYwGMBoAowGMBsBoAKMBjAbAaACjAYwGwGgAowEwGsBoAKMBMBrAaACjATAawGgAjAYwGsBoAIwGMBrAaACMBjAaAKMBjAYwGgCjAYwGMBoAowGMBsBoAKMBjAbAaACjAYwGwGgAowEwGsBoAKMBMBrAaACjATAawGgAjAYwGsBoAIwGMBrAaACMBjAaAKMBjAYwGgCjAYwGMBoAowGMBsBoAKMBjAbAaACjAYwGwGgAowEwGsBoAKMBMBrAaACjATAawGgAjAYwGsBoAIwGMBrAaABeBuh/plOqpXZgAAAAAElFTkSuQmCC",
    "base64",
  );
  const original = longContentState({ answerLines: ["Cache"] });
  const state = {
    ...original,
    card: {
      ...original.card,
      context: ["```mermaid", "flowchart LR", "  Cache[store] --> B[Cache]", "```"],
      context_leads: true,
      context_runs: [[], [], [], []],
      context_units: [{
        kind: "diagram",
        src: "/img/0123456789abcdef",
        width: 188,
        height: 114,
        alt: "diagram labels: …, …",
        regions: [
          { role: "asked", reveal_on_answer: true, x: 10, y: 50, width: 100, height: 40, unit: "px" },
          { role: "mask", reveal_on_answer: false, x: 10, y: 10, width: 100, height: 40, unit: "px" },
        ],
        revealed_alt: "diagram labels: …, Cache",
      }],
    },
  };
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  await page.route("**/img/0123456789abcdef", (route) =>
    route.fulfill({ body: rasterPng, contentType: "image/png" }),
  );
  await openApp(page);

  const question = page.locator(".region.q");
  const diagram = question.locator("img.diagram");
  await expect(diagram).toHaveAttribute("alt", "diagram labels: …, …");
  const masks = question.locator(".img-mask");
  await expect(masks).toHaveCount(2);
  await expect(masks.first()).toBeVisible();

  await page.getByRole("button", { name: /^Reveal/ }).click();
  await expect(diagram).toHaveAttribute(
    "alt",
    "diagram labels: …, Cache",
    { timeout: 5000 },
  );
  await expect(question.locator(".img-mask.reveals")).toBeHidden();
  await expect(question.locator(".img-mask:not(.reveals)")).toBeVisible();
});

test("concealing a new card restores its diagram mask and masked alt", async ({ page }) => {
  const original = longContentState({ answerLines: ["Cache"] });
  const state = {
    ...original,
    introducing: true,
    card: {
      ...original.card,
      context: ["```mermaid", "flowchart LR", "  Cache[store] --> B[Cache]", "```"],
      context_leads: true,
      context_runs: [[], [], [], []],
      context_units: [{
        kind: "diagram",
        src: "/img/0123456789abcdef",
        width: 188,
        height: 114,
        alt: "diagram labels: …, …",
        regions: [
          { role: "asked", reveal_on_answer: true, x: 10, y: 50, width: 100, height: 40, unit: "px" },
          { role: "mask", reveal_on_answer: false, x: 10, y: 10, width: 100, height: 40, unit: "px" },
        ],
        revealed_alt: "diagram labels: …, Cache",
      }],
    },
  };
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  await page.route("**/img/0123456789abcdef", (route) =>
    route.fulfill({
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="376" height="228"></svg>',
      contentType: "image/svg+xml",
    }),
  );
  await openApp(page);

  const question = page.locator(".region.q");
  const diagram = question.locator("img.diagram");
  const askedMask = question.locator(".img-mask.reveals");
  await page.getByRole("button", { name: /^Reveal/ }).click();
  await expect(askedMask).toBeHidden();
  await expect(diagram).toHaveAttribute("alt", "diagram labels: …, Cache");

  await page.locator("#ansRegion .cite-toggle").click();
  await expect(page.locator("#ansRegion")).toHaveClass(/concealed/);
  await expect(askedMask).toBeVisible();
  await expect(diagram).toHaveAttribute("alt", "diagram labels: …, …");
});

test("a revealed new card does not repeat when it will be quizzed", async ({ page }) => {
  const state = {
    ...longContentState({ answerLines: ["Read this answer once."] }),
    introducing: true,
  };
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  await openApp(page);

  await expect(
    page.getByText("new card: try to recall it, then reveal.", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Reveal" }).click();

  await expect(page.getByRole("button", { name: "Seen" })).toBeVisible();
  await expect(
    page.getByText("new card: you'll be quizzed on it in about a minute.", { exact: true }),
  ).toHaveCount(0);
});

test("a cloze context fence renders its diagram in the question region", async ({ page }) => {
  const original = longContentState({ answerLines: ["Answer"] });
  const state = {
    ...original,
    card: {
      ...original.card,
      context: ["```mermaid", "flowchart LR", " A-->B", "```", "a sentence with a gap"],
      context_leads: true,
      context_runs: [[], [], [], [], [{ text: "a sentence with a gap" }]],
      context_units: [{
        kind: "diagram",
        src: "/img/0123456789abcdef",
        width: 188,
        height: 114,
        alt: "flowchart LR\n A-->B",
      }],
    },
  };
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABXvMqOgAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/img/0123456789abcdef", (route) =>
    route.fulfill({ body: png, contentType: "image/png" }),
  );
  await openApp(page);

  const question = page.locator(".region.q");
  const diagram = question.locator("img.diagram");
  await expect(diagram).toHaveAttribute("src", "/img/0123456789abcdef");
  await expect(diagram).toHaveAttribute("alt", "flowchart LR\n A-->B");
  await expect(question.getByText("a sentence with a gap")).toBeVisible();
  await expect(question.getByText("```")).toHaveCount(0);
});

test("explain reveal renders the resolved diagram in the authored answer", async ({ page }) => {
  const back = ["```mermaid", "flowchart LR", " A-->B", "```"];
  const original = longContentState({ answerLines: back });
  const state = {
    ...original,
    mode: "explain",
    keypoints: ["A reaches B"],
    keypoint_runs: [[{ text: "A reaches B" }]],
    card: {
      ...original.card,
      back_units: [{
        kind: "diagram",
        src: "/img/0123456789abcdef",
        width: 188,
        height: 114,
        alt: "flowchart LR\n A-->B",
      }],
    },
  };
  await page.route("**/api/state", (route) => route.fulfill({ json: state }));
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABXvMqOgAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/img/0123456789abcdef", (route) =>
    route.fulfill({ body: png, contentType: "image/png" }),
  );
  await openApp(page);

  await page.getByRole("button", { name: "Reveal" }).click();
  const diagram = page.locator(".region.a img.diagram");
  await expect(diagram).toHaveAttribute("src", "/img/0123456789abcdef");
  await expect(diagram).toHaveAttribute("alt", "flowchart LR\n A-->B");
});

// The empty "Nothing due." summary (nothing reviewed or introduced) shows one
// quiet line saying when the next scheduled card comes due. The server-side
// production of `next_due_ms` on the done payload is covered by tests/api.rs;
// here the payload is mocked (as the tutor tests mock /api/ask) so the exact
// reviews==0/introduced==0 screen renders regardless of fixture scheduling, and
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
    introducing: false,
    mode: "flip",
    depth: "recall",
    input: "type",
    remaining: 0,
    initial: 0,
    reviews: 0,
    passed: 0,
    failed: 0,
    introduced: 0,
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
  const beforeSkip = await page.locator(".front-text").textContent();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/skip")),
    page.getByRole("button", { name: /^Skip/ }).click(),
  ]);
  // The response arriving is not the render: wait for the card to change
  // before reading the origin front (a two-card session always swaps on skip).
  await expect(page.locator(".front-text")).not.toHaveText(beforeSkip ?? "");
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

test("a revealed note matches its choice column width and text size", async ({ page }, testInfo) => {
  // Fresh deck: queue order is deck order, so the giraffe card (the only one
  // with a note) serves first regardless of what earlier tests engaged.
  const resetResponse = await page.request.post("/api/reset", { data: { deck: "animals/wild.md" } });
  expect(resetResponse.ok(), "the isolation reset must land").toBeTruthy();
  // Reload after the out-of-band reset: the picker listing gates the row
  // actions and was fetched before the reset landed.
  await openApp(page);
  await openWildCram(page, "Recognize");
  await answerCurrentWildCard(page);

  const choices = page.locator(".options");
  const note = page.locator(".note");
  await expect(note).toBeVisible();
  const choicesBox = await choices.boundingBox();
  const noteBox = await note.boundingBox();
  await page.screenshot({ path: testInfo.outputPath("note-layout.png"), fullPage: true });
  expect(choicesBox).not.toBeNull();
  expect(noteBox).not.toBeNull();
  expect(Math.abs((choicesBox?.width ?? 0) - (noteBox?.width ?? 0))).toBeLessThanOrEqual(1);

  const choicesFontSize = await choices.locator(".option").first().evaluate((node) => getComputedStyle(node).fontSize);
  const noteFontSize = await note.evaluate((node) => getComputedStyle(node).fontSize);
  expect(noteFontSize).toBe(choicesFontSize);
});

test("focusing a deck opens the drawer with its description, size and heatmap, no due count", async ({ page }, testInfo) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click(); // focuses the row → opens the drawer

  // A sibling row's drawer may still be animating closed (or re-rendering
  // from its in-flight fetch) as wild's opens, so scope every assertion to
  // wild's own drawer instead of counting drawers globally.
  const drawer = page.locator(".drawer").filter({ hasText: "2 cards" });
  await expect(drawer).toHaveCount(1);
  await expect(drawer.locator(".drawer-description")).toHaveText(/wild animals/i);
  // The progress funnel, top-right: both wild cards counted lib-side. The
  // note-layout test above reset wild and answered one card without ever
  // leaving it, and an abandoned sitting persists nothing (ADR 0035), so
  // both cells stay empty.
  await expect(drawer.locator(".drawer-size")).toHaveText("2 cards");
  await expect(drawer.locator(".crumb-cell")).toHaveCount(2); // one per stamped card
  await expect(drawer.locator(".crumb-cell.learning")).toHaveCount(0);
  await expect(drawer.locator(".crumb-cell.seen")).toHaveCount(0);
  await expect(drawer.locator(".crumb-cell.empty")).toHaveCount(2);
  await expect(page.locator(".drawer-due")).toHaveCount(0); // the old due count is gone
  await drawer.screenshot({ path: testInfo.outputPath("drawer.png") });
});

test("filtering a focused deck out closes its drawer", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  await expect(page.locator(".drawer").filter({ hasText: "2 cards" })).toHaveCount(1);

  await page.locator("#barFilter").fill("no deck has this name");

  await expect(page.getByText("No decks match.", { exact: true })).toBeVisible();
  await expect(page.locator(".drawer")).toHaveCount(0);
});

test("jumping to the last deck with G reveals its drawer, not just opens it", async ({ page }) => {
  // A viewport short enough that the last row sits at the bottom edge: the
  // drawer opens *below* it, so merely existing is not enough. This is the
  // reported case, and a tall viewport cannot reproduce it.
  await page.setViewportSize({ width: 900, height: 340 });
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "cats").click();
  await page.keyboard.press("Shift+G");

  const focused = page.locator(".deckrow:focus");
  await expect(focused).toHaveCount(1);
  const drawer = page.locator(".drawer-wrap");
  await expect(drawer).toHaveCount(1);
  // The drawer is animated open, so give it its height before asking where it is.
  await expect(drawer).toBeInViewport();
});

test("a repeat G-jump keeps the drawer clear of the footer", async ({ page }) => {
  // The cached-drawer path renders synchronously inside the focus handler, so
  // the browser's own scroll-the-focused-row runs afterwards and can drop the
  // drawer behind the footer legend (user report 2026-08-01: first jump
  // works, repeats hide the drawer). Same short viewport as the reveal test.
  await page.setViewportSize({ width: 900, height: 340 });
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "cats").click();
  await page.keyboard.press("Shift+G");
  // Two wraps can coexist briefly (the old drawer animating closed); the live
  // one is the later in document order.
  await page.locator(".drawer-wrap").last().waitFor();
  await page.keyboard.press("g");
  await page.waitForTimeout(400);
  await page.keyboard.press("Shift+G");
  await page.waitForTimeout(400);

  const overlap = await page.evaluate(() => {
    const wraps = document.querySelectorAll(".drawer-wrap");
    const wrap = wraps[wraps.length - 1];
    const legend = document.querySelector("#legend");
    if (!wrap || !legend) return "missing";
    const w = wrap.getBoundingClientRect();
    const l = legend.getBoundingClientRect();
    return { wrapBottom: Math.round(w.bottom), legendTop: Math.round(l.top) };
  });
  expect(overlap, "drawer and legend present").not.toBe("missing");
  expect(
    (overlap as { wrapBottom: number }).wrapBottom,
    `repeat-jump drawer must sit fully above the footer: ${JSON.stringify(overlap)}`,
  ).toBeLessThanOrEqual((overlap as { legendTop: number }).legendTop);
});

test("a card merely shown in an earlier session stays an untouched empty cell", async ({ page }) => {
  // The task-list test above selected `fronts` and rendered its only card
  // without acknowledging or grading it: presented, nothing more.
  // Presentation persists nothing (ADR 0035), so the drawer must show it as
  // untouched, not as seen.
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "fronts").click();

  // A sibling drawer can linger mid-close and re-render between
  // assertions; waiting for the TOTAL drawer count to settle at one turns
  // that race into a wait (the count assertion retries), and only then are
  // the content assertions unambiguous.
  await expect(page.locator(".drawer")).toHaveCount(1);
  const drawer = page.locator(".drawer").filter({ hasText: "1 card" });
  await expect(drawer).toHaveCount(1);
  await expect(drawer.locator(".drawer-size")).toHaveText("1 card");
  await expect(drawer.locator(".crumb-cell.seen")).toHaveCount(0);
  await expect(drawer.locator(".crumb-cell.empty")).toHaveCount(1);
  await expect(drawer.locator(".crumb-cell.learning")).toHaveCount(0);
});

// KNOWN GAP: the learned green/yellow/red heatmap bands are unreachable in
// e2e: a learned cell needs a GRADUATED card (two spaced Goods across the
// 10-minute learning hold), and the fixture contract (../README.md) bans both
// sleeps and synthesised progress. The banding itself is pinned lib-side
// (src/session.rs learned_tiers_band_current_retrievability_not_history) and
// on the wire (src/serve/contract.rs, tests/api.rs); only the CSS-class hookup
// for the three learned strings lacks browser coverage.
test.fixme("a graduated deck shows green/yellow/red learned cells in the drawer", () => {});

test("the ☰ menu opens without error", async ({ page }) => {
  await page.locator("#kebab").click();
  await expect(page.locator("#menu")).toHaveClass(/open/);
  await expect(page.locator("#mAdd")).toBeVisible(); // a picker-context item, since nothing is selected
  await expect(page.locator("#mDelete")).toBeVisible();
  await page.locator("#kebab").click(); // close it again
});

test("the shortcuts sheet opens and Escape closes it", async ({ page }) => {
  await page.locator("#kebab").click();
  await page.locator("#mShortcuts").click();

  await expect(page.locator("#sheet")).toBeVisible();
  await expect(page.locator("#sheetPanel")).toContainText("Picker shortcuts");

  await page.keyboard.press("Escape");

  await expect(page.locator("#sheet")).toBeHidden();
});

test("library removal requires the exact focused name before posting", async ({ page }) => {
  await expect(adultDeckRow(page, "Animals")).toBeFocused();
  await page.keyboard.press("ArrowDown");
  const workspace = adultDeckRow(page, "Removal Target");
  await expect(workspace).toBeFocused();
  await page.locator("#kebab").click();
  await page.locator("#mDelete").click();

  await expect(page.locator("#sheetPanel")).toContainText("1 deck, 0 cards with progress");
  const confirm = page.locator("#removeConfirm");
  const remove = page.getByRole("button", { name: "Remove permanently" });
  await confirm.fill("removal-targe");
  await expect(remove).toBeDisabled();
  await confirm.fill("removal-target");
  await expect(remove).toBeEnabled();

  const [response] = await Promise.all([
    page.waitForResponse((res) =>
      res.url().endsWith("/api/library/remove") && res.request().method() === "POST"
    ),
    remove.click(),
  ]);

  expect(response.status()).toBe(200);
  expect(response.request().postDataJSON()).toEqual({ name: "removal-target" });
  await expect(page.locator("#sheet")).toBeHidden();
  await expect(page.locator("#notice")).toHaveText(
    "removed workspace 'removal-target'; folder kept: it contains files Alix doesn't own"
  );
  await expect(adultDeckRow(page, "Removal Target")).toHaveCount(0);
});

// KNOWN GAP — reported as skipped on every run, deliberately.
//
// The fixture ships no progress store, so every card is never-seen (`introduction`)
// and the adult app posts /api/introduce, never /api/grade. Reaching a genuinely
// graded card needs one past the server's introduction cooldown (5 min default; a
// sleep or a committed pre-warmed store are both banned — see ../README.md,
// "fixture contract").
//
// Two leads worth verifying: (a) since 2026-07-14 the cooldown is configurable
// (`[review] introduction_cooldown`, "0" = none) — a fixture config with a zero
// cooldown would make graded cards reachable in one run. (b) `POST /api/select
// {cram: true}` is documented to queue cards that are not due; if it bypasses
// the cooldown for an already-introduced card, this test becomes cheap. Verify
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

test("`c` keeps the question in place and swaps section context into the answer region", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Section context, a demonstration").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recognize/ }).click(),
  ]);

  const card = page.locator("#card");
  const question = card.locator(".region.q");
  const answer = card.locator("#ansRegion");
  const sectionToggle = answer.locator(".section-toggle");
  await expect(question).toContainText("Which German verb means");
  await expect(answer.locator(".options")).toBeVisible();
  await expect(page.locator(".context.section")).toHaveCount(0);
  await expect(sectionToggle).toBeVisible();
  await expect(sectionToggle).toHaveAttribute("title", "show section context");

  const dividerBox = await card.locator(":scope > .divider").boundingBox();
  const toggleBox = await sectionToggle.boundingBox();
  expect(dividerBox).not.toBeNull();
  expect(toggleBox).not.toBeNull();
  expect(toggleBox?.y ?? 0).toBeGreaterThan(dividerBox?.y ?? Number.MAX_SAFE_INTEGER);

  const calls: string[] = [];
  page.on("request", (req) => { if (req.url().includes("/api/")) calls.push(req.url()); });

  await sectionToggle.click();
  await expect(question).toContainText("Which German verb means");
  await expect(question.locator(".front-text")).toHaveCount(1);
  await expect(answer.locator(".options")).toHaveCount(0);
  await expect(answer).toContainText("Verbs of arguing");
  await expect(answer).toContainText("must not displace the question");
  await expect(question.locator(".context.section")).toHaveCount(0);
  await expect(sectionToggle).toHaveAttribute("title", "hide section context");

  await page.keyboard.press("c");
  await expect(page.locator(".context.section")).toHaveCount(0);
  await expect(question.locator(".front-text")).toHaveCount(1);
  await expect(answer.locator(".options")).toBeVisible();
  expect(calls).toEqual([]);

  const front = await question.textContent();
  const correct = front?.includes("advocate") ? "befürworten" : "einräumen";
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose")),
    page.getByRole("button", { name: new RegExp(correct) }).click(),
  ]);
  const note = card.locator("#noteRegion");
  await expect(note).toBeVisible();
  await page.keyboard.press("c");
  await expect(answer.locator(".context.section").first()).toBeVisible();
  await expect(note).toBeVisible();
  await expect(note.locator(".context.section")).toHaveCount(0);
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/introduce")),
    page.getByRole("button", { name: "Seen" }).click(),
  ]);
  await expect(question).not.toContainText(front ?? "");
  await expect(page.locator(".context.section")).toHaveCount(0);
});

test("a gated sub-card's cell is outlined, not filled, and its parent's is not", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "gated").click(); // focuses the row -> opens the drawer

  const drawer = page.locator(".drawer").filter({ hasText: "2 cards" }).last();
  await expect(drawer.locator(".crumb-cell")).toHaveCount(2);
  // Neither card has been reviewed, so both are `unseen`. Locked is a
  // SEPARATE axis: the sub-card waits on its parent graduating, and folding
  // that into the tier would make "never shown to you" and "withheld from
  // you" the same colour.
  await expect(drawer.locator(".crumb-cell.empty")).toHaveCount(2);
  await expect(drawer.locator(".crumb-cell.locked")).toHaveCount(1);
  await expect(drawer.locator(".crumb-cell").first()).not.toHaveClass(/locked/);
  await expect(drawer.locator(".crumb-cell").last()).toHaveClass(/locked/);
});
