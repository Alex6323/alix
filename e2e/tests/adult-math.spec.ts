import { mkdir } from "node:fs/promises";

import { test, expect } from "./helpers";
import { openApp } from "./helpers";

declare function el(tag: string, cls?: string | null, text?: string): HTMLElement;
declare function frontEl(text: string, runs: any, units: any): HTMLElement;
declare function contextLine(text: string, runs: any, cls?: string): HTMLElement;
declare function appendReveal(parent: HTMLElement, lines: string[], runs: any, isList: boolean): void;
declare function renderNote(parent: HTMLElement, units: any): void;
declare function renderChoices(parent: HTMLElement): void;
declare function renderExplain(parent: HTMLElement): void;
declare let state: any;
declare let revealed: number;
declare let marks: Array<boolean | undefined>;
declare const stage: HTMLElement;

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await page.setViewportSize({ width: 390, height: 844 });
  await openApp(page);
});

test("adult card surfaces render shared math SVGs safely", async ({ page, request }) => {
  const browseResponse = await request.post("/api/browse", {
    data: { deck: "animals/math.md" },
  });
  expect(browseResponse.ok(), await browseResponse.text()).toBeTruthy();
  const browse = await browseResponse.json();

  await request.post("/api/deselect", { data: {} });
  const selectResponse = await request.post("/api/select", {
    data: { deck: "animals/math.md", depth: "recognize" },
  });
  expect(selectResponse.ok(), await selectResponse.text()).toBeTruthy();
  const selected = await selectResponse.json();

  await page.evaluate(({ cards, choiceState }) => {
    const byFront = (needle: string) => cards.find((card: any) => card.front.includes(needle));
    const choice = byFront("What does E = mc^2 describe?");
    const display = byFront("Evaluate this display formula");
    const checklist = byFront("Formula checklist");
    const cloze = cards.find((card: any) => card.context && card.context.length);
    const code = byFront("Which dollar examples stay literal?");
    const error = byFront("This formula is intentionally malformed");
    const explain = byFront("Explain the quadratic formula");
    if (!choice || !display || !checklist || !cloze || !code || !error || !explain) {
      throw new Error("math fixture cards are incomplete");
    }

    const audit = el("div", "math-audit");
    audit.style.cssText =
      "width:min(100%,720px);margin:0 auto;padding:20px;display:grid;gap:20px;overflow:hidden";

    const front = el("section", "surface-front");
    front.appendChild(frontEl(choice.front, choice.front_runs, choice.front_units));
    audit.appendChild(front);

    const context = el("section", "surface-context");
    context.appendChild(contextLine(cloze.context[0], cloze.context_runs[0]));
    audit.appendChild(context);

    const answer = el("section", "surface-answer reveal");
    appendReveal(answer, display.back, display.back_runs, false);
    audit.appendChild(answer);

    const note = el("section", "surface-note");
    renderNote(note, choice.note);
    audit.appendChild(note);

    const task = el("section", "surface-checklist");
    task.appendChild(frontEl(checklist.front, checklist.front_runs, checklist.front_units));
    audit.appendChild(task);

    state = choiceState;
    const choices = el("section", "surface-choice");
    renderChoices(choices);
    audit.appendChild(choices);

    state = {
      ...choiceState,
      mode: "explain",
      card: explain,
      keypoints: explain.back,
      keypoint_runs: explain.back_runs,
    };
    revealed = 1;
    marks = [];
    const keypoints = el("section", "surface-keypoint");
    renderExplain(keypoints);
    audit.appendChild(keypoints);

    const literal = el("section", "surface-code");
    literal.appendChild(frontEl(code.front, code.front_runs, code.front_units));
    const literalAnswer = el("div", "reveal");
    appendReveal(literalAnswer, code.back, code.back_runs, false);
    literal.appendChild(literalAnswer);
    audit.appendChild(literal);

    const failed = el("section", "surface-error");
    failed.appendChild(frontEl(error.front, error.front_runs, error.front_units));
    audit.appendChild(failed);

    stage.replaceChildren(audit);
  }, { cards: browse.cards, choiceState: selected });

  for (const surface of [
    ".surface-front",
    ".surface-context",
    ".surface-answer",
    ".surface-note",
    ".surface-checklist",
    ".surface-choice",
    ".surface-keypoint",
  ]) {
    await expect(page.locator(`${surface} svg`).first()).toBeVisible();
  }

  const labelledMath = page.locator('.math-run[role="img"]').first();
  await expect(labelledMath).toHaveAttribute("aria-label", /E = mc\^2/);
  await expect(labelledMath.locator("svg")).toHaveAttribute("aria-hidden", "true");
  expect(await page.locator(".math-run svg rect").evaluateAll((rects) => rects.filter((rect) => {
    const svg = rect.ownerSVGElement;
    if (!svg) return false;
    const box = rect.getBoundingClientRect();
    const root = svg.getBoundingClientRect();
    return box.width >= root.width * 0.95 && box.height >= root.height * 0.95;
  }).length)).toBe(0);

  const display = page.locator(".surface-answer .math-display");
  await expect(display).toBeVisible();
  expect(await display.evaluate((node) => getComputedStyle(node).display)).toBe("flex");
  expect(await display.evaluate((node) => {
    const svg = node.querySelector("svg");
    return !!svg && svg.getBoundingClientRect().width <= node.getBoundingClientRect().width + 0.5;
  })).toBeTruthy();

  await expect(page.locator(".surface-error .math-error-source")).toContainText("\\frac{1");
  await expect(page.locator(".surface-error .math-error-label")).toHaveText("math could not render");
  await expect(page.locator(".surface-code")).toContainText("$5 and $10");
  await expect(page.locator(".surface-code code").filter({ hasText: "$x$" }).first()).toBeVisible();
  await expect(page.locator(".surface-code pre")).toContainText("$x$ stays code");
  await expect(page.locator(".surface-code .math-run")).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBeTruthy();

  await mkdir("/tmp/latex-math-shots", { recursive: true });
  await page.evaluate(() => {
    document.body.style.height = "auto";
    document.body.style.overflow = "visible";
    (document.querySelector(".bar") as HTMLElement).style.display = "none";
    (document.querySelector(".legend") as HTMLElement).style.display = "none";
    (document.getElementById("crumbStrip") as HTMLElement).style.display = "none";
    const stageElement = document.getElementById("stage") as HTMLElement;
    stageElement.style.display = "block";
    stageElement.style.padding = "0";
    stageElement.style.overflow = "visible";
  });
  await page.evaluate(() => { document.documentElement.dataset.theme = "light"; });
  await page.locator(".math-audit").screenshot({ path: "/tmp/latex-math-shots/adult-light.png" });
  await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });
  await page.locator(".math-audit").screenshot({ path: "/tmp/latex-math-shots/adult-dark.png" });
});
