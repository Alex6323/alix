import assert from "node:assert/strict";
import test from "node:test";

import {
  applyKidsStudyState,
  clearKidsStudyState,
  createKidsStudyModel,
  kidsMultiMode,
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
    kidsStudyScreen({ ...model, state: { kind: "walk", phase: "predict" } }),
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
