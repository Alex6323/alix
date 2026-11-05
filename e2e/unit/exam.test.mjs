import assert from "node:assert/strict";
import test from "node:test";

import { createExam } from "../../web/alix/review/exam.js";

function harness(responses) {
  const calls = [];
  const launches = [];
  const applied = [];
  let renders = 0;
  const exam = createExam({
    api: async (path, options) => {
      calls.push({ path, options });
      return responses[path];
    },
    post: (body) => ({ method: "POST", body }),
    rememberLaunch: (deck) => launches.push(deck),
    rerender: () => renders++,
    applyStudy: (state) => applied.push(state),
    updateBusy: () => {},
    workingText: (seconds) => `${seconds}s`,
    timers: {
      setInterval: () => {
        throw new Error("unexpected exam poll");
      },
      clearInterval: () => {},
    },
    ui: { alert: () => {} },
  });
  return { applied, calls, exam, launches, renders: () => renders };
}

test("exam owns its open and close transitions", async () => {
  const generating = { phase: "generating", deck: "facts.md", thinking: false };
  const picker = { kind: "review", phase: "select" };
  const run = harness({
    "/api/exam/start": generating,
    "/api/exam/close": picker,
  });

  await run.exam.start("facts.md");

  assert.equal(run.exam.isOpen(), true);
  assert.equal(run.exam.data(), generating);
  assert.deepEqual(run.launches, ["facts.md"]);
  assert.equal(run.renders(), 1);
  assert.deepEqual(run.calls[0], {
    path: "/api/exam/start",
    options: { method: "POST", body: { deck: "facts.md" } },
  });

  await run.exam.close();

  assert.equal(run.exam.isOpen(), false);
  assert.deepEqual(run.applied, [picker]);
  assert.equal(run.calls[1].path, "/api/exam/close");
});
