import { mkdir } from "node:fs/promises";

import { test, expect } from "./helpers";
import { openApp } from "./helpers";

declare function el(tag: string, cls?: string | null, text?: string): HTMLElement;
declare function frontPrompt(card: any): HTMLElement;
declare function contextLine(text: string, runs: any): HTMLElement;
declare function answerFill(card: any): HTMLElement;
declare function renderOptions(): HTMLElement;
declare function renderWhy(parent: HTMLElement, card: any): void;
declare let state: any;
declare const stage: HTMLElement;

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await page.setViewportSize({ width: 390, height: 844 });
  await openApp(page);
});

test("kids card surfaces render shared math SVGs safely", async ({ page, request }) => {
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
      "width:min(100%,660px);margin:0 auto;padding:20px;display:grid;gap:20px;overflow:hidden";

    const front = el("section", "surface-front");
    front.appendChild(frontPrompt(choice));
    audit.appendChild(front);

    const context = el("section", "surface-context");
    context.appendChild(contextLine(cloze.context[0], cloze.context_runs[0]));
    audit.appendChild(context);

    const answer = el("section", "surface-answer");
    answer.appendChild(answerFill(display));
    audit.appendChild(answer);

    const task = el("section", "surface-checklist");
    task.appendChild(frontPrompt(checklist));
    audit.appendChild(task);

    state = choiceState;
    const choices = el("section", "surface-choice");
    choices.appendChild(renderOptions());
    audit.appendChild(choices);

    const why = el("section", "surface-note");
    renderWhy(why, choice);
    audit.appendChild(why);

    state = {
      ...choiceState,
      keypoints: explain.back,
      keypoint_runs: explain.back_runs,
    };
    const keypoints = el("section", "surface-keypoint");
    renderWhy(keypoints, { ...explain, note: [] });
    audit.appendChild(keypoints);

    const literal = el("section", "surface-code");
    literal.appendChild(frontPrompt(code));
    literal.appendChild(answerFill(code));
    audit.appendChild(literal);

    const failed = el("section", "surface-error");
    failed.appendChild(frontPrompt(error));
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
    (document.getElementById("app") as HTMLElement).style.height = "auto";
    (document.getElementById("app") as HTMLElement).style.overflow = "visible";
    (document.querySelector(".appbar") as HTMLElement).style.display = "none";
    (document.getElementById("actionbar") as HTMLElement).style.display = "none";
    document.querySelectorAll(".fade").forEach((node) => {
      (node as HTMLElement).style.display = "none";
    });
    (document.querySelector(".stage-wrap") as HTMLElement).style.display = "block";
    const stageElement = document.getElementById("stage") as HTMLElement;
    stageElement.style.position = "static";
    stageElement.style.overflow = "visible";
  });
  await page.screenshot({ path: "/tmp/latex-math-shots/kids.png", fullPage: true });
  await request.post("/api/deselect", { data: {} });
});
