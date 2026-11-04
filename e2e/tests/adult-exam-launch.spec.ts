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
  meta?: string | null;
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
    },
    "cats.md": { state: "examdue", examable: false, exam_filter: "partial" },
    "source-fact.md": { state: "new", examable: true, exam_filter: "all" },
    "fronts.md": { state: "examdue", examable: true, exam_filter: "none" },
  });
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();

  const takeExam = page.getByRole("button", { name: /^Take exam/ });
  const filter = page.getByRole("button", { name: /only what needs work/ });

  await adultDeckRow(page, "wild").click();
  await expect(takeExam).toHaveClass(/\bexam-due\b/);
  await expect(filter).toHaveText(/☑ only what needs work/);
  await filter.click();
  await expect(filter).toHaveText(/☐ only what needs work/);
  await filter.click();
  await expect(filter).toHaveText(/☑ only what needs work/);
  await expect(adultDeckRow(page, "wild").locator(".meta")).toHaveText("passed · 15.5 / 18");
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
  await page.route("**/api/exam/start", (route) => route.fulfill({
    json: { phase: "generating", deck: "wild.md", strictness: "balanced", thinking: false },
  }));
  const request = page.waitForRequest("**/api/exam/start");
  await takeExam.click();
  expect((await request).postDataJSON()).toEqual({ deck: "animals/wild.md", needs_work_only: true });
});

const sentence = (text: string) => ({ kind: "sentence", text, runs: [{ text }] });

function resultsDto({ mastered, pass, partial }: { mastered: boolean; pass: number; partial: number }) {
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
        section: 0,
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
        section: 1,
        shape: "fix",
        model: "borrows preserve ownership",
        verdict: "PASS",
        feedback: "Borrowing keeps the owner usable.",
        feedback_units: [sentence("Borrowing keeps the owner usable.")],
        missed: [],
        missed_units: [],
      },
    ],
    sections: [
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
    "source-fact.md": { state: "examdue", examable: true, exam_filter: "none" },
  });
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Source Fact").click();
  await page.route("**/api/exam/start", (route) => route.fulfill({ json: dto }));
  await page.getByRole("button", { name: /^Take exam/ }).click();
}

test("a full perfect sitting shows its count, section counts, shapes, and mastered", async ({ page }) => {
  await openResults(page, resultsDto({ mastered: true, pass: 18, partial: 0 }));

  await expect(page.locator(".exam-score")).toHaveText("18 / 18");
  await expect(page.locator(".exam-mastered")).toHaveText("mastered");
  await expect(page.locator(".exam-section")).toHaveCount(2);
  await expect(page.locator(".exam-section").first()).toContainText("Ownership");
  await expect(page.locator(".exam-section").first()).toContainText("9 / 9");
  await expect(page.locator(".exam-shape")).toHaveText(["boundary", "fix"]);
  await expect(page.getByText(/PASSED|FAILED/)).toHaveCount(0);
  await capture(page, "results-mastered");
});

test("a partial-point sitting shows 17.5 out of 18 without mastered", async ({ page }) => {
  await openResults(page, resultsDto({ mastered: false, pass: 17, partial: 1 }));

  await expect(page.locator(".exam-score")).toHaveText("17.5 / 18");
  await expect(page.locator(".exam-mastered")).toHaveCount(0);
  await expect(page.locator(".exam-section").first()).toContainText("8.5 / 9");
  await expect(page.locator(".exam-section").first()).toContainText("needs work");
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
