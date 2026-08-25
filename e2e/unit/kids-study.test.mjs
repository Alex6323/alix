import assert from "node:assert/strict";
import test from "node:test";

// The overlay observes element resizes; node has no layout, so a no-op
// observer stands in and mask geometry is simply never synced here.
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

import {
  applyKidsStudyState,
  clearKidsStudyState,
  chooseKidsAnswer,
  createKidsStudyModel,
  kidsBackCount,
  kidsChoiceMode,
  kidsMultiMode,
  kidsRevealDone,
  kidsStudyScreen,
  toggleKidsChoice,
  revealKidsAnswer,
} from "../../web/alix-kids/kids/model.js";
import { createKidsStudy } from "../../web/alix-kids/kids/study.js";

function element(tag, className, text) {
  const listeners = new Map();
  const classes = new Set((className || "").split(/\s+/).filter(Boolean));
  return {
    tag,
    className: className || "",
    textContent: text ?? "",
    children: [],
    dataset: {},
    style: {},
    classList: { add: (...names) => names.forEach((name) => classes.add(name)) },
    appendChild(child) { this.children.push(child); return child; },
    addEventListener(type, listener) { listeners.set(type, listener); },
    click() { return listeners.get("click")?.({ preventDefault() {} }); },
    hasClass(name) { return classes.has(name); },
    remove() {},
    setAttribute() {},
  };
}

function find(root, predicate) {
  if (predicate(root)) return root;
  for (const child of root.children || []) {
    const match = find(child, predicate);
    if (match) return match;
  }
  return null;
}

function review(overrides = {}) {
  return {
    kind: "review",
    phase: "review",
    study_revision: 7,
    introducing: true,
    mode: "fill",
    card: {
      id: "card-1",
      front: "Question",
      back: ["Answer"],
      context: [],
      images: [],
      images_back: [],
      note: [],
    },
    ...overrides,
  };
}

function harness(initial = review()) {
  const calls = [];
  const stage = element("main");
  const actionbar = element("footer");
  const api = async (path, options) => {
    calls.push({ path, options });
    if (path === "/api/choose") return { chosen: 0, correct: 0, passed: true };
    return { ...initial, study_revision: 99 };
  };
  const study = createKidsStudy({
    api,
    post: (body) => ({ method: "POST", body }),
    model: {
      create: createKidsStudyModel,
      apply: applyKidsStudyState,
      clear: clearKidsStudyState,
      choose: chooseKidsAnswer,
      reveal: revealKidsAnswer,
      backCount: kidsBackCount,
      choiceMode: kidsChoiceMode,
      multiMode: kidsMultiMode,
      toggle: toggleKidsChoice,
      revealDone: kidsRevealDone,
      screen: kidsStudyScreen,
    },
    rerender: () => {},
    openTutor: () => {},
    openPicker: () => {},
    refreshPicker: () => {},
    reportError: () => {},
    ui: {
      actionbar,
      appendChecklist: () => {},
      appendRuns: () => {},
      contextLine: () => element("div"),
      document: {
        body: element("body"),
        createElement: (tag) => element(tag),
        getElementById: () => null,
      },
      el: element,
      frontPrompt: () => element("div", "rev-prompt"),
      mascot: () => element("div", "mascot"),
      stage,
    },
  });
  study.apply(initial);
  return { actionbar, calls, stage, study };
}

test("kids study owns accepted state publication and revision", () => {
  const first = review();
  const run = harness(first);

  assert.equal(run.study.state(), first);
  assert.equal(run.study.revision(), 7);
  assert.equal(run.study.isOpen(), true);

  const next = review({ study_revision: 8 });
  run.study.apply(next);
  assert.equal(run.study.state(), next);
  assert.equal(run.study.revision(), 8);
});

test("kids introduction reveal is purely local and reports nothing", async () => {
  // ADR 0035: revealing persists nothing, so the endpoint is gone and the
  // client must not call it.
  const original = review();
  const run = harness(original);
  run.study.render();

  const reveal = find(run.actionbar, (node) => node.textContent === "Show me 👀");
  assert.ok(reveal);
  reveal.click();
  await Promise.resolve();
  reveal.click();
  await Promise.resolve();

  assert.deepEqual(run.calls.map((call) => call.path), []);
  assert.equal(run.study.state(), original);
  assert.equal(run.study.revision(), 7);
});

test("kids introduction choice sends only the choose", async () => {
  const original = review({ mode: "choice", choices: ["Answer", "Other"] });
  const run = harness(original);
  run.study.render();

  const option = find(run.stage, (node) => node.hasClass?.("opt-btn"));
  assert.ok(option);
  option.click();
  await Promise.resolve();

  assert.deepEqual(run.calls.map((call) => call.path), ["/api/choose"]);
  assert.equal(run.study.state(), original);
  assert.equal(run.study.revision(), 7);
});

test("kids done summary does not call a partial answer right", () => {
  // `passed` includes partials; the done screen must not praise the same
  // card as exactly right and so close at once (Codex tenth pass, P1).
  const done = review({
    phase: "done",
    finished: true,
    reviews: 1,
    passed: 1,
    failed: 0,
    partial: 1,
    introduced: 0,
  });
  const run = harness(done);
  run.study.render();
  const praised = find(run.stage, (node) =>
    (node.textContent || "").includes("right! 👀"));
  assert.equal(praised, null,
    "Partial is included in passed, but it is not an exactly-right answer");
  const close = find(run.stage, (node) =>
    (node.textContent || "").includes("So close on 1 card."));
  assert.ok(close, "the almost line still reports the work");
});

test("kids masks a context diagram until reveal, then swaps its alt", () => {
  const original = review({
    card: {
      id: "card-1",
      front: "Question",
      back: ["Cache"],
      context: ["```mermaid", "flowchart LR", "  Cache[store] --> B[Cache]", "```"],
      context_runs: [],
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
      images: [],
      images_back: [],
      note: [],
    },
  });
  const run = harness(original);
  run.study.render();

  let image = find(run.stage, (node) => node.tag === "img" && node.className === "diagram");
  assert.equal(image?.alt, "diagram labels: …, …", "pre-reveal alt is masked");
  let masks = [];
  const collect = (node) => {
    const cls = (node.className || "").split(" ")[0];
    if (cls === "rev-img-mask") masks.push(node);
    for (const child of node.children || []) collect(child);
  };
  collect(run.stage);
  assert.equal(masks.length, 2, "asked and sibling masks are drawn pre-reveal");

  const reveal = find(run.actionbar, (node) => node.textContent === "Show me 👀");
  reveal.click();
  // The fake stage accumulates children across renders (innerHTML is inert
  // on it), so drop the first render before reading the second.
  run.stage.children.length = 0;
  run.study.render();

  image = find(run.stage, (node) => node.tag === "img" && node.className === "diagram");
  assert.equal(image?.alt, "diagram labels: …, Cache", "post-reveal alt exposes the asked label");
  masks = [];
  collect(run.stage);
  assert.equal(masks.length, 1, "the asked mask drops, the sibling stays");
});

test("kids renders a context diagram unit through the fence walk", () => {
  const original = review({
    card: {
      id: "card-1",
      front: "Question",
      back: ["Answer"],
      context: ["```mermaid", "flowchart LR", " A-->B", "```", "a sentence"],
      context_runs: [],
      context_units: [{
        kind: "diagram",
        src: "/img/0123456789abcdef",
        width: 188,
        height: 114,
        alt: "flowchart LR\n A-->B",
      }],
      images: [],
      images_back: [],
      note: [],
    },
  });
  const run = harness(original);
  run.study.render();

  const image = find(run.stage, (node) =>
    node.tag === "img" && node.className === "diagram");
  assert.equal(image?.src, "/img/0123456789abcdef", "the context fence renders the diagram");
  assert.equal(image?.alt, "flowchart LR\n A-->B");
  const raw = find(run.stage, (node) =>
    typeof node.textContent === "string" && node.textContent.includes("```"));
  assert.equal(raw, null, "no raw fence marker line survives in context");
});

test("kids renders a diagram unit on answer reveal", () => {
  const original = review({
    card: {
      id: "card-1",
      front: "Question",
      back: ["```mermaid", "flowchart LR", " A-->B", "```"],
      back_runs: [],
      back_units: [{
        kind: "diagram",
        src: "/img/0123456789abcdef",
        width: 188,
        height: 114,
        alt: "flowchart LR\n A-->B",
      }],
      context: [],
      images: [],
      images_back: [],
      note: [],
    },
  });
  const run = harness(original);
  run.study.render();

  const reveal = find(run.actionbar, (node) => node.textContent === "Show me 👀");
  assert.ok(reveal, "the fill card offers reveal");
  reveal.click();
  run.study.render();

  const image = find(run.stage, (node) =>
    node.tag === "img" && node.className === "diagram");
  assert.equal(image?.src, "/img/0123456789abcdef");
  assert.equal(image?.alt, "flowchart LR\n A-->B");
});

test("kids unwraps every note body in authored order", () => {
  const original = review({
    card: {
      id: "card-1",
      front: "Question",
      back: ["Answer"],
      context: [],
      images: [],
      images_back: [],
      note: [
        { badge: "warning", units: [{ kind: "sentence", text: "First." }] },
        { units: [{ kind: "sentence", text: "Second." }] },
      ],
    },
  });
  const run = harness(original);
  run.study.render();
  const reveal = find(run.actionbar, (node) => node.textContent === "Show me 👀");
  reveal.click();
  run.stage.children.length = 0;
  run.study.render();

  const why = find(run.stage, (node) => node.className === "rev-why-text");
  assert.deepEqual(
    why.children.map((node) => node.textContent),
    ["First.", "Second."],
  );
});
