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
  const tutorEnd = source.indexOf("const trace = createTraceSession({", tutorStart);
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
  assert.match(
    tutorUi,
    /^\s*appendUnits,\s*$/m,
    "a structured tutor answer needs the shared recursive unit renderer",
  );
});

test("tutor owns its transcript and chooses the trace endpoint explicitly", async () => {
  const calls = [];
  let renders = 0;
  let tracing = true;
  const transcript = {
    transcript: [{ q: "Why?", a: "Because." }],
    thinking: false,
    can_distill: false,
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
    trace: {
      isOpen: () => tracing,
      replace: () => {},
    },
    study: {
      state: () => ({ card: null }),
      isWalking: () => false,
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
  assert.equal(calls[0].path, "/api/trace/ask");
  assert.equal(renders, 2);

  await tutor.saveNote();
  await tutor.draftCard();
  assert.equal(calls.length, 1, "the server owns the distillation gate");

  transcript.can_distill = true;
  await tutor.draftCard();
  assert.equal(calls[1].path, "/api/ask/card/draft");

  tracing = false;
  calls.length = 0;
  tutor.data().transcript.length = 0;
  await tutor.close();

  assert.equal(tutor.isOpen(), false);
  assert.equal(calls.length, 0);
});

test("during a walk the tutor asks the walk endpoint and drafts no card", async () => {
  const calls = [];
  const replaced = [];
  const transcript = {
    transcript: [],
    thinking: false,
    can_distill: true,
    status: null,
    error: null,
  };
  const walk = { kind: "walk", phase: "answer", card: null };
  const tutor = createTutor({
    api: async (path, options) => {
      calls.push(path);
      return path === "/api/walk" ? walk : transcript;
    },
    post: (body) => ({ method: "POST", body }),
    rerender: () => {},
    updateBusy: () => {},
    timers: { setInterval: () => 1, clearInterval: () => {} },
    trace: { isOpen: () => false, replace: () => {} },
    study: {
      state: () => ({ card: null }),
      isWalking: () => true,
      replaceState: (next) => replaced.push(next),
      load: () => Promise.resolve(),
    },
    ui: { document: { querySelector: () => null } },
  });

  await tutor.show();
  assert.equal(calls[0], "/api/walk/ask", "step: open; calls: " + calls.join(","));

  await tutor.draftCard();
  assert.equal(calls.length, 1, "step: draft; a walk has no card-draft route; calls: " + calls.join(","));

  await tutor.saveNote();
  assert.equal(calls[1], "/api/walk/ask/note", "step: note; calls: " + calls.join(","));

  tutor.data().transcript.length = 0;
  await tutor.close();
  assert.equal(calls.at(-1), "/api/walk", "step: close refreshes the walk, not the drill state; calls: " + calls.join(","));
  assert.deepEqual(replaced, [walk], "step: close; the refreshed walk replaces the state");
});
