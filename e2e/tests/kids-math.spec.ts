import { test, expect } from "./helpers";
import { openApp } from "./helpers";

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await page.setViewportSize({ width: 390, height: 844 });
  await openApp(page);
});

test("kids keeps the content of a bare table front visible", async ({ page }) => {
  await page.evaluate(async () => {
    const kids = await import("/kids.js");
    const dom = kids.createKidsDom({ document });
    const raw = "Before launch\n| check | state |\n|---|---|\n| fuel | ready |";
    const prompt = dom.frontPrompt({
      front: raw,
      front_runs: [{ text: raw }],
      front_units: [
        { kind: "sentence", text: "Before launch", runs: [{ text: "Before launch" }] },
        {
          kind: "table",
          aligns: ["none", "none"],
          header: [[{ text: "check" }], [{ text: "state" }]],
          rows: [[[{ text: "fuel" }], [{ text: "ready" }]]],
        },
      ],
    });
    prompt.classList.add("table-front-repro");
    document.body.appendChild(prompt);
  });

  const prompt = page.locator(".table-front-repro");
  await expect(prompt).toContainText("Before launch");
  await expect(prompt).toContainText("check");
  await expect(prompt).toContainText("state");
  await expect(prompt).toContainText("fuel");
  await expect(prompt).toContainText("ready");
});

test("kids keeps the content of a bare table note visible", async ({ page, request }) => {
  const selectResponse = await request.post("/api/select", {
    data: { deck: "animals/math.md", depth: "recall" },
  });
  expect(selectResponse.ok(), await selectResponse.text()).toBeTruthy();
  const selected = await selectResponse.json();

  await page.evaluate(async (reviewState) => {
    const kids = await import("/kids.js");
    const dom = kids.createKidsDom({ document });
    const stage = dom.el("main");
    const actionbar = dom.el("footer");
    let study: any;
    const paint = () => {
      stage.replaceChildren();
      actionbar.replaceChildren();
      study.render();
    };
    study = kids.createKidsStudy({
      api: async () => study.state(),
      post: (body: object) => ({ method: "POST", body }),
      model: {
        create: kids.createKidsStudyModel,
        apply: kids.applyKidsStudyState,
        clear: kids.clearKidsStudyState,
        choose: kids.chooseKidsAnswer,
        reveal: kids.revealKidsAnswer,
        backCount: kids.kidsBackCount,
        choiceMode: kids.kidsChoiceMode,
        multiMode: kids.kidsMultiMode,
        toggle: kids.toggleKidsChoice,
        revealDone: kids.kidsRevealDone,
        screen: kids.kidsStudyScreen,
      },
      rerender: paint,
      openTutor: () => {},
      openPicker: () => {},
      refreshPicker: () => {},
      reportError: () => {},
      ui: {
        actionbar,
        appendChecklist: dom.appendChecklist,
        appendRuns: dom.appendRuns,
        appendTable: dom.appendTable,
        contextLine: dom.contextLine,
        document,
        el: dom.el,
        frontPrompt: dom.frontPrompt,
        mascot: dom.mascot,
        stage,
      },
    });
    study.apply({
      ...reviewState,
      kind: "review",
      phase: "review",
      introducing: false,
      mode: "fill",
      choices: null,
      card: {
        ...reviewState.card,
        note: [{
          kind: "table",
          aligns: ["none", "none"],
          header: [[{ text: "reason" }], [{ text: "detail" }]],
          rows: [[[{ text: "mass" }], [{ text: "energy" }]]],
        }],
      },
    });
    const show = actionbar.querySelector(".show-btn") as HTMLButtonElement | null;
    if (!show) throw new Error("missing reveal control");
    show.click();
    const host = dom.el("div", "table-note-repro");
    host.append(stage, actionbar);
    document.body.appendChild(host);
  }, selected);

  const note = page.locator(".table-note-repro .rev-why");
  await expect(note).toContainText("reason");
  await expect(note).toContainText("detail");
  await expect(note).toContainText("mass");
  await expect(note).toContainText("energy");
});

test("kids card surfaces render shared math SVGs safely", async ({ page, request }, testInfo) => {
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
    const kids = await import("/kids.js");
    const dom = kids.createKidsDom({ document });
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
    const mathUnit = display.back_units.find(
      (unit: any) => unit.kind === "sentence" && unit.runs?.some((run: any) => run.math?.display),
    );
    if (!mathUnit) throw new Error("math fixture has no display unit");

    const stage = dom.el("main");
    const actionbar = dom.el("footer");
    let study: any;
    const paint = () => {
      stage.replaceChildren();
      actionbar.replaceChildren();
      study.render();
    };
    study = kids.createKidsStudy({
      api: async (path: string) => path === "/api/choose"
        ? { chosen: 0, correct: 0, passed: true }
        : study.state(),
      post: (body: object) => ({ method: "POST", body }),
      model: {
        create: kids.createKidsStudyModel,
        apply: kids.applyKidsStudyState,
        clear: kids.clearKidsStudyState,
        choose: kids.chooseKidsAnswer,
        reveal: kids.revealKidsAnswer,
        backCount: kids.kidsBackCount,
        choiceMode: kids.kidsChoiceMode,
        multiMode: kids.kidsMultiMode,
        toggle: kids.toggleKidsChoice,
        revealDone: kids.kidsRevealDone,
        screen: kids.kidsStudyScreen,
      },
      rerender: paint,
      openTutor: () => {},
      openPicker: () => {},
      refreshPicker: () => {},
      reportError: () => {},
      ui: {
        actionbar,
        appendChecklist: dom.appendChecklist,
        appendRuns: dom.appendRuns,
        contextLine: dom.contextLine,
        document,
        el: dom.el,
        frontPrompt: dom.frontPrompt,
        mascot: dom.mascot,
        stage,
      },
    });
    const review = (card: any, extra = {}) => ({
      ...choiceState,
      kind: "review",
      phase: "review",
      introducing: false,
      mode: "fill",
      choices: null,
      card,
      ...extra,
    });
    const rendered = (selector: string) => {
      const node = stage.querySelector(selector);
      if (!node) throw new Error(`missing rendered surface ${selector}`);
      return node.cloneNode(true);
    };
    const reveal = () => {
      const button = actionbar.querySelector(".show-btn") as HTMLButtonElement | null;
      if (!button) throw new Error("missing reveal control");
      button.click();
    };

    const audit = dom.el("div", "math-audit");
    audit.style.cssText =
      "width:min(100%,660px);margin:0 auto;padding:20px;display:grid;gap:20px;overflow:hidden";

    study.apply(review(choice));
    const front = dom.el("section", "surface-front");
    front.appendChild(rendered(".rev-prompt"));
    audit.appendChild(front);

    study.apply(review(cloze));
    const context = dom.el("section", "surface-context");
    context.appendChild(rendered(".rev-context"));
    audit.appendChild(context);

    for (const [name, lines, units] of [
      ["bare", ["$$", mathUnit.text, "$$"], [mathUnit]],
      ["fence", ["```math", mathUnit.text, "```"], [mathUnit]],
      ["unclosed", ["$$", mathUnit.text], []],
    ]) {
      study.apply(review({
        ...cloze,
        context: lines,
        context_runs: lines.map((line: string) => [{ text: line }]),
        context_units: units,
      }));
      const block = dom.el("section", `surface-context-${name}`);
      block.appendChild(rendered(".rev-card"));
      audit.appendChild(block);
    }

    study.apply(review(display));
    reveal();
    const answer = dom.el("section", "surface-answer");
    answer.appendChild(rendered(".rev-answer"));
    audit.appendChild(answer);

    study.apply(review(checklist));
    const task = dom.el("section", "surface-checklist");
    task.appendChild(rendered(".rev-prompt"));
    audit.appendChild(task);

    study.apply(choiceState);
    const choices = dom.el("section", "surface-choice");
    choices.appendChild(rendered(".rev-options"));
    audit.appendChild(choices);

    study.apply({ ...choiceState, card: choice });
    (stage.querySelector(".opt-btn") as HTMLButtonElement).click();
    await Promise.resolve();
    const why = dom.el("section", "surface-note");
    why.appendChild(rendered(".rev-why"));
    audit.appendChild(why);

    study.apply(review(explain, {
      keypoints: explain.back,
      keypoint_runs: explain.back_runs,
    }));
    reveal();
    const keypoints = dom.el("section", "surface-keypoint");
    keypoints.appendChild(rendered(".rev-why"));
    audit.appendChild(keypoints);

    study.apply(review(code));
    const literal = dom.el("section", "surface-code");
    literal.appendChild(rendered(".rev-prompt"));
    reveal();
    literal.appendChild(rendered(".rev-answer"));
    audit.appendChild(literal);

    study.apply(review(error));
    const failed = dom.el("section", "surface-error");
    failed.appendChild(rendered(".rev-prompt"));
    audit.appendChild(failed);

    document.getElementById("stage")?.replaceChildren(audit);
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

  await expect(page.locator(".surface-context-bare .math-display")).toBeVisible();
  await expect(page.locator(".surface-context-bare")).not.toContainText("$$");
  await expect(page.locator(".surface-context-fence .math-display")).toBeVisible();
  await expect(page.locator(".surface-context-fence")).not.toContainText("```math");
  await expect(page.locator(".surface-context-unclosed")).toContainText("$$");

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
  await page.screenshot({ path: testInfo.outputPath("kids.png"), fullPage: true });
  await request.post("/api/deselect", { data: {} });
});
