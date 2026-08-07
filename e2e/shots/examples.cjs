#!/usr/bin/env node
"use strict";
/*
 * Example-deck screenshots: one image per deck in docs/examples/, showing
 * what that deck's syntax produces on screen.
 *
 * Standalone and manual, like its neighbour capture.cjs, and for the same
 * reasons: it needs a browser and `cwebp`, neither of which CI has. The CI
 * half is scripts/check-example-media.py, which proves every example has an
 * image and every image belongs to an example. It cannot prove an image is
 * CURRENT. Only re-running this and finding no diff proves that.
 *
 *   node e2e/shots/examples.cjs [--only=table,cloze]
 *
 * Unlike capture.cjs this serves the committed example decks themselves,
 * copied into a scratch directory so a capture never writes review state
 * into the repository.
 */
const { chromium } = require("@playwright/test");
const { execFileSync, spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const REPO_ROOT = path.join(__dirname, "..", "..");
const EXAMPLES = path.join(REPO_ROOT, "docs", "examples");
const WORK = path.join(__dirname, ".tmp-examples");
const PORT = 7899;

function log(...parts) {
  process.stderr.write(`${parts.join(" ")}\n`);
}

function requireCwebp() {
  try {
    execFileSync("cwebp", ["-version"], { stdio: "ignore" });
  } catch {
    throw new Error(
      "cwebp is required (Debian/Ubuntu: apt install webp; macOS: brew install webp)",
    );
  }
}

/// Every example deck, as {set, name, source}. The sets are the two the
/// examples README describes; a new set is picked up without editing this.
function examples() {
  const found = [];
  for (const set of ["shapes", "syntax"]) {
    const dir = path.join(EXAMPLES, set);
    if (!fs.existsSync(dir)) continue;
    for (const file of fs.readdirSync(dir).sort()) {
      if (file.endsWith(".md")) {
        found.push({ set, name: path.basename(file, ".md"), source: path.join(dir, file) });
      }
    }
  }
  return found;
}

async function waitForServer(base) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      const response = await fetch(`${base}/api/version`);
      if (response.ok) return;
    } catch {
      // not up yet
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`server at ${base} never came up`);
}

async function main() {
  requireCwebp();
  const only = (process.argv.find((a) => a.startsWith("--only=")) || "").slice(7);
  const wanted = only ? new Set(only.split(",")) : null;
  const decks = examples().filter((d) => !wanted || wanted.has(d.name));
  if (decks.length === 0) throw new Error("no example decks matched");

  // Serve copies, never the repository: a review writes progress beside the
  // deck it drills, and an example deck must stay exactly as committed.
  fs.rmSync(WORK, { recursive: true, force: true });
  fs.mkdirSync(WORK, { recursive: true });
  for (const deck of decks) {
    fs.copyFileSync(deck.source, path.join(WORK, `${deck.name}.md`));
  }

  const server = spawn("alix", [WORK, "--port", String(PORT), "--session", "20"], {
    cwd: REPO_ROOT,
    stdio: ["ignore", "pipe", "pipe"],
  });
  server.stdout.on("data", (d) => process.stderr.write(`[alix] ${d}`));
  const stop = () => {
    try {
      server.kill("SIGTERM");
    } catch {
      // already gone
    }
  };
  process.on("exit", stop);

  const base = `http://127.0.0.1:${PORT}`;
  await waitForServer(base);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1000, height: 720 } });

  try {
    for (const deck of decks) {
      const response = await fetch(`${base}/api/select`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ deck: `${deck.name}.md` }),
      });
      if (!response.ok) throw new Error(`select failed for ${deck.name}: ${response.status}`);
      await page.goto(base, { waitUntil: "networkidle" });
      await page.locator(".card").first().waitFor({ state: "visible", timeout: 10_000 });
      await page.waitForTimeout(300);

      const out = path.join(EXAMPLES, deck.set, `${deck.name}.webp`);
      const png = path.join(WORK, `${deck.name}.png`);
      try {
        await page.screenshot({ path: png, type: "png" });
        execFileSync("cwebp", ["-quiet", "-lossless", "-z", "9", png, "-o", out]);
      } finally {
        fs.rmSync(png, { force: true });
      }
      log("wrote", path.relative(REPO_ROOT, out));
    }
  } finally {
    await browser.close();
    stop();
  }
}

main().catch((error) => {
  log(`examples: ${error.message}`);
  process.exit(1);
});
