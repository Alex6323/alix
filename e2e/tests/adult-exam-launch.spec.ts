import type { Page } from "@playwright/test";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

type PickerState = {
  state: string;
  examable: boolean;
  exam_filter: "none" | "partial" | "all";
  exam_prepared?: boolean;
  meta?: string | null;
  exam_changed?: boolean;
};

async function mockExamRows(page: Page, states: Record<string, PickerState>) {
  await page.route("**/api/decks", async (route) => {
    const response = await route.fetch();
    const catalog = await response.json();
    const rows = [
      ...catalog.recent,
      ...catalog.workspaces.flatMap((row: any) => row.members),
      ...catalog.folders.flatMap((row: any) => row.members),
    ];
    for (const [suffix, state] of Object.entries(states)) {
      const row = rows.find((candidate: any) => candidate.name.endsWith(suffix));
      expect(row, `mock row ${suffix}`).toBeDefined();
      Object.assign(row, {
        has_exam: true,
        is_trace: false,
        reviewable: true,
        reviewable_recognize: true,
        reviewable_recall: true,
        reviewable_reconstruct: false,
        last_launch: "recall",
        ...state,
      });
    }
    await route.fulfill({ response, json: catalog });
  });
}

async function preparingSequence(page: Page) {
  const response = await page.request.get("/api/exam/preparing");
  let sequence: number = (await response.json()).sequence;
  return (jobs: object[]) => ({ sequence: ++sequence, jobs });
}

async function capture(page: Page, stem: string) {
  const directory = process.env.ALIX_RENDER_DIR;
  if (!directory) return;
  for (const viewport of [
    { width: 390, height: 844 },
    { width: 1200, height: 800 },
  ]) {
    await page.setViewportSize(viewport);
    await page.screenshot({
      path: `${directory}/${stem}-${viewport.width}.png`,
      fullPage: true,
      animations: "disabled",
    });
  }
}

test("the drawer always shows the exam filter, accents only due unlocked exams, and renders the server badge", async ({ page }) => {
  await mockExamRows(page, {
    "wild.md": {
      state: "examdue",
      examable: true,
      exam_filter: "partial",
      meta: "passed · 15.5 / 18",
      exam_changed: true,
    },
    "cats.md": { state: "examdue", examable: false, exam_filter: "partial" },
    "source-fact.md": { state: "new", examable: true, exam_filter: "all" },
    "fronts.md": { state: "examdue", examable: true, exam_filter: "none" },
  });
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();

  const takeExam = page.getByRole("button", { name: /^Generate exam/ });
  const filter = page.getByRole("button", { name: /only what needs work/ });

  await adultDeckRow(page, "wild").click();
  await expect(takeExam).toHaveClass(/\bexam-due\b/);
  await expect(filter).toHaveText(/☑ only what needs work/);
  await filter.click();
  await expect(filter).toHaveText(/☐ only what needs work/);
  await filter.click();
  await expect(filter).toHaveText(/☑ only what needs work/);
  await expect(adultDeckRow(page, "wild").locator(".meta")).toHaveText("passed · 15.5 / 18");
  await expect(adultDeckRow(page, "wild").locator(".meta")).toHaveClass(/\bexam-changed\b/);
  await capture(page, "drawer-filled-filter-passed-badge");

  await adultDeckRow(page, "cats").click();
  await expect(takeExam).toBeDisabled();
  await expect(takeExam).not.toHaveClass(/\bexam-due\b/);
  await expect(filter).toBeDisabled();
  await expect(filter).toHaveText(/☐ only what needs work/);
  await capture(page, "drawer-locked-filter");

  await adultDeckRow(page, "Source Fact").click();
  await expect(takeExam).toBeEnabled();
  await expect(takeExam).not.toHaveClass(/\bexam-due\b/);
  await expect(filter).toBeEnabled();
  await capture(page, "drawer-new-unfilled");

  await adultDeckRow(page, "fronts").click();
  await expect(filter).toBeDisabled();
  await expect(filter).toHaveText(/☐ only what needs work/);
  await capture(page, "drawer-nothing-to-filter");

  await adultDeckRow(page, "wild").click();
  const stamp = await preparingSequence(page);
  const job = {
    deck: "animals/wild.md",
    state: "generating",
    elapsed: 0,
    estimate: "usually 10 to 20 minutes",
    error: null,
  };
  let generating = false;
  await page.route("**/api/exam/preparing", (route) =>
    route.fulfill({ json: stamp(generating ? [job] : []) }));
  await page.route("**/api/exam/generate", (route) => {
    generating = true;
    return route.fulfill({ json: stamp([job]) });
  });
  const request = page.waitForRequest("**/api/exam/generate");
  await takeExam.click();
  expect((await request).postDataJSON()).toEqual({ deck: "animals/wild.md", needs_work_only: true });
  const writing = page.getByRole("button", { name: /^Writing exam/ });
  await expect(writing).toHaveText("Writing exam · 0 min · usually 10 to 20 minutes");
  await expect(writing).toBeDisabled();
  await expect(page.getByRole("button", { name: "Stop", exact: true })).toBeEnabled();
  await expect(filter).toHaveCount(0);
  await capture(page, "drawer-writing-exam");
});

test("a finished exam announces itself and turns the chip into Take exam", async ({ page }) => {
  let prepared = false;
  await page.route("**/api/decks", async (route) => {
    const response = await route.fetch();
    const catalog = await response.json();
    const row = catalog.workspaces
      .flatMap((workspace: any) => workspace.members)
      .find((candidate: any) => candidate.name.endsWith("wild.md"));
    Object.assign(row, {
      has_exam: true,
      is_trace: false,
      examable: true,
      exam_filter: "none",
      exam_prepared: prepared,
    });
    await route.fulfill({ response, json: catalog });
  });
  const job = (state: string) => ({
    deck: "animals/wild.md",
    state,
    elapsed: 0,
    estimate: state === "generating" ? "usually 10 to 20 minutes" : null,
    error: null,
  });
  const stamp = await preparingSequence(page);
  let generating = false;
  await page.route("**/api/exam/generate", (route) => {
    generating = true;
    return route.fulfill({ json: stamp([job("generating")]) });
  });
  await page.route("**/api/exam/preparing", (route) =>
    route.fulfill({ json: stamp(prepared ? [job("ready")] : generating ? [job("generating")] : []) }));
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();

  await page.getByRole("button", { name: /^Generate exam/ }).click();
  await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();
  prepared = true;

  await expect(page.locator("#notice")).toHaveText("exam ready: animals/wild.md", { timeout: 15_000 });
  await expect(page.getByRole("button", { name: /^Take exam/ })).toBeEnabled();
});

test("an older preparing response cannot erase a generation just started", async ({ page }) => {
  await page.waitForLoadState("networkidle");
  await mockExamRows(page, {
    "wild.md": { state: "examdue", examable: true, exam_filter: "none", exam_prepared: false },
  });
  const job = {
    deck: "animals/wild.md",
    state: "generating",
    elapsed: 0,
    estimate: "usually 10 to 20 minutes",
    error: null,
  };
  let releaseOldResponse!: () => void;
  const oldResponseGate = new Promise<void>((resolve) => { releaseOldResponse = resolve; });
  let oldRequestStarted!: () => void;
  const oldRequest = new Promise<void>((resolve) => { oldRequestStarted = resolve; });
  let oldResponseReturned!: () => void;
  const oldReturned = new Promise<void>((resolve) => { oldResponseReturned = resolve; });
  const stamp = await preparingSequence(page);
  let preparingCalls = 0;
  await page.route("**/api/exam/preparing", async (route) => {
    preparingCalls += 1;
    if (preparingCalls === 1) {
      const old = stamp([]);
      oldRequestStarted();
      await oldResponseGate;
      await route.fulfill({ json: old });
      oldResponseReturned();
      return;
    }
    await route.fulfill({ json: stamp([job]) });
  });
  await page.route("**/api/exam/generate", (route) => route.fulfill({ json: stamp([job]) }));

  await page.locator("#navRefresh").click();
  await oldRequest;
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  await page.getByRole("button", { name: /^Generate exam/ }).click();
  await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();

  releaseOldResponse();
  await oldReturned;
  await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();
});

test("an empty refresh cannot hide a generation response still in flight", async ({ page }) => {
  await page.waitForLoadState("networkidle");
  await mockExamRows(page, {
    "wild.md": { state: "examdue", examable: true, exam_filter: "none", exam_prepared: false },
  });
  const job = {
    deck: "animals/wild.md",
    state: "generating",
    elapsed: 0,
    estimate: "usually 10 to 20 minutes",
    error: null,
  };
  let generationStarted!: () => void;
  const started = new Promise<void>((resolve) => { generationStarted = resolve; });
  let releaseGeneration!: () => void;
  const generationGate = new Promise<void>((resolve) => { releaseGeneration = resolve; });
  let generationReturned!: () => void;
  const returned = new Promise<void>((resolve) => { generationReturned = resolve; });
  let refreshAfterGenerationStarted!: () => void;
  const refreshed = new Promise<void>((resolve) => { refreshAfterGenerationStarted = resolve; });
  const stamp = await preparingSequence(page);
  let generating = false;
  await page.route("**/api/exam/preparing", async (route) => {
    if (generating) refreshAfterGenerationStarted();
    await route.fulfill({ json: stamp([]) });
  });
  await page.route("**/api/exam/generate", async (route) => {
    generating = true;
    generationStarted();
    await generationGate;
    await route.fulfill({ json: stamp([job]) });
    generationReturned();
  });

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  await page.getByRole("button", { name: /^Generate exam/ }).click();
  await started;

  await page.locator("#navRefresh").click();
  await refreshed;
  releaseGeneration();
  await returned;

  await adultDeckRow(page, "wild").click();
  await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();
});

test("an older cancellation response cannot hide a newer generation", async ({ page }) => {
  await page.waitForLoadState("networkidle");
  await mockExamRows(page, {
    "wild.md": { state: "examdue", examable: true, exam_filter: "none", exam_prepared: false },
  });
  const job = {
    deck: "animals/wild.md",
    state: "generating",
    elapsed: 0,
    estimate: "usually 10 to 20 minutes",
    error: null,
  };
  let cancelStarted!: () => void;
  const started = new Promise<void>((resolve) => { cancelStarted = resolve; });
  let releaseCancellation!: () => void;
  const cancellationGate = new Promise<void>((resolve) => { releaseCancellation = resolve; });
  let cancellationReturned!: () => void;
  const returned = new Promise<void>((resolve) => { cancellationReturned = resolve; });
  let emptyRefreshReturned!: () => void;
  const refreshed = new Promise<void>((resolve) => { emptyRefreshReturned = resolve; });
  const stamp = await preparingSequence(page);
  let generating = false;
  let cancellationPending = false;
  await page.route("**/api/exam/preparing", async (route) => {
    if (cancellationPending && !generating) emptyRefreshReturned();
    await route.fulfill({ json: stamp(generating ? [job] : []) });
  });
  await page.route("**/api/exam/generate", async (route) => {
    generating = true;
    await route.fulfill({ json: stamp([job]) });
  });
  await page.route("**/api/exam/generate/cancel", async (route) => {
    generating = false;
    cancellationPending = true;
    const cancelled = stamp([]);
    cancelStarted();
    await cancellationGate;
    await route.fulfill({ json: cancelled });
    cancellationReturned();
  });

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  await page.getByRole("button", { name: /^Generate exam/ }).click();
  await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();

  await page.getByRole("button", { name: /^Stop/ }).click();
  await started;
  await page.locator("#navRefresh").click();
  await refreshed;
  await adultDeckRow(page, "wild").click();
  await page.getByRole("button", { name: /^Generate exam/ }).click();
  await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();

  releaseCancellation();
  await returned;
  await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();
});

test("a delayed duplicate generation cannot hide an earlier generation response", async ({ page }) => {
  await page.waitForLoadState("networkidle");
  await mockExamRows(page, {
    "wild.md": { state: "examdue", examable: true, exam_filter: "none", exam_prepared: false },
  });
  const job = {
    deck: "animals/wild.md",
    state: "generating",
    elapsed: 0,
    estimate: "usually 10 to 20 minutes",
    error: null,
  };
  let firstStarted!: () => void;
  const firstRequest = new Promise<void>((resolve) => { firstStarted = resolve; });
  let secondStarted!: () => void;
  const secondRequest = new Promise<void>((resolve) => { secondStarted = resolve; });
  let releaseFirst!: () => void;
  const firstGate = new Promise<void>((resolve) => { releaseFirst = resolve; });
  let releaseSecond!: () => void;
  const secondGate = new Promise<void>((resolve) => { releaseSecond = resolve; });
  let firstReturned!: () => void;
  const firstResponse = new Promise<void>((resolve) => { firstReturned = resolve; });
  let secondReturned!: () => void;
  const secondResponse = new Promise<void>((resolve) => { secondReturned = resolve; });
  const stamp = await preparingSequence(page);
  let calls = 0;
  await page.route("**/api/exam/preparing", (route) => route.fulfill({ json: stamp(calls ? [job] : []) }));
  await page.route("**/api/exam/generate", async (route) => {
    calls += 1;
    const reply = stamp([job]);
    if (calls === 1) {
      firstStarted();
      await firstGate;
      await route.fulfill({ json: reply });
      firstReturned();
      return;
    }
    secondStarted();
    await secondGate;
    await route.fulfill({ json: reply });
    secondReturned();
  });

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  const generate = page.getByRole("button", { name: /^Generate exam/ });
  await generate.click();
  await firstRequest;
  await generate.click();
  await secondRequest;

  releaseFirst();
  await firstResponse;
  try {
    await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();
  } finally {
    releaseSecond();
    await secondResponse;
  }
});

test("a generation processed after a later stop response is still shown", async ({ page }) => {
  await page.waitForLoadState("networkidle");
  await mockExamRows(page, {
    "wild.md": { state: "examdue", examable: true, exam_filter: "none", exam_prepared: false },
    "source-fact.md": { state: "examdue", examable: true, exam_filter: "none", exam_prepared: false },
  });
  const job = (deck: string) => ({
    deck,
    state: "generating",
    elapsed: 0,
    estimate: "usually 10 to 20 minutes",
    error: null,
  });
  const stamp = await preparingSequence(page);
  let serverJobs = [job("animals/wild.md")];
  let generationStarted!: () => void;
  const started = new Promise<void>((resolve) => { generationStarted = resolve; });
  let releaseGeneration!: () => void;
  const generationGate = new Promise<void>((resolve) => { releaseGeneration = resolve; });
  let generationReturned!: () => void;
  const generated = new Promise<void>((resolve) => { generationReturned = resolve; });
  let stopReturned!: () => void;
  const stopped = new Promise<void>((resolve) => { stopReturned = resolve; });
  await page.route("**/api/exam/preparing", (route) => route.fulfill({ json: stamp(serverJobs) }));
  await page.route("**/api/exam/generate", async (route) => {
    generationStarted();
    await generationGate;
    serverJobs = [job("animals/source-fact.md")];
    await route.fulfill({ json: stamp(serverJobs) });
    generationReturned();
  });
  await page.route("**/api/exam/generate/cancel", async (route) => {
    serverJobs = [];
    await route.fulfill({ json: stamp(serverJobs) });
    stopReturned();
  });

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();
  await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();

  await adultDeckRow(page, "Source Fact").click();
  await page.getByRole("button", { name: /^Generate exam/ }).click();
  await started;
  await adultDeckRow(page, "wild").click();
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await stopped;

  releaseGeneration();
  await generated;
  await adultDeckRow(page, "Source Fact").click();
  await expect(page.getByRole("button", { name: /^Writing exam/ })).toBeVisible();
});

test("a prepared exam is taken, without the scope filter", async ({ page }) => {
  await mockExamRows(page, {
    "wild.md": { state: "examdue", examable: true, exam_filter: "partial", exam_prepared: true },
  });
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "wild").click();

  await expect(page.getByRole("button", { name: /only what needs work/ })).toHaveCount(0);
  await page.route("**/api/exam/start", (route) => route.fulfill({
    json: { phase: "answering", deck: "wild.md", strictness: "balanced", thinking: false },
  }));
  const request = page.waitForRequest("**/api/exam/start");
  await page.getByRole("button", { name: /^Take exam/ }).click();
  expect((await request).postDataJSON()).toEqual({ deck: "animals/wild.md" });
});

test("the checked filter on an all-needs-work deck reaches the exam request", async ({ page }) => {
  await mockExamRows(page, {
    "source-fact.md": { state: "new", examable: true, exam_filter: "all" },
  });
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Source Fact").click();

  const filter = page.getByRole("button", { name: /only what needs work/ });
  await expect(filter).toHaveText(/☑ only what needs work/);
  await page.route("**/api/exam/generate", (route) => route.fulfill({
    json: { deck: "animals/source-fact.md", state: "generating", elapsed: 0, estimate: null, error: null },
  }));
  const request = page.waitForRequest("**/api/exam/generate");
  await page.getByRole("button", { name: /^Generate exam/ }).click();

  expect((await request).postDataJSON()).toEqual({
    deck: "animals/source-fact.md",
    needs_work_only: true,
  });
});

const sentence = (text: string) => ({ kind: "sentence", text, runs: [{ text }] });

function resultsDto({
  mastered,
  pass,
  partial,
  examChanged = false,
  unsourcedTopics = [],
}: {
  mastered: boolean;
  pass: number;
  partial: number;
  examChanged?: boolean;
  unsourcedTopics?: string[];
}) {
  return {
    phase: "results",
    deck: "source-fact.md",
    strictness: "balanced",
    total: 2,
    current: 2,
    answer: "",
    on_last: true,
    grades: [
      {
        question: "Where does ownership end?",
        question_units: [sentence("Where does ownership end?")],
        points: ["At the end of the owning scope."],
        points_units: [[sentence("At the end of the owning scope.")]],
        answer: "At the scope boundary.",
        topic: 0,
        shape: "boundary",
        model: "scope owns destruction",
        verdict: mastered ? "PASS" : "PARTIAL",
        feedback: "The scope boundary determines destruction.",
        feedback_units: [sentence("The scope boundary determines destruction.")],
        missed: mastered ? [] : ["The owner triggers the drop."],
        missed_units: mastered ? [] : [[sentence("The owner triggers the drop.")]],
      },
      {
        question: "How should the move be fixed?",
        question_units: [sentence("How should the move be fixed?")],
        points: ["Borrow instead of moving."],
        points_units: [[sentence("Borrow instead of moving.")]],
        answer: "Borrow it.",
        topic: 1,
        shape: "fix",
        model: "borrows preserve ownership",
        verdict: "PASS",
        feedback: "Borrowing keeps the owner usable.",
        feedback_units: [sentence("Borrowing keeps the owner usable.")],
        missed: [],
        missed_units: [],
      },
    ],
    topics: [
      {
        label: "Ownership",
        cards: ["card-0123456789abcdefghjkmnpqrs"],
        pass: mastered,
        verdicts: mastered
          ? { pass: 9, partial: 0, total: 9 }
          : { pass: 8, partial: 1, total: 9 },
      },
      {
        label: "Borrowing",
        cards: ["card-1123456789abcdefghjkmnpqrs"],
        pass: true,
        verdicts: { pass: 9, partial: 0, total: 9 },
      },
    ],
    verdicts: { pass, partial, total: 18 },
    full: true,
    mastered,
    exam_changed: examChanged,
    unsourced_topics: unsourcedTopics,
    exam_high_mark: mastered ? "mastered (4 Nov)" : "passed (2 Nov)",
    gaps: mastered ? [] : ["ownership and the move model"],
    can_remediate: !mastered,
    remediated_count: null,
    unlocks: mastered ? ["advanced.md"] : [],
    thinking: false,
    error: null,
    elapsed: null,
  };
}

async function openResults(page: Page, dto: object) {
  await mockExamRows(page, {
    "source-fact.md": { state: "examdue", examable: true, exam_filter: "none", exam_prepared: true },
  });
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Source Fact").click();
  await page.route("**/api/exam/start", (route) => route.fulfill({ json: dto }));
  await page.getByRole("button", { name: /^Take exam/ }).click();
}

test("a full perfect sitting shows its count, topic counts, shapes, and mastered", async ({ page }) => {
  await openResults(
    page,
    resultsDto({ mastered: true, pass: 18, partial: 0, examChanged: true, unsourcedTopics: ["Terms", "Intro"] }),
  );

  await expect(page.locator(".exam-score")).toHaveText("18 / 18");
  await expect(page.locator(".exam-standing")).toHaveClass(/\bexam-changed\b/);
  await expect(page.locator(".exam-unsourced")).toHaveText("Not examined: Terms, Intro");
  await expect(page.locator(".exam-standing")).toHaveText("mastered (4 Nov)");
  await expect(page.locator(".exam-topic")).toHaveCount(2);
  await expect(page.locator(".exam-topic").first()).toContainText("Ownership");
  await expect(page.locator(".exam-topic").first()).toContainText("9 / 9");
  await expect(page.locator(".exam-shape")).toHaveText(["boundary", "fix"]);
  await expect(page.getByText(/PASSED|FAILED/)).toHaveCount(0);
  await capture(page, "results-mastered");
});

test("a partial-point sitting shows 17.5 out of 18 and the deck's standing", async ({ page }) => {
  await openResults(page, resultsDto({ mastered: false, pass: 17, partial: 1 }));

  await expect(page.locator(".exam-score")).toHaveText("17.5 / 18");
  await expect(page.locator(".exam-standing")).toHaveText("passed (2 Nov)");
  await expect(page.locator(".exam-standing")).not.toHaveClass(/\bexam-changed\b/);
  await expect(page.locator(".exam-unsourced")).toHaveCount(0);
  await expect(page.locator(".exam-topic").first()).toContainText("8.5 / 9");
  await expect(page.locator(".exam-topic").first()).toContainText("needs work");
  await expect(page.getByText(/PASSED|FAILED/)).toHaveCount(0);
  const questionText = page.locator(".exam-rq-text");
  const first = await questionText.nth(0).boundingBox();
  const second = await questionText.nth(1).boundingBox();
  expect(first, "first result question has layout bounds").not.toBeNull();
  expect(second, "second result question has layout bounds").not.toBeNull();
  expect(
    Math.abs(first!.x - second!.x),
    "question text stays aligned when the shape labels have different widths",
  ).toBeLessThanOrEqual(2);
  await capture(page, "results-17.5");
});
