import assert from "node:assert/strict";
import test from "node:test";

import { createKidsPicker, kidsCatalogFailed } from "../../web/alix-kids/kids/picker.js";

class Node {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.className = "";
    this.textContent = "";
    this.listeners = new Map();
  }

  appendChild(child) {
    this.children.push(child);
    return child;
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  setAttribute() {}
}

function findClass(node, className) {
  if (node.className.split(" ").includes(className)) return node;
  for (const child of node.children) {
    const found = findClass(child, className);
    if (found) return found;
  }
  return null;
}

function pickerHarness(catalog) {
  const document = { createElement: (tag) => new Node(tag) };
  const stage = new Node("main");
  const actionbar = new Node("nav");
  const el = (tag, className, text) => {
    const node = new Node(tag);
    node.className = className || "";
    node.textContent = text || "";
    return node;
  };
  let picker;
  const rerender = () => {
    stage.children = [];
    actionbar.children = [];
    picker.render();
  };
  picker = createKidsPicker({
    api: async () => catalog,
    post: (body) => body,
    openStudy: () => {},
    rerender,
    isVisible: () => true,
    ui: { actionbar, document, el, mascot: () => new Node("div"), stage },
  });
  return { picker, stage };
}

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

test("a box with damaged progress does not tell a kid it is all caught up", async () => {
  const member = {
    name: "animals/facts.md",
    label: "Facts",
    state: "error",
    reviewable: false,
    reviewable_recognize: false,
    reviewable_recall: false,
  };
  const run = pickerHarness({
    workspaces: [{
      name: "animals",
      label: "Animals",
      reviewable: false,
      members: [member],
    }],
  });

  await run.picker.load();

  assert.equal(
    findClass(run.stage, "box-ready").textContent,
    "needs a grown-up 🔧",
    "the first screen must surface damaged progress instead of a false completion claim",
  );

  const mixed = pickerHarness({
    workspaces: [{
      name: "mixed",
      label: "Mixed",
      reviewable: true,
      members: [member, { ...member, name: "mixed/ready.md", state: "due", reviewable: true }],
    }],
  });

  await mixed.picker.load();

  assert.equal(
    findClass(mixed.stage, "box-ready").textContent,
    "ready to practise",
    "a healthy ready sibling keeps the existing box-level readiness behavior",
  );
});
