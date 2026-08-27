import assert from "node:assert/strict";
import test from "node:test";

import {
  applyStudyState,
  createModel,
  currentScreen,
  enterPicker,
} from "../../web/alix/review/model.js";
import { createStudy, modeTag } from "../../web/alix/review/study.js";

function harness() {
  const walks = [];
  let renders = 0;
  const study = createStudy({
    api: async () => ({}),
    post: (body) => ({ method: "POST", body }),
    storage: { getItem: () => null, setItem: () => {} },
    model: { create: createModel, applyStudyState, currentScreen, enterPicker },
    rerender: () => renders++,
    walkData: () => null,
    replaceWalk: (walk) => walks.push(walk),
    openTutor: () => {},
    startExam: () => {},
    closeMenu: () => {},
    timers: {},
    ui: {},
  });
  return { renders: () => renders, study, walks };
}

function node(tag = "div", cls = null, text = "") {
  return {
    tag,
    cls,
    textContent: text,
    innerHTML: "",
    hidden: false,
    disabled: false,
    children: [],
    style: {},
    dataset: {},
    classList: { add() {}, remove() {} },
    appendChild(child) {
      this.children.push(child);
      return child;
    },
  };
}

// Enough of the DOM surface for the summary screen, plus a hand-cranked
// interval so the due poll fires exactly when the test says so.
function summaryHarness(state, pending) {
  const stage = node();
  const chips = [];
  const ticks = [];
  const calls = [];
  const posts = [];
  let study;
  study = createStudy({
    api: async (path, init) => {
      calls.push(path);
      posts.push(init);
      return pending;
    },
    post: (body) => ({ method: "POST", body }),
    storage: { getItem: () => null, setItem: () => {} },
    model: { create: createModel, applyStudyState, currentScreen, enterPicker },
    rerender: () => study.render(),
    walkData: () => null,
    replaceWalk: () => {},
    openTutor: () => {},
    startExam: () => {},
    closeMenu: () => {},
    lastDeck: () => "deck.md",
    timers: {
      setInterval: (fn) => {
        ticks.push(fn);
        return ticks.length;
      },
      clearInterval: () => {},
    },
    ui: {
      el: (tag, cls, text) => node(tag, cls, text),
      chip: (text, cls, onClick) => {
        const c = node("button", cls, text);
        c.onClick = onClick;
        chips.push(c);
        return c;
      },
      stage,
      deckEl: node(),
      histEl: node(),
      scoreEl: node(),
      menuWrap: node(),
      headerBreadcrumb: () => {},
      setMenuContext: () => {},
      label: () => "",
      document: { querySelector: () => null },
      window: {},
    },
  });
  study.apply(state);
  const note = () => stage.children[0].children.find((c) => c.cls === "note");
  const rows = () =>
    stage.children[0].children
      .filter((c) => c.cls === "row")
      .map((r) => ({ label: r.children[0].textContent, value: r.children[1].textContent }));
  const continueChip = () => chips.find((c) => c.textContent === "Continue");
  return { study, note, rows, continueChip, chips, tick: () => ticks[0](), calls, posts };
}

const DONE = {
  kind: "review",
  phase: "done",
  label: "Facts",
  card: null,
  reviews: 0,
  passed: 0,
  failed: 0,
  introduced: 3,
  due_left: 2,
  new_left: 5,
  met_total: 24,
  deck_total: 88,
  can_restart: false,
  next_due_ms: Date.now() + 240_000,
};

test("the summary reports the deck's standing beside the sitting's count", () => {
  const run = summaryHarness(DONE, null);
  const rows = run.rows();

  assert.equal(rows[0].label, "introduced (24 of 88 in the deck)");
  assert.equal(rows[0].value, "3", "the value column stays this sitting's count");
});

test("an expired settle gap arms the summary instead of starting the card", async () => {
  const ready = { ...DONE, phase: "review", card: { id: "card-1" }, remaining: 1 };
  const run = summaryHarness(DONE, ready);

  assert.match(run.note().textContent, /Next due in 4 min\./, "the countdown renders first");
  assert.equal(run.continueChip().disabled, true, "nothing is servable yet");

  run.tick();
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(run.study.screen(), "summary", "the summary must not navigate on a timer");
  assert.equal(run.study.state().phase, "done", "the found card is not applied");
  assert.equal(run.continueChip().disabled, false, "Continue is armed instead");
  assert.equal(run.note().textContent, "Ready when you are.");
});

test("study owns accepted state publication and screen selection", () => {
  const run = harness();
  const review = {
    kind: "review",
    phase: "review",
    label: "Facts",
    remaining: 3,
    card: { id: "card-1" },
  };

  run.study.apply(review);

  assert.equal(run.study.state(), review);
  assert.equal(run.study.screen(), "study");
  assert.deepEqual(run.walks, [null]);
  assert.equal(run.renders(), 1);

  const done = { ...review, phase: "done", card: null };
  run.study.apply(done);

  assert.equal(run.study.state(), done);
  assert.equal(run.study.screen(), "summary");
  assert.equal(run.renders(), 2);
});

test("the same load warning surfaces again for a later deck", () => {
  const notices = [];
  const document = { getElementById: () => null };
  const study = createStudy({
    api: async () => ({}),
    post: (body) => ({ method: "POST", body }),
    storage: { getItem: () => null, setItem: () => {} },
    model: { create: createModel, applyStudyState, currentScreen, enterPicker },
    rerender: () => {},
    walkData: () => null,
    replaceWalk: () => {},
    openTutor: () => {},
    startExam: () => {},
    closeMenu: () => {},
    notice: (message) => notices.push(message),
    timers: {},
    ui: { document },
  });
  const warning =
    "1 frozen diagram(s) did not resolve and fall back to source; run `alix doctor` for details";
  const state = (label, loadWarnings) => ({
    kind: "review",
    phase: "done",
    label,
    load_warnings: loadWarnings,
    save_error: null,
  });

  study.apply(state("first.md", [warning]));
  study.prepareRender();
  study.apply(state("picker", []));
  study.prepareRender();
  study.apply(state("second.md", [warning]));
  study.prepareRender();

  assert.deepEqual(
    notices,
    [warning, warning],
    "each newly opened deck gets its own one-shot warning even when the text matches",
  );
});

test("the badge names provenance and the interaction actually on screen", () => {
  const cases = [
    [{ mode: "flip" }, "flip"],
    [{ mode: "typeline" }, "typing · line"],
    [{ mode: "flip", choices: true }, "choice"],
    // An introduction card never runs the depth's check: it picks, draws, or reveals.
    [{ mode: "flip", introducing: true }, "new · reveal"],
    [{ mode: "typing", introducing: true }, "new · reveal"],
    [{ mode: "flip", introducing: true, choices: true }, "new · choice"],
    [{ mode: "flip", introducing: true, draw: true }, "new · draw"],
  ];
  for (const [state, want] of cases) {
    assert.equal(modeTag(state), want, `badge for ${JSON.stringify(state)}`);
  }
});

test("the Recall chip carries the sitting's scope so a re-select cannot widen it", async () => {
  const run = summaryHarness(
    {
      ...DONE,
      topology: "order",
      region: "intro",
      recognize_gap: { recall: 4 },
      exam_due: [],
    },
    DONE,
  );

  const recall = run.chips.find((c) => c.textContent === "Continue at Recall");
  assert.ok(recall, "a drained sitting with a Recall gap offers the chip");

  recall.onClick();
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(run.calls[run.calls.length - 1], "/api/select");
  assert.deepEqual(run.posts[run.posts.length - 1].body, {
    deck: "deck.md",
    topology: "order",
    region: "intro",
    depth: "recall",
  });
});
