import assert from "node:assert/strict";
import test from "node:test";

import {
  applyKidsStudyState,
  clearKidsStudyState,
  createKidsStudyModel,
  kidsFilledSteps,
  kidsMultiMode,
  kidsRevealDone,
  kidsStepCount,
  revealKidsAnswer,
  kidsStudyScreen,
  toggleKidsChoice,
} from "../../web/alix-kids/kids/model.js";

test("applying kids study state resets only per card state", () => {
  const model = {
    ...createKidsStudyModel(),
    state: { kind: "review", phase: "review", study_revision: 4 },
    revealed: 3,
    chosen: { correct: 1 },
    ownerSentinel: { open: true },
  };
  const dto = { kind: "review", phase: "review", study_revision: 5 };

  const next = applyKidsStudyState(model, dto);

  assert.notEqual(next, model);
  assert.equal(next.state, dto);
  assert.equal(next.revealed, 0);
  assert.equal(next.chosen, null);
  assert.equal(next.ownerSentinel, model.ownerSentinel);
});

test("kids screen selection uses dto kind and phase", () => {
  const model = createKidsStudyModel();
  assert.equal(kidsStudyScreen(model), "done");
  assert.equal(
    kidsStudyScreen({ ...model, state: { kind: "review", phase: "review" } }),
    "review",
  );
  assert.equal(
    kidsStudyScreen({ ...model, state: { kind: "review", phase: "done" } }),
    "done",
  );
  assert.equal(
    kidsStudyScreen({ ...model, state: { kind: "trace", phase: "predict" } }),
    "review",
  );
});

test("clearing kids study state does not blanket reset view state", () => {
  const sentinel = { open: true };
  const model = {
    ...createKidsStudyModel(),
    state: { kind: "review", phase: "review" },
    revealed: 2,
    chosen: { correct: 0 },
    ownerSentinel: sentinel,
  };

  const next = clearKidsStudyState(model);

  assert.equal(next.state, null);
  assert.equal(next.revealed, 2);
  assert.deepEqual(next.chosen, { correct: 0 });
  assert.equal(next.ownerSentinel, sentinel);
});

test("toggling a kids choice adds, sorts, and removes indices", () => {
  const model = createKidsStudyModel();
  const one = toggleKidsChoice(model, 2);
  assert.deepEqual(one.selected, [2]);
  const two = toggleKidsChoice(one, 0);
  assert.deepEqual(two.selected, [0, 2]);
  const back = toggleKidsChoice(two, 2);
  assert.deepEqual(back.selected, [0]);
});

test("applying kids study state clears the multi selection", () => {
  const model = { ...createKidsStudyModel(), selected: [1, 3] };
  const next = applyKidsStudyState(model, { kind: "review" });
  assert.deepEqual(next.selected, []);
});

test("kids multi mode requires choice mode and the flag", () => {
  const state = { mode: "choice", choices: ["a", "b"], choices_multiple: true };
  const base = { ...createKidsStudyModel(), state };
  assert.equal(kidsMultiMode(base), true);
  assert.equal(kidsMultiMode({ ...base, state: { ...state, choices_multiple: false } }), false);
  assert.equal(kidsMultiMode({ ...base, state: { ...state, mode: "line" } }), false);
});

const kidsCloze = {
  front: "q\nthe ⍰ and ⍰",
  front_steps: [{ front: "q\nthe a and ⍰" }, { front: "q\nthe a and b" }],
  answer_steps: [{ kind: "line", back_from: 0, back_to: 1 }],
};

test("a kids cloze card steps through its front steps, one per own blank", () => {
  const review = (mode) => ({ ...createKidsStudyModel(), state: { kind: "review", mode, card: kidsCloze } });
  assert.equal(kidsStepCount(review("line")), 2, "front steps count, not answer steps");

  let line = review("line");
  assert.equal(kidsFilledSteps(line), 0, "line: nothing filled before the first tap");
  line = revealKidsAnswer(line);
  assert.equal(kidsFilledSteps(line), 1, "line: the first tap fills one blank");
  assert.equal(kidsRevealDone(line), false, "line: one blank still open");
  line = revealKidsAnswer(line);
  assert.equal(kidsFilledSteps(line), 2, "line: the second tap fills the last blank");
  assert.equal(kidsRevealDone(line), true, "line: done once every blank is filled");

  const flip = revealKidsAnswer(review("flip"));
  assert.equal(kidsFilledSteps(flip), 2, "flip: one tap fills every blank");

  const choice = { ...review("choice"), state: { kind: "review", mode: "choice", choices: ["a", "c"], card: kidsCloze } };
  assert.equal(kidsFilledSteps(choice), 0, "choice: nothing filled before the pick");
  assert.equal(kidsFilledSteps({ ...choice, chosen: { correct: 0 } }), 2, "choice: the pick fills every blank");

  const plain = { ...createKidsStudyModel(), revealed: 1, state: { kind: "review", mode: "flip", card: { front_steps: [], answer_steps: [{ kind: "line" }] } } };
  assert.equal(kidsFilledSteps(plain), 0, "a card without front steps never fills its question");
});
