import assert from "node:assert/strict";
import test from "node:test";

import { overflowHints } from "../../assets/web/review/dom.js";

test("overflow hints follow scroll edges", () => {
  assert.deepEqual(
    overflowHints({ scrollTop: 0, clientHeight: 200, scrollHeight: 500 }),
    { overflows: true, showTop: false, showBottom: true },
  );
  assert.deepEqual(
    overflowHints({ scrollTop: 150, clientHeight: 200, scrollHeight: 500 }),
    { overflows: true, showTop: true, showBottom: true },
  );
  assert.deepEqual(
    overflowHints({ scrollTop: 300, clientHeight: 200, scrollHeight: 500 }),
    { overflows: true, showTop: true, showBottom: false },
  );
  assert.deepEqual(
    overflowHints({ scrollTop: 0, clientHeight: 200, scrollHeight: 201 }),
    { overflows: false, showTop: false, showBottom: false },
  );
});
