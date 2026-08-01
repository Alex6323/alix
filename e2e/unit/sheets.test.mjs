import assert from "node:assert/strict";
import test from "node:test";

import { createSheets } from "../../web/alix/review/sheets.js";

function harness() {
  const calls = [];
  const sheet = {
    hidden: true,
    dataset: {},
    addEventListener: () => {},
  };
  const panel = { innerHTML: "" };
  const nodes = { sheet, sheetPanel: panel };
  const sheets = createSheets({
    api: async (path, options) => {
      calls.push({ path, options });
      return {};
    },
    fetchApi: async () => {},
    post: (body) => ({ method: "POST", body }),
    withToken: (path) => path,
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
  return { calls, panel, sheet, sheets };
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
