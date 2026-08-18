import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("the screenshot progress fixture uses only the current card-state schema", async () => {
  const source = await readFile(new URL("../shots/capture.cjs", import.meta.url), "utf8");

  assert.equal(
    /\bpresented_ms\s*:/.test(source),
    false,
    "the screenshot fixture still writes the deleted presented_ms field",
  );
});
