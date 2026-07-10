#!/usr/bin/env node
"use strict";
// Copies the committed e2e fixtures into a scratch temp dir the real `alix`
// server can write into (progress.json / recent.json) without ever touching
// the repo. Idempotent: wipes and re-copies on every call, so each run starts
// from the exact same committed state.
//
// Called from two places, on purpose (belt and suspenders): Playwright's
// `globalSetup` (see global-setup.ts), and directly chained into the
// `webServer.command` shell command in playwright.config.ts. Playwright does
// not document whether globalSetup is guaranteed to finish before the
// webServer process starts, so the webServer command also runs this
// synchronously (as a plain script, before `cargo run`) to guarantee the
// decks directory exists the moment the server binary checks for it.

const fs = require("node:fs");
const path = require("node:path");

const ROOT = __dirname;
const SRC = path.join(ROOT, "fixtures", "decks");
const TMP = path.join(ROOT, ".tmp");
const DEST = path.join(TMP, "decks");

// A progress store is per-run state, never a fixture: the suite must start
// every run from a deck with NO store, so it exercises the never-seen
// (acquire) path a real kid's first session hits. Even though the committed
// fixtures never include one (enforced by .gitignore), skip any that show up
// anyway (e.g. a contributor's local stray file) rather than silently
// carrying it into the run.
function isStoreFile(src) {
  const base = path.basename(src);
  return base === "progress.json" || base === "recent.json";
}

function prepareFixtures() {
  fs.rmSync(TMP, { recursive: true, force: true });
  fs.mkdirSync(DEST, { recursive: true });
  fs.cpSync(SRC, DEST, { recursive: true, filter: (src) => !isStoreFile(src) });
  return DEST;
}

if (require.main === module) {
  const dest = prepareFixtures();
  process.stderr.write(`e2e: fixtures copied to ${dest}\n`);
}

module.exports = { prepareFixtures, DEST };
