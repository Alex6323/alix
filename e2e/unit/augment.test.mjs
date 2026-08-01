import assert from "node:assert/strict";
import test from "node:test";

import { createAugment } from "../../assets/web/review/augment.js";

function harness(responses) {
  const calls = [];
  const launches = [];
  const applied = [];
  let renders = 0;
  const augment = createAugment({
    api: async (path, options) => {
      calls.push({ path, options });
      return responses[path];
    },
    post: (body) => ({ method: "POST", body }),
    rememberLaunch: (deck) => launches.push(deck),
    rerender: () => renders++,
    applyStudy: (state) => applied.push(state),
    workingText: (seconds) => `${seconds}s`,
    backendName: () => "Claude",
    timers: {
      setInterval: () => {
        throw new Error("unexpected augment poll");
      },
      clearInterval: () => {},
    },
    ui: {},
  });
  return { applied, augment, calls, launches, renders: () => renders };
}

test("augment owns its accepted open and close transitions", async () => {
  const opened = { deck: "facts.md", rows: [], busy: false };
  const picker = { kind: "review", phase: "select" };
  const run = harness({
    "/api/augment/open": opened,
    "/api/augment/close": picker,
  });

  await run.augment.open("facts.md");

  assert.equal(run.augment.isOpen(), true);
  assert.equal(run.augment.data(), opened);
  assert.deepEqual(run.launches, ["facts.md"]);
  assert.equal(run.renders(), 1);
  assert.deepEqual(run.calls[0], {
    path: "/api/augment/open",
    options: { method: "POST", body: { deck: "facts.md" } },
  });

  await run.augment.close();

  assert.equal(run.augment.isOpen(), false);
  assert.deepEqual(run.applied, [picker]);
  assert.equal(run.calls[1].path, "/api/augment/close");
});
