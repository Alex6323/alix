import assert from "node:assert/strict";
import test from "node:test";

import { veilRuns } from "../../web/alix/review/model.js";

const shape = (runs) => runs.map((run) => (run.kept ? run.text : `[${run.text}]`)).join("");

test("the veil keeps a word's first letter, and its last past two letters", () => {
  const rows = [
    ["one letter", "a", "a"],
    ["two letters", "of", "o[f]"],
    ["three letters", "the", "t[h]e"],
    ["a long word", "immutable", "i[mmutabl]e"],
    ["attached punctuation counts as letters", "binding,", "b[inding],"],
    ["a code token is a word like any other", "let mut x = &mut x", "l[e]t m[u]t x = &[mu]t x"],
    ["several words keep every space", "a name cannot be reassigned.", "a n[am]e c[anno]t b[e] r[eassigned]."],
    ["leading and trailing space survive", "  two  ", "  t[w]o  "],
    ["a tab is whitespace too", "x\ty", "x\ty"],
    ["empty text is no runs", "", ""],
  ];
  for (const [label, text, expected] of rows) {
    assert.equal(shape(veilRuns(text)), expected, label);
  }
});

test("the veiled runs concatenate back to the text, so the width is kept", () => {
  for (const text of ["", "x", "an immutable binding: the name cannot be reassigned.", "`let mut x = &mut x`"]) {
    assert.equal(veilRuns(text).map((run) => run.text).join(""), text, JSON.stringify(text));
  }
});

test("runs alternate between kept and veiled, and no run is empty", () => {
  const runs = veilRuns("hello wide world");
  assert.ok(runs.every((run) => run.text.length > 0), "no empty run");
  for (let index = 1; index < runs.length; index++) {
    assert.notEqual(runs[index].kept, runs[index - 1].kept, `run ${index} alternates`);
  }
});
