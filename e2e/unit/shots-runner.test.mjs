import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import runner from "../shots/runner.cjs";

const { runRequested, summarize, unknownRequests, exitCodeFor } = runner;

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
    unknown: stores.unknown || [],
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

const writingSteps = (writes) =>
  Object.entries(writes).map(([n, names]) => [
    Number(n),
    async () => {
      for (const name of names) receipt.add(name);
      return true;
    },
  ]);

let receipt;
const failedAfterWriting = async (writes) => {
  receipt = new Set();
  const results = await runRequested(writingSteps(writes), all, null, receipt);
  return summarize(results).failed;
};

test("a shot that returns true without writing anything is a failure", async () => {
  assert.deepEqual(await failedAfterWriting({ 1: [] }), [1]);
});

test("a shot that writes its own numbered file passes", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-1-verify.webp"], 6: ["shot-6-trace.webp"] }),
    [],
  );
});

test("two shots swapping filenames both fail rather than crediting each other", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-2-wrong.webp"], 2: ["shot-1-wrong.webp"] }),
    [1, 2],
  );
});

test("a shot writing a second file fails until the protocol is changed on purpose", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-1-a.webp", "shot-1-b.webp"] }),
    [1],
  );
});

test("a later shot rewriting an earlier shot's file fails on an empty difference", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-1-a.webp"], 2: ["shot-1-a.webp"] }),
    [2],
  );
});

test("an earlier shot's file never satisfies a later one", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-1-verify.webp"], 2: [] }),
    [2],
  );
});

test("without a receipt the attribution is skipped, not silently failed", async () => {
  const results = await runRequested(steps({ 1: true }), all, null);
  assert.deepEqual(summarize(results).failed, []);
});

test("--only naming a shot that does not exist is unknown", () => {
  assert.deepEqual(unknownRequests([[1, null], [2, null]], new Set([1, 11])), [11]);
});

test("--only naming only known shots is empty", () => {
  assert.deepEqual(unknownRequests([[1, null], [6, null]], new Set([6])), []);
});

test("no --only at all requests everything and is never unknown", () => {
  assert.deepEqual(unknownRequests([[1, null]], null), []);
});

test("an unknown --only number alone makes the run exit nonzero", async () => {
  assert.equal(await codeAfter({}, { unknown: [11] }), 1);
});

test("both capture exits are set, and both through the shared decision", async () => {
  const source = await readFile(
    new URL("../shots/capture.cjs", import.meta.url),
    "utf8",
  );
  assert.match(source, /process\.exitCode = exitCodeFor\(\{ unknown \}\);/);
  assert.match(
    source,
    /process\.exitCode = exitCodeFor\(\{ failed, demoChanged, kidsChanged \}\);/,
  );
  assert.equal(source.match(/process\.exitCode/g).length, 2);
  assert.equal(source.match(/process\.exitCode = exitCodeFor\(/g).length, 2);
});

test("the capture records its receipt only after a successful rename", async () => {
  const source = await readFile(
    new URL("../shots/capture.cjs", import.meta.url),
    "utf8",
  );
  assert.match(source, /fs\.renameSync\(webp, out\);\n\s*capturedThisRun\.add\(filename\);/);
  assert.equal(source.match(/capturedThisRun\.add\(/g).length, 1);
  assert.match(source, /runRequested\(steps, wants, page, capturedThisRun\)/);
});

test("the capture rejects an unknown --only above the encoder probe", async () => {
  const source = await readFile(
    new URL("../shots/capture.cjs", import.meta.url),
    "utf8",
  );
  const validate = source.indexOf("unknownRequests(STEPS, ONLY)");
  const encoder = source.indexOf("requireWebpEncoder();", source.indexOf("async function main()"));
  assert.ok(validate > 0, "the capture validates --only");
  assert.ok(validate < encoder, "validation must precede the encoder probe");
  assert.equal(source.match(/unknownRequests\(/g).length, 1);
});

test("every capture step is registered once, under the number its producer writes", async () => {
  const source = await readFile(
    new URL("../shots/capture.cjs", import.meta.url),
    "utf8",
  );
  const table = source.match(/const STEPS = \[([\s\S]*?)\];/);
  assert.ok(table, "capture.cjs declares a STEPS table");
  const rows = [...table[1].matchAll(/\[(\d+), (\w+)\]/g)].map(([, n, fn]) => [
    Number(n),
    fn,
  ]);
  assert.ok(rows.length > 0, "the STEPS table has rows");
  const unparsed = table[1]
    .replace(/\[(\d+), (\w+)\]/g, "")
    .replace(/[\s,]/g, "");
  assert.equal(unparsed, "", `every STEPS row must parse, left over: ${unparsed}`);
  const ids = rows.map(([n]) => n);
  for (const n of ids) {
    assert.ok(n >= 1, `STEPS ids must be positive, got ${n}`);
  }
  assert.equal(
    new Set(ids).size,
    ids.length,
    `STEPS ids must be unique, got ${JSON.stringify(ids)}`,
  );
  for (const [n, fn] of rows) {
    assert.equal(fn, `shot${n}`, `step ${n} must be produced by shot${n}, not ${fn}`);
  }
});
