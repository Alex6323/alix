"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

function fabricatedKeys(t, answer, directive) {
  const capture = fs.readFileSync(path.join(__dirname, "capture.cjs"), "utf8");
  const start = capture.indexOf("function fabricateGraduation() {");
  const end = capture.indexOf("\nasync function shot4(", start);
  assert.notEqual(start, -1);
  assert.notEqual(end, -1);

  const root = fs.mkdtempSync(path.join(os.tmpdir(), "alix-capture-space-repro-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const heroFile = path.join(root, "hero.md");
  const demoDir = path.join(root, "demo");
  fs.writeFileSync(
    heroFile,
    "---\nid: deck-aaaaaaaaaaaaaaaaaaaaaaaaaa\n---\n\n" +
      `## Compare\n---\n${answer}\n${directive}\n` +
      "<!-- id: card-aaaaaaaaaaaaaaaaaaaaaaaaaa -->\n",
  );

  vm.runInNewContext(
    capture.slice(start, end) + "\nfabricateGraduation();",
    {
      fs,
      HERO_FILE: heroFile,
      DEMO_DIR: demoDir,
      Date,
      path,
      deckId: () => "deck-aaaaaaaaaaaaaaaaaaaaaaaaaa",
      buildAlix: () => "unused",
      execFileSync: () => {},
      log: () => {},
    },
  );

  const doc = JSON.parse(
    fs.readFileSync(
      path.join(demoDir, "progress", "deck-aaaaaaaaaaaaaaaaaaaaaaaaaa.json"),
      "utf8",
    ),
  );
  return Object.keys(doc.cards);
}

const expected = [
  "card-aaaaaaaaaaaaaaaaaaaaaaaaaa",
  "card-aaaaaaaaaaaaaaaaaaaaaaaaaa-ba1b2c3",
];

test("fabricated graduation recognizes Rust-only U+0085 whitespace", (t) => {
  assert.deepEqual(
    fabricatedKeys(t, "x", '<!-- blank: span hidden="x"\u0085b:a1b2c3 -->'),
    expected,
    "Rust treats U+0085 NEXT LINE as whitespace",
  );
});

test("fabricated graduation does not split on JavaScript-only U+FEFF whitespace", (t) => {
  assert.deepEqual(
    fabricatedKeys(
      t,
      "x\ufeffb:decoy",
      '<!-- blank: span hidden="x"\ufeffb:decoy b:a1b2c3 -->',
    ),
    expected,
    "Rust does not treat U+FEFF BYTE ORDER MARK as whitespace",
  );
});
