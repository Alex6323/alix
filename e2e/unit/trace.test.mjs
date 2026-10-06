import assert from "node:assert/strict";
import test from "node:test";

import { createTraceSession } from "../../web/alix/review/trace.js";
import { hit, hitOutsideField } from "../../web/alix/review/dom.js";

test("trace owns prediction grade and leave transitions", async () => {
  const calls = [];
  const applied = [];
  let renders = 0;
  const reveal = { kind: "trace", phase: "reveal", prediction: "next" };
  const next = { kind: "trace", phase: "predict", current: 2 };
  const picker = { kind: "review", phase: "select" };
  const responses = {
    "/api/trace/predict": reveal,
    "/api/trace/grade": next,
    "/api/trace/leave": picker,
  };
  const trace = createTraceSession({
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

  trace.open({ kind: "trace", phase: "predict", current: 1 });
  await trace.predict("next");

  assert.equal(trace.isOpen(), true);
  assert.equal(trace.data(), reveal);
  assert.deepEqual(calls[0], {
    path: "/api/trace/predict",
    options: { method: "POST", body: { text: "next" } },
  });

  await trace.grade("n");

  assert.equal(trace.data(), next);
  assert.deepEqual(calls[1].options.body, { delta: "n" });

  await trace.leave();

  assert.equal(trace.isOpen(), false);
  assert.deepEqual(applied, [picker]);
  assert.equal(renders, 3);
});

test("a rebound quit leaves a finished trace on its key and Escape no longer does", () => {
  const calls = [];
  const trace = createTraceSession({
    api: async (path) => {
      calls.push(path);
      return { kind: "review", phase: "select" };
    },
    fetchApi: async () => ({ ok: true }),
    post: (body) => ({ method: "POST", body }),
    rerender: () => {},
    applyStudy: () => {},
    sessionStorage: { getItem: () => null },
    examStart: () => {},
    tutor: { isOpen: () => false },
    ui: { hit, hitOutsideField, keys: () => ({ quit: [{ k: "q", ctrl: false }] }) },
  });
  trace.open({ kind: "trace", phase: "done" });
  const press = (key, target = { tagName: "BODY" }) =>
    trace.handleKey({ key, ctrlKey: false, target, preventDefault() {} });

  press("Escape");
  assert.deepEqual(calls, [], "Escape is not the quit key any more");

  press("q", { tagName: "TEXTAREA" });
  assert.deepEqual(calls, [], "q typed into the prediction stays text");

  press("q");
  assert.deepEqual(calls, ["/api/trace/leave"], "q leaves");
});
