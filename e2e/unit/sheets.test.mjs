import assert from "node:assert/strict";
import test from "node:test";

import { createSheets } from "../../web/alix/review/sheets.js";

function harness() {
  const calls = [];
  const reportLink = {};
  const sheet = {
    hidden: true,
    dataset: {},
    addEventListener: () => {},
  };
  const panel = {
    innerHTML: "",
    querySelector: (selector) => selector === "#bugReport" ? reportLink : null,
  };
  const nodes = { sheet, sheetPanel: panel };
  const sheets = createSheets({
    api: async (path, options) => {
      calls.push({ path, options });
      return {};
    },
    fetchApi: async () => {},
    post: (body) => ({ method: "POST", body }),
    withToken: (path) => "token:" + path,
    focusedRowName: () => null,
    notice: () => {},
    refreshPicker: () => {},
    timers: { setInterval: () => 1, clearInterval: () => {} },
    ui: {
      document: {
        addEventListener: () => {},
        getElementById: (id) => nodes[id],
      },
      FileReader: class {},
      Option: class {},
    },
  });
  return { calls, panel, reportLink, sheet, sheets };
}

function removalHarness(responses) {
  const calls = [];
  const notices = [];
  const listeners = (initial = {}) => ({
    ...initial,
    isConnected: true,
    textContent: "",
    addEventListener(type, handler) { this.listeners[type] = handler; },
    async fire(type) { return this.listeners[type]?.({ target: this }); },
    focus() { this.focused = true; },
    listeners: {},
  });
  const sheet = listeners({ hidden: true, dataset: {} });
  const nodes = { sheet };
  const panel = {
    querySelector: () => null,
    get innerHTML() { return this.html || ""; },
    set innerHTML(value) {
      this.html = value;
      if (!value.includes("removeLoading")) return;
      Object.assign(nodes, {
        removeLoading: listeners(),
        removePreviewRetry: listeners({ hidden: true }),
        removeDetails: listeners({ hidden: true }),
        removeTarget: listeners(),
        removeKind: listeners(),
        removeStakes: listeners(),
        removeArtifacts: listeners(),
        removeDependents: listeners(),
        removeConfirm: listeners({ value: "" }),
        removeGo: listeners({ disabled: true }),
        removeStatus: listeners(),
      });
    },
  };
  nodes.sheetPanel = panel;
  let refreshes = 0;
  const sheets = createSheets({
    api: async (path, options) => {
      calls.push({ path, options });
      const response = responses.shift();
      if (response instanceof Error) throw response;
      return response;
    },
    fetchApi: async () => {},
    post: (body) => ({ method: "POST", body }),
    withToken: (path) => path,
    focusedRowName: () => "animals",
    notice: (message) => notices.push(message),
    refreshPicker: () => { refreshes++; },
    timers: { setInterval: () => 1, clearInterval: () => {} },
    ui: {
      document: {
        addEventListener: () => {},
        getElementById: (id) => nodes[id],
      },
      FileReader: class {},
      Option: class {},
    },
  });
  return { calls, nodes, notices, panel, refreshes: () => refreshes, sheet, sheets };
}

test("sheets own visibility and cancel live work when closed", () => {
  const run = harness();

  run.sheets.openShortcuts();

  assert.equal(run.sheets.isOpen(), true);
  assert.equal(run.sheet.hidden, false);
  assert.match(run.panel.innerHTML, /Picker shortcuts/);

  run.sheet.dataset.shareLive = "1";
  run.sheets.close();

  assert.equal(run.sheets.isOpen(), false);
  assert.equal(run.sheet.hidden, true);
  assert.equal(run.panel.innerHTML, "");
  assert.deepEqual(run.calls, [{
    path: "/api/share/close",
    options: { method: "POST", body: {} },
  }]);
});

test("about offers one token-guarded bug report download", async () => {
  const run = harness();

  await run.sheets.openAbout();

  assert.match(run.panel.innerHTML, /Prepare a bug report/);
  assert.equal(run.reportLink.href, "token:/api/bug-report");
  assert.equal(run.reportLink.download, "");
});

test("library removal previews stakes and requires the exact focused name", async () => {
  const run = removalHarness([
    {
      target: "animals",
      kind: "workspace",
      decks: 2,
      cards_with_progress: 7,
      earliest_review_ms: 1_700_000_000_000,
      files: ["decks/one.md", "alix.toml"],
      directories: ["assets"],
      dependents: ["advanced.md"],
    },
    {
      target: "animals",
      kind: "workspace",
      removed: ["decks/one.md", "alix.toml"],
      decks_removed: 2,
      directory_removed: false,
      dependents: ["advanced.md"],
    },
  ]);

  await run.sheets.openLibraryRemoval();

  assert.deepEqual(run.calls[0], {
    path: "/api/library/remove/preview",
    options: { method: "POST", body: { name: "animals" } },
  });
  assert.equal(run.nodes.removeTarget.textContent, "animals");
  assert.match(run.nodes.removeStakes.textContent, /7 cards with progress/);
  assert.match(run.nodes.removeDependents.textContent, /advanced\.md/);
  assert.equal(run.nodes.removeConfirm.focused, true);
  assert.equal(run.nodes.removeGo.disabled, true);

  run.nodes.removeConfirm.value = "animal";
  await run.nodes.removeConfirm.fire("input");
  assert.equal(run.nodes.removeGo.disabled, true);
  run.nodes.removeConfirm.value = "animals";
  await run.nodes.removeConfirm.fire("input");
  assert.equal(run.nodes.removeGo.disabled, false);

  await run.nodes.removeGo.fire("click");

  assert.deepEqual(run.calls[1], {
    path: "/api/library/remove",
    options: { method: "POST", body: { name: "animals" } },
  });
  assert.equal(run.sheet.hidden, true);
  assert.equal(run.refreshes(), 1);
  assert.deepEqual(run.notices, [
    "removed workspace 'animals'; folder kept: it contains files Alix doesn't own",
  ]);
});

test("an in-flight library removal posts only once", async () => {
  let finishRemoval;
  const removal = new Promise((resolve) => { finishRemoval = resolve; });
  const run = removalHarness([
    {
      target: "animals",
      kind: "workspace",
      decks: 2,
      cards_with_progress: 0,
      earliest_review_ms: null,
      files: ["alix.toml"],
      directories: [],
      dependents: [],
    },
    removal,
  ]);
  await run.sheets.openLibraryRemoval();
  run.nodes.removeConfirm.value = "animals";
  await run.nodes.removeConfirm.fire("input");

  const first = run.nodes.removeGo.fire("click");
  await run.nodes.removeGo.fire("click");

  assert.equal(run.nodes.removeGo.disabled, true);
  assert.equal(run.calls.length, 2, "preview plus one removal request");
  finishRemoval({
    target: "animals",
    kind: "workspace",
    removed: ["alix.toml"],
    decks_removed: 2,
    directory_removed: true,
    dependents: [],
  });
  await first;
});

test("partial library removal stays in the sheet and directs recovery", async () => {
  const error = Object.assign(new Error("failed"), {
    status: 500,
    body: {
      error: "removal incomplete",
      completed: ["decks/one.md"],
      failed: "progress/deck-one.json",
      recovery: "Run alix doctor to inspect and repair the remaining artifacts.",
    },
  });
  const run = removalHarness([
    {
      target: "animals",
      kind: "workspace",
      decks: 2,
      cards_with_progress: 7,
      earliest_review_ms: null,
      files: ["decks/one.md", "alix.toml"],
      directories: [],
      dependents: [],
    },
    error,
  ]);
  await run.sheets.openLibraryRemoval();
  run.nodes.removeConfirm.value = "animals";
  await run.nodes.removeConfirm.fire("input");

  await run.nodes.removeGo.fire("click");

  assert.equal(run.sheet.hidden, false);
  assert.equal(run.nodes.removeGo.disabled, true);
  assert.equal(run.nodes.removeGo.textContent, "Removal stopped");
  assert.match(run.nodes.removeStatus.textContent, /decks\/one\.md/);
  assert.match(run.nodes.removeStatus.textContent, /progress\/deck-one\.json/);
  assert.match(run.nodes.removeStatus.textContent, /alix doctor/);
});

test("a raced active session leaves library removal ready to retry", async () => {
  const run = removalHarness([
    {
      target: "animals",
      kind: "workspace",
      decks: 2,
      cards_with_progress: 0,
      earliest_review_ms: null,
      files: ["alix.toml"],
      directories: [],
      dependents: [],
    },
    Object.assign(new Error("busy"), { status: 409 }),
  ]);
  await run.sheets.openLibraryRemoval();
  run.nodes.removeConfirm.value = "animals";
  await run.nodes.removeConfirm.fire("input");

  await run.nodes.removeGo.fire("click");

  assert.match(run.nodes.removeStatus.textContent, /study session is active/);
  assert.equal(run.nodes.removeGo.textContent, "Try again");
  assert.equal(run.nodes.removeGo.disabled, false);
});
