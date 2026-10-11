import assert from "node:assert/strict";
import test from "node:test";

class Node {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.className = "";
    this.dataset = {};
    this.style = {};
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

const document = {
  createElement: (tag) => new Node(tag),
  createTextNode: (text) => {
    const node = new Node("#text");
    node.textContent = text;
    return node;
  },
};
globalThis.document = document;

const adult = await import("../../web/alix/review/dom.js");
const { createKidsDom } = await import("../../web/alix-kids/kids/dom.js");
const kids = createKidsDom({ document });

function descendants(node) {
  return node.children.flatMap((child) => [child, ...descendants(child)]);
}
function byClass(node, cls) {
  return descendants(node).filter((child) => child.className.split(" ").includes(cls));
}

const clients = [
  ["adult", adult.appendRuns, (units) => adult.frontEl("", null, units)],
  ["kids", kids.appendRuns, (units) => kids.frontPrompt({ front_units: units })],
];

test("an asked blank and a hidden blank render as the same pill, the hidden one muted", () => {
  for (const [client, appendRuns] of clients) {
    const parent = new Node("div");
    appendRuns(parent, [{ text: "the " }, { text: "⍰", bold: true }, { text: " sits by the ⬚" }]);
    const pills = byClass(parent, "blank-pill");
    assert.equal(pills.length, 2, `${client}: one pill per marker`);
    assert.equal(pills[0].className, "blank-pill", `${client}: the asked blank is the accent pill`);
    assert.equal(pills[1].className, "blank-pill muted", `${client}: the hidden blank is the muted pill`);
    assert.equal(byClass(parent, "hole").length + byClass(parent, "muted-hole").length, 0,
      `${client}: no old hole chips remain`);
  }
});

test("a filled blank is highlighted and keeps its own styling", () => {
  for (const [client, appendRuns] of clients) {
    const parent = new Node("div");
    appendRuns(parent, [{ text: "the " }, { text: "lunate", bold: true, answer: true }, { text: " sits" }]);
    const filled = byClass(parent, "blank-filled");
    assert.equal(filled.length, 1, `${client}: one filled span`);
    assert.equal(filled[0].textContent, "lunate", `${client}: the filled span holds the answer`);
    assert.equal(filled[0].children[0].tagName, "STRONG", `${client}: the bold run stays bold inside`);
    assert.equal(parent.textContent, "the lunate sits", `${client}: the text reads in order`);
  }
});

test("a blank inside a front code fence renders as a pill", () => {
  for (const [client, , front] of clients) {
    const wrap = front([{ kind: "code", lines: ["let ⍰ = ⬚;"] }]);
    const pills = byClass(wrap, "blank-pill");
    assert.deepEqual(pills.map((pill) => pill.className), ["blank-pill", "blank-pill muted"],
      `${client}: the fence's markers are pills`);
    assert.equal(wrap.textContent, "let ⍰ = ⬚;", `${client}: the code text is otherwise unchanged`);
  }
});
