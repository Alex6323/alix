import assert from "node:assert/strict";
import test from "node:test";

import { createTutor } from "../../web/alix/review/tutor.js";

test("tutor owns its transcript and chooses the walk endpoint explicitly", async () => {
  const calls = [];
  let renders = 0;
  let walking = true;
  const transcript = {
    transcript: [{ q: "Why?", a: "Because." }],
    thinking: false,
    status: null,
    error: null,
  };
  const tutor = createTutor({
    api: async (path, options) => {
      calls.push({ path, options });
      return transcript;
    },
    post: (body) => ({ method: "POST", body }),
    rerender: () => renders++,
    updateBusy: () => {},
    timers: { setInterval: () => 1, clearInterval: () => {} },
    walk: {
      isOpen: () => walking,
      replace: () => {},
    },
    study: {
      state: () => ({ card: null }),
      replaceState: () => {},
      load: () => Promise.resolve(),
    },
    ui: {
      document: { querySelector: () => null },
    },
  });

  await tutor.show();

  assert.equal(tutor.isOpen(), true);
  assert.equal(tutor.data(), transcript);
  assert.equal(calls[0].path, "/api/walk/ask");
  assert.equal(renders, 2);

  walking = false;
  calls.length = 0;
  tutor.data().transcript.length = 0;
  await tutor.close();

  assert.equal(tutor.isOpen(), false);
  assert.equal(calls.length, 0);
});
