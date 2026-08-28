import assert from "node:assert/strict";
import test from "node:test";

import capture from "../shots/capture.cjs";

const { runRequested, summarize } = capture;

const steps = (outcomes) =>
  Object.entries(outcomes).map(([n, outcome]) => [
    Number(n),
    async () => {
      if (outcome instanceof Error) throw outcome;
      return outcome;
    },
  ]);

const all = () => true;

test("a requested shot that returns false is a failure, not a skip", async () => {
  const results = await runRequested(steps({ 1: true, 2: false, 3: true }), all, null);
  const { lines, failed } = summarize(results);
  assert.deepEqual(failed, [2]);
  assert.deepEqual(lines, ["shot 1: captured", "shot 2: FAILED", "shot 3: captured"]);
});

test("a requested shot that throws is a failure", async () => {
  const results = await runRequested(steps({ 1: new Error("no chip"), 2: true }), all, null);
  assert.deepEqual(summarize(results).failed, [1]);
});

test("every requested shot capturing leaves nothing failed", async () => {
  const results = await runRequested(steps({ 1: true, 2: true }), all, null);
  assert.deepEqual(summarize(results).failed, []);
});

test("an unrequested shot never enters the result map", async () => {
  const results = await runRequested(
    steps({ 1: false, 6: true, 10: false }),
    (n) => n === 6,
    null,
  );
  assert.deepEqual(Object.keys(results), ["6"]);
  assert.deepEqual(summarize(results).failed, []);
});
