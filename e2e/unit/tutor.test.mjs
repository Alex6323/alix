import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { createTutor } from "../../web/alix/review/tutor.js";

// Codex's wiring guard, narrowed to the `ui` object itself after they showed
// the block-wide match stayed green with `appendTable` moved into `timers`.
test("the adult app wires table rendering into the tutor", async () => {
  const source = await readFile(
    new URL("../../web/alix/review/app.js", import.meta.url),
    "utf8",
  );
  const tutorStart = source.indexOf("const tutor = createTutor({");
  const tutorEnd = source.indexOf("const walk = createWalk({", tutorStart);
  const tutorWiring = source.slice(tutorStart, tutorEnd);
  const uiStart = tutorWiring.indexOf("\n  ui: {");
  const uiEnd = tutorWiring.indexOf("\n  },", uiStart);
  assert.ok(uiStart >= 0 && uiEnd > uiStart, "the tutor is wired with a `ui` object");
  const tutorUi = tutorWiring.slice(uiStart, uiEnd);

  assert.match(
    tutorUi,
    /^\s*appendTable,\s*$/m,
    "a table answer crashes the tutor unless app.js injects appendTable into its `ui`",
  );
});

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
