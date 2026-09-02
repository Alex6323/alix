#!/usr/bin/env node
"use strict";
// Prepares the scratch decks dir a real `alix` server can write into
// (`progress/` / recent.json) without ever touching the repo. Idempotent:
// wipes and re-copies on every call, so each run starts from the exact same
// committed state. Each web client under test gets its OWN copy
// (`.tmp/<name>/decks`), so the kids and adult servers' stores never collide
// or leak into each other.
//
// Called from two places per client, on purpose (belt and suspenders):
// Playwright's `globalSetup` (see global-setup.ts), and directly chained
// into that client's `webServer.command` shell command in
// playwright.config.ts. Playwright does not document whether globalSetup is
// guaranteed to finish before the webServer processes start, so each
// webServer command also runs this synchronously (as a plain script, before
// `cargo run`) to guarantee its decks directory exists the moment the server
// binary checks for it.

const fs = require("node:fs");
const path = require("node:path");

const ROOT = __dirname;
const SRC = path.join(ROOT, "fixtures", "decks");
const TMP = path.join(ROOT, ".tmp");
// One scratch copy per server in playwright.config.ts.
const CLIENTS = ["kids", "adult", "kids-graded"];

function isPrivateState(src) {
  const base = path.basename(src);
  return base === "progress" || base === "recent.json";
}

function prepareFixtures(name) {
  const clientTmp = path.join(TMP, name);
  const dest = path.join(clientTmp, "decks");
  fs.rmSync(clientTmp, { recursive: true, force: true });
  fs.mkdirSync(dest, { recursive: true });
  fs.cpSync(SRC, dest, { recursive: true, filter: (src) => !isPrivateState(src) });
  return dest;
}

if (require.main === module) {
  const name = process.argv[2];
  if (!CLIENTS.includes(name)) {
    process.stderr.write(`e2e: prepare-fixtures.cjs needs one of ${CLIENTS.join(", ")} as its argument\n`);
    process.exit(1);
  }
  const dest = prepareFixtures(name);
  process.stderr.write(`e2e: ${name} fixtures copied to ${dest}\n`);
}

module.exports = { prepareFixtures, CLIENTS };
