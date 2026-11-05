import type { Page } from "@playwright/test";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

const sentence = (runs: { text: string; code?: boolean }[]) => ({
  kind: "sentence",
  text: runs.map((r) => (r.code ? `\`${r.text}\`` : r.text)).join(""),
  runs,
});

const QUESTION_UNITS = [
  sentence([{ text: "Which line fails, and what does " }, { text: "mut", code: true }, { text: " apply to?" }]),
  { kind: "code", lines: ["let (mut v, w) = (vec![1], 42);", "v.push(4);", "w += 1;"] },
];

const base = {
  deck: "source-fact.md",
  strictness: "balanced",
  total: 1,
  current: 0,
  answer: "",
  on_last: true,
  grades: [],
  sections: [],
  verdicts: null,
  full: true,
  mastered: false,
  gaps: [],
  can_remediate: false,
  remediated_count: null,
  unlocks: [],
  thinking: false,
  error: null,
  elapsed: null,
};

async function startMockedExam(page: Page, dto: object) {
  await page.route("**/api/decks", async (route) => {
    const response = await route.fetch();
    const catalog = await response.json();
    for (const row of catalog.workspaces.flatMap((workspace: any) => workspace.members)) {
      if (row.name.endsWith("source-fact.md")) row.exam_prepared = true;
    }
    await route.fulfill({ response, json: catalog });
  });
  await page.locator("#navRefresh").click();
  await page.route("**/api/exam/start", (route) => route.fulfill({ json: dto }));
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Source Fact").click();
  await page.getByRole("button", { name: /^Take exam/ }).click();
}

test("an exam question renders inline code and numbers its program's lines", async ({ page }) => {
  await startMockedExam(page, {
    ...base,
    phase: "answering",
    question: "raw text the units replace",
    question_units: QUESTION_UNITS,
  });
  const question = page.locator(".exam-q");
  await expect(question.locator("code"), "inline code: one code run").toHaveText("mut");
  await expect(question, "inline code: no literal backticks").not.toContainText("`");
  await expect(question.locator(".exam-code .source-number"), "program: numbered lines").toHaveText(["1", "2", "3"]);
  await expect(question.locator(".exam-code .source-text").nth(2), "program: line 3").toHaveText("w += 1;");
  if (process.env.ALIX_RENDER_DIR) await page.screenshot({ path: `${process.env.ALIX_RENDER_DIR}/exam-question.png`, animations: "disabled" });
});

test("an answer given before leaving reopens read-only, an open one stays editable", async ({ page }) => {
  await startMockedExam(page, {
    ...base,
    phase: "answering",
    question: "Why?",
    question_units: null,
    total: 2,
    on_last: false,
    answer: "kept answer",
    answer_locked: true,
  });
  const input = page.locator(".exam-input");
  await expect(input, "locked: the kept answer").toHaveValue("kept answer");
  await expect(input, "locked: read-only").not.toBeEditable();

  await page.unroute("**/api/exam/start");
  await page.route("**/api/exam/answer", (route) => route.fulfill({
    json: { ...base, phase: "answering", total: 2, current: 1, question: "How?", question_units: null, answer: "", answer_locked: false },
  }));
  await input.press("Shift+Enter");
  await expect(input, "open: editable").toBeEditable();
});

test("exam results render the question, feedback, and rubric points from their units", async ({ page }) => {
  await startMockedExam(page, {
    ...base,
    phase: "results",
    current: 1,
    question: null,
    question_units: null,
    sections: [{
      label: "Bindings",
      cards: ["card-0123456789abcdefghjkmnpqrs"],
      pass: false,
      verdicts: { pass: 0, partial: 1, total: 1 },
    }],
    verdicts: { pass: 0, partial: 1, total: 1 },
    grades: [
      {
        question: "raw",
        question_units: QUESTION_UNITS,
        points: ["raw point"],
        points_units: [[sentence([{ text: "Only " }, { text: "v", code: true }, { text: " is mutable." }])]],
        answer: "w is not mutable",
        section: 0,
        shape: "boundary",
        model: "pattern binding mutability",
        verdict: "PARTIAL",
        feedback: "raw feedback",
        feedback_units: [sentence([{ text: "Right line, but " }, { text: "mut", code: true }, { text: " binds per name." }])],
        missed: ["raw miss"],
        missed_units: [[sentence([{ text: "The fix " }, { text: "(mut v, mut w)", code: true }])]],
      },
    ],
  });
  const result = page.locator(".exam-result");
  await expect(result.locator(".exam-rq .exam-code .source-number"), "results question: numbered lines").toHaveText(["1", "2", "3"]);
  await expect(result.locator(".vfb code"), "feedback: code run").toHaveText("mut");
  await expect(result.locator(".exam-points code"), "rubric point: code run").toHaveText("v");
  await expect(result.locator(".exam-missed code"), "missed point: code run").toHaveText("(mut v, mut w)");
  await expect(result, "results: no literal backticks").not.toContainText("`");
  if (process.env.ALIX_RENDER_DIR) await page.screenshot({ path: `${process.env.ALIX_RENDER_DIR}/exam-results.png`, animations: "disabled" });
});

test("an exam whose questions could not be generated shows the error without a working indicator", async ({ page }) => {
  await startMockedExam(page, {
    ...base,
    phase: "generating",
    total: 0,
    question: null,
    question_units: null,
    thinking: false,
    error: "cannot read `exam:` /decks/src: Is a directory (os error 21)",
  });
  await expect(page.locator(".exam-error"), "failed generation: the error shows").toContainText("Is a directory");
  await expect(page.locator(".exam-wait"), "failed generation: no 'Preparing your exam…'").toHaveCount(0);
  await expect(page.locator(".exam-elapsed"), "failed generation: no pulsing working dot").toHaveCount(0);
  await expect(page.locator(".chip", { hasText: /^Close/ }), "failed generation: Close is offered").toBeVisible();
});
