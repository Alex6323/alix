import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import runner from "../shots/runner.cjs";
import capture from "../shots/capture.cjs";

const { registryProblems, runRequested, summarize, unknownRequests, exitCodeFor } =
  runner;

const named = (n, fn) =>
  Object.defineProperty(fn, "name", { value: `shot${n}` });

const steps = (outcomes) =>
  Object.entries(outcomes).map(([n, outcome]) => [
    Number(n),
    `shot-${n}-x.webp`,
    named(n, async () => {
      if (outcome instanceof Error) throw outcome;
      return outcome;
    }),
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
    `shot-${n}-x.webp`,
    named(n, async () => {
      for (const name of names) receipt.add(name);
      return true;
    }),
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

test("a shot that writes its declared file passes", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-1-x.webp"], 6: ["shot-6-x.webp"] }),
    [],
  );
});

test("a correctly numbered file that is not the declared one fails", async () => {
  assert.deepEqual(await failedAfterWriting({ 1: ["shot-1-other.webp"] }), [1]);
});

test("two shots swapping filenames both fail rather than crediting each other", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-2-x.webp"], 2: ["shot-1-x.webp"] }),
    [1, 2],
  );
});

test("a shot writing a second file fails until the protocol is changed on purpose", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-1-x.webp", "shot-1-b.webp"] }),
    [1],
  );
});

test("a later shot rewriting an earlier shot's file fails on an empty difference", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-1-x.webp"], 2: ["shot-1-x.webp"] }),
    [2],
  );
});

test("an earlier shot's file never satisfies a later one", async () => {
  assert.deepEqual(
    await failedAfterWriting({ 1: ["shot-1-x.webp"], 2: [] }),
    [2],
  );
});

test("without a receipt the attribution is skipped, not silently failed", async () => {
  const results = await runRequested(steps({ 1: true }), all, null);
  assert.deepEqual(summarize(results).failed, []);
});

test("--only naming a shot that does not exist is unknown", () => {
  assert.deepEqual(unknownRequests(steps({ 1: true, 2: true }), new Set([1, 11])), [11]);
});

test("--only naming only known shots is empty", () => {
  assert.deepEqual(unknownRequests([[1, null], [6, null]], new Set([6])), []);
});

test("no --only at all requests everything and is never unknown", () => {
  assert.deepEqual(unknownRequests([[1, null]], null), []);
});

test("an unknown --only number alone makes the run exit nonzero", async () => {
  assert.equal(await codeAfter({ 1: true }, { unknown: [11] }), 1);
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
  assert.match(source, /runRequested\(SHOTS, wants, page, capturedThisRun\)/);
});

test("the capture rejects an unknown --only above the encoder probe", async () => {
  const source = await readFile(
    new URL("../shots/capture.cjs", import.meta.url),
    "utf8",
  );
  const validate = source.indexOf("unknownRequests(SHOTS, ONLY)");
  const encoder = source.indexOf("requireWebpEncoder();", source.indexOf("async function main()"));
  assert.ok(validate > 0, "the capture validates --only");
  assert.ok(validate < encoder, "validation must precede the encoder probe");
  assert.equal(source.match(/unknownRequests\(/g).length, 1);
});

const good = (n) => [n, `shot-${n}-x.webp`, named(n, async () => true)];

test("the runner refuses a registry it cannot trust, row by row", async () => {
  const cases = [
    ["an empty registry", []],
    ["no registry at all", null],
    ["a row that is not a triple", [[1, "shot-1-x.webp"]]],
    ["an id of zero", [[0, "shot-0-x.webp", named(0, async () => true)]]],
    ["a fractional id", [[1.5, "shot-1-x.webp", named(1, async () => true)]]],
    ["a duplicated id", [good(1), [1, "shot-1-y.webp", named(1, async () => true)]]],
    ["a file numbered for another shot", [[1, "shot-2-x.webp", named(1, async () => true)]]],
    ["a file that is not a webp", [[1, "shot-1-x.png", named(1, async () => true)]]],
    ["a producer that is not shotN", [good(1), [2, "shot-2-x.webp", named(9, async () => true)]]],
    ["a producer that is not a function", [[1, "shot-1-x.webp", "shot1"]]],
  ];
  for (const [label, rows] of cases) {
    await assert.rejects(
      () => runRequested(rows, all, null),
      /the shot registry is invalid/,
      label,
    );
  }
});

test("a registry the runner accepts reports no problems", () => {
  assert.deepEqual(registryProblems([good(1), good(10)]), []);
});

test("the runner hands each producer its declared filename", async () => {
  const seen = [];
  const rows = [1, 2].map((n) => [
    n,
    `shot-${n}-x.webp`,
    named(n, async (page, out) => {
      seen.push([n, page, out]);
      return true;
    }),
  ]);
  await runRequested(rows, all, "PAGE");
  assert.deepEqual(seen, [
    [1, "PAGE", "shot-1-x.webp"],
    [2, "PAGE", "shot-2-x.webp"],
  ]);
});

test("the capture's own registry is one the runner accepts", () => {
  assert.deepEqual(registryProblems(capture.SHOTS), []);
});

test("an invalid registry rejects before any producer runs", async () => {
  const ran = [];
  const shots = [
    [1, "shot-1-a.webp", named(1, async () => (ran.push(1), true))],
    [1, "shot-1-b.webp", named(1, async () => (ran.push("dup"), true))],
    [2, "shot-2-c.webp", named(2, async () => (ran.push(2), true))],
  ];
  await assert.rejects(() => runRequested(shots, () => true, {}, new Set()));
  assert.deepEqual(ran, [], "no producer may run once the registry is invalid");
});
