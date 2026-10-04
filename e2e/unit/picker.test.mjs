import assert from "node:assert/strict";
import test from "node:test";

import { createPicker } from "../../web/alix/review/picker.js";

function harness(responses) {
  const calls = [];
  const applied = [];
  const traces = [];
  const stored = new Map();
  const picker = createPicker({
    api: async (path, options) => {
      calls.push({ path, options });
      return responses.shift();
    },
    post: (body) => ({ method: "POST", body }),
    sessionStorage: {
      setItem: (key, value) => stored.set(key, value),
      getItem: (key) => stored.get(key) || null,
      removeItem: (key) => stored.delete(key),
    },
    currentState: () => ({ phase: "select" }),
    isBrowsing: () => false,
    examIsOpen: () => false,
    augmentIsOpen: () => false,
    traceIsOpen: () => false,
    tutorIsOpen: () => false,
    applyStudy: (state) => applied.push(state),
    openTraceSession: (trace) => traces.push(trace),
    openBrowse: () => {},
    startExam: () => {},
    openAugment: () => {},
    notice: () => {},
    timers: { setTimeout: () => {} },
    ui: {
      navRefresh: { addEventListener: () => {} },
      window: { addEventListener: () => {} },
    },
  });
  return { applied, calls, picker, stored, traces };
}

test("picker owns review and trace launch transitions", async () => {
  const review = { kind: "review", phase: "review", card: { id: "card-1" } };
  const trace = { kind: "trace", phase: "predict", deck: "trace.md" };
  const run = harness([review, trace]);

  await run.picker.select("facts.md", "Foundations", "Basics", "recall", false);

  assert.deepEqual(run.applied, [review]);
  assert.deepEqual(run.traces, []);
  assert.equal(run.stored.get("alix.lastDeck"), "facts.md");
  assert.deepEqual(run.calls[0], {
    path: "/api/select",
    options: {
      method: "POST",
      body: {
        deck: "facts.md",
        topology: "Foundations",
        region: "Basics",
        depth: "recall",
        cram: false,
      },
    },
  });

  await run.picker.select("trace.md", null, null, null, false);

  assert.deepEqual(run.traces, [trace]);
  assert.equal(run.stored.get("alix.lastDeck"), "trace.md");
});
