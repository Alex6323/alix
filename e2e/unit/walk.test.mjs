import assert from "node:assert/strict";
import test from "node:test";

import { createWalk } from "../../assets/web/review/walk.js";

test("walk owns prediction grade and leave transitions", async () => {
  const calls = [];
  const applied = [];
  let renders = 0;
  const reveal = { kind: "walk", phase: "reveal", prediction: "next" };
  const next = { kind: "walk", phase: "predict", current: 2 };
  const picker = { kind: "review", phase: "select" };
  const responses = {
    "/api/walk/predict": reveal,
    "/api/walk/grade": next,
    "/api/walk/leave": picker,
  };
  const walk = createWalk({
    api: async (path, options) => {
      calls.push({ path, options });
      return responses[path];
    },
    fetchApi: async () => ({ ok: true }),
    post: (body) => ({ method: "POST", body }),
    rerender: () => renders++,
    applyStudy: (state) => applied.push(state),
    sessionStorage: { getItem: () => null },
    examStart: () => {},
    tutor: { isOpen: () => false },
    ui: {},
  });

  walk.open({ kind: "walk", phase: "predict", current: 1 });
  await walk.predict("next");

  assert.equal(walk.isOpen(), true);
  assert.equal(walk.data(), reveal);
  assert.deepEqual(calls[0], {
    path: "/api/walk/predict",
    options: { method: "POST", body: { text: "next" } },
  });

  await walk.grade("n");

  assert.equal(walk.data(), next);
  assert.deepEqual(calls[1].options.body, { delta: "n" });

  await walk.leave();

  assert.equal(walk.isOpen(), false);
  assert.deepEqual(applied, [picker]);
  assert.equal(renders, 3);
});
