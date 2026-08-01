import assert from "node:assert/strict";
import test from "node:test";

import {
  applyStudyState,
  createModel,
  currentScreen,
  enterPicker,
} from "../../web/alix/review/model.js";

test("applying a new card resets only card scoped client state", () => {
  const walk = { kind: "walk", phase: "reveal", current: 2 };
  const model = {
    ...createModel({ getItem: () => "1" }),
    state: { kind: "review", study_revision: 4 },
    walk,
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
  assert.equal(next.walk, walk);
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
    walk: { kind: "walk", phase: "predict" },
  };

  const next = enterPicker(model);

  assert.equal(next.state, null);
  assert.equal(next.browsing, null);
  assert.equal(next.walk, null);
  assert.equal(currentScreen(next), "picker");
});

test("screen selection uses explicit dto discriminants", () => {
  const base = createModel({ getItem: () => null });
  assert.equal(currentScreen({ ...base, state: { kind: "review", phase: "review" } }), "study");
  assert.equal(currentScreen({ ...base, state: { kind: "review", phase: "done" } }), "summary");
  assert.equal(currentScreen({ ...base, walk: { kind: "walk", phase: "predict" } }), "walk");
  assert.equal(currentScreen({ ...base, walk: { phase: "predict" } }), "picker");
});
