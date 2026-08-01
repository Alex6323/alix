import assert from "node:assert/strict";
import test from "node:test";

import { kidsOverflowHints } from "../../web/alix-kids/kids/dom.js";

test("kids overflow hints follow scroll geometry", () => {
  assert.deepEqual(
    kidsOverflowHints({ scrollTop: 0, clientHeight: 200, scrollHeight: 500 }),
    { showTop: false, showBottom: true },
  );
  assert.deepEqual(
    kidsOverflowHints({ scrollTop: 150, clientHeight: 200, scrollHeight: 500 }),
    { showTop: true, showBottom: true },
  );
  assert.deepEqual(
    kidsOverflowHints({ scrollTop: 300, clientHeight: 200, scrollHeight: 500 }),
    { showTop: true, showBottom: false },
  );
  assert.deepEqual(
    kidsOverflowHints({ scrollTop: 0, clientHeight: 200, scrollHeight: 203 }),
    { showTop: false, showBottom: false },
  );
});
