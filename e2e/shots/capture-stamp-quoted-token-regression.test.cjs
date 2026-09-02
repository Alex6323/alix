"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

test("fabricated graduation uses the stamp token rather than b-colon text inside hidden", () => {
  const capture = fs.readFileSync(path.join(__dirname, "capture.cjs"), "utf8");
  const start = capture.indexOf("function fabricateGraduation() {");
  const end = capture.indexOf("\nasync function shot4(", start);
  assert.notEqual(start, -1);
  assert.notEqual(end, -1);

  const root = fs.mkdtempSync(path.join(os.tmpdir(), "alix-capture-token-repro-"));
  const heroFile = path.join(root, "hero.md");
  const demoDir = path.join(root, "demo");
  fs.writeFileSync(
    heroFile,
    "---\nid: deck-aaaaaaaaaaaaaaaaaaaaaaaaaa\n---\n\n" +
      "## Compare\nthe marker is b:decoy\n" +
      "<!-- blank: span hidden=\"b:decoy\" b:a1b2c3 -->\n" +
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
  assert.ok(
    doc.cards["card-aaaaaaaaaaaaaaaaaaaaaaaaaa-ba1b2c3"],
    "the directive's actual stamp must identify the graduated sub-card",
  );
});
