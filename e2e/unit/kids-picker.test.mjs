import assert from "node:assert/strict";
import test from "node:test";

import { kidsCatalogFailed } from "../../web/alix-kids/kids/picker.js";

test("kids catalog failure preserves the previous catalog and becomes retryable", () => {
  const catalog = { workspaces: [{ name: "animals" }] };
  const state = {
    currentBox: { name: "animals" },
    currentDeck: null,
    selectError: false,
    deckList: catalog,
    loadError: false,
  };

  const next = kidsCatalogFailed(state);

  assert.notEqual(next, state);
  assert.equal(next.deckList, catalog);
  assert.equal(next.currentBox, state.currentBox);
  assert.equal(next.loadError, true);
});
