import assert from "node:assert/strict";
import test from "node:test";

import {
  applyKidsStudyState,
  clearKidsStudyState,
  createKidsStudyModel,
  kidsStudyScreen,
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
