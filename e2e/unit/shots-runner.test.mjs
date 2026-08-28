import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import runner from "../shots/runner.cjs";

const { runRequested, summarize, exitCodeFor } = runner;

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

const codeAfter = async (outcomes, stores = {}) => {
  const results = await runRequested(steps(outcomes), all, null);
  const { failed } = summarize(results);
  return exitCodeFor({
    failed,
    demoChanged: stores.demoChanged || [],
    kidsChanged: stores.kidsChanged || [],
  });
};

test("a clean run of every requested shot exits zero", async () => {
  assert.equal(await codeAfter({ 1: true, 2: true }), 0);
});

test("a requested shot returning false makes the run exit nonzero", async () => {
  assert.equal(await codeAfter({ 1: true, 2: false }), 1);
});

test("a requested shot throwing makes the run exit nonzero", async () => {
  assert.equal(await codeAfter({ 1: new Error("no chip") }), 1);
});

test("a real demo store mutation alone makes the run exit nonzero", async () => {
  assert.equal(await codeAfter({ 1: true }, { demoChanged: ["progress/x.json"] }), 1);
});

test("a real kids store mutation alone makes the run exit nonzero", async () => {
  assert.equal(await codeAfter({ 1: true }, { kidsChanged: ["progress/y.json"] }), 1);
});

// The decision above is only worth anything if the capture actually uses it.
// `main` needs a browser, so the wiring is guarded at the source, the same way
// the tutor's renderer wiring is.
test("the capture derives its exit code from the shared decision", async () => {
  const source = await readFile(
    new URL("../shots/capture.cjs", import.meta.url),
    "utf8",
  );
  assert.match(source, /process\.exitCode = exitCodeFor\(/);
  assert.equal(source.match(/process\.exitCode/g).length, 1);
});
