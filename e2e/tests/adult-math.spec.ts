import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await page.setViewportSize({ width: 390, height: 844 });
  await openApp(page);
  await expect(page.locator(".deckrow").first()).toBeVisible();
});

test("adult context blocks render paired bare math and keep unmatched openers literal", async ({ page, request }) => {
  const browseResponse = await request.post("/api/browse", {
    data: { deck: "animals/math.md" },
  });
  expect(browseResponse.ok(), await browseResponse.text()).toBeTruthy();
  const browse = await browseResponse.json();
  const display = browse.cards.find((card: any) => card.front.includes("Evaluate this display formula"));
  const mathUnit = display?.back_units?.find(
    (unit: any) => unit.kind === "sentence" && unit.runs?.some((run: any) => run.math?.display),
  );
  expect(mathUnit, "the fixture must provide one reusable display-math unit").toBeTruthy();

  await page.evaluate(async (unit) => {
    const { appendContext, el } = await import("/review.js");
    const audit = el("div", "context-block-audit");
    for (const [name, lines, units] of [
      ["bare", ["$$", unit.text, "$$"], [unit]],
      ["fence", ["```math", unit.text, "```"], [unit]],
      ["unclosed", ["$$", unit.text], []],
    ]) {
      const surface = el("section", `surface-${name}`);
      appendContext(surface, lines, null, units);
      audit.appendChild(surface);
    }
    document.getElementById("stage")!.replaceChildren(audit);
  }, mathUnit);

  await expect(page.locator(".surface-bare svg")).toBeVisible();
  await expect(page.locator(".surface-bare .math-error")).toHaveCount(0);
  await expect(page.locator(".surface-bare")).not.toContainText("$$");
  await expect(page.locator(".surface-fence svg")).toBeVisible();
  await expect(page.locator(".surface-fence .math-error")).toHaveCount(0);
  await expect(page.locator(".surface-fence")).not.toContainText("```math");
  await expect(page.locator(".surface-unclosed")).toContainText("$$");
  await expect(page.locator(".surface-unclosed")).toContainText(mathUnit.text);
});

test("adult card surfaces render shared math SVGs safely", async ({ page, request }, testInfo) => {
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

  await page.evaluate(async ({ cards, choiceState }) => {
    const {
      appendChoiceOptions,
      appendKeypointList,
      appendReveal,
      contextLine,
      el,
      frontEl,
      renderNote,
    } = await import("/review.js");
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

    const choices = el("section", "surface-choice");
    appendChoiceOptions(choices, {
      choices: choiceState.choices,
      choiceRuns: choiceState.choice_runs,
    });
    audit.appendChild(choices);

    const keypoints = el("section", "surface-keypoint");
    appendKeypointList(keypoints, {
      keypoints: explain.back,
      keypointRuns: explain.back_runs,
      marks: [],
      cursor: 0,
    });
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

    document.getElementById("stage")!.replaceChildren(audit);
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

  const standalone = page.locator(".surface-context .math-inline.math-standalone");
  await expect(standalone).toBeVisible();
  expect(await standalone.evaluate((node) => {
    const svg = node.querySelector("svg");
    if (!svg) return false;
    return svg.getBoundingClientRect().height >=
      Number.parseFloat(getComputedStyle(node).fontSize) * 1.4;
  })).toBeTruthy();
  await expect(page.locator(".surface-front .math-standalone")).toHaveCount(0);

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
  await page.locator(".math-audit").screenshot({ path: testInfo.outputPath("adult-light.png") });
  await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });
  await page.locator(".math-audit").screenshot({ path: testInfo.outputPath("adult-dark.png") });
});

test("a tall display formula on the question side scales into the capped question region", async ({ page, request }) => {
  await page.setViewportSize({ width: 1000, height: 600 });
  const browseResponse = await request.post("/api/browse", {
    data: { deck: "animals/math.md" },
  });
  expect(browseResponse.ok(), await browseResponse.text()).toBeTruthy();
  const browse = await browseResponse.json();
  const explain = browse.cards.find((card: any) => card.front.includes("Explain the quadratic formula"));
  const inlineRun = explain?.back_runs?.[0]?.find((run: any) => run.math?.svg);
  expect(inlineRun, "the fixture must carry the quadratic formula as a rendered math run").toBeTruthy();
  // The same SVG serves inline and display math; flagging it display puts the
  // fraction on the question side as a block, taller than the capped region.
  const formula = { ...inlineRun, math: { ...inlineRun.math, display: true } };
  const contextCard = { ...explain, context: [inlineRun.text], context_runs: [[formula]], context_units: [] };
  const frontCard = { ...explain, front: inlineRun.text, front_runs: [formula], front_units: null, context: [], context_runs: [], context_units: [] };
  await page.route("**/api/browse", (route) =>
    route.fulfill({ json: { cards: [contextCard, frontCard], label: "tall formulas" } }),
  );
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
  await expect(page.locator(".deckrow").first(), errors.join("\n")).toBeVisible();
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Math").hover();
  await page.keyboard.press("b");

  const question = page.locator(".region.q");
  const formulaStaysInside = async (where: string) => {
    const svg = question.locator(".math-display svg");
    await expect(svg).toBeVisible();
    const region = await question.boundingBox();
    const drawn = await svg.boundingBox();
    expect(region && drawn, `${where}: the question region and its formula must lay out`).toBeTruthy();
    expect(
      drawn!.y + drawn!.height,
      `${where}: formula bottom ${drawn!.y + drawn!.height} must not pass the question region's bottom ${region!.y + region!.height}`,
    ).toBeLessThanOrEqual(region!.y + region!.height);
  };
  await formulaStaysInside("context formula");
  await expect(question.locator(".context")).toHaveCount(1);
  await page.keyboard.press("ArrowRight");
  await expect(question.locator(".context")).toHaveCount(0);
  await formulaStaysInside("front formula");
});
