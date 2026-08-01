import assert from "node:assert/strict";
import test from "node:test";

import {
  applyKidsStudyState,
  clearKidsStudyState,
  chooseKidsAnswer,
  createKidsStudyModel,
  kidsBackCount,
  kidsChoiceMode,
  kidsRevealDone,
  kidsStudyScreen,
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
    acquire: true,
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

test("kids acquire reveal reports the encounter once without publishing returned state", async () => {
  const original = review();
  const run = harness(original);
  run.study.render();

  const reveal = find(run.actionbar, (node) => node.textContent === "Show me 👀");
  assert.ok(reveal);
  reveal.click();
  await Promise.resolve();
  reveal.click();
  await Promise.resolve();

  assert.deepEqual(run.calls.map((call) => call.path), ["/api/reveal"]);
  assert.equal(run.study.state(), original);
  assert.equal(run.study.revision(), 7);
});

test("kids acquire choice reports the encounter before choosing", async () => {
  const original = review({ mode: "choice", choices: ["Answer", "Other"] });
  const run = harness(original);
  run.study.render();

  const option = find(run.stage, (node) => node.hasClass?.("opt-btn"));
  assert.ok(option);
  option.click();
  await Promise.resolve();

  assert.deepEqual(run.calls.map((call) => call.path), ["/api/reveal", "/api/choose"]);
  assert.equal(run.study.state(), original);
  assert.equal(run.study.revision(), 7);
});
