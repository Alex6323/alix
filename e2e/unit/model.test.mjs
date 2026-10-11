import assert from "node:assert/strict";
import test from "node:test";

import {
  applyStudyState,
  createModel,
  currentScreen,
  enterPicker,
  filledSteps,
  questionAt,
  revealStepCount,
} from "../../web/alix/review/model.js";

test("applying a new card resets only card scoped client state", () => {
  const trace = { kind: "trace", phase: "reveal", current: 2 };
  const model = {
    ...createModel({ getItem: () => "1" }),
    state: { kind: "review", study_revision: 4 },
    trace,
    revealed: 3,
    citationView: true,
    feedback: { passed: false },
    marks: [true, false],
    drawStrokes: [{ tool: "pen", points: [] }],
    drawerSelection: { deck: "facts.md", topology: "topic", region: "one" },
    keys: { reveal: [{ k: " ", ctrl: false }] },
  };
  const dto = { kind: "review", phase: "review", study_revision: 5 };

  const next = applyStudyState(model, dto);

  assert.notEqual(next, model);
  assert.equal(next.state, dto);
  assert.equal(next.trace, trace);
  assert.equal(next.revealed, 0);
  assert.equal(next.citationView, false);
  assert.equal(next.feedback, null);
  assert.deepEqual(next.marks, []);
  assert.deepEqual(next.drawStrokes, []);
  assert.equal(next.drawerSelection, model.drawerSelection);
  assert.equal(next.keys, model.keys);
  assert.equal(next.drawToggle, true);
});

test("picker refresh preserves no stale study state", () => {
  const model = {
    ...createModel({ getItem: () => null }),
    state: { kind: "review", phase: "review" },
    browsing: { cards: [{}], index: 0 },
    trace: { kind: "trace", phase: "predict" },
  };

  const next = enterPicker(model);

  assert.equal(next.state, null);
  assert.equal(next.browsing, null);
  assert.equal(next.trace, null);
  assert.equal(currentScreen(next), "picker");
});

test("screen selection uses explicit dto discriminants", () => {
  const base = createModel({ getItem: () => null });
  assert.equal(currentScreen({ ...base, state: { kind: "review", phase: "review" } }), "study");
  assert.equal(currentScreen({ ...base, state: { kind: "review", phase: "done" } }), "summary");
  assert.equal(currentScreen({ ...base, trace: { kind: "trace", phase: "predict" } }), "trace");
  assert.equal(currentScreen({ ...base, trace: { phase: "predict" } }), "picker");
  for (const [phase, screen] of [["front", "walk"], ["answer", "walk"], ["done", "walk-done"]]) {
    assert.equal(
      currentScreen({ ...base, state: { kind: "walk", phase } }),
      screen,
      `walk phase ${phase} maps to ${screen}`,
    );
  }
});

const clozeCard = {
  front: "q\nthe ⍰ and ⍰",
  front_runs: [{ text: "q\nthe ⍰ and ⍰" }],
  front_steps: [
    { front: "q\nthe a and ⍰", front_runs: [{ text: "q\nthe " }, { text: "a", answer: true }, { text: " and ⍰" }] },
    { front: "q\nthe a and b", front_runs: [{ text: "q\nthe " }, { text: "a", answer: true }, { text: " and " }, { text: "b", answer: true }] },
  ],
  answer_steps: [{ kind: "line", back_from: 0, back_to: 1 }],
};
const plainCard = {
  front: "q",
  front_runs: [{ text: "q" }],
  front_steps: [],
  answer_steps: [
    { kind: "line", back_from: 0, back_to: 1 },
    { kind: "line", back_from: 1, back_to: 2 },
    { kind: "line", back_from: 2, back_to: 3 },
  ],
};

test("a cloze card reveals in front steps and every other card in answer steps", () => {
  assert.equal(revealStepCount(clozeCard), 2, "cloze: one step per own blank");
  assert.equal(revealStepCount(plainCard), 3, "plain: one step per answer step");
  assert.equal(revealStepCount(null), 0, "no card: nothing to reveal");
});

test("the filled step count follows the reveal mode on a cloze card", () => {
  const rows = [
    ["flip, unrevealed", { mode: "flip", revealed: 0, answered: false }, 0],
    ["flip, revealed", { mode: "flip", revealed: 2, answered: true }, 2],
    ["line, first press", { mode: "line", revealed: 1, answered: false }, 1],
    ["line, second press", { mode: "line", revealed: 2, answered: true }, 2],
    ["line, beyond the steps", { mode: "line", revealed: 5, answered: true }, 2],
    ["typing, before the check", { mode: "typing", revealed: 0, answered: false }, 0],
    ["typing, after the check", { mode: "typing", revealed: 0, answered: true }, 2],
    ["choice, after the pick", { mode: "choice", revealed: 0, answered: true }, 2],
    ["concealed after reveal", { mode: "flip", revealed: 2, answered: true, concealed: true }, 0],
  ];
  for (const [label, view, expected] of rows) {
    assert.equal(filledSteps({ card: clozeCard, ...view }), expected, label);
  }
  assert.equal(
    filledSteps({ card: plainCard, mode: "flip", revealed: 3, answered: true }),
    0,
    "a card without front steps never fills its question",
  );
});

test("the question shown is the masked front until a step fills it", () => {
  assert.equal(questionAt(clozeCard, 0), clozeCard, "nothing filled: the masked question");
  assert.equal(questionAt(clozeCard, 1), clozeCard.front_steps[0], "one filled: the first step");
  assert.equal(questionAt(clozeCard, 2), clozeCard.front_steps[1], "all filled: the last step");
  assert.equal(questionAt(plainCard, 0), plainCard, "a plain card is its own question");
});
