import assert from "node:assert/strict";
import test from "node:test";

import {
  applyStudyState,
  createModel,
  currentScreen,
  enterPicker,
} from "../../assets/web/review/model.js";
import { createStudy } from "../../assets/web/review/study.js";

function harness() {
  const walks = [];
  let renders = 0;
  const study = createStudy({
    api: async () => ({}),
    post: (body) => ({ method: "POST", body }),
    storage: { getItem: () => null, setItem: () => {} },
    model: { create: createModel, applyStudyState, currentScreen, enterPicker },
    rerender: () => renders++,
    walkData: () => null,
    replaceWalk: (walk) => walks.push(walk),
    openTutor: () => {},
    startExam: () => {},
    closeMenu: () => {},
    timers: {},
    ui: {},
  });
  return { renders: () => renders, study, walks };
}

test("study owns accepted state publication and screen selection", () => {
  const run = harness();
  const review = {
    kind: "review",
    phase: "review",
    label: "Facts",
    remaining: 3,
    card: { id: "card-1" },
  };

  run.study.apply(review);

  assert.equal(run.study.state(), review);
  assert.equal(run.study.screen(), "study");
  assert.deepEqual(run.walks, [null]);
  assert.equal(run.renders(), 1);

  const done = { ...review, phase: "done", card: null };
  run.study.apply(done);

  assert.equal(run.study.state(), done);
  assert.equal(run.study.screen(), "summary");
  assert.equal(run.renders(), 2);
});
