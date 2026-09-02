#!/usr/bin/env node
"use strict";
/*
 * Landing-page carousel screenshot capture — alix.study.
 *
 * Standalone. NOT part of `make e2e` / CI: it lives beside the e2e suite but
 * has its own entry point (this file) and is never picked up by
 * `playwright test` (testDir there is `./tests`, this lives in `./shots`,
 * and this is a plain Node script, not a `*.spec.ts`). Run it manually:
 *
 *   node e2e/shots/capture.cjs [--fresh] [--only=1,2,3,...]
 *
 * Requires `cwebp` (Debian/Ubuntu: `apt install webp`; macOS:
 * `brew install webp`). Playwright captures a temporary PNG; this script
 * encodes it as lossless WebP and keeps only the WebP in site/img/.
 *
 * See docs/product/2026-07-01-web-screenshots.md for the shot list and the
 * conventions this follows (viewport, theme-per-shot, captions).
 *
 * Safety: this NEVER serves or writes into ~/alix-demo or ~/alix-kids
 * directly. Both are copied into e2e/shots/.tmp/ once (reused on later runs
 * unless --fresh) and only the copies are served/graded/augmented.
 */
const { execFileSync, spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const {
  runRequested,
  summarize,
  unknownRequests,
  exitCodeFor,
} = require("./runner.cjs");

const REPO_ROOT = path.join(__dirname, "..", "..");
const SHOTS_DIR = __dirname;
const WORK = path.join(SHOTS_DIR, ".tmp");
const DEMO_SRC = path.join(os.homedir(), "alix-demo");
const KIDS_SRC = path.join(os.homedir(), "alix-kids");
const DEMO_DIR = path.join(WORK, "demo");
const KIDS_DIR = path.join(WORK, "kids");
const KIDS_CONFIG = path.join(SHOTS_DIR, "kids.toml");
const DEMO_CONFIG = path.join(SHOTS_DIR, "demo.toml");
const OUT_DIR = path.join(REPO_ROOT, "site", "img");

const DEMO_PORT = 7801;
const KIDS_PORT = 7802;
const DEMO_BASE = `http://127.0.0.1:${DEMO_PORT}`;
const KIDS_BASE = `http://127.0.0.1:${KIDS_PORT}`;

// The one demo source (spec: "one demo deck for every shot"): the rust-book
// workspace. The hero fact deck (01) is the entry point of its `requires:`
// chain, pre-augmented (choices/notes/keypoints/topology) by ensureAugmented().
const HERO_DECK = "what-is-ownership.md";
const HERO_FILE = path.join(DEMO_DIR, "what-is-ownership.md");
const TRACE_DECK = "workspace-showcase/ownership-move.md"; // the shipped example trace (docs/examples), copied into the demo dir
// runAugment("topology") passes no `--with`, and the unguided auto-name is
// pinned by the lib's generate_topology_names_it_pedagogical_order_when_unguided
// test — this must match, or shot 8's topology-scoped /api/select refuses.
const TOPOLOGY_NAME = "pedagogical order";

// User ruling 2026-07-11: one theme across every shot (the house default —
// see web/shared/theme.js's `DEFAULT`/THEMES[0], id "dark", name "alix") —
// not the spec's original per-shot variety. Shot 9 is the one place theme
// variety still shows (the popover's own swatch grid), so it's the only shot
// that touches the theme mechanism beyond this default.
const DEFAULT_THEME = "dark";

const VIEWPORT = { width: 1440, height: 900 };
const SCALE = 2;

const argv = process.argv.slice(2);
const FRESH = argv.includes("--fresh");
const onlyArg = argv.find((a) => a.startsWith("--only="));
const ONLY = onlyArg ? new Set(onlyArg.split("=")[1].split(",").map(Number)) : null;
const wants = (n) => !ONLY || ONLY.has(n);

function log(...a) {
  console.log("[shots]", ...a);
}
function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function requireWebpEncoder() {
  try {
    execFileSync("cwebp", ["-version"], { stdio: "ignore" });
  } catch {
    throw new Error(
      "cwebp is required to capture site screenshots (Debian/Ubuntu: apt install webp; macOS: brew install webp)",
    );
  }
}

// ---- fixtures: copy once, never touch the originals -----------------------

function copyOnce(src, dest, label) {
  if (fs.existsSync(dest) && !FRESH) {
    log(`reusing existing ${label} copy at`, dest);
    return;
  }
  if (!fs.existsSync(src)) {
    throw new Error(`${label} source not found: ${src}`);
  }
  fs.rmSync(dest, { recursive: true, force: true });
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  fs.cpSync(src, dest, { recursive: true });
  log(`copied ${label}:`, src, "->", dest);
}

// A snapshot of every progress/recent file's mtime+size under a decks root —
// compared before/after against the REAL ~/alix-demo and ~/alix-kids to prove
// this script never wrote into them.
function snapshotStoreFiles(root) {
  const out = {};
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (/(^|\/)progress\/[^/]+\.json$/.test(p) || entry.name === "recent.json") {
        const st = fs.statSync(p);
        out[p] = `${st.mtimeMs}:${st.size}`;
      }
    }
  };
  walk(root);
  return out;
}

function diffSnapshots(before, after) {
  const changed = [];
  for (const k of new Set([...Object.keys(before), ...Object.keys(after)])) {
    if (before[k] !== after[k]) changed.push(k);
  }
  return changed;
}

// ---- deck parsing (CommonMark card format) --------------------------------

function parseDeck(file) {
  const lines = fs.readFileSync(file, "utf8").split("\n");
  const cards = [];
  let cur = null;
  for (const raw of lines) {
    const line = raw.trim();
    if (line.startsWith("## ")) {
      cur = { front: line.slice(3).trim(), back: [] };
      cards.push(cur);
    } else if (!line || line.startsWith(">") || line.startsWith("<!--") || line.startsWith("---")) {
      // machine directive / note / frontmatter / blank — skip
    } else if (cur) {
      cur.back.push(line);
    }
  }
  return cards;
}

// ---- augment cache (choices/notes/keypoints/topology on the hero deck) ----

function heroAugmentState() {
  const augPath = path.join(DEMO_DIR, "augment", `${deckId(HERO_FILE)}.json`);
  if (!fs.existsSync(augPath)) return { distractors: 0, note: 0, keypoints: 0, topology: false };
  const data = JSON.parse(fs.readFileSync(augPath, "utf8"));
  const cards = parseDeck(HERO_FILE);
  const heroIds = new Set(); // we don't have ids without the app; approximate by counting any card entries with each key
  let distractors = 0,
    note = 0,
    keypoints = 0;
  for (const v of Object.values(data.cards || {})) {
    if (v.distractors) distractors++;
    if (v.note) note++;
    if (v.keypoints) keypoints++;
  }
  const topology = Array.isArray(data.topologies) && data.topologies.length > 0;
  return { distractors, note, keypoints, topology, cardCount: cards.length };
}

function deckId(file) {
  const text = fs.readFileSync(file, "utf8");
  const match = text.match(/^id:\s*"?(deck-[^"\r\n]+)"?\s*$/m);
  if (!match) throw new Error(`no id in ${file}`);
  return match[1];
}

function runAugment(target) {
  log("augmenting hero deck --target", target, "(real Claude call, this can take a while)…");
  // The checkout build, never PATH's `alix`: the server reads the augment cache
  // with this checkout's fingerprint rule, and a PATH binary one commit behind
  // writes entries it then refuses as stale.
  execFileSync(buildAlix(), ["deck", "augment", HERO_FILE, "--target", target], {
    stdio: "inherit",
    cwd: REPO_ROOT,
  });
}

function ensureAugmented() {
  const state = heroAugmentState();
  log("hero deck augment cache:", state);
  // The hero deck has 10 cards; a handful (atomic answers) are skipped for
  // keypoints on purpose (augment.rs), so we only require "some" coverage,
  // not full coverage, before treating each target as already present.
  if (state.distractors < 5) runAugment("choices");
  if (state.note < 5) runAugment("notes");
  if (state.keypoints < 5) runAugment("keypoints");
  if (!state.topology) runAugment("order"); // the target that computes the topology
}

// ---- server lifecycle -------------------------------------------------

const children = [];

// A crashed prior run (this script, or an ad-hoc debug session) can leave an
// `alix` process bound to DEMO_PORT/KIDS_PORT. Without this, startServer()'s
// child fails fast ("Address already in use") but waitForServer() still
// happily finds *something* answering /api/version on that port — the
// leftover process — and the run silently proceeds against stale state
// instead of this run's fresh copy. Best-effort; fine to no-op if `fuser`
// isn't installed or nothing's listening.
function freePort(port) {
  try {
    execFileSync("fuser", ["-k", `${port}/tcp`], { stdio: "ignore" });
  } catch {
    // nothing was listening, or `fuser` isn't installed — either way, fine
  }
}

function waitForServer(base, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  return (async () => {
    while (Date.now() < deadline) {
      try {
        const res = await fetch(`${base}/api/version`);
        if (res.ok) return;
      } catch {
        // not up yet
      }
      await sleep(300);
    }
    throw new Error(`server at ${base} never came up`);
  })();
}

// Photograph the checkout, not whatever `alix` happens to be installed: an
// older binary treats a new frontmatter key as unknown and silently
// photographs the fallback.
let alixBinary = null;
function buildAlix() {
  if (!alixBinary) {
    log("building the checkout (cargo build --release)");
    execFileSync("cargo", ["build", "--release", "--quiet"], {
      cwd: REPO_ROOT,
      stdio: ["ignore", "inherit", "inherit"],
    });
    alixBinary = path.join(REPO_ROOT, "target", "release", "alix");
  }
  return alixBinary;
}

function startServer(dir, port, extraArgs = []) {
  const args = [dir, "--port", String(port), "--session", "20", ...extraArgs];
  log("starting: alix", args.join(" "));
  const child = spawn(buildAlix(), args, { cwd: REPO_ROOT, stdio: ["ignore", "pipe", "pipe"] });
  child.stdout.on("data", (d) => process.stdout.write(`[alix:${port}] ${d}`));
  child.stderr.on("data", (d) => process.stderr.write(`[alix:${port}] ${d}`));
  children.push(child);
  return child;
}

function stopAll() {
  for (const c of children) {
    try {
      c.kill("SIGTERM");
    } catch {
      // already gone
    }
  }
}
process.on("exit", stopAll);
for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, () => {
    stopAll();
    process.exit(1);
  });
}

// ---- tiny JSON API client (talks straight to the running alix server) -----

async function api(base, method, urlPath, body, revision) {
  const headers = {};
  if (body !== undefined) headers["content-type"] = "application/json";
  if (revision !== undefined) headers["x-alix-study-revision"] = String(revision);
  const res = await fetch(`${base}${urlPath}`, {
    method,
    headers: Object.keys(headers).length ? headers : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    throw new Error(`${method} ${urlPath} -> ${res.status}`);
  }
  const text = await res.text();
  return text ? JSON.parse(text) : null;
}

// ---- theme -----------------------------------------------------------

async function setTheme(page, base, id) {
  await page.goto(`${base}/`, { waitUntil: "domcontentloaded" });
  await page.evaluate((tid) => localStorage.setItem("alix-theme", tid), id);
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForTimeout(350);
}

// The header <alix-logo> (web/shared/alix-logo.js) is a custom element that
// plays a ~2.62s one-shot "birth" animation on every connect/reset (page
// load, theme reload, and — see review.html's updateBusy()/replayLogo() — it
// also replays after any AI call finishes, since the ask/exam poll loops
// toggle its `loop` attribute on and off). There's no class/attribute it
// flips when settled, but the component tracks its own progress internally
// (`_elapsed` vs. `restAt`, both plain instance properties — see
// alix-logo.js `_play()`/`get restAt()`), so poll those directly rather than
// guessing a fixed delay. Absent element (e.g. kids.html only creates one
// transiently, for its pre-load splash) resolves immediately — nothing to
// wait for.
async function settleLogo(page) {
  await page
    .waitForFunction(
      () => {
        const el = document.querySelector("alix-logo");
        if (!el) return true;
        if (typeof el._elapsed !== "number" || typeof el.restAt !== "number") return true;
        return el._elapsed >= el.restAt;
      },
      { timeout: 6000 },
    )
    .catch(() => log("WARNING: alix-logo did not settle within 6s — capturing anyway"));
}

// Wait for every currently-running CSS animation/transition on the page to
// finish (drawer/reveal/panel entrances: cardIn/deal/revealIn/DRAWER_MS, all
// ~0.2-0.35s per review.html's own comments) — covers anything a fixed sleep
// would guess at. Polled rather than a one-shot check: a finishing animation
// can itself trigger another (e.g. a re-render), so this settles only once
// nothing is running for one whole poll tick.
async function settleAnimations(page) {
  await page
    .waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"), { timeout: 3000 })
    .catch(() => log("WARNING: a CSS animation was still running after 3s — capturing anyway"));
}

// Every screenshot goes through here — settle the header logo's own
// requestAnimationFrame-driven "birth" animation (not a CSS animation, so
// settleAnimations() can't see it) and any CSS entrance transition, then a
// small buffer for the final paint, before the pixels are read.
// `ready` is a CSS selector unique to the screen this shot claims to show —
// required, not optional. Its own bug class (shot 4: the exam genuinely ran
// and finished server-side, but the screenshot was taken after a page
// *reload*, which only round-trips StateDto — exam/walk progress is
// client-side-only JS state, so the reload silently rendered the picker
// underneath instead) is exactly why this is enforced here, in one place,
// rather than left to each shot function to remember. A screen mismatch
// throws — never writes a WebP of the wrong screen; the caller's try/catch
// turns that into an honest SKIP in the summary.
const capturedThisRun = new Set();

// One table answers both what runs and what `--only` may name. The shot
// functions are declarations, so they hoist above this.
const SHOTS = [
  [1, "shot-1-verify.webp", shot1],
  [2, "shot-2-tutor.webp", shot2],
  [3, "shot-3-modes.webp", shot3],
  [4, "shot-4-exam.webp", shot4],
  [5, "shot-5-augment.webp", shot5],
  [6, "shot-6-trace.webp", shot6],
  [7, "shot-7-picker.webp", shot7],
  [8, "shot-8-topology.webp", shot8],
  [9, "shot-9-themes.webp", shot9],
  [10, "shot-10-kids.webp", shot10],
];
Object.freeze(SHOTS);

async function shot(page, filename, ready) {
  if (path.extname(filename) !== ".webp") {
    throw new Error(`screenshot output must be .webp: ${filename}`);
  }
  await page
    .locator(ready)
    .first()
    .waitFor({ state: "visible", timeout: 10_000 });
  await settleLogo(page);
  await settleAnimations(page);
  await page.waitForTimeout(200);
  const out = path.join(OUT_DIR, filename);
  const stem = path.basename(filename, ".webp");
  const png = path.join(WORK, `${stem}.png`);
  const webp = path.join(WORK, filename);
  try {
    await page.screenshot({ path: png, type: "png" });
    execFileSync("cwebp", ["-quiet", "-lossless", "-z", "9", png, "-o", webp]);
    fs.renameSync(webp, out);
    capturedThisRun.add(filename);
  } finally {
    fs.rmSync(png, { force: true });
    fs.rmSync(webp, { force: true });
  }
  log("wrote", path.relative(REPO_ROOT, out));
}

// ---- setup: establish real Recall schedules on the hero deck's 10 cards ---
// This single batch feeds shots (1) explain/keypoints, which needs a card
// established at Recall so Reconstruct is immediately due, and (8), whose
// topology heatmap reads Recall retrievability. demo.toml zeroes the introduction
// cooldown so phase 2 can grade right after introducing.
async function establishHeroSchedules(page) {
  log("introducing all hero-deck cards (phase 1/2)…");
  let s = await api(DEMO_BASE, "POST", "/api/select", { deck: HERO_DECK, depth: "recall", session: 20 });
  let guard = 0;
  while (s && s.kind === "review" && s.phase === "review" && guard++ < 20) {
    if (s.introducing) {
      s = await api(DEMO_BASE, "POST", "/api/introduce", {}, s.study_revision);
    } else {
      break;
    }
  }
  log("grading all hero-deck cards at Recall (phase 2/2)…");
  s = await api(DEMO_BASE, "POST", "/api/select", { deck: HERO_DECK, depth: "recall", session: 20 });
  guard = 0;
  const gradedIds = [];
  let idx = 0;
  while (s && s.kind === "review" && s.phase === "review" && guard++ < 20) {
    const front = s.card && s.card.front;
    // A representative red-to-green spread for the topology heatmap: mostly
    // pass, with a couple of misses/partials sprinkled in.
    const pattern = ["passed", "passed", "failed", "passed", "partly", "passed", "failed", "passed", "passed", "partly"];
    const grade = pattern[idx % pattern.length];
    idx++;
    gradedIds.push({ id: s.card && s.card.id, front, grade });
    s = await api(DEMO_BASE, "POST", "/api/grade", { grade }, s.study_revision);
  }
  log(
    "graded",
    gradedIds.length,
    "cards:",
    gradedIds.map((c) => `${c.front} -> ${c.grade}`).join("; "),
  );

  // Stagger review recency so the topology heatmap shows a genuine red->green
  // spread rather than "everything reviewed a second ago" (FSRS retrievability
  // is ~1.0 right at last_review_ms regardless of grade). This edits ONLY the
  // scratch copy's per-deck progress document, never the real ~/alix-demo, and only the
  // `last_review_ms`/`due_ms` timestamps, not the grades or history just
  // recorded for real above. The wire DTO carries no schedule timestamps,
  // so this reads the store file directly.
  backdateRecallReviews();

  return gradedIds;
}

function backdateRecallReviews() {
  const storePath = path.join(
    DEMO_DIR,
    "progress",
    `${deckId(HERO_FILE)}.json`,
  );
  if (!fs.existsSync(storePath)) {
    log("WARNING: no progress document to backdate at", storePath);
    return;
  }
  const store = JSON.parse(fs.readFileSync(storePath, "utf8"));
  const cards = store.cards || {};
  const ids = Object.keys(cards).filter((id) => cards[id] && cards[id].recall);
  const dayMs = 86_400_000;
  const spread = [0, 1, 2, 4, 6, 9, 12, 16, 20, 25]; // days back, one per card
  ids.forEach((id, i) => {
    const back = spread[i % spread.length] * dayMs;
    cards[id].recall.last_review_ms = Math.max(0, cards[id].recall.last_review_ms - back);
  });
  fs.writeFileSync(storePath, JSON.stringify(store, null, 2));
  log("backdated review recency on", ids.length, "cards for heatmap variety");
}

// ---- shot 1: explain-mode keypoints checklist -----------------------------

async function shot1(page, out) {
  log("== shot 1: explain-mode keypoints ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  let s = await api(DEMO_BASE, "POST", "/api/select", { deck: HERO_DECK, depth: "reconstruct", session: 20 });
  let guard = 0;
  // Skip cards without a cached keypoints list (a couple of atomic answers
  // were deliberately skipped by `alix deck augment --target keypoints`);
  // the guard covers introduce+grade for every card of the grown deck.
  while (s && s.kind === "review" && s.phase === "review" && guard++ < 40) {
    const hasKp = Array.isArray(s.keypoints) && s.keypoints.length > 0;
    if (hasKp) break;
    if (s.introducing) s = await api(DEMO_BASE, "POST", "/api/introduce", {}, s.study_revision);
    else s = await api(DEMO_BASE, "POST", "/api/grade", { grade: "passed" }, s.study_revision);
  }
  if (!s || !Array.isArray(s.keypoints) || s.keypoints.length === 0) {
    log("FAILED shot 1: no reconstruct-depth card with cached keypoints was reachable");
    return false;
  }
  log("shot 1 card:", s.card.front, "-", s.keypoints.length, "keypoints");
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  // The typed target is the gradeable `line` steps only: a quotation is answer
  // content that is never typed, so joining raw `back` would type its `>`.
  const answer = (s.card.answer_steps || [])
    .filter((step) => step.kind === "line")
    .flatMap((step) => (s.card.back || []).slice(step.back_from, step.back_to))
    .join(" ");
  await page.locator(".explain-input").fill(answer);
  await page.locator(".explain-input").press("Shift+Enter");
  await page.waitForTimeout(400);
  // Mark every point via the legend's "Yes" chip (`answerKeypoint`), not by
  // clicking the `.kp-list li.pt` items directly: on a cited card (this deck's
  // cards all carry `at:` directives) the answer region ALSO has its own onclick
  // (source<->answer swap, review.html's `onCiteClick`), and a keypoint <li>
  // click bubbles into it — the first click silently swaps the whole panel to
  // the citation excerpt instead of marking the point. The legend chip lives
  // outside that region, so it doesn't bubble into it.
  const n = await page.locator(".kp-list li.pt").count();
  const yesBtn = page.getByRole("button", { name: "Yes" });
  for (let i = 0; i < n; i++) {
    await yesBtn.click();
    await page.waitForTimeout(120);
  }
  await page.waitForTimeout(300);
  await shot(page, out, ".kp-list");
  return true;
}

// ---- shot 2: ask-tutor panel (real Claude call) ---------------------------

async function shot2(page, out) {
  log("== shot 2: ask-tutor ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  // A graduated scratch store serves review cards immediately (cram), instead
  // of trapping the fresh hero in its introduction loop.
  fabricateGraduation();
  // cram:true: on a re-run against a reused scratch copy, every Recall card
  // may already be graded and not due again for days — without it, /api/select
  // can come back with nothing current to review (a disabled/absent primary
  // chip, per shot 2's own earlier failure).
  let s = await api(DEMO_BASE, "POST", "/api/select", { deck: HERO_DECK, depth: "recall", session: 20, cram: true });
  let guard = 0;
  while (s && s.kind === "review" && s.phase === "review" && s.introducing && guard++ < 15) {
    s = await api(DEMO_BASE, "POST", "/api/introduce", {}, s.study_revision);
  }
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  // Reveal the card first: Ask tutor is withheld while `!fullyRevealed()`, and
  // an answer can take several steps (one per gradeable line, one per quotation
  // or table block), so drive the primary chip until the ask chip appears
  // rather than clicking it once. Checking for the chip BEFORE each click is
  // what keeps the loop from clicking a grade chip once the reveal is done.
  const askChip = page.locator(".chip.ask");
  for (let step = 0; step < 12 && !(await askChip.count()); step++) {
    const revealBtn = page.locator(".chip.primary");
    if (!(await revealBtn.count())) break;
    await revealBtn.first().click();
    await page.waitForTimeout(200);
  }
  if (!(await askChip.count())) {
    log("FAILED shot 2: no .chip.ask visible after revealing every answer step");
    return false;
  }
  await askChip.first().click();
  await page.waitForTimeout(300);
  // The "Send" chip lives in the shared #legend footer, not inside
  // .ask-panel (renderAsk() appends it via legend.appendChild, a sibling of
  // the panel) — Shift+Enter on the textarea (the same keydown handler) is
  // simpler and matches how a real user sends it anyway.
  await page.locator(".ask-input").fill("Why is pushing to the stack faster than allocating on the heap?");
  await page.locator(".ask-input").press("Shift+Enter");
  log("waiting for the real tutor response (up to 120s)…");
  // Wait for an actual answer, not just ".ask-thinking" hidden: that div is
  // rebuilt on every ~400ms poll tick (fillAskLog), so it can be transiently
  // absent from the DOM between polls even while still thinking — a shot
  // landing in that gap captured a bare "Thinking…" panel with no answer.
  await page
    .locator(".ask-a")
    .first()
    .waitFor({ state: "visible", timeout: 120_000 })
    .catch(() => log("WARNING: no .ask-a appeared within 120s — capturing current state anyway"));
  await page.waitForTimeout(400);
  await shot(page, out, ".ask-a");
  return true;
}

// ---- shot 3: multiple-choice with real AI distractors ---------------------

async function shot3(page, out) {
  log("== shot 3: multiple-choice ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  // Recognize is unscheduled/boolean — no introduction cooldown, so this is
  // reachable immediately, even on a totally fresh card. cram:true covers a
  // re-run where every card is already past its first Recognize pass.
  let s = await api(DEMO_BASE, "POST", "/api/select", {
    deck: HERO_DECK,
    depth: "recognize",
    session: 20,
    cram: true,
  });
  let guard = 0;
  while (s && s.kind === "review" && s.phase === "review" && guard++ < 15) {
    const hasChoices = Array.isArray(s.choices) && s.choices.length > 1;
    if (hasChoices) break;
    s = await api(DEMO_BASE, "POST", "/api/skip", {}, s.study_revision);
  }
  if (!s || !Array.isArray(s.choices) || s.choices.length < 2) {
    log("FAILED shot 3: no Recognize card with multiple choices was reachable");
    return false;
  }
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  await shot(page, out, ".options .option");
  return true;
}

// ---- shot 4: AI exam (real Claude generate + grade calls) -----------------

function bestAnswer(corpus, question) {
  const norm = (t) =>
    t
      .toLowerCase()
      .replace(/[^a-z0-9 ]/g, " ")
      .split(/\s+/)
      .filter((w) => w.length > 2);
  const qWords = new Set(norm(question));
  let best = null,
    bestScore = -1;
  for (const c of corpus) {
    const words = norm(c.front + " " + c.back.join(" "));
    const score = words.filter((w) => qWords.has(w)).length;
    if (score > bestScore) {
      bestScore = score;
      best = c;
    }
  }
  return best ? best.back.join(" ") : "";
}

// Exam progress is CLIENT-side JS state in review.html (`examData`, a plain
// top-level `let`) with NO server-side reload-resume — unlike a review
// session's StateDto, `GET /api/state` doesn't carry it. A first version of
// this shot drove the exam via raw node-side fetch + a final `page.goto` to
// render the result, and it silently screenshotted the PICKER: the server
// had genuinely finished (confirmed separately, by polling /api/exam
// directly), but the reload never told the *browser* about it. So this
// drives the exam through the page's own functions instead — reachable
// directly since they're plain identifiers in review.html's classic
// (non-module) <script>, exactly like `state`/`examData` are for reading.
function pollExam(page) {
  return page.evaluate(() => {
    if (typeof examData === "undefined" || !examData) return null;
    const { phase, thinking, current, total, question, on_last, error, passed } = examData;
    return { phase, thinking, current, total, question, on_last, error, passed };
  });
}

// The exam's picker chip needs a graduated deck. Fabricate that
// PRECONDITION in the scratch store (never the real demo dirs), template
// matching the store's persisted document schema; the exam itself then runs
// for real through the page. `alix stats --store` is the loud schema check:
// the real binary must parse the fabricated document before any shot uses it.
function fabricateGraduation() {
  const text = fs.readFileSync(HERO_FILE, "utf8");
  const hero = deckId(HERO_FILE);
  // Session cards for a blank card are the per-span sub-ids (`-b<stamp>`,
  // the stamp minted into each `<!-- blank: ... b:x -->` line), not the base
  // token; the store tolerates unmatched extras as orphans. The stamp scan
  // mirrors the whitespace-token grammar of src/parser/region.rs::tokens: a
  // quoted run stays inside its token, and only a whole token starting with
  // `b:` is the stamp.
  function blankStamp(comment) {
    const body = comment.slice("<!-- blank:".length, -"-->".length);
    let quoted = false;
    let token = "";
    let stamp = null;
    const flush = () => {
      if (stamp === null && token.startsWith("b:")) stamp = token.slice(2);
      token = "";
    };
    for (let i = 0; i < body.length; i++) {
      const ch = body[i];
      if (quoted) {
        if (ch === "\\") token += body[++i] ?? "";
        else if (ch === '"') quoted = false;
        else token += ch;
        continue;
      }
      if (ch === '"') quoted = true;
      // \s plus NEXT LINE minus BOM equals Rust's char::is_whitespace.
      else if (ch === "\u0085" || (/\s/.test(ch) && ch !== "\uFEFF")) flush();
      else token += ch;
    }
    flush();
    return stamp;
  }
  const ids = [];
  for (const block of text.split(/\n## /).slice(1)) {
    const m = block.match(/<!-- id: (card-[a-z0-9]+) -->/);
    if (!m) continue;
    ids.push(m[1]);
    for (const cm of block.matchAll(/<!-- blank:[\s\S]*?-->/g)) {
      const stamp = blankStamp(cm[0]);
      if (stamp) ids.push(`${m[1]}-b${stamp}`);
    }
  }
  if (!ids.length) throw new Error(`no card ids in ${HERO_FILE}`);
  const now = Date.now();
  const day = 86_400_000;
  const doc = {
    version: 1,
    deck_id: hero,
    subject: path.basename(HERO_FILE),
    revision: 1,
    cards: Object.fromEntries(
      ids.map((id) => [
        id,
        {
          introduced_ms: now - 30 * day,
          recall: {
            stability: 30.0,
            difficulty: 5.0,
            reps: 6,
            lapses: 0,
            state: 2,
            scheduled_days: 30,
            last_review_ms: now - day,
            due_ms: now + 29 * day,
            learning_goods: 3,
          },
          total_reviews: 6,
          total_passes: 6,
          streak: 6,
        },
      ]),
    ),
    deck: { last_depth: "recall" },
    writer: { device: "shots", at_ms: now },
  };
  const progressDir = path.join(DEMO_DIR, "progress");
  fs.mkdirSync(progressDir, { recursive: true });
  fs.writeFileSync(path.join(progressDir, `${hero}.json`), JSON.stringify(doc, null, 2));
  execFileSync(buildAlix(), ["stats", HERO_FILE, "--store", DEMO_DIR], { stdio: "pipe" });
  log("fabricated a graduated store for", hero);
}

async function shot4(page, out) {
  log("== shot 4: AI exam ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  const corpus = parseDeck(HERO_FILE);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  fabricateGraduation();
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(600);

  // Drive the page the way a user reaches an exam: focus the graduated deck's
  // row; its primary chip reads "Take exam" (picker.js examPrimary).
  const row = page.locator(".deckrow", { hasText: "what-is-ownership" });
  if (!(await row.count())) {
    log("FAILED shot 4: hero row not listed");
    return false;
  }
  await row.first().click();
  await page.waitForTimeout(300);
  const chip = page.locator(".chip.primary", { hasText: "Take exam" });
  if (!(await chip.count())) {
    log("FAILED shot 4: no Take exam chip (deck not exam-due?)");
    return false;
  }
  await chip.first().click();

  // Question generation is genuinely slow (a real model call); err long.
  log("waiting for the exam's first question (up to 240s)…");
  await page.locator(".exam-input").waitFor({ state: "visible", timeout: 240_000 });

  for (let guard = 0; guard < 20; guard++) {
    const progress = (await page.locator(".exam-progress").textContent()) || "";
    const question = (await page.locator(".exam-q").textContent()) || "";
    const [, at, total] = progress.match(/(\d+)\s*\/\s*(\d+)/) || [];
    const answer = bestAnswer(corpus, question);
    log(`exam Q${at}/${total}: ${question.slice(0, 90)}`);
    await page.locator(".exam-input").fill(answer);
    await page.locator(".exam-input").press("Shift+Enter");
    if (at === total) {
      // Last answer submits the whole exam; real grading takes a while.
      log("submitted — waiting for grading (up to 240s)…");
      break;
    }
    // Otherwise Shift+Enter advanced to the next question.
    await page
      .locator(".exam-progress", { hasText: `Question ${Number(at) + 1}` })
      .waitFor({ state: "visible", timeout: 30_000 });
  }

  const verdict = page.locator(".exam-pass, .exam-fail");
  const arrived = await verdict
    .waitFor({ state: "visible", timeout: 240_000 })
    .then(() => true)
    .catch(() => false);
  if (!arrived) {
    log("FAILED shot 4: exam did not reach results");
    return false;
  }
  await page.waitForTimeout(400);
  await shot(page, out, ".exam-pass, .exam-fail");
  return true;
}

// ---- shot 5: augment screen -----------------------------------------------

async function shot5(page, out) {
  log("== shot 5: augment screen ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  const workspaceRow = page.locator(".deckrow").filter({ hasText: "Rust book" }).first();
  await workspaceRow.click();
  await page.waitForTimeout(400);
  const heroRow = page.locator(".deckrow").filter({ hasText: "Stack" }).first();
  await heroRow.click();
  await page.waitForTimeout(250);
  const augBtn = page.getByRole("button", { name: "Augment" });
  if (!(await augBtn.count())) {
    log("FAILED shot 5: no Augment chip visible for the focused deck");
    return false;
  }
  await augBtn.first().click();
  await page.waitForTimeout(400);
  await shot(page, out, ".aug-card");
  return true;
}

// ---- shot 6: trace walk checkpoint -----------------------------------------

async function shot6(page, out) {
  log("== shot 6: trace walk ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  // A walk isn't resumable across a hard reload the way a review session is
  // (GET /api/state only round-trips StateDto) — launch it through the real
  // picker click flow instead, same as a user would.
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  const workspaceRow = page.locator(".deckrow").filter({ hasText: "Workspace Showcase" }).first();
  await workspaceRow.click();
  await page.waitForTimeout(400);
  const traceRow = page.locator(".deckrow").filter({ hasText: "Rust ownership moves" }).first();
  if (!(await traceRow.count())) {
    log("FAILED shot 6: trace deck row not found in the drilled workspace");
    return false;
  }
  await traceRow.click();
  await page.waitForTimeout(250);
  const learnBtn = page.getByRole("button", { name: /^Learn/ });
  if (await learnBtn.count()) await learnBtn.first().click();
  else await traceRow.press("Enter");
  await page.waitForTimeout(500);
  const field = page.locator(".wfield");
  if (await field.count()) {
    await field.fill("It moves: s1 is invalidated and s2 becomes the sole owner of the heap data.");
    await field.press("Shift+Enter");
    await page.waitForTimeout(400);
  }
  if (!(await page.locator(".source-excerpt").count())) {
    log("FAILED shot 6: no .source-excerpt rendered — walk did not reach the reveal phase");
    return false;
  }
  await shot(page, out, ".source-excerpt");
  return true;
}

// ---- shot 7: picker, workspace expanded ------------------------------------

async function shot7(page, out) {
  log("== shot 7: picker ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  const workspaceRow = page.locator(".deckrow").filter({ hasText: "Rust" }).first();
  await workspaceRow.click();
  await page.waitForTimeout(400);
  // .tree: the dependency-tree branch-line guides only render once drilled
  // INTO the workspace — confirms this isn't still the collapsed list.
  await shot(page, out, ".deckrow .tree");
  return true;
}

// ---- shot 8: topology heatmap in review ------------------------------------

async function shot8(page, out) {
  log("== shot 8: topology heatmap ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  // Cram: the sitting must open in review phase for the crumb strip to
  // render, and after the graduation pass the due set depends on FSRS
  // intervals racing the backdate spread; the heatmap tiers read the store
  // either way.
  const s = await api(DEMO_BASE, "POST", "/api/select", {
    deck: HERO_DECK,
    topology: TOPOLOGY_NAME,
    depth: "recall",
    session: 20,
    cram: true,
  });
  if (!s || s.kind !== "review") {
    log("FAILED shot 8: topology-scoped select did not return a review session");
    return false;
  }
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  const crumb = page.locator("#crumbStrip");
  if (!(await crumb.count())) {
    log("FAILED shot 8: no #crumbStrip rendered for this session");
    return false;
  }
  await shot(page, out, "#crumbStrip");
  return true;
}

// ---- shot 9: theme gallery --------------------------------------------------

async function shot9(page, out) {
  log("== shot 9: theme gallery ==");
  // User ruling: every OTHER shot stays on DEFAULT_THEME — this is the one
  // place theme variety still shows, via the popover's own swatch grid, not
  // by committing a different theme to the app. Leave the default active.
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  // #theme-open lives inside the ☰ menu (role="menuitem") — open that first.
  await page.locator("#kebab").click();
  await page.waitForTimeout(200);
  await page.locator("#theme-open").click();
  await page.waitForTimeout(250);
  await shot(page, out, ".theme-panel.show");
  return true;
}

// ---- shot 10: kids client ---------------------------------------------------

async function shot10(page, out) {
  log("== shot 10: kids client ==");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(`${KIDS_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(500);
  const box = page.locator(".box").first();
  if (!(await box.count())) {
    log("FAILED shot 10: no .box on the kids home screen");
    return false;
  }
  await box.click();
  await page.waitForTimeout(400);
  // Not just .first(): the Animals box's alphabetically-first deck
  // (life-cycles.md) is an ORDERED-SEQUENCE answer (Egg/Caterpillar/
  // Chrysalis/Butterfly) that Recognize can't build real MC distractors for
  // (`choices` comes back null, an honest fallback — see isRecognizeFallback
  // in review.html), so it renders a reveal prompt, not tap options.
  // wild-animals.md has authored distractors on its first card.
  let deckRow = page.locator(".deck-row", { hasText: "wild-animals" }).first();
  if (!(await deckRow.count())) deckRow = page.locator(".deck-row").first();
  if (!(await deckRow.count())) {
    log("FAILED shot 10: no .deck-row inside the box");
    return false;
  }
  await deckRow.click();
  await page.waitForTimeout(300);
  const tapBtn = page.getByRole("button", { name: "Tap the answer" });
  if (await tapBtn.count()) {
    // The depth button renders disabled (`.caught-up`) when the deck carries no
    // recognizable cards, which is honest and not a UI fault. Clicking it anyway
    // spends 30s in Playwright's retry loop and reports a click timeout instead
    // of the reason.
    if (!(await tapBtn.first().isEnabled())) {
      log("FAILED shot 10: the Tap the answer depth button is disabled for this deck");
      return false;
    }
    await tapBtn.click();
    await page.waitForTimeout(400);
  }
  const opts = page.locator(".opt-btn");
  if (!(await opts.count())) {
    log("FAILED shot 10: no .opt-btn tap-the-answer options rendered");
    return false;
  }
  // Pick the correct option so the "Got it!" + mascot state shows. The
  // answer comes from the deck file itself (the module frontend exposes no
  // page globals): the first authored `- [x]` line is the served card's
  // correct choice.
  const deckText = fs.readFileSync(
    path.join(KIDS_DIR, "animals", "decks", "wild-animals.md"),
    "utf8",
  );
  const correctText = (deckText.match(/^- \[x\] (.+)$/m) || [])[1]?.trim() || null;
  let target = opts.first();
  if (correctText) {
    const byText = page.locator(".opt-btn", { hasText: correctText });
    if (await byText.count()) target = byText.first();
  }
  await target.click();
  await page.waitForTimeout(500);
  await shot(page, out, ".opt-correct");
  return true;
}

// ---- main ------------------------------------------------------------------

async function main() {
  const unknown = unknownRequests(SHOTS, ONLY);
  if (unknown.length) {
    console.error(`[shots] --only named no such shot: ${unknown.join(", ")}`);
    process.exitCode = exitCodeFor({ unknown });
    return;
  }
  requireWebpEncoder();
  fs.mkdirSync(OUT_DIR, { recursive: true });
  fs.mkdirSync(WORK, { recursive: true });

  const beforeDemo = snapshotStoreFiles(DEMO_SRC);
  const beforeKids = snapshotStoreFiles(KIDS_SRC);

  copyOnce(DEMO_SRC, DEMO_DIR, "alix-demo");
  copyOnce(KIDS_SRC, KIDS_DIR, "alix-kids");
  // The scratch never inherits the source's progress (the real demo dir may
  // carry stale-format documents; shots fabricate the state they need).
  fs.rmSync(path.join(DEMO_DIR, "progress"), { recursive: true, force: true });
  fs.rmSync(path.join(DEMO_DIR, "recent.json"), { force: true });
  // The showcase workspace comes from the repo, never from the personal copy.
  // ~/alix-demo's is an older vintage with no `title:` and a `#` body heading
  // that is section context now, so the catalog labels its row from the first
  // card and shot 6's text match cannot find it.
  const showcaseDest = path.join(DEMO_DIR, "workspace-showcase");
  fs.rmSync(showcaseDest, { recursive: true, force: true });
  fs.cpSync(path.join(REPO_ROOT, "docs", "examples", "workspace-showcase"), showcaseDest, {
    recursive: true,
  });

  if (wants(1) || wants(2) || wants(3) || wants(8)) ensureAugmented();

  freePort(DEMO_PORT);
  freePort(KIDS_PORT);
  startServer(DEMO_DIR, DEMO_PORT, ["--config", DEMO_CONFIG]);
  startServer(KIDS_DIR, KIDS_PORT, ["--config", KIDS_CONFIG]);
  await waitForServer(DEMO_BASE);
  await waitForServer(KIDS_BASE);

  const { chromium } = require("@playwright/test");
  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: SCALE });
  const page = await context.newPage();

  const results = {};
  try {
    // Cheap setup that several shots depend on, done once up front regardless
    // of --only so re-running a single shot later stays fast.
    if (wants(1) || wants(8)) {
      await establishHeroSchedules(page);
    }

    Object.assign(
      results,
      await runRequested(SHOTS, wants, page, capturedThisRun),
    );
  } finally {
    await browser.close();
    stopAll();
  }

  await sleep(500);
  const afterDemo = snapshotStoreFiles(DEMO_SRC);
  const afterKids = snapshotStoreFiles(KIDS_SRC);
  const demoChanged = diffSnapshots(beforeDemo, afterDemo);
  const kidsChanged = diffSnapshots(beforeKids, afterKids);

  log("=== summary ===");
  const { lines, failed } = summarize(results);
  for (const line of lines) log(line);
  log("~/alix-demo files changed:", demoChanged.length ? demoChanged : "none");
  log("~/alix-kids files changed:", kidsChanged.length ? kidsChanged : "none");
  if (demoChanged.length || kidsChanged.length) {
    console.error("[shots] WARNING: real demo/kids progress files changed — investigate before trusting this run");
  }
  if (failed.length) {
    console.error(`[shots] requested shots that did not capture: ${failed.join(", ")}`);
  }
  process.exitCode = exitCodeFor({ failed, demoChanged, kidsChanged });
}

module.exports = { SHOTS };

if (require.main === module) {
  main().catch((err) => {
    console.error(err);
    stopAll();
    process.exit(1);
  });
}
