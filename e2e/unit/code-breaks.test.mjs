import assert from "node:assert/strict";
import fs from "node:fs";
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

const vectors = JSON.parse(
  fs.readFileSync(new URL("../../tests/fixtures/code-breaks.json", import.meta.url), "utf8"),
);

const clients = [
  ["adult", (parent, runs) => adult.appendRuns(parent, runs)],
  ["kids", (parent, runs) => kids.appendRuns(parent, runs)],
];

test("a code run breaks only after its punctuation tokens, per the shared vectors", () => {
  for (const [client, appendRuns] of clients) {
    for (const { code, segments } of vectors) {
      const parent = new Node("div");
      appendRuns(parent, [{ text: "see " }, { text: code, code: true }, { text: "a::b" }]);
      const element = parent.children.find((child) => child.tagName === "CODE");
      const pieces = element.children.map((child) => (child.tagName === "WBR" ? "|" : child.textContent));
      const expected = segments.flatMap((segment, index) => (index === 0 ? [segment] : ["|", segment]));
      assert.deepEqual(pieces, expected, `${client} \`${code}\`: a <wbr> after each token`);
      assert.equal(parent.textContent, `see ${code}a::b`, `${client} \`${code}\`: the text is unchanged`);
    }
  }
});
