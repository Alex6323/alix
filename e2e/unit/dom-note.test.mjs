import assert from "node:assert/strict";
import test from "node:test";

// dom.js reaches for the global `document`, so the shim must exist before the
// module is imported. Enough surface for renderNote's sentence and code arms.
class Node {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.className = "";
    this.dataset = {};
    this.ownText = "";
  }

  set textContent(value) {
    this.ownText = value;
    this.children = [];
  }

  get textContent() {
    return this.ownText + this.children.map((child) => child.textContent).join("");
  }

  appendChild(child) {
    this.children.push(child);
    return child;
  }

  setAttribute() {}
}

globalThis.document = {
  createElement: (tag) => new Node(tag),
  createTextNode: (text) => {
    const node = new Node("#text");
    node.textContent = text;
    return node;
  },
};

const { renderNote } = await import("../../web/alix/review/dom.js");

const sentence = (text) => ({ kind: "sentence", text, runs: [{ text }] });

test("a note arrives wrapped in its badge and still renders its body", () => {
  const parent = new Node("div");
  renderNote(parent, [
    { badge: "warning", units: [sentence("Back up this directory.")] },
  ]);

  assert.equal(parent.children.length, 1, "one note is one .note block");
  const [note] = parent.children;
  assert.equal(note.className, "note");
  assert.equal(note.dataset.badge, "warning", "the badge reaches the view model");
  const [chip, body] = note.children;
  assert.equal(chip.className, "note-badge", "a badged note leads with its chip");
  assert.equal(chip.textContent, "warning", "the chip carries the wire spelling");
  assert.equal(
    body.textContent,
    "Back up this directory.",
    "the body renders; an unwrapped NoteUnit would match no arm and render nothing",
  );
});

test("a plain note renders its body under the NOTE badge word", () => {
  const parent = new Node("div");
  renderNote(parent, [{ badge: "note", units: [sentence("Its note column.")] }]);

  const [note] = parent.children;
  assert.equal(note.dataset.badge, "note", "a table column is a plain NOTE");
  const [word, ...body] = note.children;
  assert.equal(word.className, "note-badge");
  assert.equal(word.textContent, "note");
  assert.equal(body.map((child) => child.textContent).join(""), "Its note column.");
});

test("several notes stack as siblings, each keeping its own badge", () => {
  const parent = new Node("div");
  renderNote(parent, [
    { badge: "note", units: [sentence("First.")] },
    { badge: "caution", units: [sentence("Second.")] },
  ]);

  assert.deepEqual(
    parent.children.map((note) => [
      note.dataset.badge,
      note.children
        .filter((child) => child.className !== "note-badge")
        .map((child) => child.textContent)
        .join(""),
    ]),
    [
      ["note", "First."],
      ["caution", "Second."],
    ],
    "one box per note, so no badge overwrites another",
  );
});

test("no notes and an empty body both render nothing", () => {
  for (const notes of [undefined, [], [{ badge: "note", units: [] }]]) {
    const parent = new Node("div");
    renderNote(parent, notes);
    assert.equal(parent.children.length, 0, `${JSON.stringify(notes)} renders no block`);
  }
});
